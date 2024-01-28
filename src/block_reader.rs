use alloy_primitives::{Address, Bloom};
use chrono::DateTime;
use reth_primitives::constants::MIN_PROTOCOL_BASE_FEE_U256;
use reth_primitives::{hex, Bytes, B256};
use reth_rpc_types::ExecutionPayloadV1;
use std::collections::HashMap;
use std::str::FromStr;

pub struct FileBlockReader {
    blocks: HashMap<u64, ExecutionPayloadV1>,
}

fn iso8601_to_epoch(date_str: &str) -> u64 {
    let date = DateTime::parse_from_rfc3339(date_str).expect("Failed to parse date");
    date.timestamp() as u64
}

impl FileBlockReader {
    pub fn new(path: String) -> Self {
        let null_hash =
            B256::from_str("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap();
        let mut blocks = HashMap::new();
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_path(path)
            .unwrap();

        for result in reader.records() {
            let record = result.unwrap();
            let timestamp = iso8601_to_epoch(record.get(0).unwrap());
            let block_number = record.get(1).unwrap().parse::<u64>().unwrap() - 36;
            // native block hash
            let extra_data_string = record.get(2).unwrap().to_string();
            let extra_data =
                Bytes::copy_from_slice(hex::decode(extra_data_string.as_str()).unwrap().as_slice());
            let block_hash = B256::from_str(record.get(3).unwrap()).unwrap();
            let parent_hash = B256::from_str(record.get(4).unwrap()).unwrap();
            let receipts_root = B256::from_str(record.get(5).unwrap()).unwrap();
            //let trx_root = record.get(6).unwrap();
            let gas_used = record.get(7).unwrap().parse::<u64>().unwrap();
            blocks.insert(
                block_number,
                ExecutionPayloadV1 {
                    parent_hash,
                    fee_recipient: Address::ZERO,
                    state_root: null_hash,
                    receipts_root,
                    logs_bloom: Bloom::default(),
                    prev_randao: Default::default(),
                    block_number,
                    gas_limit: 0x7fffffffu64,
                    gas_used,
                    timestamp,
                    extra_data,
                    base_fee_per_gas: MIN_PROTOCOL_BASE_FEE_U256,
                    block_hash,
                    transactions: vec![],
                },
            );
        }

        FileBlockReader { blocks }
    }

    pub fn get_block(&self, block_num: u64) -> Option<&ExecutionPayloadV1> {
        self.blocks.get(&block_num)
    }
}

#[cfg(test)]
mod tests {
    use crate::block_reader::FileBlockReader;

    #[test]
    fn block_reader() {
        let reader = FileBlockReader::new("blocks.csv".to_string());
        let block_zero = reader.get_block(0);
        assert!(block_zero.is_some());
    }
}
