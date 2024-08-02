use crate::{
    block::Block, translator::TranslatorConfig, types::translator_types::NameToAddressCache,
};
use alloy::primitives::FixedBytes;
use antelope::api::client::{APIClient, DefaultProvider};
use hex::encode;
use std::str::FromStr;
use tokio::{sync::mpsc, time::Instant};
use tracing::{debug, error, info};

pub async fn final_processor(
    config: TranslatorConfig,
    api_client: APIClient<DefaultProvider>,
    mut rx: mpsc::Receiver<Block>,
    tx: Option<mpsc::Sender<(FixedBytes<32>, Block)>>,
) {
    let mut last_log = Instant::now();
    let mut unlogged_blocks = 0;
    let mut unlogged_transactions = 0;

    let mut parent_hash = FixedBytes::from_str(&config.prev_hash)
        .expect("Prev hash config is not a valid 32 byte hex string");

    let validate_hash = if config.validate_hash.is_some() {
        Some(
            FixedBytes::from_str(&config.validate_hash.unwrap())
                .expect("Validate hash config is not a valid 32 byte hex string"),
        )
    } else {
        None
    };
    let mut validated = false;

    let native_to_evm_cache = NameToAddressCache::new(api_client);

    while let Some(mut block) = rx.recv().await {
        unlogged_blocks += 1;
        unlogged_transactions += block.transactions.len();
        debug!(
            "Finalizing block #{} with #{} transactions",
            block.block_num,
            block.transactions.len()
        );

        let header = block
            .generate_evm_data(parent_hash, config.block_delta, &native_to_evm_cache)
            .await;

        let block_hash = header.hash_slow();

        if !validated && validate_hash.is_some() {
            if validate_hash.unwrap() == block_hash {
                validated = true;
            } else {
                error!(
                    "Initial hash validation failed!, expected: \"{}\" got: \"{}\"",
                    validate_hash.unwrap(),
                    block_hash
                );
                error!("{:#?}", header);
                panic!("Initial hash validation failed!");
            }
        }

        if last_log.elapsed().as_secs_f64() > 1.0 {
            let blocks_sec = unlogged_blocks as f64 / last_log.elapsed().as_secs_f64();
            let trx_sec = unlogged_transactions as f64 / last_log.elapsed().as_secs_f64();
            info!(
                "Block #{} 0x{} - processed {} blocks/sec and {} tx/sec",
                block.block_num,
                encode(block_hash),
                blocks_sec,
                trx_sec
            );
            //info!("Block map is {} long", block_map.len());
            unlogged_blocks = 0;
            unlogged_transactions = 0;
            last_log = Instant::now();
        }
        // TODO: Fork handling, hashing, all the things...

        if tx.is_some() && tx.clone().unwrap().send((block_hash, block)).await.is_err() {
            error!("Failed to send finished block to exit stream!!");
            break;
        }
        parent_hash = block_hash;
    }
}
