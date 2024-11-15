use crate::block::ProcessingEVMBlock;
use eyre::Result;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, info};

pub async fn evm_block_processor(
    mut block_rx: Receiver<ProcessingEVMBlock>,
    block_tx: Sender<ProcessingEVMBlock>,
) -> Result<()> {
    while let Some(mut block) = block_rx.recv().await {
        debug!("Processing block {}", block.block_num);
        block.deserialize();
        if block_tx.is_closed() {
            continue;
        }
        if let Err(send_err) = block_tx.send(block).await {
            error!(
                "Failed to send block to final processor, error: {:?}",
                send_err
            );
            break;
        }
    }
    info!("Exiting EVM block processor...");
    Ok(())
}
