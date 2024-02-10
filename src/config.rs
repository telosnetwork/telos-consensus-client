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
    /// The path to the blocks.csv file (temporary for POC
    pub blocks_csv: String,
    /// The path to the dump-block-bytes.dat
    pub blocks_bytes: String,
}
