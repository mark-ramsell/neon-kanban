use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

#[derive(Debug, Error)]
pub enum JiraServiceError {
    #[error("HTTP client error: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Authentication failed - token invalid")]
    AuthenticationFailed,
    #[error("Access forbidden - insufficient permissions")]
    AccessForbidden,
    #[error("Resource not found: {0}")]
    NotFound(String),
    #[error("API error: {0}")]
    ApiError(String),
}

#[derive(Clone)]
pub struct JiraService {
    client: Client,
    cloudid: String,  // CRITICAL: Store cloudid, not site URL
    access_token: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraProject {
    pub id: String,
    pub key: String,
    pub name: String,
    #[serde(rename = "projectTypeKey")]
    pub project_type_key: String,
    pub description: Option<String>,
    pub lead: Option<JiraUser>,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraUser {
    #[serde(rename = "accountId")]
    pub account_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "emailAddress")]
    pub email_address: Option<String>,
    #[serde(rename = "avatarUrls")]
    pub avatar_urls: Option<JiraAvatarUrls>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraAvatarUrls {
    #[serde(rename = "16x16")]
    pub small: String,
    #[serde(rename = "24x24")]
    pub medium: String,
    #[serde(rename = "32x32")]
    pub large: String,
    #[serde(rename = "48x48")]
    pub xlarge: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraIssue {
    pub id: String,
    pub key: String,
    pub fields: JiraIssueFields,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraIssueFields {
    pub summary: String,
    pub description: Option<String>,
    pub status: JiraStatus,
    pub assignee: Option<JiraUser>,
    pub reporter: JiraUser,
    pub project: JiraProject,
    #[serde(rename = "issuetype")]
    pub issue_type: JiraIssueType,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraStatus {
    pub id: String,
    pub name: String,
    #[serde(rename = "statusCategory")]
    pub status_category: JiraStatusCategory,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraStatusCategory {
    pub id: u32,
    pub name: String,
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraIssueType {
    pub id: String,
    pub name: String,
    pub description: String,
    pub subtask: bool,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct CreateIssueRequest {
    pub project_key: String,
    pub summary: String,
    pub description: Option<String>,
    pub issue_type_name: String,  // e.g., "Task", "Bug", "Story"
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct UpdateIssueRequest {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub assignee_account_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraConnectionStatus {
    pub connected: bool,
    pub site_name: String,
    pub user: Option<JiraUser>,
    pub accessible_projects: u32,
    pub granted_scopes: Vec<String>,
}

impl JiraService {
    pub fn new(cloudid: String, access_token: String) -> Self {
        Self {
            client: Client::new(),
            cloudid,
            access_token,
        }
    }

    /// CORRECTED: All API calls use cloudid-based URLs
    fn base_url(&self) -> String {
        format!("https://api.atlassian.com/ex/jira/{}", self.cloudid)
    }

    /// Get current user information
    pub async fn get_user_info(&self) -> Result<JiraUser, JiraServiceError> {
        let url = format!("{}/rest/api/2/myself", self.base_url());
        
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Accept", "application/json")
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Get all projects accessible to the user
    pub async fn get_projects(&self) -> Result<Vec<JiraProject>, JiraServiceError> {
        let url = format!("{}/rest/api/2/project", self.base_url());
        
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Accept", "application/json")
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Get specific project by key
    pub async fn get_project(&self, project_key: &str) -> Result<JiraProject, JiraServiceError> {
        let url = format!("{}/rest/api/2/project/{}", self.base_url(), project_key);
        
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Accept", "application/json")
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Create a new issue in Jira
    pub async fn create_issue(
        &self,
        issue: &CreateIssueRequest,
    ) -> Result<JiraIssue, JiraServiceError> {
        let url = format!("{}/rest/api/2/issue", self.base_url());
        
        // Build issue payload following Jira's format
        let payload = serde_json::json!({
            "fields": {
                "project": {
                    "key": issue.project_key
                },
                "summary": issue.summary,
                "description": issue.description.as_ref().unwrap_or(&"".to_string()),
                "issuetype": {
                    "name": issue.issue_type_name
                }
            }
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Update an existing issue
    pub async fn update_issue(
        &self,
        issue_key: &str,
        update: &UpdateIssueRequest,
    ) -> Result<(), JiraServiceError> {
        let url = format!("{}/rest/api/2/issue/{}", self.base_url(), issue_key);
        
        let mut fields = serde_json::Map::new();
        
        if let Some(summary) = &update.summary {
            fields.insert("summary".to_string(), serde_json::Value::String(summary.clone()));
        }
        
        if let Some(description) = &update.description {
            fields.insert("description".to_string(), serde_json::Value::String(description.clone()));
        }
        
        if let Some(assignee_id) = &update.assignee_account_id {
            fields.insert("assignee".to_string(), serde_json::json!({"accountId": assignee_id}));
        }

        let payload = serde_json::json!({ "fields": fields });

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            self.handle_error_response(response).await
        }
    }

    /// Get issue by key
    pub async fn get_issue(&self, issue_key: &str) -> Result<JiraIssue, JiraServiceError> {
        let url = format!("{}/rest/api/2/issue/{}", self.base_url(), issue_key);
        
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Accept", "application/json")
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Test connection and get site information
    pub async fn test_connection(&self) -> Result<JiraConnectionStatus, JiraServiceError> {
        // Get user info to test authentication
        let user = self.get_user_info().await.ok();
        
        // Get projects to test permissions
        let projects = self.get_projects().await.unwrap_or_default();
        
        Ok(JiraConnectionStatus {
            connected: user.is_some(),
            site_name: format!("Jira Cloud Site ({})", self.cloudid),
            user,
            accessible_projects: projects.len() as u32,
            granted_scopes: vec![], // Would need to be passed from the calling context
        })
    }

    /// Helper to handle successful responses
    async fn handle_response<T>(&self, response: reqwest::Response) -> Result<T, JiraServiceError>
    where
        T: for<'de> Deserialize<'de>,
    {
        if response.status().is_success() {
            let data: T = response.json().await?;
            Ok(data)
        } else {
            self.handle_error_response(response).await
        }
    }

    /// Helper to handle error responses
    async fn handle_error_response<T>(&self, response: reqwest::Response) -> Result<T, JiraServiceError> {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();

        match status.as_u16() {
            401 => Err(JiraServiceError::AuthenticationFailed),
            403 => Err(JiraServiceError::AccessForbidden),
            404 => Err(JiraServiceError::NotFound(error_text)),
            _ => Err(JiraServiceError::ApiError(format!(
                "HTTP {}: {}",
                status,
                error_text
            ))),
        }
    }
}