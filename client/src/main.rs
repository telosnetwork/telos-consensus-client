extern crate alloc;

use crate::config::{AppConfig, CliArgs};
use crate::main_utils::{parse_log_level, run_client};
use clap::Parser;
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use tokio_retry::strategy::FixedInterval;
use tokio_retry::Retry;
use tracing::info;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod auth;
mod client;
mod config;
mod data;
mod execution_api_client;
mod json_rpc;
mod main_utils;

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
