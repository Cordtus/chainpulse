use std::time::Duration;

use sqlx::SqlitePool;
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::{config::Chains, metrics::Metrics, Result};

pub async fn run(_chains: Chains, _metrics: Metrics) -> Result<()> {
    // Get database path from first chain's config (they all use the same DB)
    let _db_path = if let Some((_, _endpoint)) = _chains.endpoints.iter().next() {
        // Extract database path from the endpoint's config
        // For now, we'll need to pass the database pool from main.rs
        warn!("Stuck packet monitoring requires database access - will be fully implemented when database pool is passed");
        return Ok(());
    } else {
        warn!("No chains configured for stuck packet monitoring");
        return Ok(());
    };
}

// This function should be called periodically to check for stuck packets
pub async fn check_stuck_packets(db: &SqlitePool, metrics: &Metrics) -> Result<()> {
    let stuck_threshold_secs = 900; // 15 minutes
    
    // Query for stuck packets with user data
    let query = r#"
        SELECT 
            t.chain as src_chain,
            p.dst_channel,
            p.src_channel,
            COUNT(*) as stuck_count,
            COUNT(CASE WHEN p.sender IS NOT NULL THEN 1 END) as with_user_data,
            MIN(CAST((strftime('%s', 'now') - strftime('%s', p.created_at)) AS INTEGER)) as max_age
        FROM packets p
        JOIN txs t ON p.tx_id = t.id
        WHERE p.effected = 0 
          AND CAST((strftime('%s', 'now') - strftime('%s', p.created_at)) AS INTEGER) > ?
        GROUP BY t.chain, p.dst_channel, p.src_channel
    "#;
    
    match sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(query)
        .bind(stuck_threshold_secs)
        .fetch_all(db)
        .await
    {
        Ok(rows) => {
            for (src_chain, dst_channel, src_channel, stuck_count, with_user_data, max_age) in rows {
                // Update detailed stuck packet metrics
                metrics.ibc_stuck_packets_detailed(
                    &src_chain,
                    &dst_channel,
                    &src_channel,
                    &dst_channel,
                    with_user_data > 0,
                    stuck_count,
                );
                
                // Update packet age metrics
                if max_age > 0 {
                    metrics.ibc_packet_age_unrelayed(
                        &src_chain,
                        &dst_channel,
                        &src_channel,
                        max_age as f64,
                    );
                }
                
                // Also update the legacy stuck packets metric
                metrics.ibc_stuck_packets(
                    &src_chain,
                    &dst_channel,
                    &src_channel,
                    stuck_count,
                );
                
                if stuck_count > 0 {
                    info!(
                        "Found {} stuck packets on channel {} -> {} ({}s old, {} with user data)",
                        stuck_count, src_channel, dst_channel, max_age, with_user_data
                    );
                }
            }
        }
        Err(e) => {
            error!("Error checking for stuck packets: {}", e);
        }
    }
    
    Ok(())
}

// Background task that runs periodically  
pub async fn stuck_packet_monitor(db: SqlitePool, metrics: Metrics) -> Result<()> {
    let mut check_interval = interval(Duration::from_secs(60)); // Check every minute
    
    info!("Starting stuck packet monitor");
    
    loop {
        check_interval.tick().await;
        
        if let Err(e) = check_stuck_packets(&db, &metrics).await {
            error!("Error in stuck packet monitor: {}", e);
        }
    }
}