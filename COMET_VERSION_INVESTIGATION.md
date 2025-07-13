# CometBFT Version Investigation Report

## Current State

ChainPulse uses the `comet_version` configuration parameter to specify which Tendermint/CometBFT protocol version to use when connecting to blockchain nodes. This parameter maps to `tendermint_rpc::client::CompatMode`.

### Currently Supported Versions
- **0.34** (default) - Maps to `CompatMode::V0_34`
- **0.37** - Maps to `CompatMode::V0_37`

### Why Version Compatibility Matters

1. **Protocol Differences**: Different versions of Tendermint/CometBFT have changes in:
   - RPC API endpoints
   - Data serialization formats
   - WebSocket event structures
   - Block and transaction formats

2. **Chain Requirements**: Different Cosmos chains run different versions:
   - Older chains may still use Tendermint v0.34
   - Newer chains use CometBFT v0.37 or v0.38
   - Some chains have custom modifications

3. **Correct Data Parsing**: Using the wrong compatibility mode can lead to:
   - Failed connections
   - Incorrect data parsing
   - Missing events
   - Serialization errors

## Current Implementation

### Configuration (src/config.rs)
```rust
// Uses tendermint_rpc::client::CompatMode
use tendermint_rpc::{client::CompatMode as CometVersion, WebSocketClientUrl};

// Default to v0.34
pub fn comet_version() -> CometVersion {
    CometVersion::V0_34
}

// Only accepts "0.34" or "0.37"
match version.as_str() {
    "0.37" => Ok(CometVersion::V0_37),
    "0.34" => Ok(CometVersion::V0_34),
    _ => Err(serde::de::Error::custom(format!(
        "invalid CometBFT version: {}, available: 0.34, 0.37",
        version
    ))),
}
```

### Usage (src/collect.rs)
```rust
// CompatMode is passed to WebSocket client builder
let (client, driver) = WebSocketClient::builder(ws_url.clone())
    .compat_mode(compat_mode)
    .build()
    .await?;
```

## Missing Support: CometBFT v0.38

### Current Limitation
- ChainPulse uses tendermint-rs v0.32, which only supports V0_34 and V0_37
- CometBFT v0.38 introduced ABCI 2.0 with significant protocol changes
- Many newer chains are adopting v0.38

### Chains Using v0.38
- Cosmos Hub (planned upgrade)
- Osmosis (planned upgrade)
- Many new chains launching with v0.38

## Upgrade Path

### Option 1: Update tendermint-rs to v0.40+ (Recommended)

**Benefits:**
- Adds `CompatMode::V0_38` support
- Better CometBFT compatibility
- Security updates and bug fixes
- Future-proof for upcoming versions

**Changes Required:**
1. Update Cargo.toml dependencies:
   ```toml
   tendermint = "0.40"
   tendermint-proto = "0.40"
   tendermint-rpc = { version = "0.40", features = ["websocket-client"] }
   ```

2. Update config.rs to handle V0_38:
   ```rust
   match version.as_str() {
       "0.38" => Ok(CometVersion::V0_38),
       "0.37" => Ok(CometVersion::V0_37),
       "0.34" => Ok(CometVersion::V0_34),
       _ => Err(...)
   }
   ```

3. Test with v0.38 chains

### Option 2: Auto-Detection (Future Enhancement)

Implement version detection by querying the node's `/status` endpoint before establishing WebSocket connection:
- Query `GET /status`
- Parse version from response
- Auto-select appropriate CompatMode

### Option 3: Version Aliases

Support multiple version formats:
- "0.34", "v0.34", "tendermint-0.34"
- "0.37", "v0.37", "cometbft-0.37"
- "0.38", "v0.38", "cometbft-0.38"

## Recommendations

1. **Immediate**: Update tendermint-rs to v0.40+ to support CometBFT v0.38
2. **Short-term**: Add clear documentation about version requirements
3. **Long-term**: Implement auto-detection for better user experience

## Testing Requirements

1. Test with chains running each version:
   - v0.34: Older Cosmos chains
   - v0.37: Current production chains
   - v0.38: Newer CometBFT chains

2. Verify packet parsing works correctly across versions
3. Ensure WebSocket events are properly received
4. Check that all API endpoints function correctly

## References

- [CometBFT Versions](https://docs.cometbft.com/)
- [tendermint-rs CompatMode](https://github.com/informalsystems/tendermint-rs)
- [ABCI 2.0 Changes](https://docs.cometbft.com/v0.38/spec/abci/)