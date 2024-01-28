use crate::client::ConsensusClient;
use crate::config::{AppConfig, CliArgs};
use clap::Parser;

mod auth;
mod block_reader;
mod client;
mod config;
mod execution_api_client;
mod json_rpc;

#[tokio::main]
async fn main() {
    let cli_args = CliArgs::parse();
    let config_contents = std::fs::read_to_string(cli_args.config).unwrap();
    let config: AppConfig = toml::from_str(&config_contents).unwrap();
    let client = ConsensusClient::new(config);
    client.run().await;
}
