use clap::Parser;
use serde::Deserialize;
use telos_translator_rs::{
    translator::TranslatorConfig,
    types::{execution_metadata::ExecutionBranchEntry, translator_types::ChainId},
};

/// Telos Consensus Client CLI Arguments
#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
pub struct CliArgs {
    /// Path to the configuration file
    #[clap(short, long, value_parser)]
    pub config: String,
    /// Start translator from clean state
    #[arg(long, default_value = "false")]
    pub clean: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Log level for the application
    pub log_level: String,

    /// EVM Chain id, Telos mainnet is 40 and testnet is 41
    pub chain_id: ChainId,

    /// Execution API http endpoint (JWT protected endpoint on reth)
    pub execution_endpoint: String,

    /// Path to the 32-byte hex JWT secret used by the Engine API.
    pub jwt_secret_path: String,

    /// Exact first block stored in the sparse execution database.
    pub execution_anchor_block_number: u64,

    /// Expected hash of the execution anchor block.
    pub execution_anchor_block_hash: String,

    /// Exact Antelope chain id expected from both HTTP and SHIP.
    pub native_chain_id: String,

    /// Native parent block of the first translated EVM block.
    pub execution_anchor_native_block_number: u32,

    /// Exact native parent hash of the first translated EVM block.
    pub execution_anchor_native_block_hash: String,

    /// Timeout for native HTTP requests and the SHIP websocket handshake, in milliseconds.
    pub native_request_timeout_ms: Option<u64>,

    /// TCP connection timeout for the execution endpoint, in milliseconds.
    pub execution_connect_timeout_ms: Option<u64>,

    /// End-to-end request timeout for the execution endpoint, in milliseconds.
    pub execution_request_timeout_ms: Option<u64>,

    /// Maximum accepted Engine API response size in bytes.
    pub execution_max_response_bytes: Option<usize>,

    /// Nodeos ship ws endpoint
    pub ship_endpoint: String,

    /// Nodeos http endpoint
    pub chain_endpoint: String,

    /// Block count in between finalize block calls while syncing
    pub batch_size: usize,

    /// The parent hash of the start_block
    pub prev_hash: String,

    /// (Optional) For testnet, skip all events until deploy block
    pub evm_deploy_block: Option<u32>,

    /// First translated EVM block; must be the child of the sparse execution anchor.
    pub evm_start_block: u32,

    /// Expected block hash of the start block. Cleared internally only after a canonical restart
    /// checkpoint restores the exact parent context.
    pub validate_hash: String,

    /// EVM block at which the explicitly configured execution context is effective.
    pub execution_context_anchor_block: u32,

    /// Native gas price effective at the start of the anchor block.
    pub execution_context_starting_gas_price: String,

    /// Native EVM revision effective at the start of the anchor block.
    pub execution_context_starting_revision: u64,

    /// Runtime-only native parent binding restored from the durable canonical checkpoint.
    #[serde(skip)]
    pub execution_context_parent_native_hash: Option<String>,

    /// Runtime-only native parent number paired with the native parent hash.
    #[serde(skip)]
    pub execution_context_parent_native_block: Option<u32>,

    /// Runtime-only VALID fork entries used to rebuild the translator branch map.
    #[serde(skip)]
    pub execution_branch_entries: Vec<ExecutionBranchEntry>,

    /// (Optional) Block number to stop on, default is U32::MAX
    pub evm_stop_block: Option<u32>,

    /// Path to the RocksDB folder
    pub data_path: String,

    /// Interval at which block hashes are stored in the database
    pub block_checkpoint_interval: u32,

    /// Maximum range between the latest reth block and the latest stored block
    pub maximum_sync_range: u32,

    /// Number of latest blocks to keep stored in the database
    pub latest_blocks_in_db_num: u32,
}

impl From<&AppConfig> for TranslatorConfig {
    fn from(config: &AppConfig) -> Self {
        let (parent_native_hash, parent_native_block) = match (
            config.execution_context_parent_native_hash.as_ref(),
            config.execution_context_parent_native_block,
        ) {
            (Some(hash), Some(block)) => (hash.clone(), block),
            _ => (
                config.execution_anchor_native_block_hash.clone(),
                config.execution_anchor_native_block_number,
            ),
        };
        Self {
            chain_id: config.chain_id.clone(),
            native_chain_id: config.native_chain_id.clone(),
            execution_anchor_native_block_number: config.execution_anchor_native_block_number,
            execution_anchor_native_block_hash: config.execution_anchor_native_block_hash.clone(),
            native_request_timeout_ms: config.native_request_timeout_ms,
            evm_start_block: config.evm_start_block,
            evm_stop_block: config.evm_stop_block,
            prev_hash: config.prev_hash.clone(),
            evm_deploy_block: config.evm_deploy_block,
            validate_hash: (!config.validate_hash.is_empty()).then(|| config.validate_hash.clone()),
            execution_context_anchor_block: config.execution_context_anchor_block,
            execution_context_starting_gas_price: config
                .execution_context_starting_gas_price
                .clone(),
            execution_context_starting_revision: config.execution_context_starting_revision,
            execution_context_parent_native_hash: Some(parent_native_hash),
            execution_context_parent_native_block: Some(parent_native_block),
            execution_branch_entries: config.execution_branch_entries.clone(),
            http_endpoint: config.chain_endpoint.clone(),
            ship_endpoint: config.ship_endpoint.clone(),
            raw_message_channel_size: 1000,
            block_message_channel_size: 1000,
            final_message_channel_size: 1000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_is_current() {
        toml::from_str::<AppConfig>(include_str!("../config-example.toml")).unwrap();
    }

    #[test]
    fn first_child_hash_is_required_in_input_config() {
        let config = include_str!("../config-example.toml")
            .lines()
            .filter(|line| !line.starts_with("validate_hash ="))
            .collect::<Vec<_>>()
            .join("\n");
        let error = toml::from_str::<AppConfig>(&config).unwrap_err();
        assert!(error.to_string().contains("validate_hash"));
    }
}
