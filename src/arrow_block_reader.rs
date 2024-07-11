use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use csv::{ReaderBuilder, WriterBuilder};
use serde::{Serialize, Deserialize};
use alloy_primitives::{FixedBytes, Address, Bytes, Bloom, B256};
use alloy_primitives::hex::FromHex;
use arrowbatch::proto::ArrowBatchTypes;
use reth_primitives::constants::MIN_PROTOCOL_BASE_FEE_U256;
use reth_primitives::{hex, U256};
use reth_rpc_types::ExecutionPayloadV1;

use reth_telos::{
    TelosAccountTableRow,
    TelosAccountStateTableRow
};

use arrowbatch::reader::{ArrowBatchContext, ArrowBatchReader};
use tokio::time::sleep;

extern crate base64;

macro_rules! null_hash {
    () => {
        B256::from_hex("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421").unwrap()
    };
}


/* Example:
 * Block 348379125
 *
 *  {
 *      "hash": "21c06816edc8c7cb136549830aa5bd2595821195c013e8d25a1fd22dc721228d",
 *      "raw": "+EiAgIJSCJRwv/1+l2FcSUYuV9Y9LR7WHmjBNIlpTOYy0AItQACAKqAUw9gZ6J6S+koY8mdTgrJ6w8w+s+qX1kj3zkex0YeNbYA="
 *  }
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TxStruct {
    hash: String,
    raw: String
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountDelta {
    pub index: u64,
    pub address: String,
    pub account: String,
    pub nonce: u64,
    pub code: String,
    pub balance: String
}

impl AccountDelta {
    pub fn to_reth_type(&self) -> TelosAccountTableRow {
        let code_bytes = base64::decode(self.code.clone()).expect("Could not b64 decode code on account delta");
        TelosAccountTableRow {
            address: Address::from_hex(self.address.clone()).expect("Could not parse address on account delta"),
            account: self.account.clone(),
            nonce: self.nonce,
            code: Bytes::from_iter(code_bytes),
            balance: U256::from_str_radix(&self.balance, 16).expect("Could not parse balance on account delta")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStateDelta {
    pub index: u64,
    pub key: String,
    pub value: String
}

impl AccountStateDelta {
    pub fn to_reth_type(&self, addr: &Address) -> TelosAccountStateTableRow {
        TelosAccountStateTableRow {
            address: addr.clone(),
            key: U256::from_str_radix(&self.key, 16).expect("Could not parse key on account state delta"),
            value: U256::from_str_radix(&self.value, 16).expect("Could not parse value on account state delta")
        }
    }
}


/* Example:
 * Block 332317496
 *
 *  [
 *      0,
 *      1
 *  ]
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RevisionChange(u64, u64);

/* Example:
 * Block 261916623
 *
 *  [
 *      0,
 *      "0000000000000000000000000000000000000000000000000000007548a6d7b3"
 *  ]
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GasPriceChange(
    u64,
    U256
);

/* Example OpenWallet:
 * Block 348379125
 *
 *  [
 *      0,
 *      "deposit.evm",
 *      "70bffd7e97615c49462e57d63d2d1ed61e68c134"
 *  ]
 *
 * Example Create:
 * Block 344680644
 *
 *  [
 *      0,
 *      "iyanuadesuyi",
 *      "create"
 *  ]
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AddressCreationEvent(u64, String, String);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AddressMapRecord {
    index: u64,
    address: Address
}

pub const ADDR_MAP_PATH: &str = "addr_map.csv";

fn read_csv_to_hashmap(file_path: &str) -> Result<HashMap<u64, Address>, Box<dyn std::error::Error>> {
    let mut rdr = ReaderBuilder::new().from_path(file_path)?;
    let mut map = HashMap::new();

    for result in rdr.deserialize() {
        let record: AddressMapRecord = result?;
        map.insert(record.index, record.address);
    }

    Ok(map)
}

fn append_record_to_csv(file_path: &str, record: &AddressMapRecord) -> Result<(), Box<dyn std::error::Error>> {
    let file_exists = Path::new(file_path).exists();
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(file_path)?;

    let mut wtr = WriterBuilder::new()
        .has_headers(!file_exists)
        .from_writer(file);

    if !file_exists {
        wtr.write_record(&["index", "address"])?;
    }

    wtr.serialize(record)?;
    wtr.flush()?;

    Ok(())
}

#[derive(Serialize, Debug, Clone)]
pub struct FullExecutionPayload {
    pub payload: ExecutionPayloadV1,
    pub statediffs_account: Vec<TelosAccountTableRow>,
    pub statediffs_accountstate: Vec<TelosAccountStateTableRow>,
    pub revision_changes: Vec<(u64,u64)>,
    pub gas_price_changes: Vec<(u64,U256)>,
    pub new_addresses_using_create: Vec<(u64,U256)>,
    pub new_addresses_using_openwallet: Vec<(u64,U256)>
}

pub struct ArrowFileBlockReader {
    last_block: Option<FullExecutionPayload>,
    reader: ArrowBatchReader,
    addr_map: Arc<Mutex<HashMap<u64, Address>>>
}

impl ArrowFileBlockReader {
    pub fn new(context: Arc<Mutex<ArrowBatchContext>>) -> Self {
        let last_block = None::<FullExecutionPayload>;

        let context_clone = Arc::clone(&context);

        // TODO: Remove this once we have live updating
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(5)).await;
                context_clone.lock().unwrap().reload_on_disk_buckets();
            }
        });

        let addr_map = if fs::metadata(ADDR_MAP_PATH).is_ok() {
            read_csv_to_hashmap(ADDR_MAP_PATH).unwrap()
        } else {
            HashMap::new()
        };

        ArrowFileBlockReader {
            last_block,
            reader: ArrowBatchReader::new(context.clone()),
            addr_map: Arc::new(Mutex::new(addr_map))
        }
    }

    pub fn get_latest_block(&self) -> Option<FullExecutionPayload> {
        // TODO: remove this once websocket live updating is implemented
        let latest_block_num = self.reader.context.lock().unwrap().last_ordinal.unwrap();
        self.get_block(latest_block_num)
    }

    pub fn get_block(&self, block_num: u64) -> Option<FullExecutionPayload> {
        if let Some(ref last_block) = self.last_block {
            if last_block.payload.block_number == block_num {
                return Some(last_block.clone());
            }
            if last_block.payload.block_number > block_num {
                return None;
            }
        }
        let block = self.reader.get_row(block_num)?;

        let timestamp = match block[2] {
            ArrowBatchTypes::U64(value) => value,
            _ => panic!("Invalid type for timestamp")
        };
        let block_hash = match &block[3] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for block_hash")
        };
        let evm_block_hash = match &block[4] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for evm_block_hash")
        };
        let evm_parent_block_hash = match &block[5] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for evm_parent_block_hash")
        };
        let receipt_hash = match &block[6] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for receipt_hash")
        };
        let gas_used = match &block[8] {
            ArrowBatchTypes::UVar(value) => value.clone().to_string().parse().expect("Gas used is not a valid u64"),
            _ => panic!("Invalid type for receipt_hash")
        };
        let logs_bloom = match &block[9] {
            ArrowBatchTypes::Bytes(bloom_bytes) => bloom_bytes,
            _ => panic!("Invalid type for logs_bloom")
        };

        let mut txs = Vec::new();
        match &block[12] {
            ArrowBatchTypes::StructArray(values) => {
                 for tx_struct_value in values {
                     let tx_struct: TxStruct = serde_json::from_value(tx_struct_value.clone()).unwrap();
                     txs.push(base64::decode(tx_struct.raw).unwrap().into());
                 }
            },
            _ => panic!("Invalid type for transactions")
        };

        let extra_data =
            Bytes::copy_from_slice(hex::decode(block_hash.as_str()).unwrap().as_slice());

        let mut statediffs_account: Vec<TelosAccountTableRow> = Vec::new();
        match &block[13] {
            ArrowBatchTypes::StructArray(values) => {
                for acc_delta_value in values {
                    let acc_delta: AccountDelta = serde_json::from_value(acc_delta_value.clone()).unwrap();
                    let mut addr_map = self.addr_map.lock().unwrap();
                    if !addr_map.contains_key(&acc_delta.index) {
                        let record = AddressMapRecord {
                            index: acc_delta.index,
                            address: Address::from_hex(acc_delta.address.clone()).expect("Failed to decode account delta address")
                        };
                        addr_map.insert(record.index, record.address);
                        append_record_to_csv(ADDR_MAP_PATH, &record).expect("failed to append record to addr map file");
                    }
                    drop(addr_map);
                    statediffs_account.push(acc_delta.to_reth_type());
                }
            },
            _ => panic!("Invalid type for account deltas")
        };

        let mut statediffs_accountstate = Vec::new();
        match &block[14] {
            ArrowBatchTypes::StructArray(values) => {
                for acc_state_delta_value in values {
                    let acc_state_delta: AccountStateDelta = serde_json::from_value(acc_state_delta_value.clone()).unwrap();
                    let addr_map = self.addr_map.lock().unwrap();
                    let addr = addr_map.get(&acc_state_delta.index)
                        .expect("Cannot figure out address for account state delta");
                    statediffs_accountstate.push(acc_state_delta.to_reth_type(addr));
                    drop(addr_map);
                }
            },
            _ => panic!("Invalid type for account state deltas")
        };

        let mut gas_price_changes = Vec::new();
        match &block[15] {
            ArrowBatchTypes::StructArray(values) => {
                for gas_price_change_value in values {
                    let gas_price_change: GasPriceChange = serde_json::from_value(gas_price_change_value.clone())
                        .expect("Could not deserialize gas change");

                    gas_price_changes.push((
                        gas_price_change.0,
                        gas_price_change.1
                    ));
                }
            },
            _ => panic!("Invalid type for gas price changes")
        };

        let mut revision_changes = Vec::new();
        match &block[16] {
            ArrowBatchTypes::StructArray(values) => {
                for rev_change_value in values {
                    let rev_change: RevisionChange = serde_json::from_value(rev_change_value.clone())
                        .expect("Could not deserialize rev change");

                    revision_changes.push((rev_change.0, rev_change.1));
                }
            },
            _ => panic!("Invalid type for revision changes")
        };

        let mut new_addresses_using_openwallet = Vec::new();
        match &block[17] {
            ArrowBatchTypes::StructArray(values) => {
                for wallet_event_value in values {
                    let wallet_event: AddressCreationEvent = serde_json::from_value(wallet_event_value.clone())
                        .expect("Could not deserialize new open wallet");

                    new_addresses_using_openwallet.push((
                        wallet_event.0,
                        U256::from_str_radix(wallet_event.2.as_str(), 16).expect("Invalid address on open wallet event")
                    ));
                }
            },
            _ => panic!("Invalid type for new openwallet addresses")
        };

        let mut new_addresses_using_create = Vec::new();
        match &block[18] {
            ArrowBatchTypes::StructArray(values) => {
                for wallet_event_value in values {
                    let wallet_event: AddressCreationEvent = serde_json::from_value(wallet_event_value.clone())
                        .expect("Could not deserialize new create wallet");

                    let acc_row: &TelosAccountTableRow = statediffs_account.iter().find(|delta| delta.account == wallet_event.1)
                        .expect("Could not find a matching account delta for the create event");

                    new_addresses_using_create.push((wallet_event.0, U256::from_be_slice(acc_row.address.as_slice())));
                }
            },
            _ => panic!("Invalid type for new create addresses")
        };


        Some(
            FullExecutionPayload {
                payload: ExecutionPayloadV1 {
                    parent_hash: FixedBytes::from_hex(evm_parent_block_hash).unwrap(),
                    fee_recipient: Address::ZERO,
                    state_root: null_hash!(),
                    receipts_root: FixedBytes::from_hex(receipt_hash).unwrap(),
                    logs_bloom: Bloom::from_slice(&logs_bloom),
                    prev_randao: Default::default(),
                    block_number: block_num,
                    gas_limit: 0x7fffffffu64,
                    gas_used,
                    timestamp,
                    extra_data,
                    base_fee_per_gas: MIN_PROTOCOL_BASE_FEE_U256,
                    block_hash: FixedBytes::from_hex(evm_block_hash).unwrap(),
                    transactions: txs
                },
                statediffs_account,
                statediffs_accountstate,
                revision_changes,
                gas_price_changes,
                new_addresses_using_create,
                new_addresses_using_openwallet
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use arrowbatch::reader::{ArrowBatchConfig, ArrowBatchContext};
    use reth_primitives::U256;

    use crate::arrow_block_reader::{ArrowFileBlockReader, FullExecutionPayload};

    const GAS_CHANGE_BLOCK: u64 = 261916623;
    const REV_CHANGE_BLOCK: u64 = 332317496;
    const CREATE_WALLET_BLOCK: u64 = 344680644;
    const OPEN_WALLET_BLOCK: u64 = 348379125;

    #[tokio::test]
    async fn test_arrow_block_reader() {
        let config = ArrowBatchConfig {
            data_dir: "/data/arrow-data-full".to_string(),
            bucket_size: 10_000_000_u64,
            dump_size: 100_000_u64
        };

        let context = ArrowBatchContext::new(config);

        let reader = ArrowFileBlockReader::new(context.clone());

        let mut target_block: FullExecutionPayload;

        target_block = reader.get_block(GAS_CHANGE_BLOCK).unwrap();
        assert_eq!(target_block.gas_price_changes.len(), 1);
        assert_eq!(target_block.gas_price_changes.first().unwrap().0, 0);
        assert_eq!(
            target_block.gas_price_changes.first().unwrap().1,
            U256::from_str_radix("0000000000000000000000000000000000000000000000000000007548a6d7b3", 16).unwrap()
        );

        target_block = reader.get_block(REV_CHANGE_BLOCK).unwrap();
        assert_eq!(target_block.revision_changes.len(), 1);
        assert_eq!(target_block.revision_changes.first().unwrap().0, 0);
        assert_eq!(target_block.revision_changes.first().unwrap().1, 1);

        target_block = reader.get_block(CREATE_WALLET_BLOCK).unwrap();
        assert_eq!(target_block.new_addresses_using_create.len(), 1);
        assert_eq!(target_block.new_addresses_using_create.first().unwrap().0, 0);
        assert_eq!(
            target_block.new_addresses_using_create.first().unwrap().1,
            U256::from_str_radix("f42c8cc248b4f6548861e3bae31d065505e379f1", 16).unwrap()
        );

        target_block = reader.get_block(OPEN_WALLET_BLOCK).unwrap();
        assert_eq!(target_block.new_addresses_using_openwallet.len(), 1);
        assert_eq!(target_block.new_addresses_using_openwallet.first().unwrap().0, 0);
        assert_eq!(
            target_block.new_addresses_using_openwallet.first().unwrap().1,
            U256::from_str_radix("70bffd7e97615c49462e57d63d2d1ed61e68c134", 16).unwrap()
        );
    }
}
