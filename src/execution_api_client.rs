use std::fmt;
use std::fmt::Display;

use log::debug;
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use reth_rpc_types::Block;
use serde_json::{json, Value};
use thiserror::Error;
use tracing::info;

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
pub struct JsonErrors(Vec<JsonError>);

impl fmt::Display for JsonErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self(errors) = self;
        let errors = errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<&str>>()
            .join(", ");
        write!(f, "{errors}")
    }
}

impl From<Vec<JsonError>> for JsonErrors {
    fn from(value: Vec<JsonError>) -> Self {
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
            return Err(ExecutionApiError::ExecutionApi(errors.into()));
        } else {
            debug!("Successfully executor batch. {:?}", json_response);
        }

        Ok(json_response)
    }

    /// Gets latest finalized block
    pub async fn get_latest_finalized_block(&self) -> Result<Option<Block>, ExecutionApiError> {
        let request = RpcRequest {
            method: ExecutionApiMethod::BlockByNumber,
            params: json!([BlockStatus::Finalized.to_string(), true]),
        };
        let result = self.rpc(request).await?.result;

        if result.is_null() {
            return Ok(None);
        }

        serde_json::from_value(result)
            .map_err(|_| ExecutionApiError::CannotDeserialize)
            .map(Some)
    }
}
