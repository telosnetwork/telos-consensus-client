extern crate alloc;

use std::fs;

use clap::Parser;
use eyre::{Context, Result};
use telos_consensus_client::{
    config::{AppConfig, CliArgs},
    main_utils::{parse_log_level, run_client},
};
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

    if let Err(error) = run_client(args, config).await {
        error!("Stopping consensus client, run failed!");
        return Err(error).wrap_err("Stopping consensus client, run failed!");
    }

    info!("Consensus client Finished!");
    Ok(())
}
