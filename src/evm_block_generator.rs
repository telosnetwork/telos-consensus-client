use std::sync::Arc;
use std::thread::sleep;
use dashmap::DashMap;
use crate::block::Block;

pub async fn evm_block_generator(start_block: u32, block_map: Arc<DashMap<u32, Block>>) {
    let mut block_num = start_block;
    loop {
        if let Some(mut block) = block_map.get_mut(&block_num) {
            block.to_evm();
            drop(block);
            block_map.remove(&block_num);
            block_num += 1;
        }
        sleep(std::time::Duration::from_millis(5));
    }
}