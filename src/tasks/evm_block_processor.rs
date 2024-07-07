use std::sync::Arc;
use crate::block::Block;
use crate::types::types::{BlockOrSkip, NameToAddressCache};
use antelope::api::client::{APIClient, DefaultProvider};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use tracing::{debug, error};

pub async fn evm_block_processor(
    block_rx: Arc<Mutex<Receiver<Block>>>,
    block_tx: Sender<BlockOrSkip>,
    api_client: APIClient<DefaultProvider>,
) {
    let native_to_evm_cache = NameToAddressCache::new(api_client);

    while let Some(mut block) = block_rx.lock().await.recv().await {
        debug!("Processing block {}", block.block_num);
        block.process(&native_to_evm_cache).await;
        if block_tx.send(BlockOrSkip::Block(block)).await.is_err() {
            error!("Failed to send block to final processor!!");
            break;
        }
    }
}
