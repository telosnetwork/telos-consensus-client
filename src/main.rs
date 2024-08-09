use crate::client::ConsensusClient;
use crate::config::{AppConfig, CliArgs};
use clap::Parser;
use log::{error, info};

mod auth;
mod client;
mod config;
mod execution_api_client;
mod json_rpc;

#[tokio::main]
async fn main() {
    let cli_args = CliArgs::parse();
    let config_contents = std::fs::read_to_string(cli_args.config).unwrap();
    let config: AppConfig = toml::from_str(&config_contents).unwrap();
    env_logger::init();

    let mut client = ConsensusClient::new(config, context).await;
    let result = client.run().await;
    match result {
        Ok(()) => {
            info!("Reached stop block, consensus client run finished!")
        }
        Err(e) => {
            error!("Consensus client run failed! Error: {:?}", e);
        }
    }
}
