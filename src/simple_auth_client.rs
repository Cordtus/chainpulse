use async_tungstenite::{
    tokio::connect_async_with_config,
    tungstenite::{
        client::IntoClientRequest,
        http::HeaderValue,
        Message,
    },
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tendermint::Block;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub enum AuthMethod {
    None,
    Basic { username: String, password: String },
    Bearer { token: String },
    ApiKey { header_name: String, key: String },
}

/// Simple authenticated WebSocket client for block subscriptions
pub struct SimpleAuthClient {
    url: String,
    auth_method: AuthMethod,
}

impl SimpleAuthClient {
    pub fn new(url: String, auth_method: AuthMethod) -> Self {
        Self { url, auth_method }
    }
    
    /// Subscribe to blocks and return a stream
    pub async fn subscribe_blocks(
        self,
    ) -> Result<BlockStream, Box<dyn std::error::Error + Send + Sync>> {
        // Initialize rustls crypto provider if not already initialized
        let _ = rustls::crypto::ring::default_provider().install_default();
        
        // Build request with authentication
        info!("Connecting to WebSocket URL: {}", self.url);
        let mut request = self.url.into_client_request()?;
        
        match &self.auth_method {
            AuthMethod::None => {}
            AuthMethod::Basic { username, password } => {
                let credentials = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    format!("{}:{}", username, password)
                );
                let auth_header = format!("Basic {}", credentials);
                debug!("Using Basic Auth with username: {}", username);
                request.headers_mut().insert(
                    "Authorization",
                    HeaderValue::from_str(&auth_header)?,
                );
                
                // Add Origin header - some WebSocket servers require this
                if let Ok(origin) = HeaderValue::from_str(&format!("https://{}", request.uri().host().unwrap_or("localhost"))) {
                    request.headers_mut().insert("Origin", origin);
                }
            }
            _ => return Err("Unsupported auth method".into()),
        }
        
        info!("Connecting to WebSocket with authentication...");
        debug!("Request headers: {:?}", request.headers());
        
        let result = connect_async_with_config(request, None).await;
        match &result {
            Ok(_) => info!("WebSocket handshake successful"),
            Err(e) => error!("WebSocket handshake failed: {:?}", e),
        }
        let (ws_stream, _) = result?;
        info!("WebSocket connection established");
        
        let (mut write, mut read) = ws_stream.split();
        
        // Send subscription request
        let subscribe_msg = r#"{"jsonrpc":"2.0","method":"subscribe","params":{"query":"tm.event = 'NewBlock'"},"id":1}"#;
        write.send(Message::Text(subscribe_msg.to_string().into())).await?;
        
        // Read subscription response
        if let Some(Ok(Message::Text(response))) = read.next().await {
            debug!("Subscription response: {}", response);
        }
        
        Ok(BlockStream {
            read: Arc::new(Mutex::new(read)),
        })
    }
}

/// Stream of blocks from WebSocket
pub struct BlockStream {
    read: Arc<Mutex<futures::stream::SplitStream<async_tungstenite::WebSocketStream<async_tungstenite::tokio::ConnectStream>>>>,
}

impl BlockStream {
    /// Get next block
    pub async fn next(&mut self) -> Option<Block> {
        let mut read = self.read.lock().await;
        
        while let Some(result) = read.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    // Try to parse as event
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        // Check if it's a block event
                        if let Some(block_json) = json["result"]["data"]["value"]["block"].as_object() {
                            // Parse block
                            if let Ok(block) = serde_json::from_value::<Block>(serde_json::Value::Object(block_json.clone())) {
                                return Some(block);
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("WebSocket closed");
                    return None;
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    return None;
                }
                _ => {} // Ignore other message types
            }
        }
        
        None
    }
}