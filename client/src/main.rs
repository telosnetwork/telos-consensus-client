extern crate alloc;

use std::fs;

use clap::Parser;
use eyre::{Context, Result};
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use telos_consensus_client::{
    config::{AppConfig, CliArgs},
    main_utils::{parse_log_level, run_client},
};
use tokio_retry::{strategy::FixedInterval, Retry};
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

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

    if let Err(error) =
        Retry::spawn(retry_strategy, || run_client(args.clone(), config.clone())).await
    {
        error!("Stopping consensus client, run failed!");
        return Err(error).wrap_err("Stopping consensus client, run failed!");
    }

    info!("Consensus client Finished!");
    Ok(())
}
