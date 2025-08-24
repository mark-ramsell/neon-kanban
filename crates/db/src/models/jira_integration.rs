use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct JiraConfig {
    pub id: String,
    pub user_config_id: String,
    pub cloudid: String,
    pub site_name: String,
    pub site_url: String,
    pub client_id: String,
    pub client_secret: String,  // Will be encrypted
    pub access_token: Option<String>,  // Will be encrypted
    pub refresh_token: Option<String>,  // Will be encrypted
    pub token_expires_at: Option<DateTime<Utc>>,
    pub granted_scopes: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct JiraProject {
    pub id: String,
    pub jira_config_id: String,
    pub jira_project_id: String,
    pub project_key: String,
    pub project_name: String,
    pub project_type: Option<String>,
    pub cached_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct CreateJiraConfig {
    pub user_config_id: String,
    pub cloudid: String,
    pub site_name: String,
    pub site_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub access_token: String,
    pub refresh_token: String,
    pub token_expires_at: DateTime<Utc>,
    pub granted_scopes: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct UpdateJiraConfig {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub granted_scopes: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraResource {
    pub id: String,           // This is the cloudid
    pub name: String,         // Site display name
    pub url: String,          // Site URL like "https://company.atlassian.net"
    pub scopes: Vec<String>,  // Granted scopes for this site
    pub avatar_url: String,
}

impl JiraConfig {
    pub async fn create(_pool: &SqlitePool, _config: CreateJiraConfig) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        // TODO: Implement when database is ready
        Ok(id)
    }

    pub async fn find_by_user_and_cloudid(
        _pool: &SqlitePool,
        _user_config_id: &str,
        _cloudid: &str,
    ) -> Result<Option<JiraConfig>> {
        // TODO: Implement when database is ready
        Ok(None)
    }

    pub async fn find_by_user(
        _pool: &SqlitePool,
        _user_config_id: &str,
    ) -> Result<Vec<JiraConfig>> {
        // TODO: Implement when database is ready
        Ok(vec![])
    }

    pub async fn update_tokens(
        _pool: &SqlitePool,
        _id: &str,
        _update: UpdateJiraConfig,
    ) -> Result<()> {
        // TODO: Implement when database is ready
        Ok(())
    }

    pub async fn delete(_pool: &SqlitePool, _id: &str) -> Result<()> {
        // TODO: Implement when database is ready
        Ok(())
    }
}

impl JiraProject {
    pub async fn create_or_update_batch(
        _pool: &SqlitePool,
        _jira_config_id: &str,
        _projects: Vec<JiraProject>,
    ) -> Result<()> {
        // TODO: Implement when database is ready
        Ok(())
    }

    pub async fn find_by_config(
        _pool: &SqlitePool,
        _jira_config_id: &str,
    ) -> Result<Vec<JiraProject>> {
        // TODO: Implement when database is ready
        Ok(vec![])
    }
}