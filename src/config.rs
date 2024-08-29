use clap::Parser;
use serde::Deserialize;
use tracing::level_filters::LevelFilter;

/// Telos Consensus Client CLI Arguments
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct CliArgs {
    /// Path to the configuration file
    #[clap(short, long, value_parser)]
    pub config: String,
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
}
