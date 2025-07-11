use std::time::Duration;

use tokio::time::sleep;
use tracing::warn;

use crate::{config::Chains, metrics::Metrics, Result};

pub async fn run(_chains: Chains, _metrics: Metrics) -> Result<()> {
    // TODO: Implement universal stuck packet monitoring
    // For now, this is a placeholder that logs a warning
    warn!("Stuck packet monitoring is not yet implemented. This feature will be added in a future version.");
    
    // Keep the service running but do nothing
    loop {
        sleep(Duration::from_secs(3600)).await; // Sleep for 1 hour
    }
}