use std::fs;

use clap::Parser;
use eyre::{Context, Report, Result};
use log::info;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::Translator;
use tokio::sync::mpsc;

use crate::client::ConsensusClient;
use crate::config::{AppConfig, CliArgs};

mod auth;
mod client;
mod config;
mod execution_api_client;
mod json_rpc;

fn read_config() -> Result<AppConfig> {
    let CliArgs { config } = CliArgs::parse();
    let contents = fs::read_to_string(config).map_err(Report::new)?;
    Ok(toml::from_str(&contents)?)
}

async fn launch(mut translator: Translator, tx: mpsc::Sender<TelosEVMBlock>) -> Result<()> {
    translator.launch(Some(tx)).await
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let config = read_config()?;

    let translator = Translator::new(config.clone().into());
    let consensus_client = ConsensusClient::new(config).await;

    let (tx, rx) = mpsc::channel::<TelosEVMBlock>(1000);

    _ = tokio::try_join!(
        tokio::spawn(consensus_client.run(rx)),
        tokio::spawn(launch(translator, tx))
    )
    .wrap_err("Consensus client run failed")?;

    Ok(info!("Reached stop block, consensus client run finished!"))
}
