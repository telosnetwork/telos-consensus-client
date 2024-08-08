use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/*
 * leap-mock client
 */
#[derive(Serialize, Deserialize, Debug)]
pub struct SetNumParams {
    pub num: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SetJumpsParams {
    pub jumps: Vec<(u32, u32)>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SetBlockInfo {
    pub blocks: Vec<String>,
    pub index: u32,
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

    pub async fn set_block(&self, params: SetNumParams) -> Result<String, LeapMockError> {
        let url = format!("{}/set_block", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&params)
            .send()
            .await?
            .json::<MockResponse<String>>()
            .await?;

        match resp.result {
            Some(result) => Ok(result),
            None => Err(LeapMockError::ApiError(
                resp.error.unwrap_or_else(|| "Unknown error".to_string()),
            )),
        }
    }

    pub async fn set_jumps(&self, params: SetJumpsParams) -> Result<String, LeapMockError> {
        let url = format!("{}/set_jumps", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&params)
            .send()
            .await?
            .json::<MockResponse<String>>()
            .await?;

        match resp.result {
            Some(result) => Ok(result),
            None => Err(LeapMockError::ApiError(
                resp.error.unwrap_or_else(|| "Unknown error".to_string()),
            )),
        }
    }

    pub async fn set_block_info(&self, params: SetBlockInfo) -> Result<String, LeapMockError> {
        let url = format!("{}/set_block_info", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&params)
            .send()
            .await?
            .json::<MockResponse<String>>()
            .await?;

        match resp.result {
            Some(result) => Ok(result),
            None => Err(LeapMockError::ApiError(
                resp.error.unwrap_or_else(|| "Unknown error".to_string()),
            )),
        }
    }
}
