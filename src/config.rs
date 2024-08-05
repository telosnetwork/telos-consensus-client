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
    /// The base URL of the execution API
    pub base_url: String,
    /// The JWT secret used to sign the JWT token
    pub jwt_secret: String,
    /// The path to the arrow file directory of blocks
    pub arrow_data: String,
    /// The path to store address map
    pub address_map: String,
    /// Block count in between finalize block calls
    pub batch_size: u64,
    /// Nodeos http endpoint
    pub chain_endpoint: String,

    pub stop_block: u64,
}
