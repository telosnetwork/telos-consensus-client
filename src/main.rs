use crate::client::ConsensusClient;
use crate::config::{AppConfig, CliArgs};
use clap::Parser;
use tokio::sync::oneshot;
use tracing::level_filters::LevelFilter;
use tracing::{error, info};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

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
    let log_level_filter = parse_log_level(&config.log_level).unwrap();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(log_level_filter)
        .init();

    info!("Starting Telos consensus client...");

    let (_, receiver) = oneshot::channel();

    let mut client = ConsensusClient::new(config).await.unwrap();
    info!("Created client...");
    let result = client.run(receiver);
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
        "off" => Ok(LevelFilter::OFF),
        "error" => Ok(LevelFilter::ERROR),
        "warn" => Ok(LevelFilter::WARN),
        "info" => Ok(LevelFilter::INFO),
        "debug" => Ok(LevelFilter::DEBUG),
        "trace" => Ok(LevelFilter::TRACE),
        _ => Err(format!("Unknown log level: {}", s)),
    }
}
