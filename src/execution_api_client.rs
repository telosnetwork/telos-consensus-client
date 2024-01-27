use crate::auth::{strip_prefix, Auth, Error, JwtKey};
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use serde::Serialize;
use std::fmt::Display;
use serde_json::{json, Value};
use crate::json_rpc::JsonRequestBody;

pub enum ExecutionApiMethod {
    NewPayloadV1,
}

impl Display for ExecutionApiMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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

    pub async fn rpc<T: Serialize>(
        &self,
        method: ExecutionApiMethod,
        payload: T,
    ) -> Result<String, String> {
        let id: Value = json!(1);
        const jsonrpc: &str = "2.0";
        let method = method.to_string();
        let rpc_payload = JsonRequestBody {
            jsonrpc,
            method: method.as_str(),
            params: json!(payload),
            id,
        };

        let request = self
            .client
            .post(&self.base_url)
            .bearer_auth(self.jwt_secret.generate_token().unwrap())
            .json(&rpc_payload)
            .header(CONTENT_TYPE, "application/json");

        let response = request.send().await.unwrap();
        Ok(response.text().await.unwrap())
    }
}
