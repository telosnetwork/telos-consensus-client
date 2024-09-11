extern crate alloc;

use crate::client::Error::CannotStartConsensusClient;
use crate::client::{ConsensusClient, Error};
use crate::config::{AppConfig, CliArgs};
use alloc::string::String;
use clap::Parser;
use eyre::Result;
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use tokio::sync::{mpsc, oneshot};
use tokio_retry::strategy::FixedInterval;
use tokio_retry::Retry;
use tracing::level_filters::LevelFilter;
use tracing::{info, warn};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod auth;
mod client;
mod config;
mod data;
mod execution_api_client;
mod json_rpc;

#[tokio::main]
async fn main() {
    let args = CliArgs::parse();
    let config_contents = std::fs::read_to_string(&args.config).unwrap();
    let config: AppConfig = toml::from_str(&config_contents).unwrap();
    let log_level_filter = parse_log_level(&config.log_level).unwrap();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(log_level_filter)
        .init();

    let retry_strategy = FixedInterval::from_millis(config.retry_interval.unwrap_or(8000u64))
        .take(config.max_retry.unwrap_or(8u8).as_usize());

    let result = Retry::spawn(retry_strategy, || run_client(args.clone(), config.clone())).await;
    match result {
        Ok(()) => {
            info!("Consensus client Finished!");
        }
        Err(e) => {
            info!("Stopping consensus client, run failed! Error: {:?}", e);
        }
    }
}

async fn run_client(args: CliArgs, config: AppConfig) -> Result<(), Error> {
    info!("Starting Telos consensus client...");
    let (_sender, receiver) = oneshot::channel();
    let (tr_sender, tr_receiver) = mpsc::channel::<()>(1);
    let mut client = ConsensusClient::new(&args, config.clone())
        .await
        .map_err(|e| {
            warn!("Consensus client creation failed: {}", e);
            warn!("Retrying...");
            CannotStartConsensusClient(e.to_string())
        })?;

    info!("Started client, awaiting result...");
    // Run the client and handle the result
    if let Err(e) = client.run(receiver, tr_sender.clone(), tr_receiver).await {
        warn!("Consensus client run failed! Error: {:?}", e);
        // Send a signal to indicate failure
        if let Err(e) = tr_sender.send(()).await {
            warn!("Cannot send shutdown signal! Error: {:?}", e);
        }
        warn!("Retrying...");
        return Err(e);
    }

    info!("Reached stop block/signal, consensus client run finished!");
    Ok(())
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
