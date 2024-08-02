use crate::block::Block;
use crate::types::translator_types::BlockOrSkip;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::debug;

pub async fn order_preserving_queue(
    mut rx: mpsc::Receiver<BlockOrSkip>,
    tx: mpsc::Sender<Block>,
    queue: Arc<Mutex<BinaryHeap<Reverse<Block>>>>,
) {
    let mut next_sequence = 1;

    while let Some(block_or_skip) = rx.recv().await {
        let block = match block_or_skip {
            BlockOrSkip::Block(block) => block,
            BlockOrSkip::Skip(sequence) => {
                debug!("Skipping block with sequence #{}", sequence);
                next_sequence = sequence + 1;
                continue;
            }
        };
        debug!(
            "Handling order for block #{} with sequence #{}, next sequence is {}",
            block.block_num, block.sequence, next_sequence
        );
        let mut queue = queue.lock().await;
        queue.push(Reverse(block));

        while let Some(Reverse(block)) = queue.peek() {
            if block.sequence == next_sequence {
                let block = queue.pop().unwrap().0;
                debug!(
                    "Pushing next block #{} in sequence #{}",
                    block.block_num, block.sequence
                );
                if tx.send(block).await.is_err() {
                    break;
                }
                next_sequence += 1;
            } else {
                break;
            }
        }
    }
}
