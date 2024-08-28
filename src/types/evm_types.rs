use alloy::primitives::aliases::BlockTimestamp;
use alloy::primitives::{Address, Log, B256};
use antelope::chain::asset::Asset;
use antelope::chain::binary_extension::BinaryExtension;
use antelope::chain::checksum::{Checksum160, Checksum256};
use antelope::chain::name::Name;
use antelope::chain::time::TimePoint;
use antelope::chain::Packer;
use antelope::serializer::Decoder;
use antelope::serializer::Encoder;
use antelope::util::hex_to_bytes;
use antelope::StructPacker;
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct EOSConfigRow {
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

#[derive(Debug, Clone, Default, Deserialize)]
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
        fn address(&self) -> Address {
            let padded = format!("{:0>40}", self.address);
            padded.parse().expect("Invalid address")
        }
    }

    let log_helpers = Vec::<LogHelper>::deserialize(deserializer)?;
    let mut logs = vec![];
    for log in log_helpers {
        let address = log.address();
        let topics = log
            .topics
            .into_iter()
            .map(|topic| to_b256(&topic))
            .collect();
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
