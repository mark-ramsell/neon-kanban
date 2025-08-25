use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;
use url::Url;

use super::secure_storage::{JiraCredentialManager, SecureStorageFactory};

#[derive(Debug, Error)]
pub enum JiraAuthError {
    #[error("HTTP client error: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("OAuth error: {0}")]
    OAuth(String),
    #[error("Token expired")]
    TokenExpired,
    #[error("Invalid token")]
    InvalidToken,
    #[error("Access revoked")]
    AccessRevoked,
    #[error("Secure storage error: {0}")]
    SecureStorage(#[from] super::secure_storage::SecureStorageError),
    #[error("No OAuth credentials configured")]
    NoCredentialsConfigured,
}

pub struct JiraAuthService {
    pub client_id: String,
    pub client_secret: SecretString,
    pub redirect_uri: String,
    pub client: Client,
    pub credential_manager: JiraCredentialManager,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraOAuthStartResponse {
    pub authorization_url: String,
    pub state: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraTokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u32,          // 3600 seconds (1 hour)
    pub scope: String,
    pub token_type: String,       // "Bearer"
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct JiraResource {
    pub id: String,               // This is the cloudid
    pub name: String,             // Site display name  
    pub url: String,              // Site URL like "https://company.atlassian.net"
    pub scopes: Vec<String>,      // Granted scopes for this site
    #[serde(rename = "avatarUrl")]
    pub avatar_url: String,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: String,
    error_description: Option<String>,
}

impl JiraAuthService {
    /// Create a new JiraAuthService with secure storage
    /// Attempts to load credentials from secure storage first, falls back to environment variables
    pub async fn new() -> Result<Self, JiraAuthError> {
        let storage = SecureStorageFactory::create().await;
        let credential_manager = JiraCredentialManager::new(storage);
        
        // Try to get credentials from secure storage first
        let (client_id, client_secret) = match credential_manager.get_oauth_credentials().await? {
            Some((id, secret)) => (id, secret),
            None => {
                // Fall back to environment variables
                let client_id = option_env!("JIRA_CLIENT_ID")
                    .unwrap_or("your-jira-client-id")
                    .to_string();
                let client_secret = option_env!("JIRA_CLIENT_SECRET")
                    .unwrap_or("your-jira-client-secret")
                    .to_string();
                
                (client_id, client_secret)
            }
        };
        
        let redirect_uri = option_env!("JIRA_REDIRECT_URI")
            .unwrap_or("http://localhost:3000/settings")
            .to_string();

        Ok(Self {
            client_id,
            client_secret: SecretString::new(client_secret.into()),
            redirect_uri,
            client: Client::new(),
            credential_manager,
        })
    }

    pub async fn with_credentials(client_id: String, client_secret: String, redirect_uri: String) -> Self {
        let storage = SecureStorageFactory::create().await;
        let credential_manager = JiraCredentialManager::new(storage);
        
        Self {
            client_id,
            client_secret: SecretString::new(client_secret.into()),
            redirect_uri,
            client: Client::new(),
            credential_manager,
        }
    }

    /// Store OAuth credentials in secure storage
    pub async fn store_oauth_credentials(&self, client_id: &str, client_secret: &str) -> Result<(), JiraAuthError> {
        self.credential_manager.store_oauth_credentials(client_id, client_secret).await?;
        Ok(())
    }

    /// Store site-specific tokens in secure storage
    pub async fn store_site_tokens(&self, cloudid: &str, access_token: &str, refresh_token: &str) -> Result<(), JiraAuthError> {
        self.credential_manager.store_site_tokens(cloudid, access_token, refresh_token).await?;
        Ok(())
    }

    /// Retrieve site-specific tokens from secure storage
    pub async fn get_site_tokens(&self, cloudid: &str) -> Result<Option<(String, String)>, JiraAuthError> {
        let tokens = self.credential_manager.get_site_tokens(cloudid).await?;
        Ok(tokens)
    }

    /// Delete all credentials for a specific site
    pub async fn delete_site_credentials(&self, cloudid: &str) -> Result<(), JiraAuthError> {
        self.credential_manager.delete_site_credentials(cloudid).await?;
        Ok(())
    }

    /// Delete all OAuth credentials
    pub async fn delete_oauth_credentials(&self) -> Result<(), JiraAuthError> {
        self.credential_manager.delete_oauth_credentials().await?;
        Ok(())
    }

    /// Update OAuth credentials and store them securely
    pub async fn update_oauth_credentials(&mut self, client_id: String, client_secret: String) -> Result<(), JiraAuthError> {
        // Store in secure storage first
        self.store_oauth_credentials(&client_id, &client_secret).await?;
        
        // Update in-memory values
        self.client_id = client_id;
        self.client_secret = SecretString::new(client_secret.into());
        
        Ok(())
    }

    /// Generate OAuth authorization URL for Jira Cloud
    /// CORRECTED: Uses proper Atlassian OAuth endpoints with required parameters
    pub async fn get_authorization_url(&self, state: &str) -> Result<String, JiraAuthError> {
        let mut auth_url = Url::parse("https://auth.atlassian.com/authorize")?;
        
        auth_url.query_pairs_mut()
            .append_pair("audience", "api.atlassian.com")  // CRITICAL: Required for API access
            .append_pair("client_id", &self.client_id)
            .append_pair("scope", "read:jira-work write:jira-work read:jira-user")
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("state", state)
            .append_pair("response_type", "code")
            .append_pair("prompt", "consent");  // RECOMMENDED: Ensures user sees consent screen

        Ok(auth_url.to_string())
    }

    /// Exchange authorization code for access and refresh tokens
    /// CRITICAL: Uses client_secret (no PKCE-only flow available)
    pub async fn exchange_code_for_tokens(
        &self,
        code: &str,
        state: &str,
    ) -> Result<JiraTokenResponse, JiraAuthError> {
        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("client_id", &self.client_id);
        params.insert("client_secret", self.client_secret.expose_secret());
        params.insert("code", code);
        params.insert("redirect_uri", &self.redirect_uri);

        let response = self
            .client
            .post("https://auth.atlassian.com/oauth/token")
            .form(&params)
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = response.status();
        if status.is_success() {
            // Avoid logging token body; just parse
            let token_response: JiraTokenResponse = response.json().await?;
            Ok(token_response)
        } else {
            // Try to capture response text for diagnostics
            let text = response.text().await.unwrap_or_default();
            // Try parse structured error if possible
            let parsed: Result<TokenErrorResponse, _> = serde_json::from_str(&text);
            if let Ok(err) = parsed {
                Err(JiraAuthError::OAuth(format!(
                    "{}: {}",
                    err.error,
                    err.error_description.unwrap_or_default()
                )))
            } else {
                Err(JiraAuthError::OAuth(format!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    text
                )))
            }
        }
    }

    /// Refresh access token using refresh token
    /// CORRECTED: Handle rotating refresh tokens (new refresh token returned)
    pub async fn refresh_access_token(
        &self,
        refresh_token: &str,
    ) -> Result<JiraTokenResponse, JiraAuthError> {
        let mut params = HashMap::new();
        params.insert("grant_type", "refresh_token");
        params.insert("client_id", &self.client_id);
        params.insert("client_secret", self.client_secret.expose_secret());
        params.insert("refresh_token", refresh_token);

        let response = self
            .client
            .post("https://auth.atlassian.com/oauth/token")
            .form(&params)
            .header("Accept", "application/json")
            .send()
            .await?;

        if response.status().is_success() {
            let token_response: JiraTokenResponse = response.json().await?;
            Ok(token_response)
        } else {
            let status = response.status();
            let error_response: TokenErrorResponse = response.json().await
                .unwrap_or_else(|_| TokenErrorResponse {
                    error: "refresh_failed".to_string(),
                    error_description: Some(format!("HTTP {}", status)),
                });

            match error_response.error.as_str() {
                "invalid_grant" => Err(JiraAuthError::InvalidToken),
                _ => Err(JiraAuthError::OAuth(format!(
                    "{}: {}",
                    error_response.error,
                    error_response.error_description.unwrap_or_default()
                ))),
            }
        }
    }

    /// Get accessible resources (sites) for the current access token
    /// CORRECTED: Returns sites with cloudids for API calls
    pub async fn get_accessible_resources(
        &self,
        access_token: &str,
    ) -> Result<Vec<JiraResource>, JiraAuthError> {
        let response = self
            .client
            .get("https://api.atlassian.com/oauth/token/accessible-resources")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Accept", "application/json")
            .send()
            .await?;

        if response.status().is_success() {
            let resources: Vec<JiraResource> = response.json().await?;
            Ok(resources)
        } else {
            let status = response.status();
            match status.as_u16() {
                401 => Err(JiraAuthError::InvalidToken),
                403 => Err(JiraAuthError::AccessRevoked),
                _ => Err(JiraAuthError::OAuth(format!(
                    "Failed to get accessible resources: HTTP {}",
                    status
                ))),
            }
        }
    }

    /// Revoke access token and refresh token
    pub async fn revoke_tokens(&self, access_token: &str) -> Result<(), JiraAuthError> {
        let mut params = HashMap::new();
        params.insert("token", access_token);
        params.insert("client_id", &self.client_id);
        params.insert("client_secret", self.client_secret.expose_secret());

        let response = self
            .client
            .post("https://auth.atlassian.com/oauth/revoke")
            .form(&params)
            .header("Accept", "application/json")
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(JiraAuthError::OAuth(format!(
                "Failed to revoke token: HTTP {}",
                response.status()
            )))
        }
    }

    /// Check if token needs refresh (expires within 5 minutes)
    pub fn should_refresh_token(&self, expires_at: DateTime<Utc>) -> bool {
        let now = Utc::now();
        let refresh_threshold = now + Duration::minutes(5);
        expires_at <= refresh_threshold
    }

    /// Generate a secure state parameter for OAuth flow
    pub fn generate_state() -> String {
        use rand::Rng;
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                                 abcdefghijklmnopqrstuvwxyz\
                                 0123456789";
        const STATE_LEN: usize = 32;
        let mut rng = rand::rng();

        (0..STATE_LEN)
            .map(|_| {
                let idx = rng.random_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect()
    }
}

/// Helper to calculate token expiration time
pub fn calculate_token_expiry(expires_in: u32) -> DateTime<Utc> {
    Utc::now() + Duration::seconds(expires_in as i64)
}