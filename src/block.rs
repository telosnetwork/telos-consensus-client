use crate::transaction::TelosEVMTransaction;
use crate::types::env::{ANTELOPE_EPOCH_MS, ANTELOPE_INTERVAL_MS};
use crate::types::evm_types::{
    AccountRow, AccountStateRow, CreateAction, EvmContractConfigRow, OpenWalletAction,
    PrintedReceipt, RawAction, SetRevisionAction, TransferAction, WithdrawAction,
};
use crate::types::names::*;
use crate::types::ship_types::{
    ActionTrace, ContractRow, GetBlocksResultV0, SignedBlock, TableDelta, TransactionTrace,
};
use crate::types::translator_types::NameToAddressCache;
use alloy::primitives::{Bloom, Bytes, FixedBytes, B256, U256};
use alloy_consensus::constants::{EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH};
use alloy_consensus::{Eip658Value, Header, Receipt, ReceiptWithBloom, TxEnvelope};
use alloy_rlp::{encode, Encodable};
use antelope::chain::checksum::Checksum256;
use antelope::chain::name::Name;
use antelope::serializer::Packer;
use reth_rpc_types::ExecutionPayloadV1;
use reth_trie_common::root::ordered_trie_root_with_encoder;
use std::cmp::Ordering;
use tracing::warn;

const MINIMUM_FEE_PER_GAS: u128 = 7;

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
    Config(EvmContractConfigRow),
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
    pub new_gas_price: Option<(usize, U256)>,
    pub new_revision: Option<(usize, u32)>,
    pub new_wallets: Vec<WalletEvents>,
    pub lib_num: u32,
    pub lib_hash: Checksum256,
}

#[derive(Clone)]
pub struct TelosEVMBlock {
    pub header: Header,
    pub block_num: u32,
    pub block_hash: B256,
    pub lib_num: u32,
    pub lib_hash: B256,
    pub transactions: Vec<TelosEVMTransaction>,

    pub new_gas_price: Option<(usize, U256)>,
    pub new_revision: Option<(usize, u32)>,
    pub new_wallets: Vec<WalletEvents>,
    pub account_rows: Vec<AccountRow>,
    pub account_state_rows: Vec<AccountStateRow>,
}

impl From<&TelosEVMBlock> for ExecutionPayloadV1 {
    fn from(block: &TelosEVMBlock) -> Self {
        let base_fee_per_gas = block
            .header
            .base_fee_per_gas
            .filter(|&fee| fee > MINIMUM_FEE_PER_GAS)
            .unwrap_or(MINIMUM_FEE_PER_GAS);

        let transactions = block
            .transactions
            .iter()
            .map(|transaction| Bytes::from(encode(&transaction.envelope)))
            .collect::<Vec<_>>();

        ExecutionPayloadV1 {
            parent_hash: block.header.parent_hash,
            fee_recipient: block.header.beneficiary,
            state_root: block.header.state_root,
            receipts_root: block.header.receipts_root,
            logs_bloom: block.header.logs_bloom,
            prev_randao: B256::ZERO,
            block_number: block.block_num as u64,
            gas_limit: block.header.gas_limit as u64,
            gas_used: block.header.gas_used as u64,
            timestamp: block.header.timestamp,
            extra_data: block.header.extra_data.clone(),
            base_fee_per_gas: U256::from(base_fee_per_gas),
            block_hash: block.block_hash,
            transactions,
        }
    }
}

pub fn decode<T: Packer + Default>(raw: &[u8]) -> T {
    let mut result = T::default();
    result.unpack(raw);
    result
}

impl ProcessingEVMBlock {
    pub fn new(
        chain_id: u64,
        block_num: u32,
        block_hash: Checksum256,
        lib_num: u32,
        lib_hash: Checksum256,
        result: GetBlocksResultV0,
    ) -> Self {
        Self {
            block_num,
            block_hash,
            lib_num,
            lib_hash,
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
        self.signed_block = self.result.block.as_deref().map(decode);

        if self.result.traces.is_none() {
            warn!("No block traces found for block: {}", self.block_num);
        }

        self.block_traces = self.result.traces.as_deref().map(decode).or(Some(vec![]));

        if self.result.deltas.is_none() {
            warn!("No deltas found for block: {}", self.block_num);
        };

        // TODO: Handle present: false here?  How to account for empty/deleted rows?
        self.contract_rows = self.result.deltas.as_deref().map(|deltas| {
            decode::<Vec<TableDelta>>(deltas)
                .iter()
                .filter(|TableDelta::V0(delta)| delta.name == "contract_row")
                .map(|TableDelta::V0(delta)| delta.rows.as_slice())
                .flat_map(|rows| rows.iter().map(|row| row.data.as_slice()).map(decode))
                .collect::<Vec<ContractRow>>()
        });
    }

    fn find_config_row(&self) -> Option<&EvmContractConfigRow> {
        return self.decoded_rows.iter().find_map(|row| {
            if let DecodedRow::Config(config) = row {
                Some(config)
            } else {
                None
            }
        });
    }

    async fn handle_action(
        &mut self,
        action: Box<dyn BasicTrace + Send>,
        native_to_evm_cache: &NameToAddressCache,
    ) {
        let action_name = action.action_name();
        let action_account = action.action_account();
        let action_receiver = action.receiver();

        if action_account == EOSIO_EVM && action_name == INIT {
            let config_delta_row = self
                .find_config_row()
                .expect("Table delta for the init action not found");

            let gas_price = U256::from_be_slice(&config_delta_row.gas_price.data);

            self.new_gas_price = Some((self.transactions.len(), gas_price));
        } else if action_account == EOSIO_EVM && action_name == RAW {
            // Normally signed EVM transaction
            let raw: RawAction = decode(&action.data());
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
            let withdraw_action: WithdrawAction = decode(&action.data());
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
            let transfer_action: TransferAction = decode(&action.data());
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
                .find_config_row()
                .expect("Table delta for the doresources action not found");

            let gas_price = U256::from_be_slice(&config_delta_row.gas_price.data);

            self.new_gas_price = Some((self.transactions.len(), gas_price));
        } else if action_account == EOSIO_EVM && action_name == SETREVISION {
            let rev_action: SetRevisionAction = decode(&action.data());

            self.new_revision = Some((self.transactions.len(), rev_action.new_revision));
        } else if action_account == EOSIO_EVM && action_name == OPENWALLET {
            let wallet_action: OpenWalletAction = decode(&action.data());

            self.new_wallets.push(WalletEvents::OpenWallet(
                self.transactions.len(),
                wallet_action,
            ));
        } else if action_account == EOSIO_EVM && action_name == CREATE {
            let wallet_action: CreateAction = decode(&action.data());
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
                            self.decoded_rows.push(DecodedRow::Config(decode(&r.value)));
                        } else if r.table == Name::new_from_str("account") {
                            self.decoded_rows
                                .push(DecodedRow::Account(decode(&r.value)));
                        } else if r.table == Name::new_from_str("accountstate") {
                            self.decoded_rows
                                .push(DecodedRow::AccountState(decode(&r.value)));
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
