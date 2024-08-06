use crate::{block::Block, types::translator_types::BlockOrSkip};
use eyre::Result;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error};

pub async fn evm_block_processor(
    mut block_rx: Receiver<Block>,
    block_tx: Sender<BlockOrSkip>,
) -> Result<()> {
    while let Some(mut block) = block_rx.recv().await {
        debug!("Processing block {}", block.block_num);
        block.deserialize();
        if block_tx.send(BlockOrSkip::Block(block)).await.is_err() {
            error!("Failed to send block to final processor!!");
            break;
        }
    }
    Ok(())
}
