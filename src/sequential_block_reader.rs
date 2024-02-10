use reth_primitives::constants::MIN_PROTOCOL_BASE_FEE_U256;
use reth_primitives::{hex, Block, Header};
use reth_rpc_types::ExecutionPayloadV1;
use std::fs::File;
use std::io::{BufRead, BufReader};
use alloy_rlp::Decodable;
use reth_rpc_types_compat::engine::payload::try_block_to_payload_v1;


pub struct SequentialFileBlockReader {
    last_block: Option<ExecutionPayloadV1>,
    reader: BufReader<File>,
}

impl SequentialFileBlockReader {
    pub fn new(path: String) -> Self {
        let last_block = None::<ExecutionPayloadV1>;

        let file = match File::open(&path) {
            Err(why) => panic!("couldn't open {}: {}", &path, why),
            Ok(file) => file,
        };
        let reader = BufReader::new(file);
        SequentialFileBlockReader { last_block , reader }
    }

    pub fn read_next_block(&mut self) -> Option<ExecutionPayloadV1>{
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                let bytes = hex::decode(&line[0..line.len() - 1]).unwrap();
                let bytes_buf = &mut bytes.as_ref();
                let block = Block::decode(bytes_buf).unwrap();
                let mut payload = try_block_to_payload_v1(block.seal_slow());
                payload.base_fee_per_gas = MIN_PROTOCOL_BASE_FEE_U256;
                return Some(payload);
            },  
        }
    }

    pub fn get_block(&mut self, block_num: u64) -> Option<ExecutionPayloadV1> {
        if block_num == 180698823 {
            let bytes = hex!("f9021ef90219a087bb009caefe5447b3c4beafb6cc168d031a73eb9c6bb0718b5b5972448908c2a01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347940000000000000000000000000000000000000000a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421b901000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000080840ac53ec7847fffffff808461782354a00ac53eebcfa1cf139272bf58540d6dc53f0d22785f9e3d74cb9bc333b597e1bfa00000000000000000000000000000000000000000000000000000000000000000880000000000000000c0c0");
            let bytes_buf = &mut bytes.as_ref();
            let block = Block::decode(bytes_buf).unwrap();
            let mut payload = try_block_to_payload_v1(block.seal_slow());
            payload.base_fee_per_gas = MIN_PROTOCOL_BASE_FEE_U256;
            return Some(payload);
        }
        if self.last_block.is_some() && self.last_block.clone().unwrap().block_number == block_num {
            return self.last_block.clone();
        } if self.last_block.is_some() && self.last_block.clone().unwrap().block_number > block_num {
            return None;
        }
        loop {
            match self.read_next_block() {
                None => {
                    return None;
                },
                Some(payload) => {
                    if payload.block_number == block_num {
                        self.last_block = Some(payload);
                        return self.last_block.clone();
                    } else {
                        continue;
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::sequential_block_reader::SequentialFileBlockReader;

    #[test]
    fn block_reader() {
        let mut reader = SequentialFileBlockReader::new("dump-block-bytes.dat".to_string());
        assert_eq!(reader.read_next_block().unwrap().block_number,180698824);
        assert_eq!(reader.get_block(180698860).unwrap().block_number,180698860);
        assert_eq!(reader.get_block(180698861).unwrap().block_number,180698861);
        assert_eq!(reader.get_block(180698860),None);
        assert_eq!(reader.get_block(180698862).unwrap().block_number,180698862);
        assert_eq!(reader.get_block(180698861),None);
    }
}
