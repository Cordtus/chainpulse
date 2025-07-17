use async_trait::async_trait;
use tendermint::{block::Height, Block};
use tendermint_rpc::event::Event;

use super::{BlockResults, BlockSubscription, ChainClient, Result};
use crate::simple_auth_client::{AuthMethod, SimpleAuthClient};

/// Client wrapper for authenticated connections
pub struct AuthClient {
    url: String,
    auth_method: AuthMethod,
    version: String,
}

impl AuthClient {
    /// Create a new authenticated client
    pub async fn new(
        url: String,
        version: String,
        username: String,
        password: String,
    ) -> Result<Self> {
        let auth_method = AuthMethod::Basic { username, password };

        Ok(Self {
            url,
            auth_method,
            version,
        })
    }
}

#[async_trait]
impl ChainClient for AuthClient {
    async fn subscribe_blocks(&self) -> Result<BlockSubscription> {
        // Create a new SimpleAuthClient instance for this subscription
        let client = SimpleAuthClient::new(self.url.clone(), self.auth_method.clone());
        let mut block_stream = client.subscribe_blocks().await?;

        // Create a channel to bridge between BlockStream and our Event stream
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Spawn a task to convert blocks to events
        tokio::spawn(async move {
            while let Some(block) = block_stream.next().await {
                let event = Event {
                    query: "tm.event='NewBlock'".to_string(),
                    data: tendermint_rpc::event::EventData::NewBlock {
                        block: Some(block),
                        result_begin_block: None,
                        result_end_block: None,
                    },
                    events: None,
                };

                if tx.send(Ok(event)).await.is_err() {
                    break; // Receiver dropped
                }
            }
        });

        // Convert receiver to stream
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn get_block(&self, _height: Height) -> Result<Block> {
        // SimpleAuthClient doesn't have a get_block method
        // For now, return an error - this would need to be implemented
        Err("get_block not implemented for AuthClient".into())
    }

    async fn get_block_results(&self, height: Height) -> Result<BlockResults> {
        // SimpleAuthClient doesn't support block results
        // Return empty results
        Ok(BlockResults {
            height,
            txs_results: vec![],
        })
    }

    fn supports_events(&self) -> bool {
        // Auth client has limited event support
        false
    }
}
