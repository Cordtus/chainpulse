use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tendermint::chain;
pub use tendermint_rpc::client::CompatMode as CometVersion;
use tendermint_rpc::WebSocketClientUrl;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Global {
    #[serde(default = "default::ibc_versions")]
    pub ibc_versions: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub global: Global,
    pub chains: Chains,
    pub database: Database,
    pub metrics: Metrics,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RawConfig {
    #[serde(default)]
    pub global: Global,
    pub chains: RawChains,
    pub database: Database,
    pub metrics: Metrics,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RawChains {
    #[serde(flatten)]
    pub endpoints: BTreeMap<chain::Id, RawEndpoint>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RawEndpoint {
    pub url: String,
    #[serde(default = "crate::config::default::comet_version_str")]
    pub comet_version: String,
    #[serde(default = "crate::config::default::ibc_version")]
    pub ibc_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChainsReference {
    pub chains: BTreeMap<String, ChainInfo>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChainInfo {
    pub chain_id: String,
    pub rpc: String,
    pub websocket: String,
    pub username: String,
    pub password: String,
    #[serde(default = "crate::config::default::comet_version_str")]
    pub comet_version: String,
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let content = fs::read_to_string(&path)?;
        let raw_config: RawConfig =
            toml::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Load chains reference if available
        let chains_ref_path = path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("chains.json");

        let chains_ref = if chains_ref_path.exists() {
            let chains_ref_content = fs::read_to_string(&chains_ref_path)?;
            Some(
                serde_json::from_str::<ChainsReference>(&chains_ref_content)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            )
        } else {
            None
        };

        // Process chains, expanding references
        let mut expanded_chains = BTreeMap::new();
        for (chain_id_str, raw_endpoint) in raw_config.chains.endpoints {
            if raw_endpoint.url.starts_with("ref:") {
                let network_name = raw_endpoint.url.strip_prefix("ref:").unwrap();
                if let Some(ref chains_ref) = chains_ref {
                    if let Some(chain_info) = chains_ref.chains.get(network_name) {
                        let url = WebSocketClientUrl::from_str(&chain_info.websocket)
                            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                        let comet_compat = match chain_info.comet_version.as_str() {
                            "0.37" => CometVersion::V0_37,
                            _ => CometVersion::V0_34,
                        };
                        expanded_chains.insert(
                            chain_id_str,
                            Endpoint {
                                url,
                                comet_version: comet_compat,
                                version: chain_info.comet_version.clone(),
                                ibc_version: raw_endpoint.ibc_version,
                                username: Some(chain_info.username.clone()),
                                password: Some(chain_info.password.clone()),
                            },
                        );
                    } else {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Unknown chain reference: {}", network_name),
                        ));
                    }
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Chain reference '{}' used but chains.json not found",
                            network_name
                        ),
                    ));
                }
            } else {
                let url = WebSocketClientUrl::from_str(&raw_endpoint.url)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                let comet_compat = match raw_endpoint.comet_version.as_str() {
                    "0.37" => CometVersion::V0_37,
                    _ => CometVersion::V0_34,
                };
                expanded_chains.insert(
                    chain_id_str,
                    Endpoint {
                        url,
                        comet_version: comet_compat,
                        version: raw_endpoint.comet_version.clone(),
                        ibc_version: raw_endpoint.ibc_version,
                        username: raw_endpoint.username,
                        password: raw_endpoint.password,
                    },
                );
            }
        }

        Ok(Config {
            global: raw_config.global,
            chains: Chains {
                endpoints: expanded_chains,
            },
            database: raw_config.database,
            metrics: raw_config.metrics,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Chains {
    pub endpoints: BTreeMap<chain::Id, Endpoint>,
}

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub url: WebSocketClientUrl,
    pub comet_version: CometVersion,
    pub version: String,
    pub ibc_version: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Endpoint {
    /// Get the version string for this endpoint
    pub fn version_string(&self) -> &str {
        &self.version
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Database {
    pub path: PathBuf,
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct Metrics {
    pub enabled: bool,
    pub port: u16,

    #[serde(default)]
    pub populate_on_start: bool,
}

mod default {
    use super::*;

    pub fn comet_version() -> CometVersion {
        CometVersion::V0_34
    }

    pub fn comet_version_str() -> String {
        "0.34".to_string()
    }


    pub fn ibc_version() -> String {
        "v1".to_string()
    }

    pub fn ibc_versions() -> Vec<String> {
        vec!["v1".to_string()]
    }
}

mod comet_version {
    use super::*;
    use serde::{Deserialize, Serializer};

    pub fn serialize<S>(version: &CometVersion, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let version = match version {
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
            "0.37" => Ok(CometVersion::V0_37),
            "0.34" => Ok(CometVersion::V0_34),
            // 0.38 uses V0_34 as placeholder but we track the real version
            "0.38" => Ok(CometVersion::V0_34),
            _ => Err(serde::de::Error::custom(format!(
                "invalid CometBFT version: {}, available: 0.34, 0.37, 0.38",
                version
            ))),
        }
    }
}
