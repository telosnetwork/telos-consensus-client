use alloy_primitives::{FixedBytes, Address, Bytes, Bloom, B256};
use alloy_primitives::hex::FromHex;
use reth_primitives::constants::MIN_PROTOCOL_BASE_FEE_U256;
use reth_primitives::{hex, Block, Header};
use reth_rpc_types::ExecutionPayloadV1;
use std::fs::File;
use std::io::{BufRead, BufReader};
use alloy_rlp::Decodable;
use reth_rpc_types_compat::engine::payload::try_block_to_payload_v1;

use arrowbatch::batch::{ArrowBatchConfig, ArrowBatchReader, ArrowBatchTypes};


pub struct ArrowFileBlockReader {
    last_block: Option<ExecutionPayloadV1>,
    reader: ArrowBatchReader,
}

impl ArrowFileBlockReader {
    pub fn new(path: String) -> Self {
        let last_block = None::<ExecutionPayloadV1>;

        let mut reader = ArrowBatchReader::new(
            &ArrowBatchConfig{
                data_dir: path,
                bucket_size: 10_000_000,
                dump_size: 100_000
            }
        );
        reader.reload_on_disk_buckets();

        ArrowFileBlockReader {
            last_block, reader
        }
    }

    pub fn get_block(&mut self, block_num: u64) -> Option<ExecutionPayloadV1> {
        if self.last_block.is_some() && self.last_block.clone().unwrap().block_number == block_num {
            return self.last_block.clone();
        } if self.last_block.is_some() && self.last_block.clone().unwrap().block_number > block_num {
            return None;
        }
        let block_row = self.reader.get_root_row(block_num).unwrap();
        let null_hash =
            B256::from_hex("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap();

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

        Some(
            ExecutionPayloadV1 {
                parent_hash: FixedBytes::from_hex(evm_parent_block_hash).unwrap(),
                fee_recipient: Address::ZERO,
                state_root: null_hash,
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
                transactions: vec![]
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::arrow_block_reader::ArrowFileBlockReader;

    #[test]
    fn block_reader() {
        let mut reader = ArrowFileBlockReader::new("../telosevm-translator/arrow-data-beta".to_string());
        assert_eq!(reader.get_block(180698860).unwrap().block_number,180698860);
        assert_eq!(reader.get_block(180698861).unwrap().block_number,180698861);
        assert_eq!(reader.get_block(180698862).unwrap().block_number,180698862);
    }
}
