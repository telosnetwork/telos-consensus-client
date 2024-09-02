use std::fmt;
use std::fmt::Display;

use log::debug;
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use reth_rpc_types::Block;
use serde_json::{json, Value};
use tracing::info;

use crate::auth::{Auth, Error, JwtKey};
use crate::execution_api_client::ExecutionApiError::{
    ApiError, AuthError, CannotDeserialize, ExecutionApi,
};
use crate::json_rpc::{JsonError, JsonRequestBody, JsonResponseBody};

#[derive(Debug)]
pub enum ExecutionApiError {
    AuthError(Error),
    ApiError(reqwest::Error),
    CannotDeserialize,
    ExecutionApi(Vec<JsonError>),
}

impl Display for ExecutionApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError(err) => write!(f, "Authentication Error: {}", err),
            ApiError(err) => write!(f, "API Error: {}", err),
            CannotDeserialize => write!(f, "Cannot Deserialize Response"),
            ExecutionApi(errors) => {
                let errors_str = errors
                    .iter()
                    .map(|err| err.message.to_string())
                    .collect::<Vec<String>>()
                    .join(", ");
                write!(f, "Execution API Errors: [{}]", errors_str)
            }
        }
    }
}

pub enum BlockStatus {
    Latest,
}

impl Display for BlockStatus {
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

#[derive(Debug)]
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

#[derive(Debug)]
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
    pub fn new(base_url: &str, jwt_secret: &str) -> Result<Self, Error> {
        let jwt_key_encoded = jwt_secret
            .strip_prefix("0x")
            .unwrap_or(jwt_secret)
            .trim_end();
        let jwt_key_decoded = hex::decode(jwt_key_encoded).map_err(Error::InvalidKey)?;
        let jwt_key = JwtKey::from_slice(&jwt_key_decoded).map_err(Error::InvalidJwt)?;

        Ok(Self {
            client: Client::new(),
            base_url: base_url.to_string(),
            jwt_secret: Auth::new(jwt_key, None, None),
        })
    }

    pub async fn rpc(
        &self,
        rpc_request: RpcRequest,
    ) -> Result<JsonResponseBody, ExecutionApiError> {
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
        info!("rpc response: {:?}", serde_json::to_string(&json_response));
        Ok(json_response)
    }

    pub async fn rpc_batch(
        &self,
        rpc_requests: Vec<RpcRequest>,
    ) -> Result<Vec<JsonResponseBody>, ExecutionApiError> {
        const JSONRPC: &str = "2.0";

        let batch_requests = rpc_requests
            .into_iter()
            .enumerate()
            .map(|(counter, RpcRequest { method, params })| JsonRequestBody {
                jsonrpc: JSONRPC,
                method: method.to_string(),
                params,
                id: json!(counter),
            })
            .collect::<Vec<_>>();

        let token = self.jwt_secret.generate_token()?;
        let request = self
            .client
            .post(&self.base_url)
            .bearer_auth(token)
            .json(&batch_requests)
            .header(CONTENT_TYPE, "application/json");

        let response = request.send().await?;
        let json_response = response.json::<Vec<JsonResponseBody>>().await?;
        let errors: Vec<JsonError> = json_response
            .iter()
            .filter_map(|response| response.error.clone())
            .collect();

        if !errors.is_empty() {
            debug!("Errors calling executor batch. Errors: {:?}", errors);
            return Err(ExecutionApi(errors));
        } else {
            debug!("Successfully executor batch. {:?}", json_response);
        }

        Ok(json_response)
    }

    // block_by_number queries api using block number if provided or it returns the latest block.
    pub async fn block_by_number(
        &self,
        block_number: Option<u64>,
        full: bool,
    ) -> Result<Option<Block>, ExecutionApiError> {
        let block_request_param = block_number.map_or_else(
            || BlockStatus::Latest.to_string(),
            |number| format!("0x{:x}", number),
        );

        let response = self
            .rpc(RpcRequest {
                method: ExecutionApiMethod::BlockByNumber,
                params: json!([block_request_param, full]),
            })
            .await?;

        if response.result.is_null() {
            Ok(None)
        } else {
            let block =
                serde_json::from_value::<Block>(response.result).map_err(|_| CannotDeserialize)?;
            Ok(Some(block))
        }
    }
}
