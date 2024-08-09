use clap::Parser;
use serde::Deserialize;

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
    pub block_delta: Option<u8>,
    
    /// The parent hash of the start_block
    pub prev_hash: String,
    
    /// Start block to start with, should be at or before the first block of the execution node
    pub start_block: u32,

    /// (Optional) Block number to stop on, default is U32::MAX
    pub stop_block: Option<u32>,
}
