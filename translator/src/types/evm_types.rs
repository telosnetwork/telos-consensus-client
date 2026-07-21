use alloy::primitives::aliases::BlockTimestamp;
use alloy::primitives::{Address, Bytes, Log, B256};
use antelope::chain::asset::Asset;
use antelope::chain::binary_extension::BinaryExtension;
use antelope::chain::checksum::{Checksum160, Checksum256};
use antelope::chain::name::Name;
use antelope::chain::time::TimePoint;
use antelope::chain::Packer;
use antelope::serializer::Decoder;
use antelope::serializer::Encoder;
use antelope::StructPacker;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct RawAction {
    pub ram_payer: Name,
    pub tx: Vec<u8>,
    pub estimate_gas: bool,
    pub sender: Option<Checksum160>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct TransferAction {
    pub from: Name,
    pub to: Name,
    pub quantity: Asset,
    pub memo: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct EvmContractConfigRow {
    pub trx_index: u32,
    pub last_block: u32,
    pub gas_used_block: Checksum256,
    pub gas_price: Checksum256,
    pub revision: BinaryExtension<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct AccountRow {
    pub index: u64,
    pub address: Checksum160,
    pub account: Name,
    pub nonce: u64,
    pub code: Vec<u8>,
    pub balance: Checksum256,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct AccountStateRow {
    pub index: u64,
    pub key: Checksum256,
    pub value: Checksum256,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct GlobalTable {
    max_ram_size: u64,
    total_ram_bytes_reserved: u64,
    total_ram_stake: i64,
    last_producer_schedule_update: BlockTimestamp,
    last_proposed_schedule_update: BlockTimestamp,
    last_pervote_bucket_fill: TimePoint,
    pervote_bucket: i64,
    perblock_bucket: i64,
    total_unpaid_blocks: u32,
    total_activated_stake: i64,
    thresh_activated_stake_time: TimePoint,
    last_producer_schedule_size: u16,
    total_producer_vote_weight: f64,
    last_name_close: BlockTimestamp,
    block_num: u32,
    last_claimrewards: u32,
    next_payment: u32,
    new_ram_per_block: u16,
    last_ram_increase: BlockTimestamp,
    last_block_num: BlockTimestamp,
    total_producer_votepay_share: f64,
    revision: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct WithdrawAction {
    pub to: Name,
    pub quantity: Asset,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct SetRevisionAction {
    pub new_revision: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct OpenWalletAction {
    pub account: Name,
    pub address: Checksum160,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct CreateAction {
    pub account: Name,
    pub data: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrintedReceipt {
    pub charged_gas: String,
    pub trx_index: u16,
    pub block: u64,
    pub status: u8,
    pub epoch: u64,
    pub createdaddr: String,
    pub gasused: String,
    #[serde(deserialize_with = "deserialize_logs")]
    pub logs: Vec<Log>,
    pub output: String,
    pub errors: Option<Vec<String>>,
    // pub itxs: any[], // Define struct for this
}

impl Default for PrintedReceipt {
    fn default() -> Self {
        PrintedReceipt {
            charged_gas: "".to_string(),
            trx_index: 0,
            block: 0,
            status: 0,
            epoch: 0,
            createdaddr: "".to_string(),
            gasused: "5208".to_string(),
            logs: vec![],
            output: "".to_string(),
            errors: None,
        }
    }
}

fn deserialize_logs<'de, D>(deserializer: D) -> Result<Vec<Log>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct LogHelper {
        address: String,
        data: String,
        topics: Vec<String>,
    }

    impl LogHelper {
        fn address(&self) -> Result<Address, String> {
            let address = self.address.strip_prefix("0x").unwrap_or(&self.address);
            if address.len() > 40 {
                return Err(format!(
                    "log address exceeds 20 bytes: {} hex characters",
                    address.len()
                ));
            }
            let padded = format!("{address:0>40}");
            padded
                .parse()
                .map_err(|error| format!("invalid log address: {error}"))
        }
    }

    let log_helpers = Vec::<LogHelper>::deserialize(deserializer)?;
    let mut logs = vec![];
    for log in log_helpers {
        let address = log.address().map_err(D::Error::custom)?;
        let topics = log
            .topics
            .into_iter()
            .map(|topic| parse_b256(&topic).map_err(D::Error::custom))
            .collect::<Result<Vec<_>, D::Error>>()?;
        let data = log
            .data
            .parse::<Bytes>()
            .map_err(|error| D::Error::custom(format!("invalid log data: {error}")))?;
        let log = Log::new(address, topics, data)
            .ok_or_else(|| D::Error::custom("log contains more than four topics"))?;
        logs.push(log);
    }
    Ok(logs)
}

fn parse_b256(value: &str) -> Result<B256, String> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(value).map_err(|error| format!("invalid log topic: {error}"))?;
    if bytes.len() > 32 {
        return Err(format!("log topic exceeds 32 bytes: {} bytes", bytes.len()));
    }
    Ok(B256::left_padding_from(&bytes))
}

impl PrintedReceipt {
    pub fn from_console(console: &str) -> Result<Option<Self>, serde_json::Error> {
        let start_pattern = "RCPT{{";
        let end_pattern = "}}RCPT";

        if let Some(start) = console.find(start_pattern) {
            let start_index = start + start_pattern.len();
            if let Some(end) = console[start_index..].find(end_pattern) {
                let end_index = start_index + end;
                let extracted = &console[start_index..end_index];
                serde_json::from_str::<PrintedReceipt>(extracted).map(Some)
            } else {
                warn!("End pattern not found.");
                Ok(None)
            }
        } else {
            warn!("Start pattern not found.");
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn receipt_with_logs(logs: serde_json::Value) -> serde_json::Value {
        json!({
            "charged_gas": "0",
            "trx_index": 0,
            "block": 1,
            "status": 1,
            "epoch": 0,
            "createdaddr": "",
            "gasused": "5208",
            "logs": logs,
            "output": "",
            "errors": null
        })
    }

    #[test]
    fn malformed_console_receipt_is_an_error() {
        assert!(PrintedReceipt::from_console("RCPT{{not-json}}RCPT").is_err());
        assert!(PrintedReceipt::from_console("no receipt here")
            .unwrap()
            .is_none());
    }

    #[test]
    fn malformed_log_fields_are_rejected_without_panicking() {
        let invalid_topic = receipt_with_logs(json!([{
            "address": "1",
            "data": "0x",
            "topics": ["not-hex"]
        }]));
        assert!(serde_json::from_value::<PrintedReceipt>(invalid_topic).is_err());

        let too_many_topics = receipt_with_logs(json!([{
            "address": "1",
            "data": "0x",
            "topics": ["00", "00", "00", "00", "00"]
        }]));
        assert!(serde_json::from_value::<PrintedReceipt>(too_many_topics).is_err());

        let oversized_address = receipt_with_logs(json!([{
            "address": "111111111111111111111111111111111111111111",
            "data": "0x",
            "topics": []
        }]));
        assert!(serde_json::from_value::<PrintedReceipt>(oversized_address).is_err());
    }
}
