use axum::{
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use services::services::jira_auth::JiraAuthService;
use services::services::secure_storage::{JiraCredentialManager, SecureStorageFactory};
use ts_rs::TS;
use utils::response::ApiResponse;

use crate::{error::ApiError, DeploymentImpl};

pub fn router(_deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    Router::new()
        .route("/jira/oauth/start", post(oauth_start))
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

/// POST /api/jira/oauth/start
/// Start OAuth flow - returns authorization URL
async fn oauth_start(
    State(_deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<JiraOAuthStartResponse>>, ApiError> {
    let jira_auth = JiraAuthService::new().await
        .map_err(|_| ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, "Failed to initialize Jira auth service")))?;
    let state = JiraAuthService::generate_state();
    let authorization_url = jira_auth.get_authorization_url(&state).await
        .map_err(|_| ApiError::Io(std::io::Error::new(std::io::ErrorKind::Other, "OAuth start failed")))?;

    Ok(ResponseJson(ApiResponse::success(JiraOAuthStartResponse {
        authorization_url,
        state,
    })))
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
) -> Result<ResponseJson<ApiResponse<Vec<String>>>, ApiError> {
    // TODO: Implement get accessible sites
    Ok(ResponseJson(ApiResponse::success(vec![])))
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