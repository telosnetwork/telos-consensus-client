use std::cmp::Ordering;
use alloy::primitives::Address;
use alloy_consensus::Header;
use antelope::chain::Decoder;
use antelope::chain::name::Name;
use antelope::name;
use moka::sync::Cache;
use tracing::info;
use crate::transaction::Transaction;
use crate::types::evm_types::{RawAction, PrintedReceipt, TransferAction};
use crate::types::ship_types::{ActionTrace, GetBlocksResultV0, SignedBlock, TableDelta, TransactionTrace};

const EOSIO: u64 = 6138663577826885632u64;
const EOSIO_STAKE: u64 = 6138663591134630912u64;
const EOSIO_RAM: u64 = 6138663590285017088u64;
const EOSIO_EVM: u64 = 6138663583658016768u64;
const EOSIO_TOKEN: u64 = 6138663591592764928u64;
const RAW: u64 = 13382446292731428864u64;
const WITHDRAW: u64 = 16407410437513019392u64;
const TRANSFER: u64 = 14829575313431724032u64;
const DORESOURCES: u64 = 5561572063795392512u64;
const SYSTEM_ACCOUNTS: [u64; 3] = [EOSIO, EOSIO_STAKE, EOSIO_RAM];

pub trait BasicTrace {
    fn action_name(&self) -> u64;
    fn action_account(&self) -> u64;
    fn receiver(&self) -> u64;
    fn console(&self) -> String;
    fn data(&self) -> Vec<u8>;
}

impl BasicTrace for ActionTrace {
    fn action_name(&self) -> u64 {
        match self {
            ActionTrace::V0(a) => a.act.name.n,
            ActionTrace::V1(a) => a.act.name.n,
        }
    }

    fn action_account(&self) -> u64 {
        match self {
            ActionTrace::V0(a) => a.act.account.n,
            ActionTrace::V1(a) => a.act.account.n,
        }
    }

    fn receiver(&self) -> u64 {
        match self {
            ActionTrace::V0(a) => a.receiver.n,
            ActionTrace::V1(a) => a.receiver.n,
        }
    }

    fn console(&self) -> String {
        match self {
            ActionTrace::V0(a) => a.console.clone(),
            ActionTrace::V1(a) => a.console.clone(),
        }
    }

    fn data(&self) -> Vec<u8> {
        match self {
            ActionTrace::V0(a) => a.act.data.clone(),
            ActionTrace::V1(a) => a.act.data.clone(),
        }
    }
}

#[derive(Clone)]
pub struct Block {
    pub block_num: u32,
    result: GetBlocksResultV0,
    signed_block: Option<SignedBlock>,
    block_traces: Option<Vec<TransactionTrace>>,
    block_deltas: Option<Vec<TableDelta>>,
    transactions: Vec<Transaction>,
}

pub fn decode_raw(raw: &[u8]) -> RawAction {
    let mut decoder = Decoder::new(raw);
    let raw = &mut RawAction::default();
    decoder.unpack(raw);
    raw.clone()
}

pub fn decode_transfer(raw: &[u8]) -> TransferAction {
    let mut decoder = Decoder::new(raw);
    let transfer = &mut TransferAction::default();
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
            transactions: vec![],
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

    fn handle_action(&mut self, action: Box<dyn BasicTrace>) {
        let action_name = action.action_name();
        let action_account = action.action_account();
        let action_receiver = action.receiver();

        if action_account == EOSIO_EVM && action_name == RAW {
            // Normally signed EVM transaction
            let raw = decode_raw(&action.data());
            let printed_receipt = PrintedReceipt::from_console(action.console());
            if printed_receipt.is_none() {
                panic!("No printed receipt found for raw action in block: {}", self.block_num);
            }
            let transaction = Transaction::from_raw_action(raw, printed_receipt.unwrap());
            self.transactions.push(transaction);
        } else if action_account == EOSIO_EVM && action_name == WITHDRAW {
            // Withdrawal from EVM
            // let transaction = Transaction::from_withdraw(action);
            // self.transactions.push(transaction);
        } else if action_account == EOSIO_TOKEN && action_name == TRANSFER && action_receiver == EOSIO_EVM {
            // Deposit/transfer to EVM
            let transfer = decode_transfer(&action.data());
            if SYSTEM_ACCOUNTS.contains(&transfer.from.n) {
                return;
            }

            // let transaction = Transaction::from_transfer(transfer);
            // self.transactions.push(transaction);
        } else if action_account == EOSIO_EVM && action_name == DORESOURCES {
            // TODO: Handle doresources action
        }
    }

    pub fn generate_evm_data(&mut self, native_to_evm_cache: Cache<Name, Address>) {
        if self.signed_block.is_none() || self.block_traces.is_none() || self.block_deltas.is_none() {
            panic!("Block::to_evm called on a block with missing data");
        }

        let traces = self.block_traces.clone().unwrap();

        for t in traces {
            match t {
                TransactionTrace::V0(t) => {
                    for action in t.action_traces {
                        self.handle_action(Box::new(action));
                    }
                }
            }
        }

        let block_header = Header {
            parent_hash: Default::default(),
            ommers_hash: Default::default(),
            beneficiary: Default::default(),
            state_root: Default::default(),
            transactions_root: Default::default(),
            receipts_root: Default::default(),
            withdrawals_root: None,
            logs_bloom: Default::default(),
            difficulty: Default::default(),
            number: 0,
            gas_limit: 0,
            gas_used: 0,
            timestamp: 0,
            mix_hash: Default::default(),
            nonce: Default::default(),
            base_fee_per_gas: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_root: None,
            extra_data: Default::default(),
        };
        info!("Block hash: {:?}", block_header.hash_slow());

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