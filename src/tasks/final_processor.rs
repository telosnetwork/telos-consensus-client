use crate::block::Block;
use crate::translator::START_BLOCK;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info};

pub async fn final_processor(mut rx: mpsc::Receiver<Block>) {
    let mut last_log = std::time::Instant::now();
    let mut unlogged_blocks = 0;
    let mut unlogged_transactions = 0;

    while let Some(block) = rx.recv().await {
        unlogged_blocks += 1;
        unlogged_transactions += block.transactions.len();
        debug!("Finalizing block #{}", block.block_num);
        if last_log.elapsed().as_secs_f64() > 1.0 {
            let blocks_sec = unlogged_blocks as f64 / last_log.elapsed().as_secs_f64();
            let trx_sec = unlogged_transactions as f64 / last_log.elapsed().as_secs_f64();
            info!(
                "Block #{} - processed {} blocks/sec and {} tx/sec",
                block.block_num, blocks_sec, trx_sec
            );
            //info!("Block map is {} long", block_map.len());
            unlogged_blocks = 0;
            unlogged_transactions = 0;
            last_log = Instant::now();
        }
        // TODO: Fork handling, hashing, all the things...
    }
}
