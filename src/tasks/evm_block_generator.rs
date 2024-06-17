use std::sync::Arc;
use std::thread::sleep;
use dashmap::DashMap;
use moka::sync::{Cache, CacheBuilder};
use crate::block::Block;

pub async fn evm_block_generator(start_block: u32, block_map: Arc<DashMap<u32, Block>>) {
    let mut block_num = start_block;
    // let native_to_evm_cache = CacheBuilder::new(10_000);
    loop {
        if let Some(mut block) = block_map.get_mut(&block_num) {
            // block.generate_evm_data();
            drop(block);
            block_map.remove(&block_num);
            block_num += 1;
        }
        sleep(std::time::Duration::from_millis(5));
    }
}