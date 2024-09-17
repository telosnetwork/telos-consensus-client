extern crate alloc;

use std::fs;

use crate::config::{AppConfig, CliArgs};
use crate::main_utils::{parse_log_level, run_client};
use clap::Parser;
use eyre::{Context, Result};
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use tokio_retry::{strategy::FixedInterval, Retry};
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

mod auth;
mod client;
mod config;
mod data;
mod execution_api_client;
mod json_rpc;
mod main_utils;

#[tokio::main]
async fn main() -> Result<()> {
    let args = CliArgs::parse();
    let config_contents = fs::read_to_string(&args.config)?;
    let config: AppConfig = toml::from_str(&config_contents)?;
    let log_level_filter = parse_log_level(&config.log_level)?;

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(log_level_filter)
        .init();

    let retry_interval = config.retry_interval.unwrap_or(8000u64);
    let max_retries = config.max_retry.unwrap_or(8u8).as_usize();
    let retry_strategy = FixedInterval::from_millis(retry_interval).take(max_retries);

    Retry::spawn(retry_strategy, || run_client(args.clone(), config.clone()))
        .await
        .wrap_err("Stopping consensus client, run failed!")?;

    info!("Consensus client Finished!");
    Ok(())
}
