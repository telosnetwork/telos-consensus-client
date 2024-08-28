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
