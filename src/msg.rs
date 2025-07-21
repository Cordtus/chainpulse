use std::fmt;

use ibc_proto::{
    google::protobuf::Any,
    ibc::{
        apps::transfer::v1::MsgTransfer,
        core::{
            channel::v1::{
                MsgAcknowledgement, MsgChannelOpenAck, MsgChannelOpenConfirm, MsgChannelOpenInit,
                MsgChannelOpenTry, MsgRecvPacket, MsgTimeout, Packet,
            },
            client::v1::{MsgCreateClient, MsgUpdateClient},
        },
    },
};

use prost::Message;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};

/// IBC Fungible Token Transfer packet data structure
/// This structure is standard across IBC v1 and will have a compatibility layer in IBC v2
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FungibleTokenPacketData {
    pub denom: String,
    pub amount: String,
    pub sender: String,
    pub receiver: String,
    #[serde(default)]
    pub memo: String,
}

/// Enhanced packet info that works for both IBC v1 and future v2
#[derive(Debug, Clone)]
pub struct UniversalPacketInfo {
    pub sequence: u64,
    pub source_channel: String,
    pub destination_channel: String,
    pub source_port: String,
    pub destination_port: String,
    pub timeout_timestamp: Option<u64>,
    pub timeout_height: Option<ibc_proto::ibc::core::client::v1::Height>,

    // User data (when available)
    pub sender: Option<String>,
    pub receiver: Option<String>,
    pub amount: Option<String>,
    pub denom: Option<String>,
    pub transfer_memo: Option<String>,

    // Version info for future compatibility
    pub ibc_version: String, // "v1" or "v2"
    
    // Data integrity
    pub data_hash: String,
}

impl UniversalPacketInfo {
    /// Extract user data from a packet if it's a fungible token transfer
    pub fn from_packet(packet: &Packet) -> Self {
        let (sender, receiver, denom, amount, transfer_memo) = if packet.source_port == "transfer" {
            match serde_json::from_slice::<FungibleTokenPacketData>(&packet.data) {
                Ok(ft_data) => (
                    Some(ft_data.sender),
                    Some(ft_data.receiver),
                    Some(ft_data.denom),
                    Some(ft_data.amount),
                    Some(ft_data.memo),
                ),
                Err(_) => (None, None, None, None, None),
            }
        } else {
            (None, None, None, None, None)
        };
        
        // Calculate data hash for integrity and deduplication
        let mut hasher = Sha256::new();
        hasher.update(&packet.data);
        let data_hash = format!("{:x}", hasher.finalize());

        Self {
            sequence: packet.sequence,
            source_channel: packet.source_channel.clone(),
            destination_channel: packet.destination_channel.clone(),
            source_port: packet.source_port.clone(),
            destination_port: packet.destination_port.clone(),
            timeout_timestamp: if packet.timeout_timestamp == 0 {
                None
            } else {
                Some(packet.timeout_timestamp)
            },
            timeout_height: packet.timeout_height.clone(),
            sender,
            receiver,
            amount,
            denom,
            transfer_memo,
            ibc_version: "v1".to_string(),
            data_hash,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Msg {
    // Client
    CreateClient(MsgCreateClient),
    UpdateClient(MsgUpdateClient),

    // Channel
    RecvPacket(MsgRecvPacket),
    Acknowledgement(MsgAcknowledgement),
    Timeout(MsgTimeout),

    /// Channel handshake
    ChanOpenInit(MsgChannelOpenInit),
    ChanOpenTry(MsgChannelOpenTry),
    ChanOpenAck(MsgChannelOpenAck),
    ChanOpenConfirm(MsgChannelOpenConfirm),

    // Transfer
    Transfer(MsgTransfer),

    // Other
    Other(Any),
}

impl Msg {
    pub fn is_ibc(&self) -> bool {
        if let Self::Other(other) = self {
            other.type_url.starts_with("/ibc")
        } else {
            true
        }
    }

    pub fn is_relevant(&self) -> bool {
        matches!(
            self,
            Self::RecvPacket(_) | Self::Acknowledgement(_) | Self::Timeout(_) | Self::Transfer(_)
        )
    }

    pub fn packet(&self) -> Option<&Packet> {
        match self {
            Self::RecvPacket(msg) => msg.packet.as_ref(),
            Self::Acknowledgement(msg) => msg.packet.as_ref(),
            Self::Timeout(msg) => msg.packet.as_ref(),
            _ => None,
        }
    }

    pub fn signer(&self) -> Option<&str> {
        match self {
            Self::CreateClient(msg) => Some(&msg.signer),
            Self::UpdateClient(msg) => Some(&msg.signer),
            Self::RecvPacket(msg) => Some(&msg.signer),
            Self::Acknowledgement(msg) => Some(&msg.signer),
            Self::Timeout(msg) => Some(&msg.signer),
            Self::ChanOpenInit(msg) => Some(&msg.signer),
            Self::ChanOpenTry(msg) => Some(&msg.signer),
            Self::ChanOpenAck(msg) => Some(&msg.signer),
            Self::ChanOpenConfirm(msg) => Some(&msg.signer),
            Self::Transfer(msg) => Some(&msg.sender),
            _ => None,
        }
    }
    
    /// Get transfer details from MsgTransfer
    pub fn transfer(&self) -> Option<&MsgTransfer> {
        match self {
            Self::Transfer(msg) => Some(msg),
            _ => None,
        }
    }

    pub fn decode(msg: Any) -> crate::Result<Self> {
        match msg.type_url.as_str() {
            "/ibc.core.client.v1.MsgCreateClient" => MsgCreateClient::decode(msg.value.as_slice())
                .map(Msg::CreateClient)
                .map_err(Into::into),

            "/ibc.core.client.v1.MsgUpdateClient" => MsgUpdateClient::decode(msg.value.as_slice())
                .map(Msg::UpdateClient)
                .map_err(Into::into),

            "/ibc.core.channel.v1.MsgTimeout" => MsgTimeout::decode(msg.value.as_slice())
                .map(Msg::Timeout)
                .map_err(Into::into),

            "/ibc.core.channel.v1.MsgRecvPacket" => MsgRecvPacket::decode(msg.value.as_slice())
                .map(Msg::RecvPacket)
                .map_err(Into::into),

            "/ibc.core.channel.v1.MsgAcknowledgement" => {
                MsgAcknowledgement::decode(msg.value.as_slice())
                    .map(Msg::Acknowledgement)
                    .map_err(Into::into)
            }

            "/ibc.core.channel.v1.MsgChannelOpenInit" => {
                MsgChannelOpenInit::decode(msg.value.as_slice())
                    .map(Msg::ChanOpenInit)
                    .map_err(Into::into)
            }

            "/ibc.core.channel.v1.MsgChannelOpenTry" => {
                MsgChannelOpenTry::decode(msg.value.as_slice())
                    .map(Msg::ChanOpenTry)
                    .map_err(Into::into)
            }

            "/ibc.core.channel.v1.MsgChannelOpenAck" => {
                MsgChannelOpenAck::decode(msg.value.as_slice())
                    .map(Msg::ChanOpenAck)
                    .map_err(Into::into)
            }

            "/ibc.core.channel.v1.MsgChannelOpenConfirm" => {
                MsgChannelOpenConfirm::decode(msg.value.as_slice())
                    .map(Msg::ChanOpenConfirm)
                    .map_err(Into::into)
            }

            "/ibc.applications.transfer.v1.MsgTransfer" => {
                MsgTransfer::decode(msg.value.as_slice())
                    .map(Msg::Transfer)
                    .map_err(Into::into)
            }

            _ => Ok(Msg::Other(msg)),
        }
    }
}

impl fmt::Display for Msg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Msg::CreateClient(_msg) => {
                write!(f, "CreateClient")
            }

            Msg::UpdateClient(msg) => {
                write!(f, "UpdateClient: {}", msg.client_id)
            }

            Msg::RecvPacket(msg) => {
                let packet = msg.packet.as_ref().unwrap();

                write!(
                    f,
                    "RecvPacket: {} -> {}",
                    packet.source_channel, packet.destination_channel
                )
            }

            Msg::Timeout(msg) => {
                let packet = msg.packet.as_ref().unwrap();

                write!(
                    f,
                    "Timeout: {} -> {}",
                    packet.source_channel, packet.destination_channel
                )
            }

            Msg::Acknowledgement(msg) => {
                let packet = msg.packet.as_ref().unwrap();

                write!(
                    f,
                    "Acknowledgement: {} -> {}",
                    packet.source_channel, packet.destination_channel
                )
            }

            Msg::ChanOpenInit(msg) => {
                write!(f, "ChanOpenInit: {}", msg.port_id)
            }

            Msg::ChanOpenTry(msg) => {
                write!(f, "ChanOpenTry: {}", msg.port_id)
            }

            Msg::ChanOpenAck(msg) => {
                write!(f, "ChanOpenAck: {}/{}", msg.channel_id, msg.port_id)
            }

            Msg::ChanOpenConfirm(msg) => {
                write!(f, "ChanOpenConfirm: {}/{}", msg.channel_id, msg.port_id)
            }

            Msg::Transfer(msg) => {
                write!(f, "Transfer: {}/{}", msg.source_channel, msg.source_port)
            }

            Msg::Other(msg) => {
                write!(f, "Unhandled msg: {}", msg.type_url)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fungible_token_packet_data() {
        let data = r#"{
            "denom": "uosmo",
            "amount": "1000000",
            "sender": "osmo1sender123",
            "receiver": "cosmos1receiver456",
            "memo": "test transfer"
        }"#;

        let parsed: FungibleTokenPacketData = serde_json::from_str(data).unwrap();

        assert_eq!(parsed.denom, "uosmo");
        assert_eq!(parsed.amount, "1000000");
        assert_eq!(parsed.sender, "osmo1sender123");
        assert_eq!(parsed.receiver, "cosmos1receiver456");
        assert_eq!(parsed.memo, "test transfer");
    }

    #[test]
    fn test_parse_fungible_token_packet_data_no_memo() {
        let data = r#"{
            "denom": "uatom",
            "amount": "5000000",
            "sender": "cosmos1sender789",
            "receiver": "osmo1receiver012"
        }"#;

        let parsed: FungibleTokenPacketData = serde_json::from_str(data).unwrap();

        assert_eq!(parsed.denom, "uatom");
        assert_eq!(parsed.amount, "5000000");
        assert_eq!(parsed.sender, "cosmos1sender789");
        assert_eq!(parsed.receiver, "osmo1receiver012");
        assert_eq!(parsed.memo, "");
    }

    #[test]
    fn test_universal_packet_info_from_transfer_packet() {
        use ibc_proto::ibc::core::channel::v1::Packet;

        let ft_data = FungibleTokenPacketData {
            denom: "uosmo".to_string(),
            amount: "1000000".to_string(),
            sender: "osmo1sender".to_string(),
            receiver: "cosmos1receiver".to_string(),
            memo: "test".to_string(),
        };

        let packet = Packet {
            sequence: 123,
            source_port: "transfer".to_string(),
            source_channel: "channel-0".to_string(),
            destination_port: "transfer".to_string(),
            destination_channel: "channel-141".to_string(),
            data: serde_json::to_vec(&ft_data).unwrap(),
            timeout_height: None,
            timeout_timestamp: 1234567890,
        };

        let info = UniversalPacketInfo::from_packet(&packet);

        assert_eq!(info.sequence, 123);
        assert_eq!(info.source_channel, "channel-0");
        assert_eq!(info.destination_channel, "channel-141");
        assert_eq!(info.source_port, "transfer");
        assert_eq!(info.destination_port, "transfer");
        assert_eq!(info.sender, Some("osmo1sender".to_string()));
        assert_eq!(info.receiver, Some("cosmos1receiver".to_string()));
        assert_eq!(info.amount, Some("1000000".to_string()));
        assert_eq!(info.denom, Some("uosmo".to_string()));
        assert_eq!(info.transfer_memo, Some("test".to_string()));
        assert_eq!(info.ibc_version, "v1");
        assert_eq!(info.timeout_timestamp, Some(1234567890));
    }

    #[test]
    fn test_universal_packet_info_from_non_transfer_packet() {
        use ibc_proto::ibc::core::channel::v1::Packet;

        let packet = Packet {
            sequence: 456,
            source_port: "icahost".to_string(),
            source_channel: "channel-1".to_string(),
            destination_port: "icacontroller".to_string(),
            destination_channel: "channel-2".to_string(),
            data: vec![1, 2, 3, 4], // Non-JSON data
            timeout_height: None,
            timeout_timestamp: 0,
        };

        let info = UniversalPacketInfo::from_packet(&packet);

        assert_eq!(info.sequence, 456);
        assert_eq!(info.source_channel, "channel-1");
        assert_eq!(info.destination_channel, "channel-2");
        assert_eq!(info.source_port, "icahost");
        assert_eq!(info.destination_port, "icacontroller");
        assert_eq!(info.sender, None);
        assert_eq!(info.receiver, None);
        assert_eq!(info.amount, None);
        assert_eq!(info.denom, None);
        assert_eq!(info.transfer_memo, None);
        assert_eq!(info.ibc_version, "v1");
        assert_eq!(info.timeout_timestamp, None);
    }
}
