[![Cosmos ecosystem][cosmos-shield]][cosmos-link]

[![Crates.io][crates-image]][crates-link]
[![Build Status][build-image]][build-link]
[![Apache 2.0 Licensed][license-image]][license-link]
![Rust Stable][rustc-image]
![Rust 1.69+][rustc-version]

# Chain Pulse

ChainPulse monitors IBC packet flow across Cosmos blockchains. It tracks packet delivery, identifies stuck transfers, and provides real-time data through REST APIs.

ChainPulse connects to blockchain nodes via WebSocket, stores packet data in SQLite, and exports metrics to Prometheus. It supports CometBFT/Tendermint v0.34, v0.37, and v0.38 protocols.

## Installation

1. Clone this repository
   ```shell
   $ git clone https://github.com/informalsystems/chainpulse
   ```

2. Build the `chainpulse` executable
   ```shell
   $ cargo build --release
   ```

3. The `chainpulse` executable can now be found in `target/release`

## Docker

Alternatively, Docker images are available on [Docker Hub](https://hub.docker.com/r/informalsystems/chainpulse/tags).

Read the next section, then get started with:

```
$ docker run informalsystems/chainpulse:latest --config chainpulse.toml
```

## Configuration

Create a configuration file at `chainpulse.toml` with the following content:

```toml
[chains.osmosis-1]
url = "wss://rpc.osmosis.zone/websocket"
comet_version = "0.34"

[database]
path = "data.db"

[metrics]
enabled = true
port    = 3000

# Monitor packets that stay unrelayed for too long
stuck_packets = true
```

### Configuration Options

**Required:**
- `url` - WebSocket endpoint for the chain
- `database.path` - SQLite database location
- `metrics.enabled` - Enable metrics and API server

**Optional:**
- `comet_version` - Protocol version: "0.34", "0.37", or "0.38" (default: "0.34")
- `stuck_packets` - Monitor undelivered packets (default: true)
- `metrics.port` - HTTP server port (default: 3000)

### Authentication

ChainPulse supports authenticated connections to private RPC endpoints:

```toml
[chains.private-chain]
url = "wss://private-rpc.example.com/websocket"
comet_version = "0.34"
username = "your-username"
password = "your-password"
```

The custom WebSocket client handles Basic Authentication during handshake. This works around standard library limitations.

### Chain References

ChainPulse supports referencing chains by network name from a `chains.json` file. This allows you to configure chains without exposing credentials in your configuration:

```toml
# Reference chains by network name
[chains.osmosis-1]
url = "ref:osmosis"

[chains.cosmoshub-4]
url = "ref:cosmoshub"

# Override specific settings if needed
[chains.stride-1]
url = "ref:stride"
comet_version = "0.37"
```

When using `ref:` prefix, ChainPulse will look up the chain details from `chains.json` in the same directory as your config file. The chains.json file should contain endpoint URLs and authentication credentials. 

To set this up:
1. Copy `chains.example.json` to `chains.json`
2. Update it with your actual RPC endpoints and credentials
3. The `chains.json` file is already in `.gitignore` to keep your credentials secure

## Usage

```
Collect and analyze txs containing IBC messages, export the collected metrics for Prometheus

Usage: chainpulse [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to the configuration file [default: chainpulse.toml]
  -h, --help             Print help
```

Run the collector using the configuration file above to collect packet metrics on Osmosis:

```shell
$ chainpulse --config chainpulse.toml
2023-05-26T10:17:28.378380Z  INFO Metrics server listening at http://localhost:3000/metrics
2023-05-26T10:17:28.386951Z  INFO collect{chain=osmosis}: Connecting to wss://rpc.osmosis.zone/websocket...
2023-05-26T10:17:29.078725Z  INFO collect{chain=osmosis}: Subscribing to NewBlock events...
2023-05-26T10:17:29.254485Z  INFO collect{chain=osmosis}: Waiting for new blocks...
...
```

## API Reference

ChainPulse provides REST endpoints at `http://localhost:3000/api/v1/`. All endpoints return JSON.

### Find Packets by User
Track transfers sent or received by any address:

```bash
# Packets sent by address
GET /api/v1/packets/by-user?address={address}&role=sender

# Packets received by address
GET /api/v1/packets/by-user?address={address}&role=receiver
```

**Example response:**
```json
{
  "packets": [{
    "chain_id": "osmosis-1",
    "sequence": 892193,
    "sender": "osmo1...",
    "receiver": "noble1...",
    "amount": "30371228",
    "denom": "uusdc",
    "age_seconds": 120
  }]
}
```

### Find Stuck Packets
Identify transfers that failed to relay:

```bash
GET /api/v1/packets/stuck?min_age_seconds=3600&limit=10
```

Returns packets undelivered for the specified time. Use this to monitor relay health.

### Get Packet Details
Look up specific packet information:

```bash
GET /api/v1/packets/{chain}/{channel}/{sequence}

# Example:
GET /api/v1/packets/osmosis-1/channel-750/892193
```

### Check Channel Congestion
View channels with delivery backlogs:

```bash
GET /api/v1/channels/congestion
```

Returns channels sorted by stuck packet count with total stuck value per token.

## How It Works

### Packet Tracking
ChainPulse extracts data from IBC fungible token transfers:
- Sender and receiver addresses
- Transfer amount and token type
- Channel routing information
- Relay attempts and status

### Stuck Detection
Packets undelivered for 15+ minutes are marked as stuck. The monitor:
- Checks every 60 seconds
- Groups by channel
- Calculates total stuck value

### Integration Examples

**Wallet Integration:**
```javascript
// Show pending transfers
const pending = await fetch(`/api/v1/packets/by-user?address=${userAddress}&role=sender`);
const transfers = await pending.json();
```

**Relay Monitoring:**
```bash
# Alert on congested channels
curl /api/v1/channels/congestion | jq '.channels[] | select(.stuck_count > 100)'
```

## Prometheus Metrics

Access metrics at `http://localhost:3000/metrics`.

### Packet Flow Metrics
- `ibc_effected_packets` - Successfully delivered packets (labeled by relayer)
- `ibc_uneffected_packets` - Failed packet deliveries
- `ibc_frontrun_counter` - Packets delivered by competing relayers

### Stuck Packet Metrics
- `ibc_stuck_packets` - Count per channel
- `ibc_stuck_packets_detailed` - Includes user data flag
- `ibc_packet_age_seconds` - Time since packet creation

### System Health Metrics
- `chainpulse_chains` - Active chain connections
- `chainpulse_packets` - Total packets processed
- `chainpulse_txs` - Total transactions processed
- `chainpulse_errors` - Connection errors per chain
- `chainpulse_reconnects` - WebSocket reconnection count

### Example Prometheus Query
```promql
# Alert on channels with >100 stuck packets
ibc_stuck_packets > 100

# Calculate packet delivery rate
rate(ibc_effected_packets[5m]) / rate(chainpulse_packets[5m])
```

## Attribution

This project is heavily inspired and partly ported from @clemensgg's [relayer-metrics-exporter][clemensgg-metrics]

## License

Copyright © 2023 Informal Systems Inc. and Hermes authors.

Licensed under the Apache License, Version 2.0 (the "License"); you may not use the files in this repository except in compliance with the License. You may obtain a copy of the License at

    https://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software distributed under the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License for the specific language governing permissions and limitations under the License.


[cosmos-shield]: https://img.shields.io/static/v1?label=&labelColor=1B1E36&color=1B1E36&message=cosmos%20ecosystem&style=for-the-badge&logo=data:image/svg+xml;base64,PD94bWwgdmVyc2lvbj0iMS4wIiBlbmNvZGluZz0idXRmLTgiPz4KPCEtLSBHZW5lcmF0b3I6IEFkb2JlIElsbHVzdHJhdG9yIDI0LjMuMCwgU1ZHIEV4cG9ydCBQbHVnLUluIC4gU1ZHIFZlcnNpb246IDYuMDAgQnVpbGQgMCkgIC0tPgo8c3ZnIHZlcnNpb249IjEuMSIgaWQ9IkxheWVyXzEiIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgeG1sbnM6eGxpbms9Imh0dHA6Ly93d3cudzMub3JnLzE5OTkveGxpbmsiIHg9IjBweCIgeT0iMHB4IgoJIHZpZXdCb3g9IjAgMCAyNTAwIDI1MDAiIHN0eWxlPSJlbmFibGUtYmFja2dyb3VuZDpuZXcgMCAwIDI1MDAgMjUwMDsiIHhtbDpzcGFjZT0icHJlc2VydmUiPgo8c3R5bGUgdHlwZT0idGV4dC9jc3MiPgoJLnN0MHtmaWxsOiM2RjczOTA7fQoJLnN0MXtmaWxsOiNCN0I5Qzg7fQo8L3N0eWxlPgo8cGF0aCBjbGFzcz0ic3QwIiBkPSJNMTI1Mi42LDE1OS41Yy0xMzQuOSwwLTI0NC4zLDQ4OS40LTI0NC4zLDEwOTMuMXMxMDkuNCwxMDkzLjEsMjQ0LjMsMTA5My4xczI0NC4zLTQ4OS40LDI0NC4zLTEwOTMuMQoJUzEzODcuNSwxNTkuNSwxMjUyLjYsMTU5LjV6IE0xMjY5LjQsMjI4NGMtMTUuNCwyMC42LTMwLjksNS4xLTMwLjksNS4xYy02Mi4xLTcyLTkzLjItMjA1LjgtOTMuMi0yMDUuOAoJYy0xMDguNy0zNDkuOC04Mi44LTExMDAuOC04Mi44LTExMDAuOGM1MS4xLTU5Ni4yLDE0NC03MzcuMSwxNzUuNi03NjguNGM2LjctNi42LDE3LjEtNy40LDI0LjctMmM0NS45LDMyLjUsODQuNCwxNjguNSw4NC40LDE2OC41CgljMTEzLjYsNDIxLjgsMTAzLjMsODE3LjksMTAzLjMsODE3LjljMTAuMywzNDQuNy01Ni45LDczMC41LTU2LjksNzMwLjVDMTM0MS45LDIyMjIuMiwxMjY5LjQsMjI4NCwxMjY5LjQsMjI4NHoiLz4KPHBhdGggY2xhc3M9InN0MCIgZD0iTTIyMDAuNyw3MDguNmMtNjcuMi0xMTcuMS01NDYuMSwzMS42LTEwNzAsMzMycy04OTMuNSw2MzguOS04MjYuMyw3NTUuOXM1NDYuMS0zMS42LDEwNzAtMzMyCglTMjI2Ny44LDgyNS42LDIyMDAuNyw3MDguNkwyMjAwLjcsNzA4LjZ6IE0zNjYuNCwxNzgwLjRjLTI1LjctMy4yLTE5LjktMjQuNC0xOS45LTI0LjRjMzEuNi04OS43LDEzMi0xODMuMiwxMzItMTgzLjIKCWMyNDkuNC0yNjguNCw5MTMuOC02MTkuNyw5MTMuOC02MTkuN2M1NDIuNS0yNTIuNCw3MTEuMS0yNDEuOCw3NTMuOC0yMzBjOS4xLDIuNSwxNSwxMS4yLDE0LDIwLjZjLTUuMSw1Ni0xMDQuMiwxNTctMTA0LjIsMTU3CgljLTMwOS4xLDMwOC42LTY1Ny44LDQ5Ni44LTY1Ny44LDQ5Ni44Yy0yOTMuOCwxODAuNS02NjEuOSwzMTQuMS02NjEuOSwzMTQuMUM0NTYsMTgxMi42LDM2Ni40LDE3ODAuNCwzNjYuNCwxNzgwLjRMMzY2LjQsMTc4MC40CglMMzY2LjQsMTc4MC40eiIvPgo8cGF0aCBjbGFzcz0ic3QwIiBkPSJNMjE5OC40LDE4MDAuNGM2Ny43LTExNi44LTMwMC45LTQ1Ni44LTgyMy03NTkuNVMzNzQuNCw1ODcuOCwzMDYuOCw3MDQuN3MzMDAuOSw0NTYuOCw4MjMuMyw3NTkuNQoJUzIxMzAuNywxOTE3LjQsMjE5OC40LDE4MDAuNHogTTM1MS42LDc0OS44Yy0xMC0yMy43LDExLjEtMjkuNCwxMS4xLTI5LjRjOTMuNS0xNy42LDIyNC43LDIyLjYsMjI0LjcsMjIuNgoJYzM1Ny4yLDgxLjMsOTk0LDQ4MC4yLDk5NCw0ODAuMmM0OTAuMywzNDMuMSw1NjUuNSw0OTQuMiw1NzYuOCw1MzcuMWMyLjQsOS4xLTIuMiwxOC42LTEwLjcsMjIuNGMtNTEuMSwyMy40LTE4OC4xLTExLjUtMTg4LjEtMTEuNQoJYy00MjIuMS0xMTMuMi03NTkuNi0zMjAuNS03NTkuNi0zMjAuNWMtMzAzLjMtMTYzLjYtNjAzLjItNDE1LjMtNjAzLjItNDE1LjNjLTIyNy45LTE5MS45LTI0NS0yODUuNC0yNDUtMjg1LjRMMzUxLjYsNzQ5Ljh6Ii8+CjxjaXJjbGUgY2xhc3M9InN0MSIgY3g9IjEyNTAiIGN5PSIxMjUwIiByPSIxMjguNiIvPgo8ZWxsaXBzZSBjbGFzcz0ic3QxIiBjeD0iMTc3Ny4zIiBjeT0iNzU2LjIiIHJ4PSI3NC42IiByeT0iNzcuMiIvPgo8ZWxsaXBzZSBjbGFzcz0ic3QxIiBjeD0iNTUzIiBjeT0iMTAxOC41IiByeD0iNzQuNiIgcnk9Ijc3LjIiLz4KPGVsbGlwc2UgY2xhc3M9InN0MSIgY3g9IjEwOTguMiIgY3k9IjE5NjUiIHJ4PSI3NC42IiByeT0iNzcuMiIvPgo8L3N2Zz4K
[cosmos-link]: https://cosmos.network
[crates-image]: https://img.shields.io/crates/v/chainpulse.svg
[crates-link]: https://crates.io/crates/chainpulse
[build-image]: https://github.com/informalsystems/chainpulse/workflows/Rust/badge.svg
[build-link]: https://github.com/informalsystems/chainpulse/actions?query=workflow%3ARust
[license-image]: https://img.shields.io/badge/license-Apache_2.0-blue.svg
[license-link]: https://github.com/informalsystems/chainpulse/blob/master/LICENSE
[rustc-image]: https://img.shields.io/badge/rustc-stable-blue.svg
[rustc-version]: https://img.shields.io/badge/rustc-1.69+-blue.svg
[clemensgg-metrics]: https://github.com/clemensgg/relayer-metrics-exporter
