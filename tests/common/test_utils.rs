use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

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

    pub gas_used: Option<String>,
    pub gas_limit: Option<String>,
    pub size: Option<String>,
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
pub struct RawEvmTx {
    pub hash: String,
    pub trx_index: u32,
    pub block: u32,
    pub block_hash: String,
    pub to: String,
    pub input_data: String,
    pub input_trimmed: String,
    pub value: String,
    pub value_d: String,
    pub nonce: String,
    pub gas_price: String,
    pub gas_limit: String,
    pub status: u8,
    pub itxs: Vec<String>,
    pub epoch: u64,
    pub createdaddr: String,
    pub gasused: String,
    pub gasusedblock: String,
    pub charged_gas_price: String,
    pub output: String,
    pub v: String,
    pub r: String,
    pub s: String,
    pub from: String,
}

use serde_json::Result as SerdeResult;

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
