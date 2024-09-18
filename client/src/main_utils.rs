use std::cmp;

use crate::client::Error::{CannotStartConsensusClient, TranslatorShutdown};
use crate::client::{ConsensusClient, Error, Shutdown};
use crate::config::{AppConfig, CliArgs};
use eyre::eyre;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::Translator;
use tokio::sync::mpsc;
use tracing::level_filters::LevelFilter;
use tracing::{info, warn};

pub async fn run_client(args: CliArgs, config: AppConfig) -> Result<Shutdown, Error> {
    let client = build_consensus_client(&args, config).await?;
    let client_shutdown = client.shutdown_handle();
    let translator = Translator::new((&client.config).into());

    let (block_sender, block_receiver) = mpsc::channel::<TelosEVMBlock>(1000);

    info!("Telos consensus client starting, awaiting result...");
    let client_handle = tokio::spawn(client.run(block_receiver));

    let translator_shutdown = translator.shutdown_handle();

    info!(
        evm_start_block = translator.config.evm_start_block,
        evm_stop_block = ?translator.config.evm_stop_block,
        "Telos translator client launching, awaiting result...",
    );
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
    config: AppConfig,
) -> Result<ConsensusClient, Error> {
    let mut client = ConsensusClient::new(args, config).await.map_err(|e| {
        warn!("Consensus client creation failed: {}", e);
        warn!("Retrying...");
        CannotStartConsensusClient(e.to_string())
    })?;

    info!(
        "Created client with latest EVM block: {:?}",
        client.latest_evm_number()
    );

    let lib = client.db.get_lib()?;

    if let Some(lib_number) = lib.as_ref().map(|lib| lib.number) {
        info!("Last stored LIB: {lib_number}");
    }

    let latest_number = lib
        .as_ref()
        .map(|lib| lib.number + client.config.chain_id.block_delta())
        .zip(client.latest_evm_number())
        .map(|(lib, latest)| cmp::min(lib, latest));

    let last_checked = match latest_number {
        Some(latest_number) => client.db.get_block_or_prev(latest_number)?,
        None => None,
    };

    if let Some(last_checked) = last_checked.as_ref() {
        info!(
            "Last stored final block: {}, {}",
            last_checked.number, last_checked.hash
        );
    }

    if let Some(last_checked) = last_checked {
        if client.is_in_start_stop_range(last_checked.number + 1) {
            client.config.evm_start_block = last_checked.number + 1;
            client.config.prev_hash = last_checked.hash
        }
    }

    if let Some(sync_range) = client.sync_range() {
        if sync_range > client.config.maximum_sync_range {
            return Err(Error::RangeAboveMaximum(sync_range));
        }
    }
    Ok(client)
}

pub fn parse_log_level(s: &str) -> eyre::Result<LevelFilter> {
    match s.to_lowercase().as_str() {
        "off" => Ok(LevelFilter::OFF),
        "error" => Ok(LevelFilter::ERROR),
        "warn" => Ok(LevelFilter::WARN),
        "info" => Ok(LevelFilter::INFO),
        "debug" => Ok(LevelFilter::DEBUG),
        "trace" => Ok(LevelFilter::TRACE),
        _ => Err(eyre!("Unknown log level: {s}")),
    }
}
