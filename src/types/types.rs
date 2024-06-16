use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};
use crate::block::Block;

pub struct PriorityQueue {
    heap: Arc<Mutex<BinaryHeap<Block>>>,
}

impl PriorityQueue {
    pub fn new() -> Self {
        PriorityQueue {
            heap: Arc::new(Mutex::new(BinaryHeap::new())),
        }
    }

    pub fn push(&self, item: Block) {
        let mut heap = self.heap.lock().unwrap();
        heap.push(item);
    }

    pub fn pop(&self) -> Option<Block> {
        let mut heap = self.heap.lock().unwrap();
        heap.pop()
    }
}

impl Clone for PriorityQueue {
    fn clone(&self) -> Self {
        PriorityQueue {
            heap: Arc::clone(&self.heap),
        }
    }
}
