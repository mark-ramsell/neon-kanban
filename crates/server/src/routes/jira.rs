use axum::{
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use services::services::jira_auth::JiraAuthService;
use ts_rs::TS;
use utils::response::ApiResponse;

use crate::{error::ApiError, DeploymentImpl};

pub fn router(_deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    Router::new()
        .route("/jira/oauth/start", post(oauth_start))
        .route("/jira/configs", get(get_jira_configs))
        .route("/jira/sites/accessible", get(get_accessible_sites))
        .route("/jira/connection/test/:cloudid", post(test_connection))
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

/// GET /api/jira/sites/accessible
/// Get accessible sites for current user
async fn get_accessible_sites(
    State(_deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<String>>>, ApiError> {
    // TODO: Implement get accessible sites
    Ok(ResponseJson(ApiResponse::success(vec![])))
}

/// POST /api/jira/connection/test/:cloudid
/// Test connection to a specific Jira site
async fn test_connection(
    State(_deployment): State<DeploymentImpl>,
    Path(_cloudid): Path<String>,
) -> Result<ResponseJson<ApiResponse<String>>, ApiError> {
    // TODO: Implement connection test
    Ok(ResponseJson(ApiResponse::success("Connection test not implemented".to_string())))
}