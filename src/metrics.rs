use std::{collections::HashMap, net::SocketAddr};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::get,
    Router, Server,
};
use prometheus::{
    register_gauge_vec_with_registry, register_int_counter_vec_with_registry,
    register_int_gauge_vec_with_registry, Encoder, GaugeVec as PrometheusGaugeVec, IntCounterVec,
    IntGaugeVec, Registry, TextEncoder,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tendermint::chain;
use tracing::info;

type GaugeVec = IntGaugeVec;
type CounterVec = IntCounterVec;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
pub struct Metrics {
    /// The number of IBC packets that are effected
    /// Labels: ['chain_id', 'src_channel', 'src_port', 'dst_channel', 'dst_port', 'signer', 'memo']
    ibc_effected_packets: CounterVec,

    /// The number of IBC packets that are not effected
    /// Labels: ['chain_id', 'src_channel', 'src_port', 'dst_channel', 'dst_port', 'signer', 'memo']
    ibc_uneffected_packets: CounterVec,

    /// The number of times a signer gets frontrun by the original signer
    /// Labels: ['chain_id', 'src_channel', 'src_port', 'dst_channel', 'dst_port', 'signer', 'frontrunned_by', 'memo', 'effected_memo']
    ibc_frontrun_counter: CounterVec,

    /// The number of stuck packets on an IBC channel
    /// Labels: ['src_chain', 'dst_chain', 'src_channel']
    ibc_stuck_packets: GaugeVec,

    /// The number of chains being monitored
    chainpulse_chains: GaugeVec,

    /// The number of txs processed
    /// Labels: ['chain_id']
    chainpulse_txs: CounterVec,

    /// The number of packets processed
    /// Labels: ['chain_id']
    chainpulse_packets: CounterVec,

    /// The number of times we had to reconnect to the WebSocket
    /// Labels: ['chain_id']
    chainpulse_reconnects: CounterVec,

    /// The number of times the WebSocket connection timed out
    /// Labels: 'chain_id']
    chainpulse_timeouts: CounterVec,

    /// The number of times we encountered an error
    /// Labels: ['chain_id']
    chainpulse_errors: CounterVec,

    /// Detailed stuck packet tracking with user info
    /// Labels: ['src_chain', 'dst_chain', 'src_channel', 'dst_channel', 'has_user_data']
    ibc_stuck_packets_detailed: GaugeVec,

    /// Time since packet creation for unrelayed packets
    /// Labels: ['src_chain', 'dst_chain', 'channel']
    ibc_packet_age_unrelayed: PrometheusGaugeVec,

    /// Packets nearing timeout
    /// Labels: ['src_chain', 'dst_chain', 'src_channel', 'dst_channel', 'timeout_type']
    ibc_packets_near_timeout: GaugeVec,
    
    /// Time until packet timeout in seconds
    /// Labels: ['src_chain', 'dst_chain', 'src_channel', 'dst_channel']
    ibc_packet_timeout_seconds: PrometheusGaugeVec,
}

impl Metrics {
    pub fn new() -> (Self, Registry) {
        let registry = Registry::new();

        let ibc_effected_packets = register_int_counter_vec_with_registry!(
            "ibc_effected_packets",
            "The number of IBC packets that have been relayed and were effected",
            &[
                "chain_id",
                "src_channel",
                "src_port",
                "dst_channel",
                "dst_port",
                "signer",
                "memo",
            ],
            registry,
        )
        .unwrap();

        let ibc_uneffected_packets = register_int_counter_vec_with_registry!(
            "ibc_uneffected_packets",
            "The number of IBC packets that were relayed but not effected",
            &[
                "chain_id",
                "src_channel",
                "src_port",
                "dst_channel",
                "dst_port",
                "signer",
                "memo"
            ],
            registry
        )
        .unwrap();

        let ibc_frontrun_counter = register_int_counter_vec_with_registry!(
            "ibc_frontrun_counter",
            "The number of times a signer gets frontrun by the original signer",
            &[
                "chain_id",
                "src_channel",
                "src_port",
                "dst_channel",
                "dst_port",
                "signer",
                "frontrunned_by",
                "memo",
                "effected_memo"
            ],
            registry
        )
        .unwrap();

        let ibc_stuck_packets = register_int_gauge_vec_with_registry!(
            "ibc_stuck_packets",
            "The number of packets stuck on an IBC channel",
            &["src_chain", "dst_chain", "src_channel"],
            registry
        )
        .unwrap();

        let chainpulse_chains = register_int_gauge_vec_with_registry!(
            "chainpulse_chains",
            "The number of chains being monitored",
            &[],
            registry
        )
        .unwrap();

        let chainpulse_txs = register_int_counter_vec_with_registry!(
            "chainpulse_txs",
            "The number of txs processed",
            &["chain_id"],
            registry
        )
        .unwrap();

        let chainpulse_packets = register_int_counter_vec_with_registry!(
            "chainpulse_packets",
            "The number of packets processed",
            &["chain_id"],
            registry
        )
        .unwrap();

        let chainpulse_reconnects = register_int_counter_vec_with_registry!(
            "chainpulse_reconnects",
            "The number of times we had to reconnect to the WebSocket",
            &["chain_id"],
            registry
        )
        .unwrap();

        let chainpulse_timeouts = register_int_counter_vec_with_registry!(
            "chainpulse_timeouts",
            "The number of times the WebSocket connection timed out",
            &["chain_id"],
            registry
        )
        .unwrap();

        let chainpulse_errors = register_int_counter_vec_with_registry!(
            "chainpulse_errors",
            "The number of times an error was encountered",
            &["chain_id"],
            registry
        )
        .unwrap();

        let ibc_stuck_packets_detailed = register_int_gauge_vec_with_registry!(
            "ibc_stuck_packets_detailed",
            "Detailed stuck packet tracking with user info",
            &[
                "src_chain",
                "dst_chain",
                "src_channel",
                "dst_channel",
                "has_user_data"
            ],
            registry
        )
        .unwrap();

        let ibc_packet_age_unrelayed = register_gauge_vec_with_registry!(
            "ibc_packet_age_seconds",
            "Age of unrelayed packets in seconds",
            &["src_chain", "dst_chain", "channel"],
            registry
        )
        .unwrap();

        let ibc_packets_near_timeout = register_int_gauge_vec_with_registry!(
            "ibc_packets_near_timeout",
            "Number of packets nearing their timeout deadline",
            &["src_chain", "dst_chain", "src_channel", "dst_channel", "timeout_type"],
            registry
        )
        .unwrap();
        
        let ibc_packet_timeout_seconds = register_gauge_vec_with_registry!(
            "ibc_packet_timeout_seconds",
            "Time until packet timeout in seconds (negative if already expired)",
            &["src_chain", "dst_chain", "src_channel", "dst_channel"],
            registry
        )
        .unwrap();

        (
            Self {
                ibc_effected_packets,
                ibc_uneffected_packets,
                ibc_frontrun_counter,
                ibc_stuck_packets,
                chainpulse_chains,
                chainpulse_txs,
                chainpulse_packets,
                chainpulse_reconnects,
                chainpulse_timeouts,
                chainpulse_errors,
                ibc_stuck_packets_detailed,
                ibc_packet_age_unrelayed,
                ibc_packets_near_timeout,
                ibc_packet_timeout_seconds,
            },
            registry,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ibc_effected_packets(
        &self,
        chain_id: &chain::Id,
        src_channel: &str,
        src_port: &str,
        dst_channel: &str,
        dst_port: &str,
        signer: &str,
        memo: &str,
    ) {
        self.ibc_effected_packets
            .with_label_values(&[
                chain_id.as_ref(),
                src_channel,
                src_port,
                dst_channel,
                dst_port,
                signer,
                memo,
            ])
            .inc();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ibc_uneffected_packets(
        &self,
        chain_id: &chain::Id,
        src_channel: &str,
        src_port: &str,
        dst_channel: &str,
        dst_port: &str,
        signer: &str,
        memo: &str,
    ) {
        self.ibc_uneffected_packets
            .with_label_values(&[
                chain_id.as_ref(),
                src_channel,
                src_port,
                dst_channel,
                dst_port,
                signer,
                memo,
            ])
            .inc();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ibc_frontrun_counter(
        &self,
        chain_id: &chain::Id,
        src_channel: &str,
        src_port: &str,
        dst_channel: &str,
        dst_port: &str,
        signer: &str,
        frontrunned_by: &str,
        memo: &str,
        effected_memo: &str,
    ) {
        self.ibc_frontrun_counter
            .with_label_values(&[
                chain_id.as_ref(),
                src_channel,
                src_port,
                dst_channel,
                dst_port,
                signer,
                frontrunned_by,
                memo,
                effected_memo,
            ])
            .inc();
    }

    pub fn ibc_stuck_packets(
        &self,
        src_chain: &str,
        dst_chain: &str,
        src_channel: &str,
        value: i64,
    ) {
        self.ibc_stuck_packets
            .with_label_values(&[src_chain, dst_chain, src_channel])
            .set(value);
    }

    pub fn chainpulse_chains(&self) {
        self.chainpulse_chains.with_label_values(&[]).inc();
    }

    pub fn chainpulse_txs(&self, chain_id: &chain::Id) {
        self.chainpulse_txs
            .with_label_values(&[chain_id.as_ref()])
            .inc();
    }

    pub fn chainpulse_packets(&self, chain_id: &chain::Id) {
        self.chainpulse_packets
            .with_label_values(&[chain_id.as_ref()])
            .inc();
    }

    pub fn chainpulse_reconnects(&self, chain_id: &chain::Id) {
        self.chainpulse_reconnects
            .with_label_values(&[chain_id.as_ref()])
            .inc();
    }

    pub fn chainpulse_timeouts(&self, chain_id: &chain::Id) {
        self.chainpulse_timeouts
            .with_label_values(&[chain_id.as_ref()])
            .inc();
    }

    pub fn chainpulse_errors(&self, chain_id: &chain::Id) {
        self.chainpulse_errors
            .with_label_values(&[chain_id.as_ref()])
            .inc();
    }

    pub fn ibc_stuck_packets_detailed(
        &self,
        src_chain: &str,
        dst_chain: &str,
        src_channel: &str,
        dst_channel: &str,
        has_user_data: bool,
        value: i64,
    ) {
        self.ibc_stuck_packets_detailed
            .with_label_values(&[
                src_chain,
                dst_chain,
                src_channel,
                dst_channel,
                if has_user_data { "true" } else { "false" },
            ])
            .set(value);
    }

    pub fn ibc_packet_age_unrelayed(
        &self,
        src_chain: &str,
        dst_chain: &str,
        channel: &str,
        age_seconds: f64,
    ) {
        self.ibc_packet_age_unrelayed
            .with_label_values(&[src_chain, dst_chain, channel])
            .set(age_seconds);
    }

    pub fn ibc_packets_near_timeout(
        &self,
        src_chain: &str,
        dst_chain: &str,
        src_channel: &str,
        dst_channel: &str,
        timeout_type: &str,
        count: i64,
    ) {
        self.ibc_packets_near_timeout
            .with_label_values(&[src_chain, dst_chain, src_channel, dst_channel, timeout_type])
            .set(count);
    }

    pub fn ibc_packet_timeout_seconds(
        &self,
        src_chain: &str,
        dst_chain: &str,
        src_channel: &str,
        dst_channel: &str,
        seconds_until_timeout: f64,
    ) {
        self.ibc_packet_timeout_seconds
            .with_label_values(&[src_chain, dst_chain, src_channel, dst_channel])
            .set(seconds_until_timeout);
    }
}

pub async fn run(port: u16, registry: Registry, db: SqlitePool) -> Result<()> {
    let state = ApiState { registry, db };

    let app = Router::new()
        .route("/metrics", get(get_metrics))
        .route("/api/v1/packets/by-user", get(get_packets_by_user))
        .route("/api/v1/packets/stuck", get(get_stuck_packets))
        .route(
            "/api/v1/packets/:chain/:channel/:sequence",
            get(get_packet_details),
        )
        .route("/api/v1/channels/congestion", get(get_channel_congestion))
        .route("/api/v1/packets/expiring", get(get_expiring_packets))
        .route("/api/v1/packets/expired", get(get_expired_packets))
        .route("/api/v1/packets/duplicates", get(get_duplicate_packets))
        .with_state(state);

    let server =
        Server::bind(&SocketAddr::from(([0, 0, 0, 0], port))).serve(app.into_make_service());

    info!("Metrics server listening at http://localhost:{port}/metrics");
    server.await?;

    Ok(())
}

pub async fn get_metrics(State(state): State<ApiState>) -> String {
    let mut buffer = vec![];
    let encoder = TextEncoder::new();

    let metric_families = state.registry.gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();

    String::from_utf8(buffer).unwrap()
}

// API State and types
#[derive(Clone)]
struct ApiState {
    registry: Registry,
    db: SqlitePool,
}

#[derive(Debug, Deserialize)]
struct UserPacketsQuery {
    address: String,
    #[serde(default)]
    role: String, // sender, receiver, both (default)
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Serialize)]
struct UserPacketsResponse {
    packets: Vec<PacketInfo>,
    total: i64,
    api_version: String,
}

#[derive(Debug, Serialize)]
struct PacketInfo {
    chain_id: String,
    sequence: i64,
    src_channel: String,
    dst_channel: String,
    sender: Option<String>,
    receiver: Option<String>,
    amount: Option<String>,
    denom: Option<String>,
    age_seconds: i64,
    relay_attempts: i64,
    last_attempt_by: Option<String>,
    ibc_version: String,
}

#[derive(Debug, Deserialize)]
struct StuckPacketsQuery {
    #[serde(default = "default_min_age")]
    min_age_seconds: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_min_age() -> i64 {
    900 // 15 minutes
}

#[derive(Debug, Serialize)]
struct StuckPacketsResponse {
    packets: Vec<PacketInfo>,
    total: i64,
    api_version: String,
}

#[derive(Debug, Serialize)]
struct ChannelCongestionResponse {
    channels: Vec<ChannelCongestion>,
    api_version: String,
}

#[derive(Debug, Serialize)]
struct ChannelCongestion {
    src_channel: String,
    dst_channel: String,
    stuck_count: i64,
    oldest_stuck_age_seconds: Option<i64>,
    total_value: HashMap<String, String>,
}

// API Handlers
async fn get_packets_by_user(
    State(state): State<ApiState>,
    Query(params): Query<UserPacketsQuery>,
) -> std::result::Result<Json<UserPacketsResponse>, StatusCode> {
    // Validate address format (basic check)
    if params.address.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let role_condition = match params.role.as_str() {
        "sender" => "sender = ?",
        "receiver" => "receiver = ?",
        _ => "(sender = ? OR receiver = ?)",
    };

    // Build query to get packets
    let query = format!(
        r#"
        SELECT 
            t.chain as chain_id,
            p.sequence,
            p.src_channel,
            p.dst_channel,
            p.sender,
            p.receiver,
            p.amount,
            p.denom,
            p.ibc_version,
            p.signer as last_attempt_by,
            p.effected,
            CAST((strftime('%s', 'now') - strftime('%s', p.created_at)) AS INTEGER) as age_seconds,
            (SELECT COUNT(*) FROM packets p2 WHERE p2.src_channel = p.src_channel 
             AND p2.dst_channel = p.dst_channel AND p2.sequence = p.sequence) as relay_attempts
        FROM packets p
        JOIN txs t ON p.tx_id = t.id
        WHERE {}
        ORDER BY p.created_at DESC
        LIMIT ? OFFSET ?
        "#,
        role_condition
    );

    let packets = if params.role == "sender" || params.role == "receiver" {
        sqlx::query_as::<
            _,
            (
                String,
                i64,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
                bool,
                i64,
                i64,
            ),
        >(&query)
        .bind(&params.address)
        .bind(params.limit)
        .bind(params.offset)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as::<
            _,
            (
                String,
                i64,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
                bool,
                i64,
                i64,
            ),
        >(&query)
        .bind(&params.address)
        .bind(&params.address)
        .bind(params.limit)
        .bind(params.offset)
        .fetch_all(&state.db)
        .await
    };

    match packets {
        Ok(rows) => {
            let packets: Vec<PacketInfo> = rows
                .into_iter()
                .map(|row| PacketInfo {
                    chain_id: row.0,
                    sequence: row.1,
                    src_channel: row.2,
                    dst_channel: row.3,
                    sender: row.4,
                    receiver: row.5,
                    amount: row.6,
                    denom: row.7,
                    ibc_version: row.8.unwrap_or_else(|| "v1".to_string()),
                    last_attempt_by: Some(row.9),
                    age_seconds: row.11,
                    relay_attempts: row.12,
                })
                .collect();

            let total = packets.len() as i64;

            Ok(Json(UserPacketsResponse {
                packets,
                total,
                api_version: "1.0".to_string(),
            }))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_stuck_packets(
    State(state): State<ApiState>,
    Query(params): Query<StuckPacketsQuery>,
) -> std::result::Result<Json<StuckPacketsResponse>, StatusCode> {
    let query = r#"
        SELECT 
            t.chain as chain_id,
            p.sequence,
            p.src_channel,
            p.dst_channel,
            p.sender,
            p.receiver,
            p.amount,
            p.denom,
            p.ibc_version,
            p.signer as last_attempt_by,
            CAST((strftime('%s', 'now') - strftime('%s', p.created_at)) AS INTEGER) as age_seconds,
            (SELECT COUNT(*) FROM packets p2 WHERE p2.src_channel = p.src_channel 
             AND p2.dst_channel = p.dst_channel AND p2.sequence = p.sequence) as relay_attempts
        FROM packets p
        JOIN txs t ON p.tx_id = t.id
        WHERE p.effected = 0 
          AND CAST((strftime('%s', 'now') - strftime('%s', p.created_at)) AS INTEGER) > ?
        ORDER BY p.created_at ASC
        LIMIT ?
    "#;

    match sqlx::query_as::<
        _,
        (
            String,
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            i64,
            i64,
        ),
    >(query)
    .bind(params.min_age_seconds)
    .bind(params.limit)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            let packets: Vec<PacketInfo> = rows
                .into_iter()
                .map(|row| PacketInfo {
                    chain_id: row.0,
                    sequence: row.1,
                    src_channel: row.2,
                    dst_channel: row.3,
                    sender: row.4,
                    receiver: row.5,
                    amount: row.6,
                    denom: row.7,
                    ibc_version: row.8.unwrap_or_else(|| "v1".to_string()),
                    last_attempt_by: Some(row.9),
                    age_seconds: row.10,
                    relay_attempts: row.11,
                })
                .collect();

            let total = packets.len() as i64;

            Ok(Json(StuckPacketsResponse {
                packets,
                total,
                api_version: "1.0".to_string(),
            }))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_packet_details(
    State(state): State<ApiState>,
    Path((chain, channel, sequence)): Path<(String, String, i64)>,
) -> std::result::Result<Json<PacketInfo>, StatusCode> {
    let query = r#"
        SELECT 
            t.chain as chain_id,
            p.sequence,
            p.src_channel,
            p.dst_channel,
            p.sender,
            p.receiver,
            p.amount,
            p.denom,
            p.ibc_version,
            p.signer as last_attempt_by,
            p.effected,
            CAST((strftime('%s', 'now') - strftime('%s', p.created_at)) AS INTEGER) as age_seconds,
            (SELECT COUNT(*) FROM packets p2 WHERE p2.src_channel = p.src_channel 
             AND p2.dst_channel = p.dst_channel AND p2.sequence = p.sequence) as relay_attempts
        FROM packets p
        JOIN txs t ON p.tx_id = t.id
        WHERE t.chain = ? AND p.src_channel = ? AND p.sequence = ?
        LIMIT 1
    "#;

    match sqlx::query_as::<
        _,
        (
            String,
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            bool,
            i64,
            i64,
        ),
    >(query)
    .bind(chain)
    .bind(channel)
    .bind(sequence)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => Ok(Json(PacketInfo {
            chain_id: row.0,
            sequence: row.1,
            src_channel: row.2,
            dst_channel: row.3,
            sender: row.4,
            receiver: row.5,
            amount: row.6,
            denom: row.7,
            ibc_version: row.8.unwrap_or_else(|| "v1".to_string()),
            last_attempt_by: Some(row.9),
            age_seconds: row.11,
            relay_attempts: row.12,
        })),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_channel_congestion(
    State(state): State<ApiState>,
) -> std::result::Result<Json<ChannelCongestionResponse>, StatusCode> {
    let query = r#"
        SELECT 
            p.src_channel,
            p.dst_channel,
            COUNT(*) as stuck_count,
            MIN(CAST((strftime('%s', 'now') - strftime('%s', p.created_at)) AS INTEGER)) as oldest_stuck_age,
            GROUP_CONCAT(DISTINCT p.denom || ':' || p.amount) as amounts
        FROM packets p
        WHERE p.effected = 0 
          AND CAST((strftime('%s', 'now') - strftime('%s', p.created_at)) AS INTEGER) > 900
        GROUP BY p.src_channel, p.dst_channel
        ORDER BY stuck_count DESC
    "#;

    match sqlx::query_as::<_, (String, String, i64, Option<i64>, Option<String>)>(query)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => {
            let channels: Vec<ChannelCongestion> = rows
                .into_iter()
                .map(|row| {
                    let mut total_value = HashMap::new();
                    if let Some(amounts) = row.4 {
                        for amount_str in amounts.split(',') {
                            if let Some((denom, amount)) = amount_str.split_once(':') {
                                total_value
                                    .entry(denom.to_string())
                                    .and_modify(|e: &mut String| {
                                        if let (Ok(existing), Ok(new)) =
                                            (e.parse::<f64>(), amount.parse::<f64>())
                                        {
                                            *e = (existing + new).to_string();
                                        }
                                    })
                                    .or_insert(amount.to_string());
                            }
                        }
                    }

                    ChannelCongestion {
                        src_channel: row.0,
                        dst_channel: row.1,
                        stuck_count: row.2,
                        oldest_stuck_age_seconds: row.3,
                        total_value,
                    }
                })
                .collect();

            Ok(Json(ChannelCongestionResponse {
                channels,
                api_version: "1.0".to_string(),
            }))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// Timeout-based query endpoints

#[derive(Debug, Deserialize)]
struct ExpiringPacketsQuery {
    #[serde(default = "default_expiring_minutes")]
    minutes: i64,
}

fn default_expiring_minutes() -> i64 {
    60 // 1 hour default
}

#[derive(Debug, Serialize)]
struct ExpiringPacketsResponse {
    packets: Vec<ExpiringPacketInfo>,
    api_version: String,
}

#[derive(Debug, Serialize)]
struct ExpiringPacketInfo {
    chain_id: String,
    sequence: i64,
    src_channel: String,
    dst_channel: String,
    sender: Option<String>,
    receiver: Option<String>,
    amount: Option<String>,
    denom: Option<String>,
    seconds_until_timeout: i64,
    timeout_type: String,
    timeout_value: String,
}

async fn get_expiring_packets(
    State(state): State<ApiState>,
    Query(params): Query<ExpiringPacketsQuery>,
) -> std::result::Result<Json<ExpiringPacketsResponse>, StatusCode> {
    let query = r#"
        SELECT 
            t.chain,
            p.sequence,
            p.src_channel,
            p.dst_channel,
            p.sender,
            p.receiver,
            p.amount,
            p.denom,
            p.timeout_timestamp,
            p.timeout_height_revision_number,
            p.timeout_height_revision_height,
            (p.timeout_timestamp - strftime('%s', 'now') * 1000000000) / 1000000000 as seconds_until_timeout
        FROM packets p
        JOIN txs t ON p.tx_id = t.id
        WHERE p.effected = 0 
          AND p.timeout_timestamp IS NOT NULL
          AND p.timeout_timestamp > strftime('%s', 'now') * 1000000000
          AND p.timeout_timestamp < (strftime('%s', 'now') + ? * 60) * 1000000000
        ORDER BY p.timeout_timestamp ASC
        LIMIT 100
    "#;

    match sqlx::query(query)
        .bind(params.minutes)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => {
            let packets = rows
                .into_iter()
                .map(|row| {
                    let timeout_type = if row.get::<Option<i64>, _>(9).is_some() {
                        "height".to_string()
                    } else {
                        "timestamp".to_string()
                    };
                    
                    let timeout_value = if timeout_type == "height" {
                        format!("{}-{}", 
                            row.get::<Option<i64>, _>(9).unwrap_or(0),
                            row.get::<Option<i64>, _>(10).unwrap_or(0)
                        )
                    } else {
                        let ts = row.get::<Option<i64>, _>(8).unwrap_or(0);
                        // Convert nanoseconds to ISO timestamp
                        let secs = ts / 1_000_000_000;
                        chrono::DateTime::from_timestamp(secs, 0)
                            .map(|dt| dt.to_rfc3339())
                            .unwrap_or_else(|| ts.to_string())
                    };

                    ExpiringPacketInfo {
                        chain_id: row.get(0),
                        sequence: row.get(1),
                        src_channel: row.get(2),
                        dst_channel: row.get(3),
                        sender: row.get(4),
                        receiver: row.get(5),
                        amount: row.get(6),
                        denom: row.get(7),
                        seconds_until_timeout: row.get(11),
                        timeout_type,
                        timeout_value,
                    }
                })
                .collect();

            Ok(Json(ExpiringPacketsResponse {
                packets,
                api_version: "1.0".to_string(),
            }))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Debug, Serialize)]
struct ExpiredPacketsResponse {
    packets: Vec<ExpiredPacketInfo>,
    api_version: String,
}

#[derive(Debug, Serialize)]
struct ExpiredPacketInfo {
    chain_id: String,
    sequence: i64,
    src_channel: String,
    dst_channel: String,
    sender: Option<String>,
    receiver: Option<String>,
    amount: Option<String>,
    denom: Option<String>,
    seconds_since_timeout: i64,
    timeout_type: String,
}

async fn get_expired_packets(
    State(state): State<ApiState>,
) -> std::result::Result<Json<ExpiredPacketsResponse>, StatusCode> {
    let query = r#"
        SELECT 
            t.chain,
            p.sequence,
            p.src_channel,
            p.dst_channel,
            p.sender,
            p.receiver,
            p.amount,
            p.denom,
            p.timeout_timestamp,
            p.timeout_height_revision_number,
            p.timeout_height_revision_height,
            (strftime('%s', 'now') * 1000000000 - p.timeout_timestamp) / 1000000000 as seconds_since_timeout
        FROM packets p
        JOIN txs t ON p.tx_id = t.id
        WHERE p.effected = 0 
          AND p.timeout_timestamp IS NOT NULL
          AND p.timeout_timestamp < strftime('%s', 'now') * 1000000000
        ORDER BY p.timeout_timestamp DESC
        LIMIT 100
    "#;

    match sqlx::query(query)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => {
            let packets = rows
                .into_iter()
                .map(|row| {
                    let timeout_type = if row.get::<Option<i64>, _>(9).is_some() {
                        "height".to_string()
                    } else {
                        "timestamp".to_string()
                    };

                    ExpiredPacketInfo {
                        chain_id: row.get(0),
                        sequence: row.get(1),
                        src_channel: row.get(2),
                        dst_channel: row.get(3),
                        sender: row.get(4),
                        receiver: row.get(5),
                        amount: row.get(6),
                        denom: row.get(7),
                        seconds_since_timeout: row.get(11),
                        timeout_type,
                    }
                })
                .collect();

            Ok(Json(ExpiredPacketsResponse {
                packets,
                api_version: "1.0".to_string(),
            }))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Debug, Serialize)]
struct DuplicatePacketsResponse {
    duplicates: Vec<DuplicateGroup>,
    api_version: String,
}

#[derive(Debug, Serialize)]
struct DuplicateGroup {
    data_hash: String,
    count: i64,
    packets: Vec<DuplicatePacketInfo>,
}

#[derive(Debug, Serialize)]
struct DuplicatePacketInfo {
    chain_id: String,
    sequence: i64,
    src_channel: String,
    sender: Option<String>,
    created_at: String,
}

async fn get_duplicate_packets(
    State(state): State<ApiState>,
) -> std::result::Result<Json<DuplicatePacketsResponse>, StatusCode> {
    // First get duplicate hashes
    let hash_query = r#"
        SELECT data_hash, COUNT(*) as count
        FROM packets
        WHERE data_hash IS NOT NULL
        GROUP BY data_hash
        HAVING COUNT(*) > 1
        ORDER BY count DESC
        LIMIT 20
    "#;

    match sqlx::query(hash_query).fetch_all(&state.db).await {
        Ok(hash_rows) => {
            let mut duplicates = Vec::new();

            for hash_row in hash_rows {
                let data_hash: String = hash_row.get(0);
                let count: i64 = hash_row.get(1);

                // Get details for each duplicate
                let detail_query = r#"
                    SELECT 
                        t.chain,
                        p.sequence,
                        p.src_channel,
                        p.sender,
                        p.created_at
                    FROM packets p
                    JOIN txs t ON p.tx_id = t.id
                    WHERE p.data_hash = ?
                    ORDER BY p.created_at ASC
                "#;

                if let Ok(detail_rows) = sqlx::query(detail_query)
                    .bind(&data_hash)
                    .fetch_all(&state.db)
                    .await
                {
                    let packets = detail_rows
                        .into_iter()
                        .map(|row| DuplicatePacketInfo {
                            chain_id: row.get(0),
                            sequence: row.get(1),
                            src_channel: row.get(2),
                            sender: row.get(3),
                            created_at: row.get(4),
                        })
                        .collect();

                    duplicates.push(DuplicateGroup {
                        data_hash,
                        count,
                        packets,
                    });
                }
            }

            Ok(Json(DuplicatePacketsResponse {
                duplicates,
                api_version: "1.0".to_string(),
            }))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
