# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ChainPulse is a Rust-based monitoring tool for IBC (Inter-Blockchain Communication) relayers in the Cosmos ecosystem. It collects and analyzes IBC packet metrics by monitoring blockchain transactions via WebSocket connections.

## Development Commands

### Build
```bash
cargo build --release
```

### Format Code
```bash
cargo fmt --all
```

### Run Linter
```bash
cargo clippy --all-features --all-targets
```

### Run Application
```bash
# Using cargo (development)
cargo run -- --config chainpulse.toml

# Using release binary
./target/release/chainpulse --config chainpulse.toml
```

### Docker Commands
```bash
# Build Docker image
docker build -t chainpulse .

# Run with Docker
docker run -v $(pwd)/chainpulse.toml:/app/chainpulse.toml informalsystems/chainpulse:latest --config /app/chainpulse.toml
```

## Architecture Overview

### Core Components

1. **Collector Module** (`src/collect.rs`): Manages WebSocket connections to multiple chains, subscribes to NewBlock events, and processes transactions containing IBC messages.

2. **Database Layer** (`src/db.rs`): SQLite database operations for persisting packet data. Uses SQLx for async database operations.

3. **Metrics Server** (`src/metrics.rs`): Prometheus-compatible HTTP server exposing metrics on configurable port (default: 3000).

4. **Message Parser** (`src/msg.rs`): Parses IBC messages from Cosmos transactions to extract packet information.

5. **Status Monitor** (`src/status.rs`): Optional feature for tracking stuck IBC packets across channels.

### Data Flow

1. Connect to chain RPC endpoints via WebSocket
2. Subscribe to NewBlock events  
3. Parse IBC messages from each block's transactions
4. Store packet metadata in SQLite database
5. Expose aggregated metrics via HTTP endpoint for Prometheus scraping

### Key Design Patterns

- **Multi-chain Support**: Concurrent monitoring of multiple Cosmos chains
- **Resilient Connections**: Automatic WebSocket reconnection with backoff
- **Single-threaded Tokio Runtime**: Efficient async execution model
- **Database-backed Metrics**: SQLite for persistence, in-memory aggregation for performance

## Configuration

The application uses TOML configuration with three main sections:

- `[chains.<chain-id>]`: WebSocket URL and Comet/Tendermint version for each chain
- `[database]`: SQLite database file path
- `[metrics]`: HTTP server settings and feature toggles (stuck packet monitoring)

## Testing

Currently minimal test coverage. The test runner is commented out in CI workflow. When implementing tests:
- Use standard Rust testing framework (`#[cfg(test)]` modules)
- Consider using `cargo nextest` for parallel test execution (as indicated in workflow)

## Important Notes

- Requires Rust 1.69+ (uses modern async features)
- Uses distroless base image for minimal Docker containers
- Prometheus metrics use standard label conventions for IBC packets
- Database migrations are handled automatically by SQLx