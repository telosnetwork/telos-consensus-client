use clap::Parser;
use serde::Deserialize;
use telos_translator_rs::translator::TranslatorConfig;

/// Telos Consensus Client CLI Arguments
#[derive(Parser, Debug)]
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
pub struct AppConfig {
    /// Log level for the application    
    pub log_level: String,

    /// EVM Chain id, Telos mainnet is 40 and testnet is 41
    pub chain_id: u64,

    /// Execution API http endpoint (JWT protected endpoint on reth)
    pub execution_endpoint: String,

    /// The JWT secret used to sign the JWT token
    pub jwt_secret: String,

    /// Nodeos ship ws endpoint
    pub ship_endpoint: String,

    /// Nodeos http endpoint
    pub chain_endpoint: String,

    /// Block count in between finalize block calls while syncing
    pub batch_size: usize,

    /// Block delta between native block and EVM block
    pub block_delta: Option<u32>,

    /// The parent hash of the start_block
    pub prev_hash: String,

    /// Start block to start with, should be at or before the first block of the execution node
    pub start_block: u32,

    /// (Optional) Expected block hash of the start block
    pub validate_hash: Option<String>,

    /// (Optional) Block number to stop on, default is U32::MAX
    pub stop_block: Option<u32>,

    /// Path to the RocksDB folder
    pub data_path: String,

    /// Interval at which block hashes are stored in database
    pub block_checkpoint_interval: u64,

    /// Maximum range between the latest reth block and the latest stored block
    pub maximum_sync_range: u32,
}

impl From<&AppConfig> for TranslatorConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            chain_id: config.chain_id,
            start_block: config.start_block,
            stop_block: config.stop_block,
            block_delta: config.block_delta.unwrap_or(0u32),
            prev_hash: config.prev_hash.clone(),
            validate_hash: config.validate_hash.clone(),
            http_endpoint: config.chain_endpoint.clone(),
            ship_endpoint: config.ship_endpoint.clone(),
            raw_message_channel_size: 1000,
            block_message_channel_size: 1000,
            final_message_channel_size: 1000,
        }
    }
}
