use async_trait::async_trait;
use tendermint::{block::Height, Block};
use tendermint_rpc::{
    client::CompatMode, query::{EventType, Query}, Client, SubscriptionClient, 
    WebSocketClient, WebSocketClientUrl,
};

use super::{BlockResults, BlockSubscription, ChainClient, EventAttribute, Result, TxEvent, TxResult};

/// Client for v0.34 and v0.37 protocols using tendermint-rs v0.32
pub struct V034Client {
    client: WebSocketClient,
    compat_mode: CompatMode,
}

impl V034Client {
    /// Create a new v0.34/v0.37 client
    pub async fn new(ws_url: WebSocketClientUrl, version: &str) -> Result<Self> {
        let compat_mode = match version {
            "0.34" => CompatMode::V0_34,
            "0.37" => CompatMode::V0_37,
            _ => return Err(format!("Unsupported version for V034Client: {}", version).into()),
        };

        let (client, driver) = WebSocketClient::builder(ws_url)
            .compat_mode(compat_mode)
            .build()
            .await?;

        // Spawn the driver
        tokio::spawn(driver.run());

        Ok(Self { client, compat_mode })
    }
}

#[async_trait]
impl ChainClient for V034Client {
    async fn subscribe_blocks(&self) -> Result<BlockSubscription> {
        let query = Query::from(EventType::NewBlock);
        let subscription = self.client.subscribe(query).await?;
        Ok(Box::pin(subscription))
    }

    async fn get_block(&self, height: Height) -> Result<Block> {
        let response = self.client.block(height).await?;
        Ok(response.block)
    }

    async fn get_block_results(&self, height: Height) -> Result<BlockResults> {
        // Try to get block results, but handle gracefully if it fails
        match self.client.block_results(height).await {
            Ok(results) => {
                let txs_results = results.txs_results.unwrap_or_default()
                    .into_iter()
                    .map(|tx_result| {
                        let events = tx_result.events
                            .into_iter()
                            .map(|event| TxEvent {
                                type_str: event.kind,
                                attributes: event.attributes
                                    .into_iter()
                                    .map(|attr| EventAttribute {
                                        key: attr.key,
                                        value: attr.value,
                                    })
                                    .collect(),
                            })
                            .collect();
                        
                        TxResult {
                            code: tx_result.code.value(),
                            events,
                        }
                    })
                    .collect();

                Ok(BlockResults {
                    height,
                    txs_results,
                })
            }
            Err(e) => {
                // For v0.34/v0.37, block results might fail on some chains
                tracing::debug!("Could not fetch block results: {}", e);
                Ok(BlockResults {
                    height,
                    txs_results: vec![],
                })
            }
        }
    }

    fn supports_events(&self) -> bool {
        // v0.34/v0.37 have limited event support
        true
    }
}