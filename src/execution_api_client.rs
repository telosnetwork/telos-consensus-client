use crate::auth::{strip_prefix, Auth, Error, JwtKey};
use crate::json_rpc::{JsonRequestBody, JsonResponseBody};
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use reth_rpc_types::Block;
use serde::Serialize;
use serde_json::{json, Value};
use std::fmt::Display;

pub enum ExecutionApiMethod {
    BlockNumber,
    BlockByNumber,
    NewPayloadV1,
}

impl Display for ExecutionApiMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionApiMethod::BlockByNumber => write!(f, "eth_getBlockByNumber"),
            ExecutionApiMethod::BlockNumber => write!(f, "eth_blockNumber"),
            ExecutionApiMethod::NewPayloadV1 => write!(f, "engine_newPayloadV1"),
        }
    }
}

pub struct ExecutionApiClient {
    pub client: Client,
    pub base_url: String,
    pub jwt_secret: Auth,
}

impl ExecutionApiClient {
    pub fn new(base_url: String, jwt_secret: String) -> Self {
        let secret_bytes = hex::decode(strip_prefix(jwt_secret.trim_end()))
            .map_err(|e| Error::InvalidKey(format!("Invalid hex string: {:?}", e)))
            .unwrap();
        let jwt_key = JwtKey::from_slice(&secret_bytes)
            .map_err(Error::InvalidKey)
            .unwrap();
        Self {
            client: Client::new(),
            base_url,
            jwt_secret: Auth::new(jwt_key, None, None),
        }
    }

    pub async fn rpc(
        &self,
        method: ExecutionApiMethod,
        params: Value,
    ) -> Result<JsonResponseBody, String> {
        let id: Value = json!(1);
        const JSONRPC: &str = "2.0";
        let method = method.to_string();
        let rpc_payload = JsonRequestBody {
            jsonrpc: JSONRPC,
            method: method.as_str(),
            params,
            id,
        };

        let request = self
            .client
            .post(&self.base_url)
            .bearer_auth(self.jwt_secret.generate_token().unwrap())
            .json(&rpc_payload)
            .header(CONTENT_TYPE, "application/json");

        let response = request.send().await.unwrap();
        let json_response = response.json::<JsonResponseBody>().await.unwrap();
        Ok(json_response)
    }

    pub async fn block_by_number(&self, block_number: u64, full: bool) -> Result<Block, String> {
        let response = self
            .rpc(
                ExecutionApiMethod::BlockByNumber,
                json!([format!("0x{:x}", block_number), full]),
            )
            .await
            .unwrap();
        let block = serde_json::from_value::<Block>(response.result);
        Ok(block.unwrap())
    }

    pub async fn block_number(&self) -> Result<u64, String> {
        let response = self
            .rpc(ExecutionApiMethod::BlockNumber, json!([]))
            .await
            .unwrap();
        let stripped = strip_prefix(response.result.as_str().unwrap());
        Ok(u64::from_str_radix(stripped, 16).unwrap())
    }
}
