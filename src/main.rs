use std::fs;

use clap::Parser;
use eyre::{bail, Report, Result};
use log::info;

use crate::client::ConsensusClient;
use crate::config::{AppConfig, CliArgs};

mod auth;
mod client;
mod config;
mod execution_api_client;
mod json_rpc;

fn read_config() -> Result<AppConfig> {
    let CliArgs { config } = CliArgs::parse();
    let contents = fs::read_to_string(config).map_err(Report::new)?;
    Ok(toml::from_str(&contents)?)
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let config = read_config()?;
    let client = ConsensusClient::new(config).await;

    if let Err(error) = client.run().await {
        bail!("Consensus client run failed! Error: {error:?}");
    };

    Ok(info!("Reached stop block, consensus client run finished!"))
}
