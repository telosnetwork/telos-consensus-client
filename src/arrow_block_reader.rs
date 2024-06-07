use std::sync::{Arc, Mutex};
use std::time::Duration;
use alloy_primitives::{FixedBytes, Address, Bytes, Bloom, B256};
use alloy_primitives::hex::FromHex;
use arrowbatch::proto::ArrowBatchTypes;
use reth_primitives::constants::MIN_PROTOCOL_BASE_FEE_U256;
use reth_primitives::hex;
use reth_rpc_types::ExecutionPayloadV1;

use arrowbatch::reader::{ArrowBatchContext, ArrowBatchReader};
use tokio::time::sleep;

macro_rules! null_hash {
    () => {
        B256::from_hex("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421").unwrap()
    };
}


pub struct ArrowFileBlockReader {
    delta: u64,
    last_block: Option<ExecutionPayloadV1>,
    reader: ArrowBatchReader,
}

impl ArrowFileBlockReader {
    pub fn new(context: Arc<Mutex<ArrowBatchContext>>, delta: u64) -> Self {
        let last_block = None::<ExecutionPayloadV1>;

        let context_clone = Arc::clone(&context);

        // TODO: Remove this once we have live updating
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(5)).await;
                let mut context = context_clone.lock().unwrap();
                context.reload_on_disk_buckets();
            }
        });

        ArrowFileBlockReader {
            delta,
            last_block,
            reader: ArrowBatchReader::new(context.clone()),
        }
    }

    pub fn get_latest_block(&self) -> Option<ExecutionPayloadV1> {
        // TODO: remove this once websocket live updating is implemented
        let latest_block_num = self.reader.context.lock().unwrap().last_ordinal.unwrap();
        self.get_block(latest_block_num - self.delta)
    }

    pub fn get_block(&self, block_num: u64) -> Option<ExecutionPayloadV1> {
        if let Some(ref last_block) = self.last_block {
            if last_block.block_number == block_num {
                return Some(last_block.clone());
            }
            if last_block.block_number > block_num {
                return None;
            }
        }
        let block = self.reader.get_row(block_num + self.delta)?;

        let block_row = block.row;

        let timestamp = match block_row[1] {
            ArrowBatchTypes::U64(value) => value,
            _ => panic!("Invalid type for timestamp")
        };
        let block_hash = match &block_row[2] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for block_hash")
        };
        let evm_block_hash = match &block_row[3] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for evm_block_hash")
        };
        let evm_parent_block_hash = match &block_row[4] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for evm_parent_block_hash")
        };
        let receipt_hash = match &block_row[5] {
            ArrowBatchTypes::Checksum256(value) => value.clone(),
            _ => panic!("Invalid type for receipt_hash")
        };
        let gas_used = match &block_row[7] {
            ArrowBatchTypes::UVar(value) => value.clone().to_string().parse().expect("Gas used is not a valid u64"),
            _ => panic!("Invalid type for receipt_hash")
        };
        let extra_data =
            Bytes::copy_from_slice(hex::decode(block_hash.as_str()).unwrap().as_slice());

        let mut txs = Vec::new();

        if block.refs.contains_key("tx") {
            for tx in block.refs.get("tx").unwrap() {
                let tx_raw = match &tx.row[4] {
                    ArrowBatchTypes::Bytes(r) => r,
                    _ => panic!("Invalid type for tx.raw")
                };
                txs.push(alloy_primitives::Bytes::from(tx_raw.clone()));
            }
        }

        Some(
            ExecutionPayloadV1 {
                parent_hash: FixedBytes::from_hex(evm_parent_block_hash).unwrap(),
                fee_recipient: Address::ZERO,
                state_root: null_hash!(),
                receipts_root: FixedBytes::from_hex(receipt_hash).unwrap(),
                logs_bloom: Bloom::default(),
                prev_randao: Default::default(),
                block_number: block_num,
                gas_limit: 0x7fffffffu64,
                gas_used,
                timestamp,
                extra_data,
                base_fee_per_gas: MIN_PROTOCOL_BASE_FEE_U256,
                block_hash: FixedBytes::from_hex(evm_block_hash).unwrap(),
                transactions: txs
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use arrowbatch::reader::{ArrowBatchConfig, ArrowBatchContext};

    use crate::arrow_block_reader::ArrowFileBlockReader;

    #[test]
    fn test_arrow_block_reader() {
        let config = ArrowBatchConfig {
            data_dir: "/Users/jesse/repos/telos-consensus-client/mainnet_data/mainnet-arrow-data".to_string(),
            bucket_size: 10_000_000_u64,
            dump_size: 100_000_u64
        };

        let mut context = ArrowBatchContext::new(config);

        context.reload_on_disk_buckets();
        let mut reader = ArrowFileBlockReader::new(Box::new(context), 36);
        assert_eq!(reader.get_block(332933022).unwrap().block_number,332933022);
        assert_eq!(reader.get_block(332933023).unwrap().block_number,332933023);

        let first_tx_block_num = 332933024;
        let first_tx_block = reader.get_block(first_tx_block_num).unwrap();

        println!("{:#?}", first_tx_block);

        assert_eq!(first_tx_block.transactions.len(), 1);
    }
}
