use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use tendermint::{block::Height, Block};
use tendermint_rpc::{event::Event, Error as RpcError};

pub mod v034;
pub mod v038;
pub mod auth;
pub mod factory;

pub use factory::{create_client, AuthConfig};

/// Result type for client operations
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Subscription type for new blocks
pub type BlockSubscription = Pin<Box<dyn Stream<Item = std::result::Result<Event, RpcError>> + Send>>;

/// Common interface for all chain clients regardless of version or auth method
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// Subscribe to new block events
    async fn subscribe_blocks(&self) -> Result<BlockSubscription>;
    
    /// Get a specific block by height
    async fn get_block(&self, height: Height) -> Result<Block>;
    
    /// Get block results (may return limited data for older versions)
    async fn get_block_results(&self, height: Height) -> Result<BlockResults>;
    
    /// Check if this client supports enhanced event extraction
    fn supports_events(&self) -> bool {
        false
    }
}

/// Common block results structure
#[derive(Debug, Clone)]
pub struct BlockResults {
    pub height: Height,
    pub txs_results: Vec<TxResult>,
}

/// Transaction result with events
#[derive(Debug, Clone)]
pub struct TxResult {
    pub code: u32,
    pub events: Vec<TxEvent>,
}

/// Transaction event
#[derive(Debug, Clone)]
pub struct TxEvent {
    pub type_str: String,
    pub attributes: Vec<EventAttribute>,
}

/// Event attribute
#[derive(Debug, Clone)]
pub struct EventAttribute {
    pub key: String,
    pub value: String,
}