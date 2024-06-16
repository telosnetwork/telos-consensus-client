use antelope::chain::name::Name;
use std::cmp::Ordering;
use antelope::chain::Decoder;
use crate::types::evm_types::{EvmRaw, PrintedReceipt, Transfer};
use crate::types::ship_types::{ActionTrace, GetBlocksResultV0, SignedBlock, TableDelta, TransactionTrace};

const EOSIO_EVM: u64 = 6138663583658016768u64;
const EOSIO_TOKEN: u64 = 6138663591592764928u64;
const RAW: u64 = 13382446292731428864u64;
const WITHDRAW: u64 = 16407410437513019392u64;
const TRANSFER: u64 = 14829575313431724032u64;
const DORESOURCES: u64 = 5561572063795392512u64;

#[derive(Clone)]
pub struct Block {
    pub block_num: u32,
    result: GetBlocksResultV0,
    signed_block: Option<SignedBlock>,
    block_traces: Option<Vec<TransactionTrace>>,
    block_deltas: Option<Vec<TableDelta>>,
}

pub fn decode_raw(raw: &[u8]) -> EvmRaw {
    let mut decoder = Decoder::new(raw);
    let raw = &mut EvmRaw::default();
    decoder.unpack(raw);
    raw.clone()
}

pub fn decode_transfer(raw: &[u8]) -> Transfer {
    let mut decoder = Decoder::new(raw);
    let transfer = &mut Transfer::default();
    decoder.unpack(transfer);
    transfer.clone()
}

impl Block {
    pub fn new(block_num: u32, result: GetBlocksResultV0) -> Self {
        Self {
            block_num,
            result,
            signed_block: None,
            block_traces: None,
            block_deltas: None,
        }
    }

    pub fn deserialize(&mut self) {
        if let Some(b) = &self.result.block {
            let mut decoder = Decoder::new(b.as_slice().clone());
            let signed_block = &mut SignedBlock::default();
            decoder.unpack(signed_block);
            self.signed_block = Some(signed_block.clone());
        }

        if let Some(t) = &self.result.traces {
            let mut decoder = Decoder::new(t.as_slice().clone());
            let block_traces: &mut Vec<TransactionTrace> = &mut vec![];
            decoder.unpack(block_traces);
            self.block_traces = Some(block_traces.to_vec());
        }

        if let Some(d) = &self.result.deltas {
            let mut decoder = Decoder::new(d.as_slice().clone());
            let block_deltas: &mut Vec<TableDelta> = &mut vec![];
            decoder.unpack(block_deltas);
            self.block_deltas = Some(block_deltas.to_vec());
        }
    }

    pub fn to_evm(&self) {
        if self.signed_block.is_none() || self.block_traces.is_none() || self.block_deltas.is_none() {
            panic!("Block::to_evm called on a block with missing data");
        }

        let traces = self.block_traces.clone().unwrap();
        //let transactions: Vec<> = vec![];

        for t in traces {
            match t {
                TransactionTrace::V0(t) => {
                    for action in t.action_traces {
                        match action {
                            ActionTrace::V0(a) => {
                                if a.receiver.n == EOSIO_EVM {
                                    println!("EVM action trace");
                                }
                                if a.receiver.n == EOSIO_TOKEN {
                                    println!("Token action trace");
                                }
                            }
                            ActionTrace::V1(a) => {
                                if a.receiver.n == EOSIO_EVM  && a.act.name.n == RAW {
                                    let raw = decode_raw(&a.act.data);

                                    let printed_receipt = &mut PrintedReceipt::from_console(a.console);
                                    println!("EVM action trace");
                                }
                                if a.receiver.n == EOSIO_TOKEN {
                                    println!("Token action trace");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Ord for Block {
    fn cmp(&self, other: &Self) -> Ordering {
        self.block_num.cmp(&other.block_num)
    }
}

impl PartialOrd for Block {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Block {
    fn eq(&self, other: &Self) -> bool {
        self.block_num == other.block_num
    }
}

impl Eq for Block {}