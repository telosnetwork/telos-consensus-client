use alloy_primitives::hex::FromHex;
use alloy_primitives::{Address, Bloom, Bytes, FixedBytes, B256};
use antelope::chain::checksum::{Checksum160, Checksum256};
use antelope::chain::name::Name;
use antelope::chain::Packer;
use antelope::serializer::Decoder;
use antelope::serializer::Encoder;
use antelope::StructPacker;
use arrowbatch::proto::ArrowBatchTypes;
use csv::{ReaderBuilder, WriterBuilder};
use log::info;
use reth_primitives::constants::MIN_PROTOCOL_BASE_FEE_U256;
use reth_primitives::{hex, U256};
use reth_rpc_types::ExecutionPayloadV1;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reth_telos::{TelosAccountStateTableRow, TelosAccountTableRow};

use arrowbatch::reader::{ArrowBatchContext, ArrowBatchReader, ArrowRow};
use tokio::time::sleep;

use antelope::api::client::APIClient;
use antelope::api::default_provider::DefaultProvider;
use antelope::api::v1::structs::{GetTableRowsParams, IndexPosition, TableIndexType};

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct AccountRow {
    pub index: u64,
    pub address: Checksum160,
    pub account: Name,
    pub nonce: u64,
    pub code: Vec<u8>,
    pub balance: Checksum256,
}

use crate::config::AppConfig;

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
    raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountDelta {
    pub index: u64,
    pub address: String,
    pub account: String,
    pub nonce: u64,
    pub code: String,
    pub balance: String,
}

impl AccountDelta {
    pub fn to_reth_type(&self) -> TelosAccountTableRow {
        let code_bytes =
            base64::decode(self.code.clone()).expect("Could not b64 decode code on account delta");
        TelosAccountTableRow {
            address: Address::from_hex(self.address.clone())
                .expect("Could not parse address on account delta"),
            account: self.account.clone(),
            nonce: self.nonce,
            code: Bytes::from_iter(code_bytes),
            balance: U256::from_str_radix(&self.balance, 16)
                .expect("Could not parse balance on account delta"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStateDelta {
    pub index: u64,
    pub scope: String,
    pub key: String,
    pub value: String,
}

impl AccountStateDelta {
    pub fn to_reth_type(&self, addr: &Address) -> TelosAccountStateTableRow {
        TelosAccountStateTableRow {
            address: addr.clone(),
            key: U256::from_str_radix(&self.key, 16)
                .expect("Could not parse key on account state delta"),
            value: U256::from_str_radix(&self.value, 16)
                .expect("Could not parse value on account state delta"),
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
pub struct GasPriceChange(u64, String);

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
    address: Address,
}

fn read_csv_to_hashmap(
    file_path: &str,
) -> Result<HashMap<u64, Address>, Box<dyn std::error::Error>> {
    let mut rdr = ReaderBuilder::new()
        .has_headers(false)
        .from_path(file_path)?;

    let mut map = HashMap::new();

    for result in rdr.deserialize() {
        let record: AddressMapRecord = result?;
        map.insert(record.index, record.address);
    }

    Ok(map)
}

fn append_record_to_csv(
    file_path: &str,
    record: &AddressMapRecord,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(file_path)?;

    let mut wtr = WriterBuilder::new().has_headers(false).from_writer(file);

    wtr.serialize(record)?;
    wtr.flush()?;

    Ok(())
}

async fn download_address_map(client: &APIClient<DefaultProvider>) -> HashMap<u64, Address> {
    let EVM_CONTRACT = Name::new_from_str("eosio.evm");
    let ACCOUNT = Name::new_from_str("account");

    let limit: u32 = 1000;
    let mut lower_bound: u64 = 0;
    let mut more_rows = true;

    let mut address_map: HashMap<u64, Address> = HashMap::new();

    info!("downloading address map...");

    while more_rows {
        let account_result = client
            .v1_chain
            .get_table_rows::<AccountRow>(GetTableRowsParams {
                code: EVM_CONTRACT,
                table: ACCOUNT,
                scope: Some(EVM_CONTRACT),
                lower_bound: Some(TableIndexType::UINT64(lower_bound)),
                upper_bound: Some(TableIndexType::UINT64(lower_bound + limit as u64)),
                limit: Some(limit),
                reverse: None,
                index_position: Some(IndexPosition::PRIMARY),
                show_payer: None,
            })
            .await
            .unwrap();

        info!(
            "lb: {}, got {} rows",
            lower_bound,
            account_result.rows.len()
        );

        let mut last = 0;
        for row in account_result.rows {
            let address = Address::from(row.address.data);
            last = row.index;
            address_map.insert(row.index, address);
        }

        more_rows = account_result.more;
        if more_rows {
            lower_bound = last + 1;
        }
    }

    info!("done fetching address map, size: {}", address_map.len());

    address_map
}

#[derive(Serialize, Debug, Clone)]
pub struct FullExecutionPayload {
    pub payload: ExecutionPayloadV1,
    pub statediffs_account: Vec<TelosAccountTableRow>,
    pub statediffs_accountstate: Vec<TelosAccountStateTableRow>,
    pub revision_changes: Vec<(u64, u64)>,
    pub gas_price_changes: Vec<(u64, U256)>,
    pub new_addresses_using_create: Vec<(u64, U256)>,
    pub new_addresses_using_openwallet: Vec<(u64, U256)>,
}

pub struct ArrowFileBlockReader {
    config: AppConfig,
    last_block: Option<FullExecutionPayload>,
    pub reader: ArrowBatchReader,
    client: APIClient<DefaultProvider>,
    addr_map: Arc<Mutex<HashMap<u64, Address>>>,
}

impl ArrowFileBlockReader {
    pub async fn new(config: &AppConfig, context: Arc<Mutex<ArrowBatchContext>>) -> Self {
        let last_block = None::<FullExecutionPayload>;

        let context_clone = Arc::clone(&context);

        // TODO: Remove this once we have live updating
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(5)).await;
                context_clone.lock().unwrap().reload_on_disk_buckets();
            }
        });

        let client = APIClient::<DefaultProvider>::default_provider(config.chain_endpoint.clone())
            .expect("Failed to create API client");

        let addr_map = if fs::metadata(&config.address_map).is_ok() {
            info!("found address map at {}, loading...", config.address_map);
            read_csv_to_hashmap(&config.address_map).unwrap()
        } else {
            let map = download_address_map(&client).await;
            let mut keys = map.keys().collect::<Vec<&u64>>();
            keys.sort();
            for key in keys {
                let value = map.get(key).unwrap();
                let record = AddressMapRecord {
                    index: *key,
                    address: value.clone(),
                };
                append_record_to_csv(&config.address_map, &record).unwrap();
            }
            map
        };

        info!("address map size: {}", addr_map.len());

        ArrowFileBlockReader {
            config: config.clone(),
            last_block,
            reader: ArrowBatchReader::new(context.clone()),
            client,
            addr_map: Arc::new(Mutex::new(addr_map)),
        }
    }

    pub async fn get_latest_block(&self) -> Option<FullExecutionPayload> {
        // TODO: remove this once websocket live updating is implemented
        let latest_block_num = self.reader.context.lock().unwrap().last_ordinal.unwrap();
        self.get_block(latest_block_num).await
    }

    pub async fn decode_row(&self, row: &ArrowRow) -> FullExecutionPayload {
        let block_num = match row[1] {
            ArrowBatchTypes::U64(value) => value,
            _ => panic!("Invalid type for evm block num"),
        };
        let timestamp = match row[2] {
            ArrowBatchTypes::U64(value) => value,
            _ => panic!("Invalid type for timestamp"),
        };
        let block_hash = match &row[3] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for block_hash"),
        };
        let evm_block_hash = match &row[4] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for evm_block_hash"),
        };
        let evm_parent_block_hash = match &row[5] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for evm_parent_block_hash"),
        };
        let receipt_hash = match &row[6] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for receipt_hash"),
        };
        let gas_used = match &row[8] {
            ArrowBatchTypes::UVar(value) => value
                .clone()
                .to_string()
                .parse()
                .expect("Gas used is not a valid u64"),
            _ => panic!("Invalid type for receipt_hash"),
        };
        let logs_bloom = match &row[9] {
            ArrowBatchTypes::Bytes(bloom_bytes) => bloom_bytes,
            _ => panic!("Invalid type for logs_bloom"),
        };

        let mut txs = Vec::new();
        match &row[12] {
            ArrowBatchTypes::StructArray(values) => {
                for tx_struct_value in values {
                    let tx_struct: TxStruct =
                        serde_json::from_value(tx_struct_value.clone()).unwrap();
                    txs.push(base64::decode(tx_struct.raw).unwrap().into());
                }
            }
            _ => panic!("Invalid type for transactions"),
        };

        let extra_data =
            Bytes::copy_from_slice(hex::decode(block_hash.as_str()).unwrap().as_slice());

        let mut statediffs_account: Vec<TelosAccountTableRow> = Vec::new();
        match &row[13] {
            ArrowBatchTypes::StructArray(values) => {
                for acc_delta_value in values {
                    let acc_delta: AccountDelta =
                        serde_json::from_value(acc_delta_value.clone()).unwrap();
                    let mut addr_map = self.addr_map.lock().unwrap();
                    if !addr_map.contains_key(&acc_delta.index) {
                        let record = AddressMapRecord {
                            index: acc_delta.index,
                            address: Address::from_hex(acc_delta.address.clone())
                                .expect("Failed to decode account delta address"),
                        };
                        addr_map.insert(record.index, record.address);
                        append_record_to_csv(&self.config.address_map, &record)
                            .expect("failed to append record to addr map file");
                        info!("update address map: {:?}", record);
                    }
                    drop(addr_map);
                    statediffs_account.push(acc_delta.to_reth_type());
                }
            }
            _ => panic!("Invalid type for account deltas"),
        };

        let mut statediffs_accountstate = Vec::new();
        match &row[14] {
            ArrowBatchTypes::StructArray(values) => {
                for acc_state_delta_value in values {
                    let acc_state_delta: AccountStateDelta =
                        serde_json::from_value(acc_state_delta_value.clone()).unwrap();
                    let mut addr_map = self.addr_map.lock().unwrap();
                    let account_index = Name::new_from_str(&acc_state_delta.scope).n;
                    let maybe_addr = addr_map.get(&account_index);
                    let addr = if maybe_addr.is_some() {
                        maybe_addr.unwrap().clone()
                    } else {
                        info!(
                            "address for index {} not in map, doing http query...",
                            acc_state_delta.index
                        );
                        let EVM_CONTRACT = Name::new_from_str("eosio.evm");
                        let ACCOUNT = Name::new_from_str("account");
                        let account_result = self
                            .client
                            .v1_chain
                            .get_table_rows::<AccountRow>(GetTableRowsParams {
                                code: EVM_CONTRACT,
                                table: ACCOUNT,
                                scope: Some(EVM_CONTRACT),
                                lower_bound: Some(TableIndexType::UINT64(acc_state_delta.index)),
                                upper_bound: Some(TableIndexType::UINT64(acc_state_delta.index)),
                                limit: Some(1),
                                reverse: None,
                                index_position: Some(IndexPosition::PRIMARY),
                                show_payer: None,
                            })
                            .await
                            .unwrap();
                        if account_result.rows.len() == 0 {
                            panic!("get_table_rows returned 0 rows!");
                        }
                        let address_checksum = account_result.rows[0].address;
                        let address = Address::from(address_checksum.data);
                        let record = AddressMapRecord {
                            index: acc_state_delta.index,
                            address,
                        };
                        addr_map.insert(record.index, record.address);
                        append_record_to_csv(&self.config.address_map, &record)
                            .expect("failed to append record to addr map file");
                        info!("update address map: {:?}", record);
                        address
                    };
                    statediffs_accountstate.push(acc_state_delta.to_reth_type(&addr));
                    drop(addr_map);
                }
            }
            _ => panic!("Invalid type for account state deltas"),
        };

        let mut gas_price_changes = Vec::new();
        match &row[15] {
            ArrowBatchTypes::StructArray(values) => {
                for gas_price_change_value in values {
                    let gas_price_change: GasPriceChange =
                        serde_json::from_value(gas_price_change_value.clone())
                            .expect("Could not deserialize gas change");

                    gas_price_changes.push((
                        gas_price_change.0,
                        U256::from_str_radix(&gas_price_change.1, 16)
                            .expect("Could not parse hex string into address on gas price change"),
                    ));
                }
            }
            _ => panic!("Invalid type for gas price changes"),
        };

        let mut revision_changes = Vec::new();
        match &row[16] {
            ArrowBatchTypes::StructArray(values) => {
                for rev_change_value in values {
                    let rev_change: RevisionChange =
                        serde_json::from_value(rev_change_value.clone())
                            .expect("Could not deserialize rev change");

                    revision_changes.push((rev_change.0, rev_change.1));
                }
            }
            _ => panic!("Invalid type for revision changes"),
        };

        let mut new_addresses_using_openwallet = Vec::new();
        match &row[17] {
            ArrowBatchTypes::StructArray(values) => {
                for wallet_event_value in values {
                    let wallet_event: AddressCreationEvent =
                        serde_json::from_value(wallet_event_value.clone())
                            .expect("Could not deserialize new open wallet");

                    new_addresses_using_openwallet.push((
                        wallet_event.0,
                        U256::from_str_radix(wallet_event.2.as_str(), 16)
                            .expect("Invalid address on open wallet event"),
                    ));
                }
            }
            _ => panic!("Invalid type for new openwallet addresses"),
        };

        let mut new_addresses_using_create = Vec::new();
        match &row[18] {
            ArrowBatchTypes::StructArray(values) => {
                for wallet_event_value in values {
                    let wallet_event: AddressCreationEvent =
                        serde_json::from_value(wallet_event_value.clone())
                            .expect("Could not deserialize new create wallet");

                    let acc_row: &TelosAccountTableRow = statediffs_account
                        .iter()
                        .find(|delta| delta.account == wallet_event.1)
                        .expect("Could not find a matching account delta for the create event");

                    new_addresses_using_create.push((
                        wallet_event.0,
                        U256::from_be_slice(acc_row.address.as_slice()),
                    ));
                }
            }
            _ => panic!("Invalid type for new create addresses"),
        };

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
                transactions: txs,
            },
            statediffs_account,
            statediffs_accountstate,
            revision_changes,
            gas_price_changes,
            new_addresses_using_create,
            new_addresses_using_openwallet,
        }
    }

    pub async fn get_block(&self, block_num: u64) -> Option<FullExecutionPayload> {
        if let Some(ref last_block) = self.last_block {
            if last_block.payload.block_number == block_num {
                return Some(last_block.clone());
            }
            if last_block.payload.block_number > block_num {
                return None;
            }
        }
        let row = self.reader.get_row(block_num)?;

        Some(self.decode_row(&row).await)
    }
}

#[cfg(test)]
mod tests {
    use arrowbatch::reader::{ArrowBatchConfig, ArrowBatchContext};
    use reth_primitives::U256;

    use crate::{
        arrow_block_reader::{ArrowFileBlockReader, FullExecutionPayload},
        config::AppConfig,
    };

    const GAS_CHANGE_BLOCK: u64 = 261916623;
    const REV_CHANGE_BLOCK: u64 = 332317496;
    const CREATE_WALLET_BLOCK: u64 = 344680644;
    const OPEN_WALLET_BLOCK: u64 = 348379125;

    #[tokio::test]
    async fn test_arrow_block_reader() {
        let config = ArrowBatchConfig {
            data_dir: "/data/arrow-data-full".to_string(),
            bucket_size: 10_000_000_u64,
            dump_size: 100_000_u64,
        };

        let context = ArrowBatchContext::new(config);

        let app_config = AppConfig {
            base_url: "".to_string(),
            jwt_secret: "".to_string(),
            arrow_data: "".to_string(),
            address_map: "address_map.csv".to_string(),
            batch_size: 0,
            chain_endpoint: "https://mainnet.telos.net".to_string(),
        };

        let reader = ArrowFileBlockReader::new(&app_config, context.clone());

        let mut target_block: FullExecutionPayload;

        target_block = reader.get_block(GAS_CHANGE_BLOCK).await.unwrap();
        assert_eq!(target_block.gas_price_changes.len(), 1);
        assert_eq!(target_block.gas_price_changes.first().unwrap().0, 0);
        assert_eq!(
            target_block.gas_price_changes.first().unwrap().1,
            U256::from_str_radix(
                "0000000000000000000000000000000000000000000000000000007548a6d7b3",
                16
            )
            .unwrap()
        );

        target_block = reader.get_block(REV_CHANGE_BLOCK).await.unwrap();
        assert_eq!(target_block.revision_changes.len(), 1);
        assert_eq!(target_block.revision_changes.first().unwrap().0, 0);
        assert_eq!(target_block.revision_changes.first().unwrap().1, 1);

        target_block = reader.get_block(CREATE_WALLET_BLOCK).await.unwrap();
        assert_eq!(target_block.new_addresses_using_create.len(), 1);
        assert_eq!(
            target_block.new_addresses_using_create.first().unwrap().0,
            0
        );
        assert_eq!(
            target_block.new_addresses_using_create.first().unwrap().1,
            U256::from_str_radix("f42c8cc248b4f6548861e3bae31d065505e379f1", 16).unwrap()
        );

        target_block = reader.get_block(OPEN_WALLET_BLOCK).await.unwrap();
        assert_eq!(target_block.new_addresses_using_openwallet.len(), 1);
        assert_eq!(
            target_block
                .new_addresses_using_openwallet
                .first()
                .unwrap()
                .0,
            0
        );
        assert_eq!(
            target_block
                .new_addresses_using_openwallet
                .first()
                .unwrap()
                .1,
            U256::from_str_radix("70bffd7e97615c49462e57d63d2d1ed61e68c134", 16).unwrap()
        );
    }
}
