extern crate alloc;

use std::{fs, pin::Pin};

use clap::Parser;
use eyre::{Context, Result};
use futures::Future;
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use telos_consensus_client::{
    client::{Error, Shutdown},
    config::{AppConfig, CliArgs},
    main_utils::{parse_log_level, run_client},
};
use tokio_retry::{strategy::FixedInterval, Action, Retry};
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

struct RunClientAction {
    args: CliArgs,
    config: AppConfig,
    attempts: usize,
}

impl RunClientAction {
    fn new(args: CliArgs, config: AppConfig) -> Self {
        Self {
            args,
            config,
            attempts: 0,
        }
    }
}

impl Action for RunClientAction {
    type Future = Pin<Box<dyn Future<Output = Result<Shutdown, Error>>>>;

    type Item = Shutdown;

    type Error = Error;

    fn run(&mut self) -> Self::Future {
        self.attempts += 1;
        let args = self.args.clone();
        let config = self.config.clone();
        let attempt = self.attempts;
        Box::pin(async move {
            warn!(attempt, "Retrying consensus client...");
            run_client(args, config).await
        })
    }
}

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

    if let Err(error) = Retry::spawn(retry_strategy, RunClientAction::new(args, config)).await {
        error!("Stopping consensus client, run failed!");
        return Err(error).wrap_err("Stopping consensus client, run failed!");
    }

    info!("Consensus client Finished!");
    Ok(())
}
