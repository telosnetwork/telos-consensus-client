use std::rc::Rc;
use std::sync::{Arc, Mutex};
use arrowbatch::reader::{ArrowBatchConfig, ArrowBatchContext};
use crate::client::ConsensusClient;
use crate::config::{AppConfig, CliArgs};
use clap::Parser;

mod auth;
mod arrow_block_reader;
mod client;
mod config;
mod execution_api_client;
mod json_rpc;

#[tokio::main]
async fn main() {
    let cli_args = CliArgs::parse();
    let config_contents = std::fs::read_to_string(cli_args.config).unwrap();
    let config: AppConfig = toml::from_str(&config_contents).unwrap();

    let arrow_config = ArrowBatchConfig {
        data_dir: config.arrow_data.clone(),
        bucket_size: 10_000_000_u64,
        dump_size: 100_000_u64
    };

    let mut context = ArrowBatchContext::new(arrow_config);
    context.lock().unwrap().reload_on_disk_buckets();
    
    let mut client = ConsensusClient::new(config, context).await;
    client.run().await;
}
