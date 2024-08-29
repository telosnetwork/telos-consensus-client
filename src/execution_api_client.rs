use std::fmt;
use std::fmt::Display;

use alloy::primitives::private::derive_more::Display;
use reqwest::Client;
use reqwest::header::CONTENT_TYPE;
use reth_rpc_types::Block;
use serde_json::{json, Value};

use crate::auth::{Auth, Error, JwtKey, strip_prefix};
use crate::execution_api_client::ExecutionApiError::{ApiError, AuthError, CannotDeserialize};
use crate::json_rpc::{JsonRequestBody, JsonResponseBody};

#[derive(Debug, Display)]
pub enum ExecutionApiError {
    AuthError(Error),
    ApiError(reqwest::Error),
    CannotDeserialize,
}

pub enum BlockStatus {
    Latest
}

impl fmt::Display for BlockStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_str = match self {
            BlockStatus::Latest => "latest",
        };
        write!(f, "{}", status_str)
    }
}


impl From<Error> for ExecutionApiError {
    fn from(e: Error) -> Self {
        AuthError(e)
    }
}

impl From<reqwest::Error> for ExecutionApiError {
    fn from(e: reqwest::Error) -> Self {
        ApiError(e)
    }
}

pub enum ExecutionApiMethod {
    BlockByNumber,
    NewPayloadV1,
    ForkChoiceUpdatedV1,
}

impl Display for ExecutionApiMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionApiMethod::BlockByNumber => write!(f, "eth_getBlockByNumber"),
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

    pub async fn rpc(&self, rpc_request: RpcRequest) -> Result<JsonResponseBody, ExecutionApiError> {
        let id: Value = json!(1);
        const JSONRPC: &str = "2.0";
        let method = rpc_request.method.to_string();
        let rpc_payload = JsonRequestBody {
            jsonrpc: JSONRPC,
            method,
            params: rpc_request.params,
            id,
        };

        let token = self.jwt_secret.generate_token()?;
        let request = self
            .client
            .post(&self.base_url)
            .bearer_auth(token)
            .json(&rpc_payload)
            .header(CONTENT_TYPE, "application/json");

        let response = request.send().await?;
        let json_response = response.json::<JsonResponseBody>().await?;
        Ok(json_response)
    }

    pub async fn rpc_batch(
        &self,
        rpc_requests: Vec<RpcRequest>,
    ) -> Result<Vec<JsonResponseBody>, ExecutionApiError> {
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

        let token = self.jwt_secret.generate_token()?;
        let request = self
            .client
            .post(&self.base_url)
            .bearer_auth(token)
            .json(&batch_requests)
            .header(CONTENT_TYPE, "application/json");

        let response = request.send().await?;
        let json_response = response.json::<Vec<JsonResponseBody>>().await?;
        Ok(json_response)
    }

    // block_by_number queries api using block number if provided or it returns the latest block.
    pub async fn block_by_number(&self, block_number: Option<u64>, full: bool) -> Result<Block, ExecutionApiError> {
        let block_request_param: String;
        if let Some(number) = block_number {
            block_request_param = format!("0x{:x}", number);
        } else {
            block_request_param = BlockStatus::Latest.to_string();
        }

        let response = self
            .rpc(RpcRequest {
                method: ExecutionApiMethod::BlockByNumber,
                params: json!([block_request_param, full]),
            })
            .await?;

        let block = serde_json::from_value::<Block>(response.result).map_err(|_| CannotDeserialize)?;
        Ok(block)
    }
}
