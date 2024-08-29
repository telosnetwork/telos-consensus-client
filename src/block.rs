use crate::transaction::TelosEVMTransaction;
use crate::types::env::{ANTELOPE_EPOCH_MS, ANTELOPE_INTERVAL_MS};
use crate::types::evm_types::{
    AccountRow, AccountStateRow, CreateAction, EOSConfigRow, OpenWalletAction, PrintedReceipt,
    RawAction, SetRevisionAction, TransferAction, WithdrawAction,
};
use crate::types::names::*;
use crate::types::ship_types::{
    ActionTrace, ContractRow, GetBlocksResultV0, SignedBlock, TableDelta, TransactionTrace,
};
use crate::types::translator_types::NameToAddressCache;
use alloy::primitives::{Bloom, Bytes, FixedBytes, B256, U256};
use alloy_consensus::constants::{EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH};
use alloy_consensus::{Eip658Value, Header, Receipt, ReceiptWithBloom, TxEnvelope};
use alloy_rlp::Encodable;
use antelope::chain::checksum::Checksum256;
use antelope::chain::name::Name;
use antelope::chain::Decoder;
use reth_trie_common::root::ordered_trie_root_with_encoder;
use std::cmp::Ordering;
use tracing::warn;

pub trait BasicTrace {
    fn action_name(&self) -> u64;
    fn action_account(&self) -> u64;
    fn receiver(&self) -> u64;
    fn console(&self) -> String;
    fn data(&self) -> Vec<u8>;
}

#[derive(Clone)]
pub enum WalletEvents {
    OpenWallet(usize, OpenWalletAction),
    CreateWallet(usize, CreateAction),
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
pub enum DecodedRow {
    Config(EOSConfigRow),
    Account(AccountRow),
    AccountState(AccountStateRow),
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
    pub decoded_rows: Vec<DecodedRow>,
    pub transactions: Vec<TelosEVMTransaction>,
    pub new_gas_price: Option<U256>,
    pub new_revision: Option<u32>,
    pub new_wallets: Vec<WalletEvents>,
}

#[derive(Clone)]
pub struct TelosEVMBlock {
    pub header: Header,
    pub block_num: u32,
    pub block_hash: B256,
    pub transactions: Vec<TelosEVMTransaction>,

    pub new_gas_price: Option<U256>,
    pub new_revision: Option<u32>,
    pub new_wallets: Vec<WalletEvents>,
    pub account_rows: Vec<AccountRow>,
    pub account_state_rows: Vec<AccountStateRow>,
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

pub fn decode_setrevision(raw: &[u8]) -> SetRevisionAction {
    let mut decoder = Decoder::new(raw);
    let setrevision = &mut SetRevisionAction::default();
    decoder.unpack(setrevision);
    setrevision.clone()
}

pub fn decode_openwallet(raw: &[u8]) -> OpenWalletAction {
    let mut decoder = Decoder::new(raw);
    let openwallet = &mut OpenWalletAction::default();
    decoder.unpack(openwallet);
    openwallet.clone()
}

pub fn decode_create(raw: &[u8]) -> CreateAction {
    let mut decoder = Decoder::new(raw);
    let create = &mut CreateAction::default();
    decoder.unpack(create);
    create.clone()
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
            decoded_rows: vec![],
            transactions: vec![],

            new_gas_price: None,
            new_revision: None,
            new_wallets: vec![],
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
            let transaction_result = TelosEVMTransaction::from_raw_action(
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
            let transaction = TelosEVMTransaction::from_withdraw(
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

            let transaction = TelosEVMTransaction::from_transfer(
                self.chain_id,
                self.transactions.len(),
                self.block_hash,
                transfer_action,
                native_to_evm_cache,
            )
            .await;
            self.transactions.push(transaction.clone());
        } else if action_account == EOSIO_EVM && action_name == DORESOURCES {
            let config_delta_row = self
                .decoded_rows
                .iter()
                .find_map(|row| {
                    if let DecodedRow::Config(config) = row {
                        Some(config)
                    } else {
                        None
                    }
                })
                .expect("Table delta for the doresources action not found");

            let gas_price = U256::from_be_slice(&config_delta_row.gas_price.data);

            self.new_gas_price = Some(gas_price);
        } else if action_account == EOSIO_EVM && action_name == SETREVISION {
            let rev_action = decode_setrevision(&action.data());

            self.new_revision = Some(rev_action.new_revision);
        } else if action_account == EOSIO_EVM && action_name == OPENWALLET {
            let wallet_action = decode_openwallet(&action.data());

            self.new_wallets.push(WalletEvents::OpenWallet(
                self.transactions.len(),
                wallet_action,
            ));
        } else if action_account == EOSIO_EVM && action_name == CREATE {
            let wallet_action = decode_create(&action.data());
            self.new_wallets.push(WalletEvents::CreateWallet(
                self.transactions.len(),
                wallet_action,
            ));
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

        let row_deltas = self.contract_rows.clone().unwrap_or_default();

        for r in row_deltas {
            match r {
                ContractRow::V0(r) => {
                    // Global eosio.system table, since block_delta is static
                    // no need to decode
                    // if r.table == Name::new_from_str("global") {
                    //     let mut decoder = Decoder::new(r.value.as_slice());
                    //     let decoded_row = &mut GlobalTable::default();
                    //     decoder.unpack(decoded_row);
                    //     info!("Global table: {:?}", decoded_row);
                    // }
                    if r.code == Name::new_from_str("eosio.evm") {
                        if r.table == Name::new_from_str("config") {
                            let mut decoder = Decoder::new(r.value.as_slice());
                            let mut decoded_row = EOSConfigRow::default();
                            decoder.unpack(&mut decoded_row);
                            self.decoded_rows.push(DecodedRow::Config(decoded_row));
                        } else if r.table == Name::new_from_str("account") {
                            let mut decoder = Decoder::new(r.value.as_slice());
                            let mut decoded_row = AccountRow::default();
                            decoder.unpack(&mut decoded_row);
                            self.decoded_rows.push(DecodedRow::Account(decoded_row));
                        } else if r.table == Name::new_from_str("accountstate") {
                            let mut decoder = Decoder::new(r.value.as_slice());
                            let mut decoded_row = AccountStateRow::default();
                            decoder.unpack(&mut decoded_row);
                            self.decoded_rows
                                .push(DecodedRow::AccountState(decoded_row));
                        }
                    }
                }
            }
        }

        let traces = self.block_traces.clone().unwrap_or_default();

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

        let mut cumulative_gas_used = 0;
        let mut receipts: Vec<ReceiptWithBloom> = vec![];
        for transaction in &self.transactions {
            let tx_gas_used = u128::from_str_radix(&transaction.receipt.gasused, 16).unwrap();
            let logs = transaction.receipt.logs.clone();
            let mut logs_bloom = Bloom::default();
            for log in &logs {
                logs_bloom.accrue_log(log);
            }
            cumulative_gas_used += tx_gas_used;
            let full_receipt = ReceiptWithBloom {
                receipt: Receipt {
                    status: Eip658Value::Eip658(true),
                    cumulative_gas_used,
                    logs,
                },
                logs_bloom,
            };
            receipts.push(full_receipt);
        }
        let tx_root_hash =
            ordered_trie_root_with_encoder(&self.transactions, |tx, buf| match &tx.envelope {
                TxEnvelope::Legacy(_stx) => tx.envelope.encode(buf),
                envelope => {
                    buf.push(u8::from(envelope.tx_type()));

                    if envelope.is_eip1559() {
                        let stx = envelope.as_eip1559().unwrap();
                        stx.tx().encode_with_signature_fields(stx.signature(), buf);
                    } else {
                        panic!("unimplemented tx type");
                    }
                }
            });
        let receipts_root_hash =
            ordered_trie_root_with_encoder(&receipts, |r: &ReceiptWithBloom, buf| r.encode(buf));
        let mut logs_bloom = Bloom::default();
        for receipt in &receipts {
            logs_bloom.accrue_bloom(&receipt.logs_bloom);
        }

        Header {
            parent_hash,
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary: Default::default(),
            state_root: EMPTY_ROOT_HASH,
            transactions_root: tx_root_hash,
            receipts_root: receipts_root_hash,
            withdrawals_root: None,
            logs_bloom,
            difficulty: Default::default(),
            number: (self.block_num - block_delta) as u64,
            gas_limit: 0x7fffffff,
            gas_used: cumulative_gas_used,
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
