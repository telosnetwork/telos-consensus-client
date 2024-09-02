use crate::client::ConsensusClient;
use crate::config::{AppConfig, CliArgs};
use clap::Parser;
use env_logger::Builder;
use log::{error, info, LevelFilter};

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
    let log_level = parse_log_level(&config.log_level).unwrap();

    let mut builder = Builder::from_default_env();
    builder.filter_level(log_level);
    builder.filter_module(env!("CARGO_PKG_NAME"), log_level);
    builder.init();

    info!("Starting Telos consensus client...");

    let mut client = ConsensusClient::new(&config).await.unwrap();
    info!("Created client...");
    let result = client.run();
    info!("Started client, awaiting result...");
    match result.await {
        Ok(()) => {
            info!("Reached stop block, consensus client run finished!")
        }
        Err(e) => {
            error!("Consensus client run failed! Error: {:?}", e);
        }
    }
}

fn parse_log_level(s: &str) -> Result<LevelFilter, String> {
    match s.to_lowercase().as_str() {
        "off" => Ok(LevelFilter::Off),
        "error" => Ok(LevelFilter::Error),
        "warn" => Ok(LevelFilter::Warn),
        "info" => Ok(LevelFilter::Info),
        "debug" => Ok(LevelFilter::Debug),
        "trace" => Ok(LevelFilter::Trace),
        _ => Err(format!("Unknown log level: {}", s)),
    }
}
