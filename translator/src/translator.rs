use crate::block::{ProcessingEVMBlock, TelosEVMBlock};
use crate::tasks::{evm_block_processor, final_processor, raw_deserializer, ship_reader};
use crate::types::execution_metadata::ExecutionBranchEntry;
use crate::types::translator_types::ChainId;
use antelope::api::client::APIClient;
use antelope::api::default_provider::DefaultProvider;
use antelope::chain::checksum::Checksum256;
use eyre::{eyre, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tracing::info;
use url::{Host, Url};

pub fn default_channel_size() -> usize {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslatorConfig {
    pub chain_id: ChainId,
    /// Exact Antelope chain id expected from both HTTP and SHIP.
    pub native_chain_id: String,
    /// Native parent block of the first translated EVM block.
    pub execution_anchor_native_block_number: u32,
    /// Exact native parent hash of the first translated EVM block.
    pub execution_anchor_native_block_hash: String,
    /// Timeout for native HTTP requests and the SHIP websocket handshake, in milliseconds.
    pub native_request_timeout_ms: Option<u64>,
    pub evm_deploy_block: Option<u32>,
    pub evm_start_block: u32,
    pub evm_stop_block: Option<u32>,
    pub prev_hash: String,
    pub validate_hash: Option<String>,

    /// EVM block at which the explicitly configured execution context is effective.
    pub execution_context_anchor_block: u32,
    /// Native gas price effective at the start of the anchor block.
    pub execution_context_starting_gas_price: String,
    /// Native EVM revision effective at the start of the anchor block.
    pub execution_context_starting_revision: u64,

    /// Native block whose execution context is inherited by `evm_start_block`.
    #[serde(default)]
    pub execution_context_parent_native_hash: Option<String>,
    /// Native number paired with `execution_context_parent_native_hash`.
    #[serde(default)]
    pub execution_context_parent_native_block: Option<u32>,
    /// VALID fork entries restored from the consensus client's durable branch index.
    #[serde(default)]
    pub execution_branch_entries: Vec<ExecutionBranchEntry>,

    pub http_endpoint: String,
    pub ship_endpoint: String,

    #[serde(default = "default_channel_size")]
    pub raw_message_channel_size: usize,
    #[serde(default = "default_channel_size")]
    pub block_message_channel_size: usize,
    #[serde(default = "default_channel_size")]
    pub final_message_channel_size: usize,
}

impl TranslatorConfig {
    pub(crate) fn expected_native_chain_id(&self) -> Result<Checksum256> {
        Checksum256::from_hex(&self.native_chain_id)
            .map_err(|error| eyre!("Invalid native chain id: {error}"))
    }

    pub(crate) fn effective_native_parent(&self) -> Result<(&str, u32)> {
        match (
            self.execution_context_parent_native_hash.as_deref(),
            self.execution_context_parent_native_block,
        ) {
            (Some(hash), Some(block)) => Ok((hash, block)),
            (None, None) => Ok((
                &self.execution_anchor_native_block_hash,
                self.execution_anchor_native_block_number,
            )),
            _ => Err(eyre!(
                "execution context native parent hash and block number must be configured together"
            )),
        }
    }
}

pub struct Translator {
    pub config: TranslatorConfig,
    shutdown_tx: mpsc::Sender<()>,
    shutdown_rx: mpsc::Receiver<()>,
}

pub struct Shutdown(mpsc::Sender<()>);

impl Shutdown {
    pub async fn shutdown(&self) -> Result<()> {
        Ok(self.0.send(()).await?)
    }

    pub fn is_finished(&self) -> bool {
        self.0.is_closed()
    }
}

impl Translator {
    pub fn new(config: TranslatorConfig) -> Self {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        Self {
            config,
            shutdown_tx,
            shutdown_rx,
        }
    }

    pub fn shutdown_handle(&self) -> Shutdown {
        Shutdown(self.shutdown_tx.clone())
    }

    pub async fn launch(self, output_tx: Option<mpsc::Sender<TelosEVMBlock>>) -> Result<()> {
        validate_config(&self.config)?;
        validate_native_endpoint(&self.config.http_endpoint, NativeTransport::Http)?;
        validate_native_endpoint(&self.config.ship_endpoint, NativeTransport::WebSocket)?;

        let expected_native_chain_id = self.config.expected_native_chain_id()?;
        let native_timeout =
            Duration::from_millis(self.config.native_request_timeout_ms.unwrap_or(10_000));
        let provider_timeout_seconds = native_timeout.as_secs().max(1);
        let api_client = APIClient::<DefaultProvider>::default_provider(
            self.config.http_endpoint.clone(),
            Some(provider_timeout_seconds),
        )
        .map_err(|error| eyre!(error))
        .wrap_err("Failed to create API client")?;

        let native_info = timeout(native_timeout, api_client.v1_chain.get_info())
            .await
            .map_err(|_| eyre!("Native HTTP get_info request timed out"))?
            .map_err(|error| eyre!("Native HTTP get_info request failed: {error:?}"))?;
        if native_info.chain_id != expected_native_chain_id {
            return Err(eyre!(
                "Native HTTP chain id {} does not match configured chain id {}",
                native_info.chain_id.as_string(),
                expected_native_chain_id.as_string()
            ));
        }

        let (ws_stream, _) = timeout(native_timeout, connect_async(&self.config.ship_endpoint))
            .await
            .map_err(|_| {
                eyre!(
                    "Timed out connecting to SHIP endpoint {}",
                    &self.config.ship_endpoint
                )
            })?
            .map_err(|_| {
                eyre!(
                    "Failed to connect to ship at endpoint {}",
                    &self.config.ship_endpoint
                )
            })?;

        let (ws_tx, ws_rx) = ws_stream.split();

        // Buffer size here should be the readahead buffer size, in blocks.  This could get large if we are reading
        //  a block range with larges blocks/trxs, so this should be tuned based on the largest blocks we hit
        let (raw_ds_tx, raw_ds_rx) = mpsc::channel::<Vec<u8>>(self.config.raw_message_channel_size);

        let (process_tx, process_rx) =
            mpsc::channel::<ProcessingEVMBlock>(self.config.block_message_channel_size);

        let (finalize_tx, finalize_rx) =
            mpsc::channel::<ProcessingEVMBlock>(self.config.final_message_channel_size);

        let final_processor = final_processor(
            self.config.clone(),
            api_client,
            finalize_rx,
            output_tx,
            self.shutdown_tx.clone(),
        );

        let evm_block_processor = evm_block_processor(process_rx, finalize_tx);

        let raw_deserializer = raw_deserializer(self.config.clone(), raw_ds_rx, ws_tx, process_tx);

        let ship_reader = ship_reader(ws_rx, raw_ds_tx, self.shutdown_rx);

        info!("Translator launched successfully");
        tokio::try_join!(
            ship_reader,
            raw_deserializer,
            evm_block_processor,
            final_processor
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum NativeTransport {
    Http,
    WebSocket,
}

fn validate_native_endpoint(endpoint: &str, transport: NativeTransport) -> Result<()> {
    let parsed = Url::parse(endpoint).wrap_err("Native endpoint is not a valid URL")?;
    let (secure_scheme, plaintext_scheme, label) = match transport {
        NativeTransport::Http => ("https", "http", "native HTTP"),
        NativeTransport::WebSocket => ("wss", "ws", "SHIP websocket"),
    };
    if !matches!(parsed.scheme(), scheme if scheme == secure_scheme || scheme == plaintext_scheme) {
        return Err(eyre!(
            "{label} endpoint must use {secure_scheme} or {plaintext_scheme}"
        ));
    }
    if parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(eyre!(
            "{label} endpoint must have a host and must not contain credentials, a query, or a fragment"
        ));
    }

    let loopback = match parsed.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    if parsed.scheme() == plaintext_scheme && !loopback {
        return Err(eyre!(
            "plaintext {plaintext_scheme} is allowed only for a loopback {label} endpoint"
        ));
    }
    Ok(())
}

fn validate_config(config: &TranslatorConfig) -> Result<()> {
    if !matches!(config.chain_id.0, 40 | 41) {
        return Err(eyre!(
            "Unsupported Telos EVM chain id {}",
            config.chain_id.0
        ));
    }
    if config.native_request_timeout_ms == Some(0) {
        return Err(eyre!("Native request timeout must be greater than zero"));
    }
    if config.raw_message_channel_size == 0
        || config.block_message_channel_size == 0
        || config.final_message_channel_size == 0
    {
        return Err(eyre!("Translator channel sizes must be greater than zero"));
    }
    if config.execution_context_anchor_block != config.evm_start_block {
        return Err(eyre!(
            "Execution context anchor block {} must match EVM start block {}",
            config.execution_context_anchor_block,
            config.evm_start_block
        ));
    }
    let (native_parent_hash, native_parent_block) = config.effective_native_parent()?;
    Checksum256::from_hex(native_parent_hash)
        .map_err(|error| eyre!("Invalid native anchor hash: {error}"))?;
    let expected_native_parent = config
        .evm_start_block
        .checked_add(config.chain_id.block_delta())
        .and_then(|first_native_block| first_native_block.checked_sub(1))
        .ok_or_else(|| {
            eyre!("EVM start block and native block delta do not have a valid parent")
        })?;
    if native_parent_block != expected_native_parent {
        return Err(eyre!(
            "Native anchor block {native_parent_block} must be the parent {expected_native_parent} of the first translated native block"
        ));
    }
    config.expected_native_chain_id()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_is_current() {
        toml::from_str::<TranslatorConfig>(include_str!("../example-config.toml")).unwrap();
    }

    #[test]
    fn plaintext_native_endpoints_are_loopback_only() {
        assert!(validate_native_endpoint("http://127.0.0.1:8888", NativeTransport::Http).is_ok());
        assert!(validate_native_endpoint("http://[::1]:8888", NativeTransport::Http).is_ok());
        assert!(
            validate_native_endpoint("ws://localhost:18999/ship", NativeTransport::WebSocket)
                .is_ok()
        );
        assert!(
            validate_native_endpoint("http://node.example:8888", NativeTransport::Http).is_err()
        );
        assert!(
            validate_native_endpoint("ws://node.example:18999", NativeTransport::WebSocket)
                .is_err()
        );
        assert!(validate_native_endpoint(
            "https://user:secret@node.example",
            NativeTransport::Http
        )
        .is_err());
    }
}
