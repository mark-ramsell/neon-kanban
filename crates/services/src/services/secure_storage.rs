use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use anyhow::Result;
use thiserror::Error;

/// Errors that can occur during secure storage operations
#[derive(Debug, Error)]
pub enum SecureStorageError {
    #[error("Keychain access failed: {0}")]
    KeychainError(String),
    #[error("Credential not found: {0}")]
    NotFound(String),
    #[error("Storage backend unavailable")]
    BackendUnavailable,
    #[error("Invalid credential data: {0}")]
    InvalidData(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Trait for secure credential storage backends
#[async_trait::async_trait]
pub trait SecureStorage: Send + Sync {
    /// Store a credential securely
    async fn store_credential(&self, key: &str, value: &str) -> Result<(), SecureStorageError>;
    
    /// Retrieve a credential
    async fn retrieve_credential(&self, key: &str) -> Result<Option<String>, SecureStorageError>;
    
    /// Delete a credential
    async fn delete_credential(&self, key: &str) -> Result<(), SecureStorageError>;
    
    /// Check if the storage backend is available
    async fn is_available(&self) -> bool;
}

/// Keyring-based secure storage implementation (macOS, Windows, Linux)
#[cfg(feature = "keyring")]
pub struct KeyringStorage {
    service_name: String,
}

#[cfg(feature = "keyring")]
impl KeyringStorage {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
        }
    }
    
    fn create_entry(&self, key: &str) -> Result<keyring::Entry, SecureStorageError> {
        keyring::Entry::new(&self.service_name, key)
            .map_err(|e| SecureStorageError::KeychainError(format!("Failed to create entry: {}", e)))
    }
}

#[cfg(feature = "keyring")]
#[async_trait::async_trait]
impl SecureStorage for KeyringStorage {
    async fn store_credential(&self, key: &str, value: &str) -> Result<(), SecureStorageError> {
        let entry = self.create_entry(key)?;
        entry.set_password(value)
            .map_err(|e| SecureStorageError::KeychainError(format!("Failed to store credential: {}", e)))
    }
    
    async fn retrieve_credential(&self, key: &str) -> Result<Option<String>, SecureStorageError> {
        let entry = self.create_entry(key)?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecureStorageError::KeychainError(format!("Failed to retrieve credential: {}", e))),
        }
    }
    
    async fn delete_credential(&self, key: &str) -> Result<(), SecureStorageError> {
        let entry = self.create_entry(key)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // Already deleted
            Err(e) => Err(SecureStorageError::KeychainError(format!("Failed to delete credential: {}", e))),
        }
    }
    
    async fn is_available(&self) -> bool {
        // Test by trying to create a test entry
        match self.create_entry("__vibe_kanban_test__") {
            Ok(_) => true,
            Err(_) => false,
        }
    }
}

/// Fallback in-memory storage for development/testing
pub struct MemoryStorage {
    data: Arc<Mutex<HashMap<String, String>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl SecureStorage for MemoryStorage {
    async fn store_credential(&self, key: &str, value: &str) -> Result<(), SecureStorageError> {
        let mut data = self.data.lock().unwrap();
        data.insert(key.to_string(), value.to_string());
        Ok(())
    }
    
    async fn retrieve_credential(&self, key: &str) -> Result<Option<String>, SecureStorageError> {
        let data = self.data.lock().unwrap();
        Ok(data.get(key).cloned())
    }
    
    async fn delete_credential(&self, key: &str) -> Result<(), SecureStorageError> {
        let mut data = self.data.lock().unwrap();
        data.remove(key);
        Ok(())
    }
    
    async fn is_available(&self) -> bool {
        true
    }
}

/// Factory for creating the appropriate secure storage backend
pub struct SecureStorageFactory;

impl SecureStorageFactory {
    /// Create the best available secure storage backend
    pub async fn create() -> Arc<dyn SecureStorage> {
        let service_name = "vibe-kanban-jira";
        
        #[cfg(feature = "keyring")]
        {
            let keyring_storage = KeyringStorage::new(service_name);
            if keyring_storage.is_available().await {
                tracing::info!("Using keyring-based secure storage");
                return Arc::new(keyring_storage);
            } else {
                tracing::warn!("Keyring storage not available, falling back to memory storage");
            }
        }
        
        tracing::warn!("Using in-memory storage for credentials (not persistent)");
        Arc::new(MemoryStorage::new())
    }
}

/// Convenience wrapper for Jira-specific credential management
pub struct JiraCredentialManager {
    storage: Arc<dyn SecureStorage>,
}

impl JiraCredentialManager {
    pub fn new(storage: Arc<dyn SecureStorage>) -> Self {
        Self { storage }
    }
    
    /// Store OAuth client credentials (app-level)
    pub async fn store_oauth_credentials(&self, client_id: &str, client_secret: &str) -> Result<(), SecureStorageError> {
        self.storage.store_credential("oauth.client_id", client_id).await?;
        self.storage.store_credential("oauth.client_secret", client_secret).await?;
        Ok(())
    }
    
    /// Retrieve OAuth client credentials
    pub async fn get_oauth_credentials(&self) -> Result<Option<(String, String)>, SecureStorageError> {
        let client_id = self.storage.retrieve_credential("oauth.client_id").await?;
        let client_secret = self.storage.retrieve_credential("oauth.client_secret").await?;
        
        match (client_id, client_secret) {
            (Some(id), Some(secret)) => Ok(Some((id, secret))),
            _ => Ok(None),
        }
    }
    
    /// Store site-specific tokens
    pub async fn store_site_tokens(&self, cloudid: &str, access_token: &str, refresh_token: &str) -> Result<(), SecureStorageError> {
        let access_key = format!("site.{}.access_token", cloudid);
        let refresh_key = format!("site.{}.refresh_token", cloudid);
        
        self.storage.store_credential(&access_key, access_token).await?;
        self.storage.store_credential(&refresh_key, refresh_token).await?;

        // Update sites index
        let mut sites = self.list_sites().await.unwrap_or_default();
        if !sites.iter().any(|s| s == cloudid) {
            sites.push(cloudid.to_string());
            let sites_raw = serde_json::to_string(&sites)
                .map_err(|e| SecureStorageError::InvalidData(e.to_string()))?;
            self.storage
                .store_credential("sites.index", &sites_raw)
                .await?;
        }
        Ok(())
    }
    
    /// Retrieve site-specific tokens
    pub async fn get_site_tokens(&self, cloudid: &str) -> Result<Option<(String, String)>, SecureStorageError> {
        let access_key = format!("site.{}.access_token", cloudid);
        let refresh_key = format!("site.{}.refresh_token", cloudid);
        
        let access_token = self.storage.retrieve_credential(&access_key).await?;
        let refresh_token = self.storage.retrieve_credential(&refresh_key).await?;
        
        match (access_token, refresh_token) {
            (Some(access), Some(refresh)) => Ok(Some((access, refresh))),
            _ => Ok(None),
        }
    }
    
    /// Delete all credentials for a specific site
    pub async fn delete_site_credentials(&self, cloudid: &str) -> Result<(), SecureStorageError> {
        let access_key = format!("site.{}.access_token", cloudid);
        let refresh_key = format!("site.{}.refresh_token", cloudid);
        
        self.storage.delete_credential(&access_key).await?;
        self.storage.delete_credential(&refresh_key).await?;

        // Remove from sites index
        let mut sites = self.list_sites().await.unwrap_or_default();
        let before_len = sites.len();
        sites.retain(|s| s != cloudid);
        if sites.len() != before_len {
            let sites_raw = serde_json::to_string(&sites)
                .map_err(|e| SecureStorageError::InvalidData(e.to_string()))?;
            self.storage
                .store_credential("sites.index", &sites_raw)
                .await?;
        }
        Ok(())
    }
    
    /// Delete all OAuth credentials
    pub async fn delete_oauth_credentials(&self) -> Result<(), SecureStorageError> {
        self.storage.delete_credential("oauth.client_id").await?;
        self.storage.delete_credential("oauth.client_secret").await?;
        Ok(())
    }

    /// Store global OAuth tokens (used to fetch accessible resources)
    pub async fn store_oauth_tokens(&self, access_token: &str, refresh_token: &str) -> Result<(), SecureStorageError> {
        self.storage
            .store_credential("oauth.access_token", access_token)
            .await?;
        self.storage
            .store_credential("oauth.refresh_token", refresh_token)
            .await?;
        Ok(())
    }

    /// Retrieve global OAuth tokens
    pub async fn get_oauth_tokens(&self) -> Result<Option<(String, String)>, SecureStorageError> {
        let access = self.storage.retrieve_credential("oauth.access_token").await?;
        let refresh = self.storage.retrieve_credential("oauth.refresh_token").await?;
        match (access, refresh) {
            (Some(a), Some(r)) => Ok(Some((a, r))),
            _ => Ok(None),
        }
    }

    /// List stored site cloudids
    pub async fn list_sites(&self) -> Result<Vec<String>, SecureStorageError> {
        let raw = match self.storage.retrieve_credential("sites.index").await? {
            Some(s) => s,
            None => return Ok(vec![]),
        };
        let parsed: Vec<String> = serde_json::from_str(&raw)
            .map_err(|e| SecureStorageError::InvalidData(e.to_string()))?;
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_memory_storage() {
        let storage = MemoryStorage::new();
        
        // Test store and retrieve
        storage.store_credential("test_key", "test_value").await.unwrap();
        let retrieved = storage.retrieve_credential("test_key").await.unwrap();
        assert_eq!(retrieved, Some("test_value".to_string()));
        
        // Test delete
        storage.delete_credential("test_key").await.unwrap();
        let retrieved = storage.retrieve_credential("test_key").await.unwrap();
        assert_eq!(retrieved, None);
    }
    
    #[tokio::test]
    async fn test_jira_credential_manager() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = JiraCredentialManager::new(storage);
        
        // Test OAuth credentials
        manager.store_oauth_credentials("client123", "secret456").await.unwrap();
        let creds = manager.get_oauth_credentials().await.unwrap();
        assert_eq!(creds, Some(("client123".to_string(), "secret456".to_string())));
        
        // Test site tokens
        manager.store_site_tokens("cloud123", "access_token", "refresh_token").await.unwrap();
        let tokens = manager.get_site_tokens("cloud123").await.unwrap();
        assert_eq!(tokens, Some(("access_token".to_string(), "refresh_token".to_string())));
        
        // Test deletion
        manager.delete_site_credentials("cloud123").await.unwrap();
        let tokens = manager.get_site_tokens("cloud123").await.unwrap();
        assert_eq!(tokens, None);
    }
}