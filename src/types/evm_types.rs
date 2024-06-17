use alloy::primitives::{Address, address, B256, Bytes, Log};
use antelope::serializer::Decoder;
use antelope::serializer::Encoder;
use antelope::chain::asset::Asset;
use antelope::chain::checksum::Checksum160;
use antelope::chain::name::Name;
use antelope::StructPacker;
use antelope::chain::Packer;
use antelope::util::hex_to_bytes;
use serde::{Deserialize, Deserializer, Serialize};

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
    pub memo: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrintedReceipt {
    pub charged_gas: String,
    pub trx_index: u16,
    pub block: u32,
    pub status: u8,
    pub epoch: u64,
    pub createdaddr: String,
    pub gasused: String,
    #[serde(deserialize_with = "deserialize_logs")]
    pub logs: Vec<Log>,
    // pub logs: any[], // Define struct for this
    pub output: String,
    // pub errors: Option<any[],  // Define struct for this
    // pub itxs: any[], // Define struct for this
    //pub gasusedblock: String,  // Optional?
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

    let log_helpers = Vec::<LogHelper>::deserialize(deserializer)?;
    let mut logs = vec![];
    for log in log_helpers {
        let address = log.address.parse().expect("Invalid address");
        let topics = log.topics.into_iter().map(|topic| to_b256(&topic)).collect();
        let data = log.data.parse().expect("Invalid data");
        logs.push(Log::new(address, topics, data).unwrap());
    }
    Ok(logs)
}

fn to_b256(s: &str) -> B256 {
    let binding = hex_to_bytes(s);
    let b256_slice = binding.as_slice();
    if b256_slice.len() <= 32 {
        B256::left_padding_from(b256_slice)
    } else {
        panic!("Invalid B256 length");
    }
}

impl PrintedReceipt {
    pub fn from_console(console: String) -> Option<Self> {
        let start_pattern = "RCPT{{";
        let end_pattern = "}}RCPT";

        if let Some(start) = console.find(start_pattern) {
            let start_index = start + start_pattern.len();
            if let Some(end) = console[start_index..].find(end_pattern) {
                let end_index = start_index + end;
                let extracted = &console[start_index..end_index];
                let printed_receipt = serde_json::from_str::<PrintedReceipt>(extracted).unwrap();
                println!("{:?}", printed_receipt);
                Some(printed_receipt)
            } else {
                println!("End pattern not found.");
                None
            }
        } else {
            println!("Start pattern not found.");
            None
        }
    }
}