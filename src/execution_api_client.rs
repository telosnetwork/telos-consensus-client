use std::fmt;
use std::fmt::Display;

use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use reth_rpc_types::Block;
use serde_json::{json, Value};
use thiserror::Error;
use tracing::{debug};

use crate::auth::{self, Auth, Error, JwtKey};
use crate::json_rpc::{JsonError, JsonRequestBody, JsonResponseBody};

#[derive(Debug, Error)]
pub enum ExecutionApiError {
    #[error("Authentication Error: {0}")]
    AuthError(#[from] auth::Error),

    #[error("API Error: {0}")]
    ApiError(#[from] reqwest::Error),

    #[error("Cannot Deserialize Response")]
    CannotDeserialize,

    #[error("Execution API Errors: [{0}]")]
    ExecutionApi(JsonErrors),
}

#[derive(Debug)]
pub struct JsonErrors(Vec<(Value, JsonError)>);

impl fmt::Display for JsonErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self(errors) = self;
        let errors = errors
            .iter()
            .map(|(id, error)| format!("({id}, {}", error.message))
            .collect::<Vec<String>>()
            .join(", ");
        write!(f, "{errors}")
    }
}

impl From<Vec<(Value, JsonError)>> for JsonErrors {
    fn from(value: Vec<(Value, JsonError)>) -> Self {
        JsonErrors(value)
    }
}

pub enum BlockStatus {
    Finalized,
}

impl Display for BlockStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_str = match self {
            BlockStatus::Finalized => "finalized",
        };
        write!(f, "{}", status_str)
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
        debug!("rpc response: {:?}", serde_json::to_string(&json_response));
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
            .map(|(id, RpcRequest { method, params })| JsonRequestBody {
                jsonrpc: JSONRPC,
                method: method.to_string(),
                params,
                id: json!(id),
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
        let errors: Vec<(Value, JsonError)> = json_response
            .iter()
            .filter_map(|JsonResponseBody { error, id, .. }| {
                error.as_ref().map(|error| (id.clone(), error.clone()))
            })
            .collect();

        if !errors.is_empty() {
            debug!("Errors calling executor batch. Errors: {:?}", errors);
            return Err(ExecutionApiError::ExecutionApi(errors.into()));
        } else {
            debug!("Successfully executor batch. {:?}", json_response);
        }

        Ok(json_response)
    }

    /// Gets a block by number
    pub async fn get_block_by_number(
        &self,
        block_number: u64,
    ) -> Result<Option<Block>, ExecutionApiError> {
        let request = RpcRequest {
            method: ExecutionApiMethod::BlockByNumber,
            params: json!([format!("0x{:x}", block_number), true]),
        };
        let result = self.rpc(request).await?.result;

        if result.is_null() {
            return Ok(None);
        }

        serde_json::from_value(result)
            .map_err(|_| ExecutionApiError::CannotDeserialize)
            .map(Some)
    }

    /// Gets latest finalized block
    pub async fn get_latest_finalized_block(&self) -> Result<Option<Block>, ExecutionApiError> {
        let request = RpcRequest {
            method: ExecutionApiMethod::BlockByNumber,
            params: json!([BlockStatus::Finalized.to_string(), true]),
        };
        let result = self.rpc(request).await?.result;

        if result.is_null() {
            // This should only happen on a fresh chain where only genesis block exists
            return self.get_block_by_number(0).await;
        }

        serde_json::from_value(result)
            .map_err(|_| ExecutionApiError::CannotDeserialize)
            .map(Some)
    }
}
