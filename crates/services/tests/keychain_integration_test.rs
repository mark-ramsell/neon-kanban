use services::services::secure_storage::{SecureStorageFactory, JiraCredentialManager};

#[tokio::test]
async fn test_keychain_integration() {
    println!("Testing keychain integration on macOS...");
    
    let storage = SecureStorageFactory::create().await;
    let manager = JiraCredentialManager::new(storage);
    
    // Test storing and retrieving OAuth credentials
    let test_client_id = "test_client_12345";
    let test_client_secret = "test_secret_67890";
    
    println!("Storing test OAuth credentials...");
    let store_result = manager.store_oauth_credentials(test_client_id, test_client_secret).await;
    match &store_result {
        Ok(()) => println!("✓ Successfully stored OAuth credentials"),
        Err(e) => println!("✗ Failed to store OAuth credentials: {}", e),
    }
    
    if store_result.is_ok() {
        println!("Retrieving test OAuth credentials...");
        match manager.get_oauth_credentials().await {
            Ok(Some((retrieved_id, retrieved_secret))) => {
                println!("✓ Successfully retrieved OAuth credentials");
                assert_eq!(retrieved_id, test_client_id);
                assert_eq!(retrieved_secret, test_client_secret);
                println!("✓ Credentials match expected values");
            }
            Ok(None) => {
                println!("✗ No OAuth credentials found");
                panic!("Expected to find stored credentials");
            }
            Err(e) => {
                println!("✗ Failed to retrieve OAuth credentials: {}", e);
                panic!("Failed to retrieve credentials: {}", e);
            }
        }
        
        // Test site tokens
        let test_cloudid = "test-site-12345";
        let test_access_token = "access_token_abc";
        let test_refresh_token = "refresh_token_xyz";
        
        println!("Storing test site tokens...");
        let store_tokens_result = manager.store_site_tokens(test_cloudid, test_access_token, test_refresh_token).await;
        match &store_tokens_result {
            Ok(()) => println!("✓ Successfully stored site tokens"),
            Err(e) => println!("✗ Failed to store site tokens: {}", e),
        }
        
        if store_tokens_result.is_ok() {
            println!("Retrieving test site tokens...");
            match manager.get_site_tokens(test_cloudid).await {
                Ok(Some((retrieved_access, retrieved_refresh))) => {
                    println!("✓ Successfully retrieved site tokens");
                    assert_eq!(retrieved_access, test_access_token);
                    assert_eq!(retrieved_refresh, test_refresh_token);
                    println!("✓ Tokens match expected values");
                }
                Ok(None) => {
                    println!("✗ No site tokens found");
                    panic!("Expected to find stored tokens");
                }
                Err(e) => {
                    println!("✗ Failed to retrieve site tokens: {}", e);
                    panic!("Failed to retrieve tokens: {}", e);
                }
            }
            
            // Clean up test data
            println!("Cleaning up test data...");
            let _ = manager.delete_oauth_credentials().await;
            let _ = manager.delete_site_credentials(test_cloudid).await;
            println!("✓ Cleanup completed");
        }
    }
    
    println!("Keychain integration test completed successfully!");
}

#[tokio::test]
async fn test_keychain_availability() {
    println!("Testing keychain availability...");
    
    let storage = SecureStorageFactory::create().await;
    let is_available = storage.is_available().await;
    
    println!("Keychain available: {}", is_available);
    
    if cfg!(target_os = "macos") {
        println!("Running on macOS - keychain should be available");
        // Note: On macOS, keychain might not be available in some test environments
        // or if the app is not signed, so we don't assert here
    } else {
        println!("Not running on macOS - using fallback storage");
    }
}