use alloy::hex::FromHex;
use alloy::primitives::{Bytes, B256, U256};
use alloy_consensus::{Signed, TxEnvelope, TxLegacy};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use thiserror::Error;

use chrono::{DateTime, Utc};
use num_bigint::BigUint;

fn iso_to_unix(iso_str: &str) -> i64 {
    let dt: DateTime<Utc> = iso_str.parse().expect("Failed to parse ISO 8601 string");
    dt.timestamp()
}

/*
 * leap-mock client
 */
#[derive(Serialize, Deserialize, Debug)]
pub struct JumpInfo {
    pub from: u32,
    pub to: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ChainDescriptor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    pub jumps: Vec<JumpInfo>,
    pub chain_start_block: u32,
    pub chain_end_block: u32,
    pub session_start_block: u32,
    pub session_stop_block: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ship_port: Option<u16>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SetChainResponse {
    pub blocks: HashMap<u32, Vec<(u32, String)>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MockResponse<T> {
    pub result: Option<T>,
    pub error: Option<String>,
}

#[derive(Error, Debug)]
pub enum LeapMockError {
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("API error: {0}")]
    ApiError(String),
}

#[derive(Debug)]
pub struct LeapMockClient {
    base_url: String,
    client: Client,
}

impl LeapMockClient {
    pub fn new(base_url: &str) -> Self {
        LeapMockClient {
            base_url: base_url.to_string(),
            client: Client::new(),
        }
    }

    #[allow(dead_code)]
    pub async fn set_chain(
        &self,
        params: ChainDescriptor,
    ) -> Result<SetChainResponse, LeapMockError> {
        let url = format!("{}/set_chain", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&params)
            .send()
            .await?
            .json::<MockResponse<SetChainResponse>>()
            .await?;

        match resp.result {
            Some(result) => Ok(result),
            None => Err(LeapMockError::ApiError(
                resp.error.unwrap_or_else(|| "Unknown error".to_string()),
            )),
        }
    }
}

/*
 * 1.5 dataset helpers
 */

#[derive(Debug, Serialize, Deserialize)]
pub struct TelosEVM15Delta {
    #[serde(rename = "@timestamp")]
    pub timestamp: String,

    pub block_num: u32,

    #[serde(rename = "@global")]
    pub global: GlobalBlockData,

    #[serde(rename = "@evmPrevBlockHash")]
    pub evm_prev_block_hash: String,

    #[serde(rename = "@evmBlockHash")]
    pub evm_block_hash: String,

    #[serde(rename = "@blockHash")]
    pub block_hash: String,

    #[serde(rename = "@receiptsRootHash")]
    pub receipts_root_hash: String,

    #[serde(rename = "@transactionsRoot")]
    pub transactions_root: String,

    #[serde(rename = "gasUsed")]
    pub gas_used: String,
    #[serde(rename = "gasLimit")]
    pub gas_limit: String,
    pub size: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GlobalBlockData {
    pub block_num: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TelosEVM15Action {
    #[serde(rename = "@timestamp")]
    pub timestamp: String,

    #[serde(rename = "@raw")]
    pub raw: RawEvmTx,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccessList {
    pub address: String,
    pub storage_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RawEvmTx {
    pub hash: String,
    pub trx_index: u32,
    pub block: u32,
    pub block_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub input_data: String,
    pub input_trimmed: String,
    pub value: String,
    pub value_d: String,
    pub nonce: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<String>,
    pub gas_limit: String,
    pub status: u8,
    pub itxs: Vec<String>,
    pub epoch: u64,
    pub createdaddr: String,
    pub gasused: String,
    pub gasusedblock: String,
    pub charged_gas_price: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_list: Option<Vec<AccessList>>,
    pub output: String,
    pub raw: String,
    pub v: String,
    pub r: String,
    pub s: String,
    pub from: String,
}

use serde_json::Result as SerdeResult;
use telos_translator_rs::block::TelosEVMBlock;

pub struct TelosEVM15Block {
    pub block: TelosEVM15Delta,
    pub transactions: Vec<TelosEVM15Action>,
}

pub fn load_15_data() -> SerdeResult<HashMap<u32, TelosEVM15Block>> {
    let deltas_str = include_str!("testcontainer-deltas-v1.5.json");
    let deltas: Vec<TelosEVM15Delta> = serde_json::from_str(deltas_str)?;

    let block_delta = deltas[0].block_num - deltas[0].global.block_num;

    let mut blocks = HashMap::new();
    for delta in deltas {
        blocks.insert(
            delta.block_num,
            TelosEVM15Block {
                block: delta,
                transactions: vec![],
            },
        );
    }

    let action_str = include_str!("testcontainer-actions-v1.5.json");
    let actions: Vec<TelosEVM15Action> = serde_json::from_str(action_str)?;

    for action in actions {
        blocks
            .get_mut(&(action.raw.block + block_delta))
            .unwrap()
            .transactions
            .push(action);
    }

    Ok(blocks)
}
use num_traits::Num;
use telos_translator_rs::transaction::TelosEVMTransaction;

pub fn compare_legacy(stx: &Signed<TxLegacy>, tx15: &RawEvmTx, trx_index: usize, block_num: u32) {
    let tx = stx.tx();
    assert_eq!(
        tx.nonce,
        tx15.nonce.parse::<u64>().unwrap(),
        "nonce difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        tx.gas_price.to_string(),
        tx15.gas_price.clone().unwrap_or("0".to_string()),
        "gas_price difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        tx.gas_limit,
        tx15.gas_limit.parse::<u128>().unwrap(),
        "gas_limit difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    if let Some(to) = tx.to.to() {
        assert_eq!(
            to.to_string().to_lowercase(),
            tx15.to.clone().unwrap(),
            "\"to\" field difference at tx #{} of evm block #{}",
            trx_index,
            block_num
        );
    } else {
        assert!(
            tx15.to.is_none(),
            "\"to\" field missing at tx #{} of evm block #{}",
            trx_index,
            block_num
        );
    }
    assert_eq!(
        tx.value,
        U256::from_str_radix(&tx15.value, 16).unwrap(),
        "value difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        tx.input,
        Bytes::from_hex(&tx15.input_data).unwrap(),
        "input difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        stx.signature().v().to_u64(),
        tx15.v.parse::<u64>().unwrap(),
        "signature v difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        BigUint::from_str(&stx.signature().r().to_string()).unwrap(),
        BigUint::from_str_radix(&tx15.r[2..], 16).unwrap(),
        "signature r difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        BigUint::from_str(&stx.signature().s().to_string()).unwrap(),
        BigUint::from_str_radix(&tx15.s[2..], 16).unwrap(),
        "signature s difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );

    assert_eq!(
        stx.hash().to_string(),
        tx15.hash,
        "tx hash difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
}

pub fn compare_1559(envelope: &TxEnvelope, tx15: &RawEvmTx, trx_index: usize, block_num: u32) {
    let stx = envelope.as_eip1559().unwrap();
    let tx = stx.tx();
    assert_eq!(
        tx.nonce,
        tx15.nonce.parse::<u64>().unwrap(),
        "nonce difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        tx.max_priority_fee_per_gas,
        tx15.max_priority_fee_per_gas
            .clone()
            .unwrap()
            .parse::<u128>()
            .unwrap(),
        "max_priority_fee_per_gas difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        tx.max_fee_per_gas,
        tx15.max_fee_per_gas
            .clone()
            .unwrap()
            .parse::<u128>()
            .unwrap(),
        "max_fee_per_gas difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        tx.gas_limit,
        tx15.gas_limit.parse::<u128>().unwrap(),
        "gas_limit difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    if let Some(to) = tx.to.to() {
        assert_eq!(
            to.to_string().to_lowercase(),
            tx15.to.clone().unwrap(),
            "\"to\" difference at tx #{} of evm block #{}",
            trx_index,
            block_num
        );
    } else {
        assert!(
            tx15.to.is_none(),
            "\"to\" field missing at tx #{} of evm block #{}",
            trx_index,
            block_num
        );
    }
    assert_eq!(
        tx.value,
        U256::from_str_radix(&tx15.value, 16).unwrap(),
        "value difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    // assert_eq!(
    //     tx.access_list.size(),
    //     tx15.access_list.unwrap().len(),
    //     "access_list size difference at tx #{} of evm block #{}",
    //     trx_index, block_num
    // );
    assert_eq!(
        tx.input,
        Bytes::from_hex(&tx15.input_data).unwrap(),
        "input difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        stx.signature().v().to_u64(),
        tx15.v.parse::<u64>().unwrap(),
        "signature v difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        BigUint::from_str(&stx.signature().r().to_string()).unwrap(),
        BigUint::from_str_radix(&tx15.r[2..], 16).unwrap(),
        "signature r difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
    assert_eq!(
        BigUint::from_str(&stx.signature().s().to_string()).unwrap(),
        BigUint::from_str_radix(&tx15.s[2..], 16).unwrap(),
        "signature s difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );

    let mut tx_buff: Vec<u8> = Vec::new();
    tx_buff.push(u8::from(envelope.tx_type()));
    let stx = envelope.as_eip1559().unwrap();
    stx.tx()
        .encode_with_signature_fields(stx.signature(), &mut tx_buff);
    assert_eq!(
        tx_buff
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>(),
        tx15.raw,
        "tx hash difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );

    assert_eq!(
        stx.hash().to_string(),
        tx15.hash,
        "tx hash difference at tx #{} of evm block #{}",
        trx_index,
        block_num
    );
}

pub fn compare_transaction(
    tx: &TelosEVMTransaction,
    tx15: &RawEvmTx,
    trx_index: usize,
    block_num: u32,
) {
    match &tx.envelope {
        TxEnvelope::Legacy(stx) => compare_legacy(stx, tx15, trx_index, block_num),
        TxEnvelope::Eip1559(_stx) => compare_1559(&tx.envelope, tx15, trx_index, block_num),
        _ => panic!("not implemented!"),
    }
}

pub fn compare_block(block: &TelosEVMBlock, block15: &TelosEVM15Block) {
    for (i, (tx, _receipt)) in block.transactions.iter().enumerate() {
        let tx15 = &block15
            .transactions
            .get(i)
            .expect("Blocks have different amounts of transactions")
            .raw;
        compare_transaction(tx, tx15, i, block.block_num);
    }

    assert_eq!(
        block.header.timestamp,
        iso_to_unix(&block15.block.timestamp) as u64,
        "timestamp difference at evm block #{}",
        block.block_num
    );

    assert_eq!(
        block.header.number, block15.block.global.block_num as u64,
        "block num difference at evm block #{}",
        block.block_num
    );

    assert_eq!(
        block.header.parent_hash,
        B256::from_hex(&block15.block.evm_prev_block_hash).unwrap(),
        "parent hash difference at evm block #{}",
        block.block_num
    );

    assert_eq!(
        block.header.extra_data,
        Bytes::from_hex(&block15.block.block_hash).unwrap(),
        "native block hash difference at evm block #{}",
        block.block_num
    );

    assert_eq!(
        block.header.gas_used.to_string(),
        block15.block.gas_used,
        "gas used difference at evm block #{}",
        block.block_num
    );

    assert_eq!(
        block.header.transactions_root,
        B256::from_hex(&block15.block.transactions_root).unwrap(),
        "transactions root difference at evm block #{}",
        block.block_num
    );

    assert_eq!(
        block.header.receipts_root,
        B256::from_hex(&block15.block.receipts_root_hash).unwrap(),
        "receipts root difference at evm block #{}",
        block.block_num
    );

    assert_eq!(
        block.header.hash_slow(),
        B256::from_hex(&block15.block.evm_block_hash).unwrap(),
        "block hash difference at evm block #{}",
        block.block_num
    );
}
