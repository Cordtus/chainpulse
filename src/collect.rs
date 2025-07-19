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
    msg::{Msg, UniversalPacketInfo},
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

        // Try to get events if the client supports it and we're on v0.38
        if client.supports_events() && version == "0.38" {
            match client.get_block_results(height).await {
                Ok(block_results) => {
                    // Process events for enhanced data extraction
                    for (tx_idx, tx_result) in block_results.txs_results.iter().enumerate() {
                        tracing::debug!("TX {} has {} events", tx_idx, tx_result.events.len());
                        // TODO: Process events for additional packet data
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
    let Some(packet) = msg.packet() else {
        return Ok(());
    };

    metrics.chainpulse_packets(chain_id);

    let packet_info = UniversalPacketInfo::from_packet(packet);

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