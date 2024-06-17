use std::sync::Arc;
use dashmap::DashMap;
use crate::block::Block;
use crate::types::types::PriorityQueue;

pub async fn block_deserializer(queue: PriorityQueue, block_map: Arc<DashMap<u32, Block>>) {
    loop {
        if let Some(mut block) = queue.pop() {
            block.deserialize();
            block_map.insert(block.block_num, block);
        }
    }
}