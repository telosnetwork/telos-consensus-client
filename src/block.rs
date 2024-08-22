use crate::transaction::Transaction;
use crate::types::env::{ANTELOPE_EPOCH_MS, ANTELOPE_INTERVAL_MS};
use crate::types::evm_types::{PrintedReceipt, RawAction, TransferAction, WithdrawAction};
use crate::types::names::*;
use crate::types::ship_types::{
    ActionTrace, ContractRow, GetBlocksResultV0, SignedBlock, TableDelta, TransactionTrace,
};
use crate::types::translator_types::NameToAddressCache;
use alloy::primitives::{Bytes, FixedBytes, B256};
use alloy_consensus::constants::{EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH};
use alloy_consensus::Header;
use antelope::chain::checksum::Checksum256;
use antelope::chain::Decoder;
use std::cmp::Ordering;
use tracing::warn;

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
pub struct ProcessingEVMBlock {
    pub block_num: u32,
    block_hash: Checksum256,
    chain_id: u64,
    result: GetBlocksResultV0,
    signed_block: Option<SignedBlock>,
    block_traces: Option<Vec<TransactionTrace>>,
    contract_rows: Option<Vec<ContractRow>>,
    pub transactions: Vec<Transaction>,
}

#[derive(Clone)]
pub struct TelosEVMBlock {
    pub header: Header,
    pub block_num: u32,
    pub block_hash: B256,
    pub transactions: Vec<Transaction>,
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

pub fn decode_withdraw(raw: &[u8]) -> WithdrawAction {
    let mut decoder = Decoder::new(raw);
    let withdraw = &mut WithdrawAction::default();
    decoder.unpack(withdraw);
    withdraw.clone()
}

impl ProcessingEVMBlock {
    pub fn new(
        chain_id: u64,
        block_num: u32,
        block_hash: Checksum256,
        result: GetBlocksResultV0,
    ) -> Self {
        Self {
            block_num,
            block_hash,
            chain_id,
            result,
            signed_block: None,
            block_traces: None,
            contract_rows: None,
            transactions: vec![],
        }
    }

    pub fn deserialize(&mut self) {
        if let Some(b) = &self.result.block {
            let mut decoder = Decoder::new(b.as_slice());
            let signed_block = &mut SignedBlock::default();
            decoder.unpack(signed_block);
            self.signed_block = Some(signed_block.clone());
        }

        if let Some(t) = &self.result.traces {
            let mut decoder = Decoder::new(t.as_slice());
            let block_traces: &mut Vec<TransactionTrace> = &mut vec![];
            decoder.unpack(block_traces);
            self.block_traces = Some(block_traces.to_vec());
        } else {
            self.block_traces = Some(vec![]);
            warn!("No block traces found for block: {}", self.block_num);
        }

        if let Some(d) = &self.result.deltas {
            let mut decoder = Decoder::new(d.as_slice());
            let block_deltas: &mut Vec<TableDelta> = &mut vec![];
            let contract_rows: &mut Vec<ContractRow> = &mut vec![];
            decoder.unpack(block_deltas);
            for delta in block_deltas.iter_mut() {
                match delta {
                    TableDelta::V0(d) => {
                        if d.name == "contract_row" {
                            for row in d.rows.iter_mut() {
                                // TODO: Handle present: false here?  How to account for empty/deleted rows?
                                let mut row_decoder = Decoder::new(row.data.as_slice());
                                let contract_row = &mut ContractRow::default();
                                row_decoder.unpack(contract_row);
                                contract_rows.push(contract_row.clone());
                            }
                        }
                    }
                }
            }
            self.contract_rows = Some(contract_rows.to_vec());
        } else {
            self.contract_rows = Some(vec![]);
            warn!("No deltas found for block: {}", self.block_num);
        }
    }

    async fn handle_action(
        &mut self,
        action: Box<dyn BasicTrace + Send>,
        native_to_evm_cache: &NameToAddressCache,
    ) {
        let action_name = action.action_name();
        let action_account = action.action_account();
        let action_receiver = action.receiver();

        if action_account == EOSIO_EVM && action_name == RAW {
            // Normally signed EVM transaction
            let raw = decode_raw(&action.data());
            let printed_receipt = PrintedReceipt::from_console(action.console());
            if printed_receipt.is_none() {
                panic!(
                    "No printed receipt found for raw action in block: {}",
                    self.block_num
                );
            }
            let transaction_result = Transaction::from_raw_action(
                self.chain_id,
                self.transactions.len(),
                self.block_hash,
                raw,
                printed_receipt.unwrap(),
            )
            .await;

            match transaction_result {
                Ok(transaction) => {
                    self.transactions.push(transaction);
                }
                Err(e) => {
                    panic!("Error handling action. Error: {}", e);
                }
            }
        } else if action_account == EOSIO_EVM && action_name == WITHDRAW {
            // Withdrawal from EVM
            let withdraw_action = decode_withdraw(&action.data());
            let transaction = Transaction::from_withdraw(
                self.chain_id,
                self.transactions.len(),
                self.block_hash,
                withdraw_action,
                native_to_evm_cache,
            )
            .await;
            self.transactions.push(transaction);
        } else if action_account == EOSIO_TOKEN
            && action_name == TRANSFER
            && action_receiver == EOSIO_EVM
        {
            // Deposit/transfer to EVM
            let transfer_action = decode_transfer(&action.data());
            if transfer_action.to.n != EOSIO_EVM
                || SYSTEM_ACCOUNTS.contains(&transfer_action.from.n)
            {
                return;
            }

            let transaction = Transaction::from_transfer(
                self.chain_id,
                self.transactions.len(),
                self.block_hash,
                transfer_action,
                native_to_evm_cache,
            )
            .await;
            self.transactions.push(transaction.clone());
        } else if action_account == EOSIO_EVM && action_name == DORESOURCES {
            // TODO: Handle doresources action
        }
    }

    pub async fn generate_evm_data(
        &mut self,
        parent_hash: FixedBytes<32>,
        block_delta: u32,
        native_to_evm_cache: &NameToAddressCache,
    ) -> Header {
        if self.signed_block.is_none()
            || self.block_traces.is_none()
            || self.contract_rows.is_none()
        {
            panic!("Block::to_evm called on a block with missing data");
        }

        let traces = self.block_traces.clone().unwrap();

        for t in traces {
            match t {
                TransactionTrace::V0(t) => {
                    for action in t.action_traces {
                        self.handle_action(Box::new(action), native_to_evm_cache)
                            .await;
                    }
                }
            }
        }

        // let row_deltas = self.contract_rows.clone().unwrap();

        // TODO: Decode contract rows better, only decode what we need
        //   this is getting the global table to attempt to determine the block_delta value for this devnet chain
        // for r in row_deltas {
        //     match r {
        //         ContractRow::V0(r) => {
        //             if r.table == Name::new_from_str("global") {
        //                 let mut decoder = Decoder::new(r.value.as_slice());
        //                 let decoded_row = &mut GlobalTable::default();
        //                 decoder.unpack(decoded_row);
        //                 info!("Global table: {:?}", decoded_row);
        //             }
        //         }
        //     }
        // }

        // let mut bloom = Bloom::default();
        // for trx in &self.transactions {
        //     for log in trx.logs() {
        //         //bloom.accrue(log);
        //     }
        //     //bloom.accrue(&trx.bloom());
        // }

        Header {
            parent_hash,
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary: Default::default(),
            state_root: EMPTY_ROOT_HASH,
            transactions_root: EMPTY_ROOT_HASH,
            receipts_root: EMPTY_ROOT_HASH,
            withdrawals_root: None,
            logs_bloom: Default::default(),
            difficulty: Default::default(),
            number: (self.block_num - block_delta) as u64,
            gas_limit: 0x7fffffff,
            gas_used: 0,
            timestamp: (((self.signed_block.clone().unwrap().header.header.timestamp as u64)
                * ANTELOPE_INTERVAL_MS)
                + ANTELOPE_EPOCH_MS)
                / 1000,
            mix_hash: Default::default(),
            nonce: Default::default(),
            base_fee_per_gas: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_root: None,
            extra_data: Bytes::from(self.block_hash.data),
        }
    }
}

impl Ord for ProcessingEVMBlock {
    fn cmp(&self, other: &Self) -> Ordering {
        self.block_num.cmp(&other.block_num)
    }
}

impl PartialOrd for ProcessingEVMBlock {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ProcessingEVMBlock {
    fn eq(&self, other: &Self) -> bool {
        self.block_num == other.block_num
    }
}

impl Eq for ProcessingEVMBlock {}
