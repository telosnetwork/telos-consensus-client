use alloy_primitives::B256;
use alloy_rpc_types::Block;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::redirect::Policy;
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::fmt;
use std::fmt::Display;
use std::fs::OpenOptions;
use std::io::Read;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;
use tracing::debug;
use zeroize::Zeroizing;

use crate::auth::{self, Auth, Error, JwtKey, JWT_SECRET_LENGTH};
use crate::json_rpc::{JsonError, JsonRequestBody, JsonResponseBody};

#[derive(Debug, Error)]
pub enum ExecutionApiError {
    #[error("Authentication Error: {0}")]
    AuthError(#[from] auth::Error),

    #[error("API Error: {0}")]
    ApiError(#[from] reqwest::Error),

    #[error("Cannot Deserialize Response")]
    CannotDeserialize,

    #[error("Cannot read JWT secret file: {0}")]
    CannotReadJwt(#[source] std::io::Error),

    #[error("JWT secret file permissions are too broad; expected no group or world access")]
    InsecureJwtPermissions,

    #[error("Invalid JWT secret file: {0}")]
    InvalidJwtFile(String),

    #[error("JWT secret file exceeds the {maximum}-byte limit")]
    JwtFileTooLarge { maximum: usize },

    #[error("Execution API returned HTTP status {0}")]
    HttpStatus(reqwest::StatusCode),

    #[error("Execution API response is too large: {actual} bytes exceeds {maximum}")]
    ResponseTooLarge { actual: u64, maximum: usize },

    #[error("Invalid execution API response: {0}")]
    InvalidResponse(String),

    #[error("Invalid execution API endpoint: {0}")]
    InvalidEndpoint(String),

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
            .map(|(id, error)| format!("({id}, {})", error.message))
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
    Latest,
    Finalized,
}

impl Display for BlockStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_str = match self {
            BlockStatus::Latest => "latest",
            BlockStatus::Finalized => "finalized",
        };
        write!(f, "{}", status_str)
    }
}

#[derive(Debug)]
pub enum ExecutionApiMethod {
    BlockByNumber,
    ChainId,
    NewPayloadV1,
    ForkChoiceUpdatedV1,
    ExchangeCapabilities,
}

impl Display for ExecutionApiMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionApiMethod::BlockByNumber => write!(f, "eth_getBlockByNumber"),
            ExecutionApiMethod::ChainId => write!(f, "eth_chainId"),
            ExecutionApiMethod::NewPayloadV1 => write!(f, "engine_newPayloadV1"),
            ExecutionApiMethod::ForkChoiceUpdatedV1 => write!(f, "engine_forkchoiceUpdatedV1"),
            ExecutionApiMethod::ExchangeCapabilities => {
                write!(f, "engine_exchangeCapabilities")
            }
        }
    }
}

#[derive(Debug)]
pub struct RpcRequest {
    pub method: ExecutionApiMethod,
    pub params: Value,
}

#[derive(Clone)]
pub struct ExecutionApiClient {
    client: Client,
    base_url: String,
    jwt_secret: Auth,
    max_response_bytes: usize,
    transport_retries: usize,
}

const MAX_JWT_FILE_BYTES: usize = 256;

fn read_jwt_secret(path: &Path) -> Result<Zeroizing<Vec<u8>>, ExecutionApiError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(path)
        .map_err(ExecutionApiError::CannotReadJwt)?;
    let metadata = file.metadata().map_err(ExecutionApiError::CannotReadJwt)?;
    if !metadata.file_type().is_file() {
        return Err(ExecutionApiError::InvalidJwtFile(
            "secret path is not a regular file".to_string(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ExecutionApiError::InsecureJwtPermissions);
        }
    }
    if metadata.len() > MAX_JWT_FILE_BYTES as u64 {
        return Err(ExecutionApiError::JwtFileTooLarge {
            maximum: MAX_JWT_FILE_BYTES,
        });
    }

    let mut contents = Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
    (&mut file)
        .take((MAX_JWT_FILE_BYTES + 1) as u64)
        .read_to_end(&mut contents)
        .map_err(ExecutionApiError::CannotReadJwt)?;
    if contents.len() > MAX_JWT_FILE_BYTES {
        return Err(ExecutionApiError::JwtFileTooLarge {
            maximum: MAX_JWT_FILE_BYTES,
        });
    }
    Ok(contents)
}

impl ExecutionApiClient {
    pub fn new(
        base_url: &str,
        jwt_secret_path: &Path,
        connect_timeout: Duration,
        request_timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self, ExecutionApiError> {
        let parsed_url = reqwest::Url::parse(base_url)
            .map_err(|error| ExecutionApiError::InvalidEndpoint(error.to_string()))?;
        if !matches!(parsed_url.scheme(), "http" | "https")
            || !parsed_url.username().is_empty()
            || parsed_url.password().is_some()
            || parsed_url.query().is_some()
            || parsed_url.fragment().is_some()
        {
            return Err(ExecutionApiError::InvalidEndpoint(
                "endpoint must be HTTP(S) and must not contain credentials, a query, or a fragment"
                    .to_string(),
            ));
        }
        let host = parsed_url.host_str().unwrap_or_default();
        let is_loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if parsed_url.scheme() == "http" && !is_loopback {
            return Err(ExecutionApiError::InvalidEndpoint(
                "plaintext HTTP is allowed only for a loopback execution endpoint".to_string(),
            ));
        }
        let jwt_contents = read_jwt_secret(jwt_secret_path)?;
        let jwt_secret = std::str::from_utf8(&jwt_contents).map_err(|error| {
            ExecutionApiError::InvalidJwtFile(format!("secret is not UTF-8: {error}"))
        })?;
        let jwt_secret = jwt_secret.trim();
        let jwt_key_encoded = jwt_secret.strip_prefix("0x").unwrap_or(jwt_secret);
        if jwt_key_encoded.len() != JWT_SECRET_LENGTH * 2 {
            return Err(Error::InvalidJwt(format!(
                "Invalid key length. Expected {} hexadecimal characters got {}",
                JWT_SECRET_LENGTH * 2,
                jwt_key_encoded.len()
            ))
            .into());
        }
        let mut jwt_key_decoded = Zeroizing::new([0u8; JWT_SECRET_LENGTH]);
        hex::decode_to_slice(jwt_key_encoded, &mut *jwt_key_decoded).map_err(Error::InvalidKey)?;
        let jwt_key = JwtKey::from_slice(&*jwt_key_decoded).map_err(Error::InvalidJwt)?;
        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .redirect(Policy::none())
            .build()?;

        Ok(Self {
            client,
            base_url: parsed_url.to_string(),
            jwt_secret: Auth::new(jwt_key, None, None),
            max_response_bytes,
            transport_retries: 2,
        })
    }

    async fn post_json<T: Serialize + ?Sized>(
        &self,
        payload: &T,
    ) -> Result<Vec<u8>, ExecutionApiError> {
        let mut attempt = 0;
        'request: loop {
            let token = self.jwt_secret.generate_token()?;
            let result = self
                .client
                .post(&self.base_url)
                .bearer_auth(token)
                .json(payload)
                .header(CONTENT_TYPE, "application/json")
                .send()
                .await;

            let mut response = match result {
                Ok(response) => response,
                Err(error)
                    if attempt < self.transport_retries
                        && (error.is_connect() || error.is_timeout() || error.is_body()) =>
                {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(100 * attempt as u64)).await;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };

            if !response.status().is_success() {
                return Err(ExecutionApiError::HttpStatus(response.status()));
            }
            if let Some(content_length) = response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
            {
                if content_length > self.max_response_bytes as u64 {
                    return Err(ExecutionApiError::ResponseTooLarge {
                        actual: content_length,
                        maximum: self.max_response_bytes,
                    });
                }
            }

            let mut bytes = Vec::new();
            loop {
                let chunk = match response.chunk().await {
                    Ok(chunk) => chunk,
                    Err(error)
                        if attempt < self.transport_retries
                            && (error.is_connect() || error.is_timeout() || error.is_body()) =>
                    {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(100 * attempt as u64)).await;
                        continue 'request;
                    }
                    Err(error) => return Err(error.into()),
                };
                let Some(chunk) = chunk else {
                    break;
                };
                let new_length = bytes.len().saturating_add(chunk.len());
                if new_length > self.max_response_bytes {
                    return Err(ExecutionApiError::ResponseTooLarge {
                        actual: new_length as u64,
                        maximum: self.max_response_bytes,
                    });
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(bytes);
        }
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
            id: id.clone(),
        };

        let response = self.post_json(&rpc_payload).await?;
        let json_response = validate_rpc_response(&response, &id)?;
        debug!(method = %rpc_payload.method, "execution RPC succeeded");
        Ok(json_response)
    }

    pub async fn exchange_capabilities(&self) -> Result<Vec<String>, ExecutionApiError> {
        const REQUIRED_CAPABILITIES: [&str; 2] =
            ["engine_newPayloadV1", "engine_forkchoiceUpdatedV1"];
        let response = self
            .rpc(RpcRequest {
                method: ExecutionApiMethod::ExchangeCapabilities,
                params: json!([REQUIRED_CAPABILITIES]),
            })
            .await?;
        let capabilities: Vec<String> = serde_json::from_value(response.result)
            .map_err(|error| ExecutionApiError::InvalidResponse(error.to_string()))?;
        if capabilities.len() != REQUIRED_CAPABILITIES.len()
            || REQUIRED_CAPABILITIES
                .iter()
                .any(|required| !capabilities.iter().any(|capability| capability == required))
        {
            return Err(ExecutionApiError::InvalidResponse(format!(
                "execution endpoint must advertise exactly {REQUIRED_CAPABILITIES:?}, got {capabilities:?}"
            )));
        }
        Ok(capabilities)
    }

    pub async fn verify_chain(
        &self,
        expected_chain_id: u64,
        expected_anchor_number: u64,
        expected_anchor_hash: B256,
    ) -> Result<(), ExecutionApiError> {
        let response = self
            .rpc(RpcRequest {
                method: ExecutionApiMethod::ChainId,
                params: json!([]),
            })
            .await?;
        let encoded_chain_id: String = serde_json::from_value(response.result)
            .map_err(|error| ExecutionApiError::InvalidResponse(error.to_string()))?;
        let actual_chain_id = u64::from_str_radix(
            encoded_chain_id
                .strip_prefix("0x")
                .unwrap_or(&encoded_chain_id),
            16,
        )
        .map_err(|error| ExecutionApiError::InvalidResponse(error.to_string()))?;
        if actual_chain_id != expected_chain_id {
            return Err(ExecutionApiError::InvalidResponse(format!(
                "execution chain id {actual_chain_id} does not match configured chain id {expected_chain_id}"
            )));
        }

        let anchor = self
            .get_block_by_number(expected_anchor_number)
            .await?
            .ok_or_else(|| {
                ExecutionApiError::InvalidResponse(format!(
                    "execution anchor block {expected_anchor_number} is missing"
                ))
            })?;
        if anchor.header.number != expected_anchor_number
            || anchor.header.hash != expected_anchor_hash
        {
            return Err(ExecutionApiError::InvalidResponse(format!(
                "execution anchor {} ({}) does not match configured anchor {expected_anchor_number} ({expected_anchor_hash})",
                anchor.header.number,
                anchor.header.hash
            )));
        }
        Ok(())
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
    pub async fn get_latest_finalized_block(
        &self,
        fallback_anchor_number: u64,
    ) -> Result<Option<Block>, ExecutionApiError> {
        let request = RpcRequest {
            method: ExecutionApiMethod::BlockByNumber,
            params: json!([BlockStatus::Finalized.to_string(), true]),
        };
        let result = self.rpc(request).await?.result;

        if result.is_null() {
            // This can happen before the first forkchoice update on a fresh sparse database.
            return self.get_block_by_number(fallback_anchor_number).await;
        }

        serde_json::from_value(result)
            .map_err(|_| ExecutionApiError::CannotDeserialize)
            .map(Some)
    }

    /// Get latest block
    pub async fn get_latest_block(
        &self,
        fallback_anchor_number: u64,
    ) -> Result<Option<Block>, ExecutionApiError> {
        let request = RpcRequest {
            method: ExecutionApiMethod::BlockByNumber,
            params: json!([BlockStatus::Latest.to_string(), true]),
        };
        let result = self.rpc(request).await?.result;

        if result.is_null() {
            // This can happen before the first payload on a fresh sparse database.
            return self.get_block_by_number(fallback_anchor_number).await;
        }

        serde_json::from_value(result)
            .map_err(|_| ExecutionApiError::CannotDeserialize)
            .map(Some)
    }
}

fn validate_rpc_response(
    response: &[u8],
    expected_id: &Value,
) -> Result<JsonResponseBody, ExecutionApiError> {
    const JSONRPC: &str = "2.0";
    let json_response: JsonResponseBody = serde_json::from_slice(response)
        .map_err(|error| ExecutionApiError::InvalidResponse(error.to_string()))?;
    if json_response.jsonrpc != JSONRPC {
        return Err(ExecutionApiError::InvalidResponse(format!(
            "unexpected jsonrpc version {}",
            json_response.jsonrpc
        )));
    }
    if json_response.id != *expected_id {
        return Err(ExecutionApiError::InvalidResponse(format!(
            "response id {} does not match request id {expected_id}",
            json_response.id
        )));
    }
    if let Some(error) = &json_response.error {
        return Err(ExecutionApiError::ExecutionApi(
            vec![(json_response.id.clone(), error.clone())].into(),
        ));
    }
    Ok(json_response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temporary_path(test_name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "telos-jwt-{test_name}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    fn write_secure_file(path: &Path, contents: &[u8]) {
        fs::write(path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[test]
    fn validates_json_rpc_version_and_id() {
        let response = br#"{"jsonrpc":"2.0","result":{"status":"VALID"},"id":1}"#;
        let parsed = validate_rpc_response(response, &json!(1)).unwrap();
        assert_eq!(parsed.result["status"], "VALID");

        let wrong_id = validate_rpc_response(response, &json!(2)).unwrap_err();
        assert!(matches!(wrong_id, ExecutionApiError::InvalidResponse(_)));
    }

    #[test]
    fn rejects_json_rpc_errors() {
        let response =
            br#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"bad params"},"id":1}"#;
        let error = validate_rpc_response(response, &json!(1)).unwrap_err();
        assert!(matches!(error, ExecutionApiError::ExecutionApi(_)));
    }

    #[test]
    fn jwt_reader_rejects_oversized_files() {
        let path = temporary_path("oversized");
        write_secure_file(&path, &vec![b'a'; MAX_JWT_FILE_BYTES + 1]);
        assert!(matches!(
            read_jwt_secret(&path),
            Err(ExecutionApiError::JwtFileTooLarge { .. })
        ));
        fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn jwt_reader_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let target = temporary_path("target");
        let link = temporary_path("link");
        write_secure_file(
            &target,
            b"0000000000000000000000000000000000000000000000000000000000000000",
        );
        symlink(&target, &link).unwrap();
        assert!(read_jwt_secret(&link).is_err());
        fs::remove_file(link).unwrap();
        fs::remove_file(target).unwrap();
    }

    #[test]
    fn jwt_reader_accepts_a_bounded_regular_secret() {
        let path = temporary_path("valid");
        let contents = b"0000000000000000000000000000000000000000000000000000000000000000\n";
        write_secure_file(&path, contents);
        assert_eq!(&*read_jwt_secret(&path).unwrap(), contents);
        fs::remove_file(path).unwrap();
    }
}
