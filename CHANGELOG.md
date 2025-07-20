# CHANGELOG

## Unreleased

### Added
- Support for CometBFT v0.38 protocol
- Comprehensive IBC packet data extraction:
  - Packet timeout tracking (timestamp and block height)
  - SHA256 data hash for duplicate detection
  - Complete transfer details (sender, receiver, amount, denom)
- Enhanced REST API endpoints:
  - `/api/v1/packets/by-user` - Find packets by sender or receiver address
  - `/api/v1/packets/stuck` - Query undelivered packets
  - `/api/v1/packets/{chain}/{channel}/{sequence}` - Get specific packet details
  - `/api/v1/channels/congestion` - View congested channels with stuck value
  - `/api/v1/packets/expiring` - Find packets approaching timeout
  - `/api/v1/packets/expired` - Query already expired packets
  - `/api/v1/packets/duplicates` - Detect duplicate transfers
- New Prometheus metrics:
  - `ibc_packets_near_timeout` - Packets approaching timeout deadline
  - `ibc_packet_timeout_seconds` - Time until packet timeout (negative if expired)
- Automatic timeout monitoring with URGENT and EXPIRED alerts
- Enhanced stuck packet detection with user data tracking
- Authentication support for private RPC endpoints (Basic Auth)
- Chain reference system for managing credentials via `chains.json`
- Database schema auto-migration for existing installations

### Changed
- Unified collector system handles all protocol versions automatically
- Table-based chain configuration (check [`chainpulse.toml`](./chainpulse.toml) for syntax)

### Fixed
- Neutron chain block parsing now correctly handles non-standard first transactions (e.g., oracle data)

## v0.3.2

*July 26th, 2023*

- Fix a bug where `Timeout` messages were not handled.

## v0.3.1

*July 20th, 2023*

- Fix a bug where ChainPulse would fail to create the SQLite database.

## v0.3.0

*June 6th, 2023*

- Add a `populate_on_start` option to the `metrics` section in the configuration to
  populate the Prometheus metrics on start by replaying all packets present in the database so far.

  **Warning:** Use with caution if you are already tracking any of the counters with Prometheus as this
  will result in inflated results for all counters (but not gauges or histograms).
- Monitor packets stuck on IBC channels, and expose their number per channel as a new `ibc_stuck_packets` metric

## v0.2.0

*May 26th 2023*

- Add support for listening on multiple chains simultaneously
- Use a [configuration file](./README.md#configuration) instead of command-line arguments
- Add [internal metrics](./README.md/#internal-metrics)
- Add support for CometBFT 0.34 and 0.37

## v0.1.2

*May 25th 2023*

- Respond to SIGINT, SIGHUP and SIGTERM signals even when ran as PID 1, eg. in a distroless Docker container

## v0.1.1

*May 25th 2023*

- Disconnect and reconnect every 100 blocks in an attempt to keep the connection from hanging

## v0.1.0

*May 25th 2023*

Initial release
