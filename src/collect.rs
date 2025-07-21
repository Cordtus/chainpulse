use std::time::Duration;

use futures::StreamExt;
use ibc_proto::cosmos::tx::v1beta1::Tx;
use prost::Message as ProstMessage;
use sqlx::SqlitePool;
use tendermint::{
    block::Height,
    chain::{self, Id as ChainId},
    crypto::Sha256,
};
use tendermint_rpc::{event::EventData, WebSocketClientUrl};
use tokio::time;
use tracing::{error, info, warn};

use crate::{
    client::{self, AuthConfig},
    db::{PacketRow, TxRow},
    metrics::Metrics,
    msg::{self, Msg, UniversalPacketInfo},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Pool = SqlitePool;

const NEWBLOCK_TIMEOUT: Duration = Duration::from_secs(60);
const DISCONNECT_AFTER_BLOCKS: usize = 100;

#[derive(Copy, Clone, Debug, thiserror::Error)]
pub enum Outcome {
    #[error("Timeout after {0:?}")]
    Timeout(Duration),

    #[error("Disconnecting after {0} blocks")]
    BlockElapsed(usize),
}

/// Run unified collector with support for all protocol versions
pub async fn run(
    chain_id: chain::Id,
    version: &str,
    ws_url: WebSocketClientUrl,
    username: Option<String>,
    password: Option<String>,
    db: Pool,
    metrics: Metrics,
) -> Result<()> {
    loop {
        let task = collect(
            &chain_id, version, &ws_url, &username, &password, &db, &metrics,
        );

        match task.await {
            Ok(outcome) => warn!("{outcome}"),
            Err(e) => {
                metrics.chainpulse_errors(&chain_id);

                error!("{e}")
            }
        }

        metrics.chainpulse_reconnects(&chain_id);

        info!("Reconnecting in 5 seconds...");
        time::sleep(Duration::from_secs(5)).await;
    }
}

async fn collect(
    chain_id: &chain::Id,
    version: &str,
    ws_url: &WebSocketClientUrl,
    username: &Option<String>,
    password: &Option<String>,
    db: &Pool,
    metrics: &Metrics,
) -> Result<Outcome> {
    // Create appropriate client based on version and auth
    let auth_config = match (username, password) {
        (Some(user), Some(pass)) => Some(AuthConfig {
            username: user.clone(),
            password: pass.clone(),
        }),
        _ => None,
    };

    let client = client::create_client(ws_url, version, auth_config).await?;

    info!("Subscribing to NewBlock events...");
    let mut subscription = client.subscribe_blocks().await?;

    info!("Waiting for new blocks...");

    let mut count: usize = 0;

    loop {
        let next_block = time::timeout(NEWBLOCK_TIMEOUT, subscription.next()).await;
        let next_block = match next_block {
            Ok(next_block) => next_block,
            Err(_) => {
                metrics.chainpulse_timeouts(chain_id);
                return Ok(Outcome::Timeout(NEWBLOCK_TIMEOUT));
            }
        };

        count += 1;

        let Some(Ok(event)) = next_block else {
            continue;
        };

        let EventData::NewBlock { block, .. } = &event.data else {
            continue;
        };

        let Some(block) = block else {
            continue;
        };

        let height = block.header.height;
        info!("New block at height {}", height);

        // Process transactions in the block
        for tx_bytes in &block.data {
            metrics.chainpulse_txs(chain_id);

            let tx = <Tx as ProstMessage>::decode(tx_bytes.as_slice())?;
            let tx_row = insert_tx(db, chain_id, height, &tx).await?;

            let msgs = tx.body.ok_or("missing tx body")?.messages;

            for msg in msgs {
                let type_url = msg.type_url.clone();
                let msg = match Msg::decode(msg) {
                    Ok(msg) => msg,
                    Err(e) => {
                        warn!("Failed to decode message: {e}");
                        continue;
                    }
                };

                if msg.is_ibc() {
                    tracing::debug!("  {}", type_url);

                    if msg.is_relevant() {
                        process_msg(db, chain_id, &tx_row, &type_url, msg, metrics).await?;
                    }
                }
            }
        }

        // Try to get events if the client supports it
        if client.supports_events() {
            match client.get_block_results(height).await {
                Ok(block_results) => {
                    // Process events for enhanced data extraction
                    for (tx_idx, tx_result) in block_results.txs_results.iter().enumerate() {
                        tracing::debug!("TX {} has {} events", tx_idx, tx_result.events.len());
                        
                        // Get the corresponding tx_row if it exists
                        if let Some(tx_bytes) = block.data().iter().nth(tx_idx) {
                            // Decode the transaction
                            let tx = Tx::decode(tx_bytes.as_slice())?;
                            let tx_row = insert_tx(db, chain_id, height, &tx).await?;
                            
                            // Process events for this transaction
                            process_tx_events(db, chain_id, &tx_row, &tx_result.events, metrics).await?;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("Could not fetch block results: {}", e);
                }
            }
        }

        if count >= DISCONNECT_AFTER_BLOCKS {
            return Ok(Outcome::BlockElapsed(count));
        }
    }
}

async fn process_msg(
    pool: &Pool,
    chain_id: &chain::Id,
    tx_row: &TxRow,
    type_url: &str,
    msg: Msg,
    metrics: &Metrics,
) -> Result<()> {
    // Handle MsgTransfer separately since it doesn't have a packet field
    let (packet, packet_info) = if let Some(transfer) = msg.transfer() {
        // For MsgTransfer, we need to track it but we don't have the packet details yet
        // The packet will be created when the recv is processed
        // For now, we'll store the transfer info
        metrics.chainpulse_packets(chain_id);
        
        // MsgTransfer doesn't have sequence number or destination channel
        // We'll need to handle this differently
        return process_transfer(pool, chain_id, tx_row, type_url, transfer, metrics).await;
    } else if let Some(packet) = msg.packet() {
        let packet_info = UniversalPacketInfo::from_packet(packet);
        (packet, packet_info)
    } else {
        return Ok(());
    };

    metrics.chainpulse_packets(chain_id);

    tracing::debug!(
        "    Packet #{} in tx {} ({}) - {}",
        packet.sequence,
        tx_row.id,
        tx_row.hash,
        tx_row.memo
    );

    let query = r#"
        SELECT * FROM packets
        WHERE   src_channel = ? 
            AND src_port = ? 
            AND dst_channel = ? 
            AND dst_port = ? 
            AND sequence = ?
            AND msg_type_url = ?
            LIMIT 1
    "#;

    let existing: Option<PacketRow> = sqlx::query_as(query)
        .bind(&packet.source_channel)
        .bind(&packet.source_port)
        .bind(&packet.destination_channel)
        .bind(&packet.destination_port)
        .bind(packet.sequence as i64)
        .bind(type_url)
        .fetch_optional(pool)
        .await?;

    if let Some(existing) = &existing {
        let effected_tx: TxRow = sqlx::query_as("SELECT * FROM txs WHERE id = ? LIMIT 1")
            .bind(existing.tx_id)
            .fetch_one(pool)
            .await?;

        tracing::debug!(
            "        Frontrun by tx {} ({}) - {}",
            existing.tx_id,
            effected_tx.hash,
            effected_tx.memo
        );

        metrics.ibc_uneffected_packets(
            chain_id,
            &packet.source_channel,
            &packet.source_port,
            &packet.destination_channel,
            &packet.destination_port,
            msg.signer().unwrap_or(""),
            &tx_row.memo,
        );

        metrics.ibc_frontrun_counter(
            chain_id,
            &packet.source_channel,
            &packet.source_port,
            &packet.destination_channel,
            &packet.destination_port,
            msg.signer().unwrap_or(""),
            &existing.signer,
            &tx_row.memo,
            &effected_tx.memo,
        );
    } else {
        metrics.ibc_effected_packets(
            chain_id,
            &packet.source_channel,
            &packet.source_port,
            &packet.destination_channel,
            &packet.destination_port,
            msg.signer().unwrap_or(""),
            &tx_row.memo,
        );
    }

    let query = r#"
        INSERT OR IGNORE INTO packets
            (tx_id, sequence, src_channel, src_port, dst_channel, dst_port,
            msg_type_url, signer, effected, effected_signer, effected_tx, 
            sender, receiver, denom, amount, ibc_version,
            timeout_timestamp, timeout_height_revision_number, timeout_height_revision_height,
            data_hash, created_at)
        VALUES
            (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
    "#;

    sqlx::query(query)
        .bind(tx_row.id)
        .bind(packet.sequence as i64)
        .bind(&packet.source_channel)
        .bind(&packet.source_port)
        .bind(&packet.destination_channel)
        .bind(&packet.destination_port)
        .bind(type_url)
        .bind(msg.signer())
        .bind(existing.is_none())
        .bind(existing.as_ref().map(|row| &row.signer))
        .bind(existing.as_ref().map(|row| row.tx_id))
        .bind(&packet_info.sender)
        .bind(&packet_info.receiver)
        .bind(&packet_info.denom)
        .bind(&packet_info.amount)
        .bind(&packet_info.ibc_version)
        .bind(packet_info.timeout_timestamp.map(|ts| ts as i64))
        .bind(packet_info.timeout_height.as_ref().map(|h| h.revision_number as i64))
        .bind(packet_info.timeout_height.as_ref().map(|h| h.revision_height as i64))
        .bind(&packet_info.data_hash)
        .execute(pool)
        .await?;

    Ok(())
}

async fn process_tx_events(
    pool: &Pool,
    chain_id: &chain::Id,
    tx_row: &TxRow,
    events: &[client::TxEvent],
    metrics: &Metrics,
) -> Result<()> {
    for event in events {
        match event.type_str.as_str() {
            "send_packet" => {
                process_send_packet_event(pool, chain_id, tx_row, event, metrics).await?;
            }
            "recv_packet" => {
                process_recv_packet_event(pool, chain_id, tx_row, event, metrics).await?;
            }
            "acknowledge_packet" => {
                process_acknowledge_packet_event(pool, chain_id, tx_row, event, metrics).await?;
            }
            "timeout_packet" => {
                process_timeout_packet_event(pool, chain_id, tx_row, event, metrics).await?;
            }
            _ => {
                // Skip other events
            }
        }
    }
    Ok(())
}

async fn process_send_packet_event(
    pool: &Pool,
    chain_id: &chain::Id,
    tx_row: &TxRow,
    event: &client::TxEvent,
    metrics: &Metrics,
) -> Result<()> {
    // Extract packet info from send_packet event
    let mut packet_data = std::collections::HashMap::new();
    for attr in &event.attributes {
        packet_data.insert(attr.key.as_str(), attr.value.as_str());
    }
    
    let sequence = packet_data.get("packet_sequence")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let src_channel = packet_data.get("packet_src_channel").unwrap_or(&"").to_string();
    let src_port = packet_data.get("packet_src_port").unwrap_or(&"").to_string();
    let dst_channel = packet_data.get("packet_dst_channel").unwrap_or(&"").to_string();
    let dst_port = packet_data.get("packet_dst_port").unwrap_or(&"").to_string();
    let _timeout_height = packet_data.get("packet_timeout_height").unwrap_or(&"").to_string();
    let timeout_timestamp = packet_data.get("packet_timeout_timestamp")
        .and_then(|s| s.parse::<i64>().ok());
    
    // Get packet data if available
    let packet_data_hex = packet_data.get("packet_data").unwrap_or(&"");
    let (sender, receiver, amount, denom) = if src_port == "transfer" && !packet_data_hex.is_empty() {
        // Try to decode the packet data as hex
        if let Ok(data_bytes) = subtle_encoding::hex::decode(packet_data_hex) {
            if let Ok(ft_data) = serde_json::from_slice::<msg::FungibleTokenPacketData>(&data_bytes) {
                (Some(ft_data.sender), Some(ft_data.receiver), Some(ft_data.amount), Some(ft_data.denom))
            } else {
                (None, None, None, None)
            }
        } else {
            (None, None, None, None)
        }
    } else {
        (None, None, None, None)
    };
    
    tracing::debug!(
        "    SendPacket event: seq {} on channel {} -> {}",
        sequence, src_channel, dst_channel
    );
    
    metrics.chainpulse_packets(chain_id);
    
    // Insert as a packet with special msg_type_url to indicate it's from an event
    let query = r#"
        INSERT OR IGNORE INTO packets
            (tx_id, sequence, src_channel, src_port, dst_channel, dst_port,
            msg_type_url, signer, effected, sender, receiver, denom, amount, 
            timeout_timestamp, data_hash, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
    "#;
    
    sqlx::query(query)
        .bind(tx_row.id)
        .bind(sequence)
        .bind(&src_channel)
        .bind(&src_port)
        .bind(&dst_channel)
        .bind(&dst_port)
        .bind("send_packet")  // Special marker for send_packet events
        .bind("")  // No signer for events
        .bind(0)  // Not effected yet
        .bind(&sender)
        .bind(&receiver)
        .bind(&denom)
        .bind(&amount)
        .bind(timeout_timestamp)
        .bind(packet_data_hex)
        .execute(pool)
        .await?;
    
    Ok(())
}

async fn process_recv_packet_event(
    _pool: &Pool,
    _chain_id: &chain::Id,
    _tx_row: &TxRow,
    event: &client::TxEvent,
    _metrics: &Metrics,
) -> Result<()> {
    // recv_packet events are redundant with MsgRecvPacket messages
    // but we can log them for debugging
    let mut packet_data = std::collections::HashMap::new();
    for attr in &event.attributes {
        packet_data.insert(attr.key.as_str(), attr.value.as_str());
    }
    
    let sequence = packet_data.get("packet_sequence").unwrap_or(&"");
    let src_channel = packet_data.get("packet_src_channel").unwrap_or(&"");
    
    tracing::debug!(
        "    RecvPacket event: seq {} on channel {}",
        sequence, src_channel
    );
    
    Ok(())
}

async fn process_acknowledge_packet_event(
    pool: &Pool,
    _chain_id: &chain::Id,
    tx_row: &TxRow,
    event: &client::TxEvent,
    _metrics: &Metrics,
) -> Result<()> {
    // Extract packet info from acknowledge_packet event
    let mut packet_data = std::collections::HashMap::new();
    for attr in &event.attributes {
        packet_data.insert(attr.key.as_str(), attr.value.as_str());
    }
    
    let sequence = packet_data.get("packet_sequence")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let src_channel = packet_data.get("packet_src_channel").unwrap_or(&"").to_string();
    let _src_port = packet_data.get("packet_src_port").unwrap_or(&"").to_string();
    let dst_channel = packet_data.get("packet_dst_channel").unwrap_or(&"").to_string();
    let _dst_port = packet_data.get("packet_dst_port").unwrap_or(&"").to_string();
    
    tracing::debug!(
        "    AckPacket event: seq {} on channel {} -> {} acknowledged",
        sequence, src_channel, dst_channel
    );
    
    // Update the send_packet record to mark it as acknowledged
    let query = r#"
        UPDATE packets 
        SET effected = 1, effected_tx = ?
        WHERE sequence = ? AND src_channel = ? AND dst_channel = ? 
          AND msg_type_url = 'send_packet'
    "#;
    
    sqlx::query(query)
        .bind(tx_row.id)
        .bind(sequence)
        .bind(&src_channel)
        .bind(&dst_channel)
        .execute(pool)
        .await?;
    
    Ok(())
}

async fn process_timeout_packet_event(
    pool: &Pool,
    _chain_id: &chain::Id,
    tx_row: &TxRow,
    event: &client::TxEvent,
    _metrics: &Metrics,
) -> Result<()> {
    // Extract packet info from timeout_packet event
    let mut packet_data = std::collections::HashMap::new();
    for attr in &event.attributes {
        packet_data.insert(attr.key.as_str(), attr.value.as_str());
    }
    
    let sequence = packet_data.get("packet_sequence")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let src_channel = packet_data.get("packet_src_channel").unwrap_or(&"").to_string();
    let _src_port = packet_data.get("packet_src_port").unwrap_or(&"").to_string();
    let dst_channel = packet_data.get("packet_dst_channel").unwrap_or(&"").to_string();
    let _dst_port = packet_data.get("packet_dst_port").unwrap_or(&"").to_string();
    
    tracing::debug!(
        "    TimeoutPacket event: seq {} on channel {} -> {} timed out",
        sequence, src_channel, dst_channel
    );
    
    // Update the send_packet record to mark it as timed out
    let query = r#"
        UPDATE packets 
        SET effected = 1, effected_tx = ?, msg_type_url = 'timeout_packet'
        WHERE sequence = ? AND src_channel = ? AND dst_channel = ? 
          AND msg_type_url = 'send_packet'
    "#;
    
    sqlx::query(query)
        .bind(tx_row.id)
        .bind(sequence)
        .bind(&src_channel)
        .bind(&dst_channel)
        .execute(pool)
        .await?;
    
    Ok(())
}

async fn process_transfer(
    _pool: &Pool,
    chain_id: &chain::Id,
    tx_row: &TxRow,
    _type_url: &str,
    transfer: &ibc_proto::ibc::apps::transfer::v1::MsgTransfer,
    metrics: &Metrics,
) -> Result<()> {
    // MsgTransfer represents the initiation of a transfer on the source chain
    // We don't have a sequence number yet (that's assigned by the chain)
    // But we can track this as the start of a packet flow
    
    tracing::debug!(
        "    Transfer from {} on channel {} in tx {} ({})",
        transfer.sender,
        transfer.source_channel,
        tx_row.id,
        tx_row.hash
    );
    
    // For now, we'll log MsgTransfer but not insert it into packets table
    // since we don't have sequence number or destination channel info
    // We could potentially create a separate transfers table to track these
    
    // TODO: Consider creating a transfers table to track MsgTransfer messages
    // and correlate them with subsequent RecvPacket messages
    
    metrics.chainpulse_packets(chain_id);
    
    Ok(())
}

async fn insert_tx(db: &Pool, chain_id: &ChainId, height: Height, tx: &Tx) -> Result<TxRow> {
    let query = r#"
        INSERT OR IGNORE INTO txs (chain, height, hash, memo, created_at)
        VALUES (?, ?, ?, ?, datetime('now'))
    "#;

    let bytes = tx.encode_to_vec();
    let hash = tendermint::crypto::default::Sha256::digest(&bytes);
    let hash = subtle_encoding::hex::encode_upper(hash);
    let hash = String::from_utf8_lossy(&hash);

    let height = height.value() as i64;

    let memo = tx
        .body
        .as_ref()
        .map(|body| body.memo.to_string())
        .unwrap_or_default();

    sqlx::query(query)
        .bind(chain_id.as_str())
        .bind(height)
        .bind(&hash)
        .bind(memo)
        .execute(db)
        .await?;

    let query = r#"
        SELECT * FROM txs WHERE chain = ? AND hash = ? LIMIT 1
    "#;

    let tx = sqlx::query_as(query)
        .bind(chain_id.as_str())
        .bind(&hash)
        .fetch_one(db)
        .await?;

    Ok(tx)
}