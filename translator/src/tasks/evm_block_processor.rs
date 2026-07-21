use crate::block::ProcessingEVMBlock;
use eyre::{eyre, Result};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, info};

pub async fn evm_block_processor(
    mut block_rx: Receiver<ProcessingEVMBlock>,
    block_tx: Sender<ProcessingEVMBlock>,
) -> Result<()> {
    while let Some(mut block) = block_rx.recv().await {
        debug!("Processing block {}", block.block_num);
        block.deserialize()?;
        if block_tx.is_closed() {
            return Err(eyre!(
                "final processor stopped while decoded blocks remained"
            ));
        }
        block_tx
            .send(block)
            .await
            .map_err(|_| eyre!("final processor stopped while decoded blocks remained"))?;
    }
    info!("Exiting EVM block processor...");
    Ok(())
}
