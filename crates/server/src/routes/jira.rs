use axum::{
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use services::services::jira_auth::JiraAuthService;
use services::services::secure_storage::{JiraCredentialManager, SecureStorageFactory};
use services::services::jira_auth::JiraResource;
use ts_rs::TS;
use utils::response::ApiResponse;

use crate::{error::ApiError, DeploymentImpl};

pub fn router(_deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    Router::new()
        .route("/jira/oauth/start", post(oauth_start))
        .route("/jira/oauth/callback", get(oauth_callback))
        .route("/jira/configs", get(get_jira_configs))
        .route("/jira/sites/accessible", get(get_accessible_sites))
        .route(
            "/jira/credentials",
            get(get_jira_credentials_status)
                .post(set_jira_credentials)
                .delete(delete_jira_credentials),
        )
        .route("/jira/connection/test/{cloudid}", post(test_connection))
}

#[derive(Serialize, Deserialize, TS)]
pub struct JiraOAuthStartResponse {
    pub authorization_url: String,
    pub state: String,
}

#[derive(Serialize, Deserialize, TS)]
pub struct JiraOAuthStartRequest {
    pub redirect_uri: Option<String>,
}

/// POST /api/jira/oauth/start
/// Start OAuth flow - returns authorization URL
async fn oauth_start(
    State(_deployment): State<DeploymentImpl>,
    axum::Json(req): axum::Json<JiraOAuthStartRequest>,
) -> Result<ResponseJson<ApiResponse<JiraOAuthStartResponse>>, ApiError> {
    // Load stored client credentials
    let storage = SecureStorageFactory::create().await;
    let manager = JiraCredentialManager::new(storage);
    let (client_id, client_secret) = manager
        .get_oauth_credentials()
        .await
        .map_err(|e| ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?
        .ok_or_else(|| ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, "Jira OAuth credentials not configured")))?;

    let redirect_uri = req
        .redirect_uri
        .unwrap_or_else(|| option_env!("JIRA_REDIRECT_URI").unwrap_or("http://localhost:3000/settings").to_string());
    tracing::debug!(target: "server", oauth_redirect = %redirect_uri, "[Jira] Starting OAuth");

    let jira_auth = JiraAuthService::with_credentials(client_id, client_secret, redirect_uri).await;
    let state = JiraAuthService::generate_state();
    let authorization_url = jira_auth
        .get_authorization_url(&state)
        .await
        .map_err(|e| {
            tracing::error!(target: "server", error = %e, "[Jira] Failed to build authorization URL");
            ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, "OAuth start failed"))
        })?;
    tracing::debug!(target: "server", auth_url = %authorization_url, state = %state, "[Jira] Authorization URL generated");

    Ok(ResponseJson(ApiResponse::success(JiraOAuthStartResponse {
        authorization_url,
        state,
    })))
}

#[derive(Serialize, Deserialize, TS)]
pub struct JiraOAuthCallbackQuery {
    pub code: String,
    pub state: String,
    pub redirect_uri: Option<String>,
}

/// GET /api/jira/oauth/callback?code=...&state=...
/// Handle OAuth callback: exchange code for tokens and stash in secure storage
async fn oauth_callback(
    State(_deployment): State<DeploymentImpl>,
    axum::extract::Query(query): axum::extract::Query<JiraOAuthCallbackQuery>,
) -> Result<ResponseJson<ApiResponse<String>>, ApiError> {
    // Recreate service with the same redirect URI used in start (prefer env default)
    let storage = SecureStorageFactory::create().await;
    let manager = JiraCredentialManager::new(storage);
    let (client_id, client_secret) = manager
        .get_oauth_credentials()
        .await
        .map_err(|e| ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?
        .ok_or_else(|| ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, "Jira OAuth credentials not configured")))?;
    let redirect_uri = query
        .redirect_uri
        .clone()
        .unwrap_or_else(|| option_env!("JIRA_REDIRECT_URI").unwrap_or("http://localhost:3000/settings").to_string());
    tracing::debug!(target: "server", code = %query.code, state = %query.state, oauth_redirect = %redirect_uri, "[Jira] Handling OAuth callback");
    let jira_auth = JiraAuthService::with_credentials(client_id, client_secret, redirect_uri).await;

    let tokens = match jira_auth
        .exchange_code_for_tokens(&query.code, &query.state)
        .await
    {
        Ok(t) => {
            tracing::debug!(target: "server", scope = %t.scope, expires_in = t.expires_in, "[Jira] Token exchange succeeded");
            t
        }
        Err(e) => {
            tracing::error!(target: "server", error = %e, "[Jira] Token exchange failed");
            return Ok(ResponseJson(ApiResponse::error(&format!(
                "OAuth exchange failed: {}",
                e
            ))))
        }
    };

    // Save global tokens (for accessible resources fetch)
    let storage = SecureStorageFactory::create().await;
    let manager = JiraCredentialManager::new(storage);
    if let Some(refresh) = tokens.refresh_token.as_ref() {
        if let Err(e) = manager.store_oauth_tokens(&tokens.access_token, refresh).await {
            return Ok(ResponseJson(ApiResponse::error(&format!(
                "Failed to store tokens: {}",
                e
            ))));
        }
    } else if let Err(e) = manager.store_oauth_tokens(&tokens.access_token, "").await {
        return Ok(ResponseJson(ApiResponse::error(&format!(
            "Failed to store tokens: {}",
            e
        ))));
    }

    Ok(ResponseJson(ApiResponse::success("OAuth completed".to_string())))
}

/// GET /api/jira/configs
/// Get all Jira configurations for the user
async fn get_jira_configs(
    State(_deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<String>>>, ApiError> {
    // TODO: Implement get configs
    Ok(ResponseJson(ApiResponse::success(vec![])))
}

#[derive(Serialize, Deserialize, TS)]
pub struct JiraCredentialsBody {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Serialize, Deserialize, TS)]
pub struct JiraCredentialsStatus {
    pub configured: bool,
}

/// GET /api/jira/credentials
/// Check whether OAuth client credentials are configured (does not return secrets)
async fn get_jira_credentials_status(
    State(_deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<JiraCredentialsStatus>>, ApiError> {
    let storage = SecureStorageFactory::create().await;
    let manager = JiraCredentialManager::new(storage);
    let creds = manager
        .get_oauth_credentials()
        .await
        .map_err(|e| ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;
    Ok(ResponseJson(ApiResponse::success(JiraCredentialsStatus {
        configured: creds.is_some(),
    })))
}

/// POST /api/jira/credentials
/// Store OAuth client credentials securely
async fn set_jira_credentials(
    State(_deployment): State<DeploymentImpl>,
    axum::Json(body): axum::Json<JiraCredentialsBody>,
) -> Result<ResponseJson<ApiResponse<String>>, ApiError> {
    let mut service = JiraAuthService::new().await
        .map_err(|_| ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, "Failed to initialize Jira auth service")))?;
    if let Err(e) = service
        .update_oauth_credentials(body.client_id, body.client_secret)
        .await
    {
        return Ok(ResponseJson(ApiResponse::error(&format!(
            "Failed to save credentials: {}",
            e
        ))));
    }
    Ok(ResponseJson(ApiResponse::success("Credentials saved".to_string())))
}

/// DELETE /api/jira/credentials
/// Remove stored OAuth client credentials
async fn delete_jira_credentials(
    State(_deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<String>>, ApiError> {
    let service = JiraAuthService::new().await
        .map_err(|_| ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, "Failed to initialize Jira auth service")))?;
    if let Err(e) = service.delete_oauth_credentials().await {
        return Ok(ResponseJson(ApiResponse::error(&format!(
            "Failed to clear credentials: {}",
            e
        ))));
    }
    Ok(ResponseJson(ApiResponse::success(
        "Credentials cleared".to_string(),
    )))
}

/// GET /api/jira/sites/accessible
/// Get accessible sites for current user
async fn get_accessible_sites(
    State(_deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<JiraResource>>>, ApiError> {
    let storage = SecureStorageFactory::create().await;
    let manager = JiraCredentialManager::new(storage.clone());

    // Try to use a stored access token to fetch accessible sites
    let (client_id, client_secret) = match manager.get_oauth_credentials().await {
        Ok(Some(c)) => c,
        _ => {
            return Ok(ResponseJson(ApiResponse::error(
                "Jira OAuth credentials not configured",
            )))
        }
    };

    let tokens = match manager.get_oauth_tokens().await {
        Ok(Some(t)) => t,
        _ => {
            return Ok(ResponseJson(ApiResponse::error(
                "No Jira OAuth tokens present. Start OAuth first.",
            )))
        }
    };

    let service = JiraAuthService::with_credentials(
        client_id,
        client_secret,
        // use the same redirect used on start
        option_env!("JIRA_REDIRECT_URI").unwrap_or("http://localhost:3000/settings").to_string(),
    )
    .await;

    match service.get_accessible_resources(&tokens.0).await {
        Ok(resources) => Ok(ResponseJson(ApiResponse::success(resources))),
        Err(e) => Ok(ResponseJson(ApiResponse::error(&format!(
            "Failed to fetch accessible resources: {}",
            e
        )))),
    }
}

/// POST /api/jira/connection/test/{cloudid}
/// Test connection to a specific Jira site
async fn test_connection(
    State(_deployment): State<DeploymentImpl>,
    Path(_cloudid): Path<String>,
) -> Result<ResponseJson<ApiResponse<String>>, ApiError> {
    // TODO: Implement connection test
    Ok(ResponseJson(ApiResponse::success("Connection test not implemented".to_string())))
}