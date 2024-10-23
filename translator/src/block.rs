use crate::transaction::TelosEVMTransaction;
use crate::types::env::{ANTELOPE_EPOCH_MS, ANTELOPE_INTERVAL_MS, DEFAULT_GAS_LIMIT};
use crate::types::evm_types::{
    AccountRow, AccountStateRow, CreateAction, EvmContractConfigRow, OpenWalletAction,
    PrintedReceipt, RawAction, SetRevisionAction, TransferAction, WithdrawAction,
};
use crate::types::names::*;
use crate::types::ship_types::{
    ActionTrace, ContractRow, GetBlocksResultV0, SignedBlock, TableDelta, TransactionTrace,
};
use crate::types::translator_types::{ChainId, NameToAddressCache};
use alloy::primitives::{Bloom, Bytes, FixedBytes, B256, U256};
use alloy_consensus::constants::{EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH};
use alloy_consensus::{Header, Transaction, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;
use alloy_rlp::Encodable;
use alloy_rpc_types_engine::ExecutionPayloadV1;
use antelope::chain::checksum::Checksum256;
use antelope::chain::name::Name;
use antelope::serializer::Packer;
use eyre::eyre;
use reth_primitives::ReceiptWithBloom;
use reth_telos_rpc_engine_api::structs::TelosEngineAPIExtraFields;
use reth_trie_common::root::ordered_trie_root_with_encoder;
use std::cmp::{max, Ordering};
use tracing::{debug, warn};

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
            ActionTrace::V0(a) => String::from_utf8(a.console.clone()).unwrap_or_default(),
            ActionTrace::V1(a) => String::from_utf8(a.console.clone()).unwrap_or_default(),
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
    Account(bool, AccountRow),
    AccountState(bool, AccountStateRow, Name),
}

#[derive(Clone)]
pub struct ProcessingEVMBlock {
    pub block_num: u32,
    pub block_hash: Checksum256,
    pub prev_block_hash: Option<Checksum256>,
    chain_id: u64,
    result: GetBlocksResultV0,
    signed_block: Option<SignedBlock>,
    block_traces: Option<Vec<TransactionTrace>>,
    contract_rows: Option<Vec<(bool, ContractRow)>>,
    cumulative_gas_used: u64,
    dyn_gas_limit: Option<u128>,
    pub decoded_rows: Vec<DecodedRow>,
    pub transactions: Vec<(TelosEVMTransaction, ReceiptWithBloom)>,
    pub new_gas_price: Option<(u64, U256)>,
    pub new_revision: Option<(u64, u64)>,
    pub new_wallets: Vec<WalletEvents>,
    pub lib_num: u32,
    pub lib_hash: Checksum256,
    pub skip_raw_action: bool,
}

#[derive(Clone, Debug)]
pub struct TelosEVMBlock {
    pub block_num: u32,
    pub block_hash: B256,
    pub ship_hash: String,
    pub lib_num: u32,
    pub lib_hash: String,
    pub header: Header,
    pub transactions: Vec<(TelosEVMTransaction, ReceiptWithBloom)>,
    pub execution_payload: ExecutionPayloadV1,
    pub extra_fields: TelosEngineAPIExtraFields,
}

impl TelosEVMBlock {
    pub fn lib_evm_num(&self, chain_id: &ChainId) -> u32 {
        self.lib_num.saturating_sub(chain_id.block_delta())
    }

    pub fn block_num_with_delta(&self, chain_id: &ChainId) -> u32 {
        self.block_num + chain_id.block_delta()
    }

    pub fn is_final(&self, chain_id: &ChainId) -> bool {
        self.block_num_with_delta(chain_id) <= self.lib_num
    }

    pub fn is_lib(&self, chain_id: &ChainId) -> bool {
        self.block_num_with_delta(chain_id) == self.lib_num
    }
}

pub fn decode_raw_action(encoded: &[u8]) -> RawAction {
    decode::<RawAction>(encoded)
}

pub fn decode<T: Packer + Default>(raw: &[u8]) -> T {
    let mut result = T::default();
    result.unpack(raw);
    result
}

pub struct ProcessingEVMBlockArgs {
    pub chain_id: u64,
    pub block_num: u32,
    pub block_hash: Checksum256,
    pub prev_block_hash: Option<Checksum256>,
    pub lib_num: u32,
    pub lib_hash: Checksum256,
    pub result: GetBlocksResultV0,
    pub skip_raw_action: bool,
}

impl ProcessingEVMBlock {
    pub fn new(args: ProcessingEVMBlockArgs) -> Self {
        let ProcessingEVMBlockArgs {
            chain_id,
            block_num,
            block_hash,
            prev_block_hash,
            lib_num,
            lib_hash,
            result,
            skip_raw_action,
        } = args;

        Self {
            block_num,
            block_hash,
            prev_block_hash,
            lib_num,
            lib_hash,
            chain_id,
            result,
            skip_raw_action,
            signed_block: None,
            block_traces: None,
            contract_rows: None,
            cumulative_gas_used: 0,
            dyn_gas_limit: None,
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
                .filter_map(|TableDelta::V0(delta)| {
                    if delta.name == "contract_row" {
                        Some(
                            delta
                                .rows
                                .iter()
                                .map(|row| {
                                    let contract_row = decode::<ContractRow>(row.data.as_slice());
                                    (row.present, contract_row) // row.present becomes the first item in the tuple
                                })
                                .collect::<Vec<(bool, ContractRow)>>(),
                        )
                    } else {
                        None
                    }
                })
                .flatten()
                .collect::<Vec<(bool, ContractRow)>>()
        });
    }

    fn find_config_row(&self) -> Option<&EvmContractConfigRow> {
        self.decoded_rows.iter().find_map(|row| {
            if let DecodedRow::Config(config) = row {
                Some(config)
            } else {
                None
            }
        })
    }

    fn add_transaction(&mut self, transaction: TelosEVMTransaction) {
        let full_receipt = transaction.receipt(self.cumulative_gas_used);
        let gas_limit = transaction.envelope.gas_limit() + self.cumulative_gas_used as u128;
        self.cumulative_gas_used = full_receipt.receipt.cumulative_gas_used;
        self.transactions.push((transaction, full_receipt));

        if self.dyn_gas_limit.is_none() {
            self.dyn_gas_limit = Some(gas_limit);
        } else if gas_limit > self.dyn_gas_limit.unwrap() {
            self.dyn_gas_limit = Some(gas_limit)
        }
    }

    async fn handle_action(
        &mut self,
        action: Box<dyn BasicTrace + Send>,
        native_to_evm_cache: &NameToAddressCache,
    ) -> eyre::Result<()> {
        let action_name = action.action_name();
        let action_account = action.action_account();
        let action_receiver = action.receiver();

        if action_account == EOSIO_EVM && action_name == INIT && !self.skip_raw_action {
            let config_delta_row = self
                .find_config_row()
                .expect("Table delta for the init action not found");

            let gas_price = U256::from_be_slice(&config_delta_row.gas_price.data);

            self.new_gas_price = Some((self.transactions.len() as u64, gas_price));
        } else if action_account == EOSIO_EVM && action_name == RAW {
            if self.skip_raw_action {
                return Ok(());
            }
            // Normally signed EVM transaction
            let raw: RawAction = decode_raw_action(&action.data());
            let printed_receipt =
                PrintedReceipt::from_console(action.console()).ok_or_else(|| {
                    eyre::eyre!(
                        "No printed receipt found for raw action in block: {}",
                        self.block_num
                    )
                })?;

            let transaction = TelosEVMTransaction::from_raw_action(
                self.chain_id,
                self.transactions.len(),
                self.block_hash,
                raw,
                printed_receipt,
            )
            .await?;

            self.add_transaction(transaction);
            return Ok(());
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
            .await?;
            self.add_transaction(transaction);
        } else if action_account == EOSIO_TOKEN
            && action_name == TRANSFER
            && action_receiver == EOSIO_TOKEN
        {
            // Deposit/transfer to EVM
            let transfer_action: TransferAction = decode(&action.data());
            if transfer_action.to.n != EOSIO_EVM
                || SYSTEM_ACCOUNTS.contains(&transfer_action.from.n)
            {
                return Ok(());
            }

            let transaction = TelosEVMTransaction::from_transfer(
                self.chain_id,
                self.transactions.len(),
                self.block_hash,
                transfer_action,
                native_to_evm_cache,
            )
            .await?;
            self.add_transaction(transaction);
        } else if action_account == EOSIO_EVM && action_name == DORESOURCES {
            let config_delta_row = self
                .find_config_row()
                .expect("Table delta for the doresources action not found");

            let gas_price = U256::from_be_slice(&config_delta_row.gas_price.data);

            self.new_gas_price = Some((self.transactions.len() as u64, gas_price));
        } else if action_account == EOSIO_EVM && action_name == SETREVISION {
            let rev_action: SetRevisionAction = decode(&action.data());

            self.new_revision = Some((
                self.transactions.len() as u64,
                rev_action.new_revision as u64,
            ));
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
        Ok(())
    }

    pub async fn generate_evm_data(
        &mut self,
        parent_hash: FixedBytes<32>,
        block_delta: u32,
        native_to_evm_cache: &NameToAddressCache,
    ) -> eyre::Result<(Header, ExecutionPayloadV1)> {
        if self.signed_block.is_none()
            || self.block_traces.is_none()
            || self.contract_rows.is_none()
        {
            panic!("Block::to_evm called on a block with missing data");
        }

        let row_deltas = self.contract_rows.clone().unwrap_or_default();

        for delta in row_deltas {
            match delta.1 {
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
                        // delta.0 is "present" and if false, the row was removed
                        let removed = !delta.0;
                        if r.table == Name::new_from_str("config") && !self.skip_raw_action {
                            if removed {
                                panic!(
                                    "Config row removed, this should never happen: {}",
                                    self.block_num
                                );
                            }
                            self.decoded_rows.push(DecodedRow::Config(decode(&r.value)));
                        } else if r.table == Name::new_from_str("account") {
                            self.decoded_rows
                                .push(DecodedRow::Account(removed, decode(&r.value)));
                        } else if r.table == Name::new_from_str("accountstate") {
                            self.decoded_rows.push(DecodedRow::AccountState(
                                removed,
                                decode(&r.value),
                                r.scope,
                            ));
                        }
                    }
                }
            }
        }

        let traces = self.block_traces.clone().unwrap_or_default();

        for TransactionTrace::V0(t) in traces {
            for action in t.action_traces {
                if let Err(e) = self
                    .handle_action(Box::new(action), native_to_evm_cache)
                    .await
                {
                    return Err(eyre!("Error handling the action. {}", e));
                }
            }
        }

        let tx_root_hash =
            ordered_trie_root_with_encoder(&self.transactions, |(tx, _receipt), buf| {
                match &tx.envelope {
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
                }
            });
        let receipts_root_hash =
            ordered_trie_root_with_encoder(&self.transactions, |(_trx, r), buf| r.encode(buf));
        let mut logs_bloom = Bloom::default();
        for (_trx, receipt) in &self.transactions {
            logs_bloom.accrue_bloom(&receipt.bloom);
        }

        let gas_limit = if let Some(dyn_gas) = self.dyn_gas_limit {
            debug!("Dynamic gas limit: {}", dyn_gas);
            max(DEFAULT_GAS_LIMIT, dyn_gas)
        } else {
            DEFAULT_GAS_LIMIT
        };

        let header = Header {
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
            gas_limit,
            gas_used: self.cumulative_gas_used as u128,
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
        };

        let base_fee_per_gas = U256::from(
            header
                .base_fee_per_gas
                .filter(|&fee| fee > MINIMUM_FEE_PER_GAS)
                .unwrap_or(MINIMUM_FEE_PER_GAS),
        );

        let transactions = self
            .transactions
            .iter()
            .map(|(transaction, _receipt)| {
                let mut encoded = vec![];
                transaction.envelope.encode_2718(&mut encoded);
                Bytes::from(encoded)
            })
            .collect::<Vec<_>>();

        let exec_payload = ExecutionPayloadV1 {
            parent_hash,
            fee_recipient: Default::default(),
            state_root: EMPTY_ROOT_HASH,
            receipts_root: receipts_root_hash,
            logs_bloom,
            prev_randao: B256::ZERO,
            block_number: header.number,
            gas_limit: header.gas_limit as u64,
            gas_used: header.gas_used as u64,
            timestamp: header.timestamp,
            extra_data: header.extra_data.clone(),
            base_fee_per_gas,
            block_hash: header.hash_slow(),
            transactions,
        };

        Ok((header, exec_payload))
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
