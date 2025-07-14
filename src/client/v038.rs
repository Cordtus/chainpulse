use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tendermint::{block::Height, Block};
use tendermint_rpc::event::Event;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tokio_tungstenite::tungstenite::Message;

use super::{BlockResults, BlockSubscription, ChainClient, EventAttribute, Result, TxEvent, TxResult};

/// Client for v0.38 protocol with custom implementation
pub struct V038Client {
    url: String,
    request_id: Arc<AtomicU64>,
}

impl V038Client {
    /// Create a new v0.38 client
    pub async fn new(url: String) -> Result<Self> {
        // Initialize rustls crypto provider if not already done
        let _ = rustls::crypto::ring::default_provider().install_default();
        
        Ok(Self {
            url,
            request_id: Arc::new(AtomicU64::new(1)),
        })
    }

    /// Generate next request ID
    fn next_request_id(&self) -> String {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        format!("chainpulse-v038-{}", id)
    }

    /// Create a new WebSocket connection
    async fn connect(&self) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
        let (ws_stream, _) = connect_async(&self.url).await?;
        Ok(ws_stream)
    }

    /// Send JSON-RPC request and get response
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let mut ws = self.connect().await?;
        
        let request = json!({
            "jsonrpc": "2.0",
            "id": self.next_request_id(),
            "method": method,
            "params": params
        });

        ws.send(Message::Text(request.to_string())).await?;
        
        while let Some(msg) = ws.next().await {
            match msg? {
                Message::Text(text) => {
                    let response: JsonRpcResponse = serde_json::from_str(&text)?;
                    if let Some(error) = response.error {
                        return Err(format!("RPC error: {} - {}", error.code, error.message).into());
                    }
                    return Ok(response.result.unwrap_or(Value::Null));
                }
                _ => continue,
            }
        }
        
        Err("No response received".into())
    }
}

#[async_trait]
impl ChainClient for V038Client {
    async fn subscribe_blocks(&self) -> Result<BlockSubscription> {
        let (tx, rx) = mpsc::channel(100);
        let url = self.url.clone();
        let request_id = self.request_id.clone();
        
        // Spawn subscription handler
        tokio::spawn(async move {
            if let Err(e) = handle_subscription(url, request_id, tx).await {
                tracing::error!("Subscription error: {}", e);
            }
        });

        // Convert receiver to stream
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn get_block(&self, height: Height) -> Result<Block> {
        let params = json!({
            "height": height.to_string(),
        });
        
        let result = self.request("block", params).await?;
        
        // Parse v0.38 block format
        let block_data = result.get("block")
            .ok_or("Missing block in response")?;
            
        // Convert v0.38 format to tendermint-rs v0.32 Block type
        // This requires manual conversion due to format differences
        let block_json = serde_json::to_string(block_data)?;
        let block: Block = serde_json::from_str(&block_json)?;
        
        Ok(block)
    }

    async fn get_block_results(&self, height: Height) -> Result<BlockResults> {
        let params = json!({
            "height": height.to_string(),
        });
        
        let result = self.request("block_results", params).await?;
        
        // Parse v0.38 block results format
        let txs_results = result.get("txs_results")
            .and_then(|v| v.as_array())
            .unwrap_or(&Vec::new())
            .iter()
            .map(|tx_result| {
                let code = tx_result.get("code")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                    
                let events = tx_result.get("events")
                    .and_then(|v| v.as_array())
                    .unwrap_or(&Vec::new())
                    .iter()
                    .map(|event| parse_v038_event(event))
                    .collect();
                
                TxResult { code, events }
            })
            .collect();

        Ok(BlockResults {
            height,
            txs_results,
        })
    }

    fn supports_events(&self) -> bool {
        // v0.38 has full event support
        true
    }
}

/// Parse v0.38 event format
fn parse_v038_event(event: &Value) -> TxEvent {
    let type_str = event.get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
        
    let attributes = event.get("attributes")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|attr| {
            // v0.38 uses different attribute format
            if let Some(key) = attr.get("key").and_then(|v| v.as_str()) {
                if let Some(value) = attr.get("value").and_then(|v| v.as_str()) {
                    return Some(EventAttribute {
                        key: key.to_string(),
                        value: value.to_string(),
                    });
                }
            }
            
            // Handle base64 encoded attributes
            if let Some(key_b64) = attr.get("key").and_then(|v| v.as_str()) {
                if let Some(value_b64) = attr.get("value").and_then(|v| v.as_str()) {
                    // Try to decode base64
                    use base64::Engine;
                    let engine = base64::engine::general_purpose::STANDARD;
                    if let (Ok(key_bytes), Ok(value_bytes)) = (
                        engine.decode(key_b64),
                        engine.decode(value_b64),
                    ) {
                        let key = String::from_utf8_lossy(&key_bytes).to_string();
                        let value = String::from_utf8_lossy(&value_bytes).to_string();
                        return Some(EventAttribute { key, value });
                    }
                }
            }
            
            None
        })
        .collect();
    
    TxEvent {
        type_str,
        attributes,
    }
}

/// Handle WebSocket subscription for new blocks
async fn handle_subscription(
    url: String,
    request_id: Arc<AtomicU64>,
    tx: mpsc::Sender<std::result::Result<Event, tendermint_rpc::Error>>,
) -> Result<()> {
    let (mut ws, _) = connect_async(&url).await?;
    
    // Subscribe to NewBlock events
    let id = request_id.fetch_add(1, Ordering::SeqCst);
    let subscribe_request = json!({
        "jsonrpc": "2.0",
        "id": format!("chainpulse-v038-{}", id),
        "method": "subscribe",
        "params": {
            "query": "tm.event='NewBlock'"
        }
    });
    
    ws.send(Message::Text(subscribe_request.to_string())).await?;
    
    while let Some(msg) = ws.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(response) = serde_json::from_str::<Value>(&text) {
                    if let Some(result) = response.get("result") {
                        if let Some(data) = result.get("data") {
                            // Extract block data and construct Event manually
                            if let Some(value) = data.get("value") {
                                if let Some(block_json) = value.get("block") {
                                    // Try to parse the block
                                    if let Ok(block_str) = serde_json::to_string(block_json) {
                                        if let Ok(block) = serde_json::from_str::<Block>(&block_str) {
                                            // Construct Event manually
                                            let event = Event {
                                                query: "tm.event='NewBlock'".to_string(),
                                                data: tendermint_rpc::event::EventData::NewBlock {
                                                    block: Some(block),
                                                    result_begin_block: None,
                                                    result_end_block: None,
                                                },
                                                events: None,
                                            };
                                            let _ = tx.send(Ok(event)).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => continue,
        }
    }
    
    Ok(())
}

/// JSON-RPC response structure
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

/// JSON-RPC error structure
#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
}