# Upgrade Plan: Support CometBFT v0.38

## Overview

This plan outlines the steps to upgrade ChainPulse to support CometBFT v0.38 by updating the tendermint-rs dependencies from v0.32 to v0.40.

## Step 1: Update Dependencies

### Cargo.toml Changes
```toml
[dependencies]
# Update core tendermint dependencies
tendermint         = "0.40"
tendermint-proto   = "0.40"  
tendermint-rpc     = { version = "0.40", features = ["websocket-client"] }

# May need to update
ibc-proto          = { version = "0.46", default-features = false }  # Check compatibility
```

## Step 2: Code Changes

### 1. Update config.rs

Add support for v0.38 in the serialization/deserialization:

```rust
// In comet_version module
pub fn serialize<S>(version: &CometVersion, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let version = match version {
        CometVersion::V0_38 => "0.38",  // Add this
        CometVersion::V0_37 => "0.37",
        CometVersion::V0_34 => "0.34",
    };

    serializer.serialize_str(version)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<CometVersion, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = String::deserialize(deserializer)?;

    match version.as_str() {
        "0.38" => Ok(CometVersion::V0_38),  // Add this
        "0.37" => Ok(CometVersion::V0_37),
        "0.34" => Ok(CometVersion::V0_34),
        _ => Err(serde::de::Error::custom(format!(
            "invalid CometBFT version: {}, available: 0.34, 0.37, 0.38",
            version
        ))),
    }
}
```

### 2. Handle EventAttribute Changes

The EventAttribute type changed from having String fields to Vec<u8> fields and became an enum. Update event handling code:

```rust
// In collect.rs or wherever events are processed
// Old code (v0.32):
// event.attributes.key (String)
// event.attributes.value (String)

// New code (v0.40):
// Match on EventAttribute enum
match &event_attribute {
    EventAttribute::V034(attr) => {
        // attr.key and attr.value are Vec<u8>
        let key = String::from_utf8_lossy(&attr.key);
        let value = String::from_utf8_lossy(&attr.value);
    }
    EventAttribute::V037(attr) => {
        // Handle v0.37+ attributes
    }
}
```

## Step 3: Testing Strategy

### 1. Unit Tests
- Update existing tests for new EventAttribute structure
- Add tests for v0.38 configuration parsing

### 2. Integration Tests
Create test configurations for each version:

```toml
# test-v034.toml
[chains.old-chain]
url = "wss://v034.example.com/websocket"
comet_version = "0.34"

# test-v037.toml
[chains.current-chain]  
url = "wss://v037.example.com/websocket"
comet_version = "0.37"

# test-v038.toml
[chains.new-chain]
url = "wss://v038.example.com/websocket"
comet_version = "0.38"
```

### 3. Live Testing
Test against real chains:
- v0.34: Cosmos Hub (current)
- v0.37: Osmosis (current)
- v0.38: Testnet chains running v0.38

## Step 4: Migration Guide

### For Users
Update documentation to explain:
1. Which chains require which version
2. How to determine a chain's version
3. Common error messages and solutions

### Example Configuration
```toml
# Modern CometBFT chain
[chains.neutron-1]
url = "wss://neutron-rpc.example.com/websocket"
comet_version = "0.38"  # New option!

# Legacy Tendermint chain
[chains.cosmoshub-4]
url = "wss://cosmos-rpc.example.com/websocket"
comet_version = "0.34"  # Still supported
```

## Step 5: Rollback Plan

If issues arise:
1. Revert Cargo.toml changes
2. Revert code changes
3. Document any v0.38 chains that don't work
4. Create issue for specific incompatibilities

## Step 6: Future Considerations

### Auto-Detection Implementation
```rust
async fn detect_comet_version(url: &str) -> Result<CompatMode> {
    // Query /status endpoint
    let status_url = url.replace("wss://", "https://").replace("/websocket", "/status");
    let response = reqwest::get(status_url).await?;
    let status: Value = response.json().await?;
    
    // Parse version from node_info.version
    let version = status["result"]["node_info"]["version"].as_str().unwrap_or("");
    
    match version {
        v if v.contains("0.38") => Ok(CompatMode::V0_38),
        v if v.contains("0.37") => Ok(CompatMode::V0_37),
        _ => Ok(CompatMode::V0_34), // Default fallback
    }
}
```

## Timeline

1. **Phase 1** (1 day): Update dependencies and fix compilation errors
2. **Phase 2** (2 days): Update event handling for new EventAttribute structure  
3. **Phase 3** (2 days): Test against different chain versions
4. **Phase 4** (1 day): Update documentation and examples
5. **Phase 5** (ongoing): Monitor for issues and gather feedback

## Risk Assessment

- **Low Risk**: Configuration changes are backward compatible
- **Medium Risk**: EventAttribute changes may affect packet parsing
- **Low Risk**: WebSocket client changes appear minimal
- **Mitigation**: Extensive testing before release