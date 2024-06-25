use std::sync::Arc;
use std::thread::sleep;
use std::time::Instant;
use antelope::api::client::{APIClient, DefaultProvider};
use dashmap::DashMap;
use tracing::info;
use crate::block::Block;
use crate::types::types::NameToAddressCache;

pub async fn evm_block_generator(start_block: u32, block_map: Arc<DashMap<u32, Block>>, api_client: APIClient<DefaultProvider>) {
    let mut block_num = start_block;
    let mut last_log = std::time::Instant::now();
    let mut unlogged_blocks = 0;
    let mut unlogged_transactions = 0;
    let native_to_evm_cache = NameToAddressCache::new(api_client);
    loop {
        if let Some(mut block) = block_map.get_mut(&block_num) {
            block.generate_evm_data(&native_to_evm_cache).await;
            unlogged_blocks += 1;
            unlogged_transactions += block.transactions.len();
            if last_log.elapsed().as_secs_f64() > 1.0 {
                let blocks_sec =  unlogged_blocks as f64 / last_log.elapsed().as_secs_f64();
                let trx_sec =  unlogged_transactions as f64 / last_log.elapsed().as_secs_f64();
                info!("Block #{} - processed {} blocks/sec and {} tx/sec", block.block_num, blocks_sec, trx_sec);
                //info!("Block map is {} long", block_map.len());
                unlogged_blocks = 0;
                unlogged_transactions = 0;
                last_log = Instant::now();
            }

            drop(block);
            block_map.remove(&block_num);
            block_num += 1;
        }
        sleep(std::time::Duration::from_millis(5));
    }
}