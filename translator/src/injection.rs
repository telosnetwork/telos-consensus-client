use std::collections::HashMap;
use std::str::FromStr;
use alloy::primitives::{Address, Bytes, U256};
use antelope::api::client::{APIClient, DefaultProvider};
use antelope::api::v1::structs::{GetTableRowsParams, TableIndexType};
use antelope::{name, StructPacker};
use antelope::chain::checksum::{Checksum160, Checksum256};
use antelope::chain::{Encoder, Decoder, Packer};
use antelope::chain::name::Name;
use antelope::util::{bytes_to_hex, hex_to_bytes};
use reth_telos_rpc_engine_api::structs::{TelosAccountStateTableRow, TelosAccountTableRow, TelosEngineAPIExtraFields};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, StructPacker)]
pub struct ConfigRow {
    pub trx_index: u64,
    pub last_block: u64,
    pub gas_used_block: String,
    pub gas_price: String,
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

pub async fn dump_storage(telos_rpc: &str, block_delta: u32) -> TelosEVMStateJSON {
    let api_client = APIClient::<DefaultProvider>::default_provider(telos_rpc.into(), Some(5)).unwrap();
    let info = api_client.v1_chain.get_info().await.unwrap();

    let evm_block_num = info.head_block_num - block_delta;
    println!("Telos EVM Block Number: {:?}", evm_block_num);

    let query_params = GetTableRowsParams {
        code: name!("eosio.evm"),
        table: name!("config"),
        scope: None,
        lower_bound: None,
        upper_bound: None,
        limit: Some(1),
        reverse: None,
        index_position: None,
        show_payer: None,
    };
    let config_row = api_client.v1_chain.get_table_rows::<ConfigRow>(query_params).await.expect("Couldnt get config row!");
    assert_eq!(config_row.rows.len(), 1);
    let gas_price = U256::from_str(&config_row.rows.get(0).unwrap().gas_price).unwrap();

    let mut has_more = true;
    let mut lower_bound = Some(TableIndexType::UINT64(0));

    let mut accounts = Vec::new();

    while has_more {
        let query_params = GetTableRowsParams {
            code: name!("eosio.evm"),
            table: name!("account"),
            scope: None,
            lower_bound,
            upper_bound: None,
            limit: Some(5000),
            reverse: None,
            index_position: None,
            show_payer: None,
        };
        let account_rows = api_client.v1_chain.get_table_rows::<AccountRow>(query_params).await;
        if let Ok(account_rows) = account_rows {
            lower_bound = account_rows.next_key;
            has_more = lower_bound.is_some();
            for account_row in account_rows.rows {
                lower_bound = Some(TableIndexType::UINT64(account_row.index + 1));
                accounts.push(dump_account(&account_row, &api_client).await);
            }
        } else {
            panic!("Failed to fetch account row");
        }
    }

    TelosEVMStateJSON {
        evm_block_num,
        gas_price,
        accounts,
    }
}


#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountJSON {
    address: String,
    account: String,
    balance: String,
    nonce: u64,
    code: String,
    storage: HashMap<String, String>
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelosEVMStateJSON {
    evm_block_num: u32,
    gas_price: U256,
    accounts: Vec<AccountJSON>,
}

async fn dump_account(account_row: &AccountRow, api_client: &APIClient<DefaultProvider>) -> AccountJSON {
    let address = Address::from_slice(account_row.address.data.as_slice());
    let telos_balance = U256::from_be_slice(account_row.balance.data.as_slice());

    println!("Account: {:?}", address);
    println!("Telos balance: {:?}", telos_balance);
    println!("Telos nonce: {}", account_row.nonce);
    println!("Telos present code: {}", account_row.code.len() > 0);


    let storage = dump_account_storage(account_row, api_client).await;

    AccountJSON {
        address: address.to_string(),
        account: account_row.account.to_string(),
        balance: telos_balance.to_string(),
        nonce: account_row.nonce,
        code: bytes_to_hex(&account_row.code),
        storage
    }
}

async fn dump_account_storage(account_row: &AccountRow, api_client: &APIClient<DefaultProvider>) -> std::collections::HashMap<String, String> {
    let address = Address::from_slice(account_row.address.data.as_slice());

    let mut has_more = true;
    let mut lower_bound = Some(TableIndexType::UINT64(0));

    let mut storage = HashMap::new();

    while has_more {
        let scope = if account_row.index == 0 {
            Some(name!(""))
        } else {
            Some(Name::from_u64(account_row.index))
        };
        let query_params = GetTableRowsParams {
            code: name!("eosio.evm"),
            table: name!("accountstate"),
            scope: Some(scope.unwrap()),
            lower_bound,
            upper_bound: None,
            limit: Some(5000),
            reverse: None,
            index_position: None,
            show_payer: None,
        };
        let account_state_rows = api_client.v1_chain.get_table_rows::<AccountStateRow>(query_params).await;
        if let Ok(account_state_rows) = account_state_rows {
            lower_bound = account_state_rows.next_key;
            has_more = lower_bound.is_some();
            for account_state_row in account_state_rows.rows {
                let key = U256::from_be_slice(account_state_row.key.data.as_slice());
                let telos_value: U256 = U256::from_be_slice(account_state_row.value.data.as_slice());
                println!("Storage account: {:?} with scope: {:?} and key: {:?}", address, scope.unwrap(), key);
                println!("Telos Storage value: {:?}", telos_value);

                storage.insert(key.to_string(), telos_value.to_string());

                lower_bound = Some(TableIndexType::UINT64(account_state_row.index + 1));
            }
        } else {
            panic!("Failed to fetch account state row");
        }
    }

    return storage;
}

pub fn generate_extra_fields_from_json(state_json: &str) -> (u32, TelosEngineAPIExtraFields) {
    // Deserialize the JSON string back into the struct
    let telos_state: TelosEVMStateJSON = serde_json::from_str(state_json).unwrap();

    let mut exec_accounts = Vec::new();
    let mut exec_accounts_state = Vec::new();

    for account in telos_state.accounts {
        exec_accounts.push(TelosAccountTableRow {
            removed: false,
            address: account.address.parse().unwrap(),
            account: account.account,
            nonce: account.nonce,
            code: Bytes::from(hex_to_bytes(&account.code)),
            balance: account.balance.parse().unwrap(),
        });
        for (key, value) in account.storage {
            exec_accounts_state.push(TelosAccountStateTableRow {
                removed: false,
                address: account.address.parse().unwrap(),
                key: U256::from_str(&key).unwrap(),
                value: U256::from_str(&value).unwrap(),
            });
        }
    }

    (telos_state.evm_block_num, TelosEngineAPIExtraFields {
        statediffs_account: Some(exec_accounts),
        statediffs_accountstate: Some(exec_accounts_state),
        revision_changes: None,
        gasprice_changes: Some((0, telos_state.gas_price)),
        new_addresses_using_create: Some(vec![]),
        new_addresses_using_openwallet: Some(vec![]),
        receipts: Some(vec![]),
    })
}
