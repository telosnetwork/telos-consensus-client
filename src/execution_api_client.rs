use crate::auth::{strip_prefix, Auth, Error, JwtKey};
use crate::json_rpc::{JsonRequestBody, JsonResponseBody};
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use reth_rpc_types::Block;
use serde_json::{json, Value};
use std::fmt::Display;
use tracing::info;

pub enum ExecutionApiMethod {
    BlockNumber,
    BlockByNumber,
    NewPayloadV1,
    ForkChoiceUpdatedV1,
}

impl Display for ExecutionApiMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionApiMethod::BlockByNumber => write!(f, "eth_getBlockByNumber"),
            ExecutionApiMethod::BlockNumber => write!(f, "eth_blockNumber"),
            ExecutionApiMethod::NewPayloadV1 => write!(f, "engine_newPayloadV1"),
            ExecutionApiMethod::ForkChoiceUpdatedV1 => write!(f, "engine_forkchoiceUpdatedV1"),
        }
    }
}

pub struct RpcRequest {
    pub method: ExecutionApiMethod,
    pub params: Value,
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

    pub async fn rpc(&self, rpc_request: RpcRequest) -> Result<JsonResponseBody, String> {
        let id: Value = json!(1);
        const JSONRPC: &str = "2.0";
        let method = rpc_request.method.to_string();
        let rpc_payload = JsonRequestBody {
            jsonrpc: JSONRPC,
            method,
            params: rpc_request.params,
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
        info!("rpc response: {:?}", serde_json::to_string(&json_response));
        Ok(json_response)
    }

    pub async fn rpc_batch(
        &self,
        rpc_requests: Vec<RpcRequest>,
    ) -> Result<Vec<JsonResponseBody>, String> {
        let counter = 0;
        const JSONRPC: &str = "2.0";
        let mut batch_requests = vec![];
        for rpc_request in rpc_requests {
            let method = rpc_request.method.to_string();
            let id = json!(counter);
            let rpc_payload = JsonRequestBody {
                jsonrpc: JSONRPC,
                method,
                params: rpc_request.params,
                id,
            };
            batch_requests.push(rpc_payload);
        }

        let request = self
            .client
            .post(&self.base_url)
            .bearer_auth(self.jwt_secret.generate_token().unwrap())
            .json(&batch_requests)
            .header(CONTENT_TYPE, "application/json");

        let response = request.send().await.unwrap();
        let json_response = response.json::<Vec<JsonResponseBody>>().await.unwrap();
        Ok(json_response)
    }

    pub async fn block_by_number(&self, block_number: u64, full: bool) -> Result<Block, String> {
        let response = self
            .rpc(RpcRequest {
                method: ExecutionApiMethod::BlockByNumber,
                params: json!([format!("0x{:x}", block_number), full]),
            })
            .await
            .unwrap();
        let block = serde_json::from_value::<Block>(response.result);
        Ok(block.unwrap())
    }

    pub async fn block_number(&self) -> Result<u64, String> {
        let response = self
            .rpc(RpcRequest {
                method: ExecutionApiMethod::BlockNumber,
                params: json!([]),
            })
            .await
            .unwrap();
        let stripped = strip_prefix(response.result.as_str().unwrap());
        Ok(u64::from_str_radix(stripped, 16).unwrap())
    }
}
