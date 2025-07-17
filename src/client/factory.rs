use tendermint_rpc::WebSocketClientUrl;

use super::{auth::AuthClient, v034::V034Client, v038::V038Client, ChainClient, Result};

/// Authentication configuration
#[derive(Clone)]
pub struct AuthConfig {
    pub username: String,
    pub password: String,
}

/// Create a chain client based on version and authentication requirements
pub async fn create_client(
    ws_url: &WebSocketClientUrl,
    version: &str,
    auth: Option<AuthConfig>,
) -> Result<Box<dyn ChainClient>> {
    tracing::info!("Creating client for version {} at {}", version, ws_url);

    match auth {
        Some(auth_config) => {
            // Authenticated connection - use custom auth client
            tracing::info!("Using authenticated client");
            let client = AuthClient::new(
                ws_url.to_string(),
                version.to_string(),
                auth_config.username,
                auth_config.password,
            )
            .await?;
            Ok(Box::new(client))
        }
        None => {
            // Non-authenticated connection - use version-specific client
            match version {
                "0.34" | "0.37" => {
                    tracing::info!("Using V034Client for version {}", version);
                    let client = V034Client::new(ws_url.clone(), version).await?;
                    Ok(Box::new(client))
                }
                "0.38" => {
                    tracing::info!("Using V038Client for version 0.38");
                    let client = V038Client::new(ws_url.to_string()).await?;
                    Ok(Box::new(client))
                }
                _ => Err(format!("Unsupported CometBFT version: {}", version).into()),
            }
        }
    }
}
