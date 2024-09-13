use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use tokio::sync::mpsc;
use tracing::level_filters::LevelFilter;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::Translator;
use crate::config::{AppConfig, CliArgs};
use crate::client::{ConsensusClient, Error, Shutdown};
use tracing::{info, warn};
use crate::client::Error::{CannotStartConsensusClient, TranslatorShutdown};
use crate::data::Block;

pub async fn run_client(args: CliArgs, mut config: AppConfig) -> Result<Shutdown, Error> {
    let (client, lib) = build_consensus_client(&args, &mut config).await?;
    let client_shutdown = client.shutdown_handle();
    let translator = Translator::new((&config).into());

    let (block_sender, block_receiver) = mpsc::channel::<TelosEVMBlock>(1000);

    info!("Telos consensus client starting, awaiting result...");
    let client_handle = tokio::spawn(client.run(block_receiver, lib));

    let translator_shutdown = translator.shutdown_handle();

    info!("Telos translator client launching, awaiting result...");
    let translator_handle = tokio::spawn(translator.launch(Some(block_sender)));

    // Run the client and handle the result
    if let Ok(Err(error)) = client_handle.await {
        warn!("Consensus client run failed! Error: {error:?}");

        if let Err(error) = translator_shutdown.shutdown().await {
            warn!("Cannot send shutdown signal! Error: {error:?}");
            return Err(TranslatorShutdown(error.to_string()));
        }

        if let Err(error) = translator_handle.await {
            warn!("Cannot stop translator! Error: {error:?}");
            return Err(TranslatorShutdown(error.to_string()));
        }
        warn!("Retrying...");
        return Err(error);
    }

    info!("Reached stop block/signal, consensus client run finished!");
    Ok(client_shutdown)
}

pub async fn build_consensus_client(
    args: &CliArgs,
    config: &mut AppConfig,
) -> Result<(ConsensusClient, Option<Block>), Error> {
    let client = ConsensusClient::new(args, config.clone())
        .await
        .map_err(|e| {
            warn!("Consensus client creation failed: {}", e);
            warn!("Retrying...");
            CannotStartConsensusClient(e.to_string())
        })?;

    // Translator
    let lib = client.db.get_lib()?;

    let latest_number = client.min_latest_or_lib(lib.as_ref());

    let last_checked = match latest_number {
        Some(latest_number) => client.db.get_block_or_prev(latest_number)?,
        None => None,
    };

    if let Some(last_checked) = last_checked {
        if client.is_in_start_stop_range(last_checked.number + 1) {
            config.evm_start_block = last_checked.number + 1;
            config.prev_hash = last_checked.hash
        }
    }

    if let Some(sync_range) = client.sync_range() {
        if sync_range > config.maximum_sync_range.as_u64() {
            return Err(Error::RangeAboveMaximum(sync_range));
        }
    }
    Ok((client, lib))
}

pub fn parse_log_level(s: &str) -> Result<LevelFilter, String> {
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
