use crate::transaction::TelosEVMTransaction;
use crate::types::env::{ANTELOPE_EPOCH_MS, ANTELOPE_INTERVAL_MS, DEFAULT_GAS_LIMIT};
use crate::types::evm_types::{
    AccountRow, AccountStateRow, CreateAction, EvmContractConfigRow, OpenWalletAction,
    PrintedReceipt, RawAction, SetRevisionAction, TransferAction, WithdrawAction,
};
use crate::types::execution_metadata::TelosEngineAPIExtraFields;
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
use eyre::{eyre, Context};
use reth_primitives::ReceiptWithBloom;
use reth_trie_common::root::ordered_trie_root_with_encoder;
use std::any::type_name;
use std::cmp::{max, Ordering};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use tracing::debug;

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
    pub new_gas_prices: Vec<(u64, U256)>,
    pub new_revisions: Vec<(u64, u64)>,
    pub new_wallets: Vec<WalletEvents>,
    pub lib_num: u32,
    pub lib_hash: Checksum256,
    pub skip_events: bool,
}

#[derive(Clone, Debug)]
pub struct TelosEVMBlock {
    pub block_num: u32,
    pub block_hash: B256,
    pub ship_block_num: u32,
    pub ship_hash: String,
    pub ship_parent_hash: Option<String>,
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
        let expected = self.block_num.saturating_add(chain_id.block_delta());
        debug_assert_eq!(self.ship_block_num, expected);
        self.ship_block_num
    }

    pub fn is_final(&self, chain_id: &ChainId) -> bool {
        self.block_num_with_delta(chain_id) <= self.lib_num
    }

    pub fn is_lib(&self, chain_id: &ChainId) -> bool {
        self.block_num_with_delta(chain_id) == self.lib_num
    }
}

pub fn decode_raw_action(encoded: &[u8]) -> eyre::Result<RawAction> {
    decode::<RawAction>(encoded)
}

/// Decodes Antelope binary data while containing panics from the dependency's infallible
/// `Packer::unpack` API. Malformed SHIP input must stop translation, not unwind the process.
pub fn decode<T: Packer + Default>(raw: &[u8]) -> eyre::Result<T> {
    let mut result = T::default();
    let consumed = catch_unwind(AssertUnwindSafe(|| result.unpack(raw))).map_err(|_| {
        eyre!(
            "malformed Antelope binary data while decoding {}",
            type_name::<T>()
        )
    })?;
    if consumed > raw.len() {
        return Err(eyre!(
            "Antelope decoder for {} consumed {consumed} bytes from a {}-byte input",
            type_name::<T>(),
            raw.len()
        ));
    }
    Ok(result)
}

fn encode_transaction_for_trie(transaction: &TelosEVMTransaction) -> eyre::Result<Vec<u8>> {
    let mut encoded = Vec::new();
    match &transaction.envelope {
        TxEnvelope::Legacy(_) => transaction.envelope.encode(&mut encoded),
        TxEnvelope::Eip1559(signed) => {
            encoded.push(u8::from(transaction.envelope.tx_type()));
            signed
                .tx()
                .encode_with_signature_fields(signed.signature(), &mut encoded);
        }
        envelope => {
            return Err(eyre!(
                "Telos transaction type {:?} is unsupported in the transaction trie",
                envelope.tx_type()
            ));
        }
    }
    Ok(encoded)
}

pub struct ProcessingEVMBlockArgs {
    pub chain_id: u64,
    pub block_num: u32,
    pub block_hash: Checksum256,
    pub prev_block_hash: Option<Checksum256>,
    pub lib_num: u32,
    pub lib_hash: Checksum256,
    pub result: GetBlocksResultV0,
    pub skip_events: bool,
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
            skip_events,
        } = args;

        Self {
            block_num,
            block_hash,
            prev_block_hash,
            lib_num,
            lib_hash,
            chain_id,
            result,
            skip_events,
            signed_block: None,
            block_traces: None,
            contract_rows: None,
            cumulative_gas_used: 0,
            dyn_gas_limit: None,
            decoded_rows: vec![],
            transactions: vec![],

            new_gas_prices: vec![],
            new_revisions: vec![],
            new_wallets: vec![],
        }
    }

    pub fn deserialize(&mut self) -> eyre::Result<()> {
        let signed_block = self
            .result
            .block
            .as_deref()
            .filter(|payload| !payload.is_empty())
            .ok_or_else(|| eyre!("SHIP block {} is missing signed block data", self.block_num))?;
        let traces = self
            .result
            .traces
            .as_deref()
            .filter(|payload| !payload.is_empty())
            .ok_or_else(|| eyre!("SHIP block {} is missing trace data", self.block_num))?;
        let deltas = self
            .result
            .deltas
            .as_deref()
            .filter(|payload| !payload.is_empty())
            .ok_or_else(|| eyre!("SHIP block {} is missing delta data", self.block_num))?;

        self.signed_block = Some(decode(signed_block).wrap_err_with(|| {
            format!("failed to decode signed native block {}", self.block_num)
        })?);
        self.block_traces = Some(decode(traces).wrap_err_with(|| {
            format!(
                "failed to decode native traces for block {}",
                self.block_num
            )
        })?);

        let table_deltas = decode::<Vec<TableDelta>>(deltas).wrap_err_with(|| {
            format!(
                "failed to decode native deltas for block {}",
                self.block_num
            )
        })?;
        let mut contract_rows = Vec::new();
        for TableDelta::V0(delta) in table_deltas {
            if delta.name != "contract_row" {
                continue;
            }
            for row in delta.rows {
                let contract_row = decode::<ContractRow>(&row.data).wrap_err_with(|| {
                    format!(
                        "failed to decode contract-row delta for block {}",
                        self.block_num
                    )
                })?;
                contract_rows.push((row.present, contract_row));
            }
        }
        self.contract_rows = Some(contract_rows);
        Ok(())
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

    fn add_transaction(&mut self, transaction: TelosEVMTransaction) -> eyre::Result<()> {
        let full_receipt = transaction
            .receipt(self.cumulative_gas_used)
            .wrap_err_with(|| format!("invalid receipt in native block {}", self.block_num))?;
        let gas_limit = transaction
            .envelope
            .gas_limit()
            .checked_add(u128::from(self.cumulative_gas_used))
            .ok_or_else(|| {
                eyre!(
                    "dynamic gas limit overflows u128 in block {}",
                    self.block_num
                )
            })?;
        self.cumulative_gas_used = full_receipt.receipt.cumulative_gas_used;
        self.transactions.push((transaction, full_receipt));

        self.dyn_gas_limit = Some(
            self.dyn_gas_limit
                .map_or(gas_limit, |current| current.max(gas_limit)),
        );
        Ok(())
    }

    async fn handle_action(
        &mut self,
        action: Box<dyn BasicTrace + Send>,
        native_to_evm_cache: &NameToAddressCache,
    ) -> eyre::Result<()> {
        let action_name = action.action_name();
        let action_account = action.action_account();
        let action_receiver = action.receiver();

        if action_account == EOSIO_EVM && action_name == INIT {
            let config_delta_row = self.find_config_row().ok_or_else(|| {
                eyre!(
                    "init action in block {} has no EVM config table delta",
                    self.block_num
                )
            })?;

            let gas_price = U256::from_be_slice(&config_delta_row.gas_price.data);

            self.new_gas_prices
                .push((self.transactions.len() as u64, gas_price));
        } else if action_account == EOSIO_EVM && action_name == RAW {
            // Normally signed EVM transaction
            let raw = decode_raw_action(&action.data()).wrap_err_with(|| {
                format!(
                    "failed to decode raw action in native block {}",
                    self.block_num
                )
            })?;
            let console = action.console();
            let printed_receipt = PrintedReceipt::from_console(&console)
                .wrap_err_with(|| {
                    format!(
                        "malformed printed receipt in native block {}",
                        self.block_num
                    )
                })?
                .ok_or_else(|| {
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

            self.add_transaction(transaction)?;
            return Ok(());
        } else if action_account == EOSIO_EVM && action_name == WITHDRAW {
            // Withdrawal from EVM
            let withdraw_action: WithdrawAction = decode(&action.data()).wrap_err_with(|| {
                format!(
                    "failed to decode withdraw action in native block {}",
                    self.block_num
                )
            })?;
            let transaction = TelosEVMTransaction::from_withdraw(
                self.chain_id,
                self.transactions.len(),
                self.block_hash,
                withdraw_action,
                native_to_evm_cache,
            )
            .await?;
            self.add_transaction(transaction)?;
        } else if action_account == EOSIO_TOKEN
            && action_name == TRANSFER
            && action_receiver == EOSIO_TOKEN
        {
            // Deposit/transfer to EVM
            let transfer_action: TransferAction = decode(&action.data()).wrap_err_with(|| {
                format!(
                    "failed to decode transfer action in native block {}",
                    self.block_num
                )
            })?;
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
            self.add_transaction(transaction)?;
        } else if action_account == EOSIO_EVM && action_name == DORESOURCES {
            let config_delta_row = self.find_config_row().ok_or_else(|| {
                eyre!(
                    "doresources action in block {} has no EVM config table delta",
                    self.block_num
                )
            })?;

            let gas_price = U256::from_be_slice(&config_delta_row.gas_price.data);

            self.new_gas_prices
                .push((self.transactions.len() as u64, gas_price));
        } else if action_account == EOSIO_EVM && action_name == SETREVISION {
            let rev_action: SetRevisionAction = decode(&action.data()).wrap_err_with(|| {
                format!(
                    "failed to decode setrevision action in native block {}",
                    self.block_num
                )
            })?;

            self.new_revisions.push((
                self.transactions.len() as u64,
                rev_action.new_revision as u64,
            ));
        } else if action_account == EOSIO_EVM && action_name == OPENWALLET {
            let wallet_action: OpenWalletAction = decode(&action.data()).wrap_err_with(|| {
                format!(
                    "failed to decode openwallet action in native block {}",
                    self.block_num
                )
            })?;

            self.new_wallets.push(WalletEvents::OpenWallet(
                self.transactions.len(),
                wallet_action,
            ));
        } else if action_account == EOSIO_EVM && action_name == CREATE {
            let wallet_action: CreateAction = decode(&action.data()).wrap_err_with(|| {
                format!(
                    "failed to decode create action in native block {}",
                    self.block_num
                )
            })?;
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
        let signed_block = self.signed_block.clone().ok_or_else(|| {
            eyre!(
                "Cannot generate EVM block {} without signed block data",
                self.block_num
            )
        })?;
        let traces = self.block_traces.clone().ok_or_else(|| {
            eyre!(
                "Cannot generate EVM block {} without trace data",
                self.block_num
            )
        })?;
        let row_deltas = self.contract_rows.clone().ok_or_else(|| {
            eyre!(
                "Cannot generate EVM block {} without delta data",
                self.block_num
            )
        })?;

        let mut deduped_accstate_deltas = HashMap::new();

        if !self.skip_events {
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
                            if r.table == Name::new_from_str("config") {
                                if removed {
                                    return Err(eyre!(
                                        "EVM config row was removed in native block {}",
                                        self.block_num
                                    ));
                                }
                                self.decoded_rows.push(DecodedRow::Config(
                                    decode(&r.value).wrap_err_with(|| {
                                        format!(
                                            "failed to decode EVM config row in native block {}",
                                            self.block_num
                                        )
                                    })?,
                                ));
                            } else if r.table == Name::new_from_str("account") {
                                self.decoded_rows.push(DecodedRow::Account(
                                    removed,
                                    decode(&r.value).wrap_err_with(|| {
                                        format!(
                                            "failed to decode EVM account row in native block {}",
                                            self.block_num
                                        )
                                    })?,
                                ));
                            } else if r.table == Name::new_from_str("accountstate") {
                                let decoded_row: AccountStateRow =
                                    decode(&r.value).wrap_err_with(|| {
                                        format!(
                                            "failed to decode EVM account-state row in native block {}",
                                            self.block_num
                                        )
                                    })?;
                                let complex_key = (r.scope.n, decoded_row.key.data);
                                match deduped_accstate_deltas.get(&complex_key) {
                                    Some(DecodedRow::AccountState(_, prev_row, _)) => {
                                        if prev_row.index < decoded_row.index {
                                            deduped_accstate_deltas.insert(
                                                complex_key,
                                                DecodedRow::AccountState(
                                                    removed,
                                                    decoded_row,
                                                    r.scope,
                                                ),
                                            );
                                        }
                                    }
                                    Some(_) => {
                                        return Err(eyre!(
                                            "account-state delta map contains an invalid row kind in native block {}",
                                            self.block_num
                                        ));
                                    }
                                    None => {
                                        deduped_accstate_deltas.insert(
                                            complex_key,
                                            DecodedRow::AccountState(removed, decoded_row, r.scope),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            for row in deduped_accstate_deltas.values() {
                self.decoded_rows.push(row.clone());
            }

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

            // This is an exception for the wrong deployment of EVM contract in the testnet on native block #276210867 which caused revision become zero
            if self.chain_id == 41 && self.block_num == 276210867 {
                self.new_revisions.push((0, 0));
            }
        }

        let trie_transactions = self
            .transactions
            .iter()
            .map(|(transaction, _)| encode_transaction_for_trie(transaction))
            .collect::<eyre::Result<Vec<_>>>()?;
        let tx_root_hash = ordered_trie_root_with_encoder(&trie_transactions, |encoded, buf| {
            buf.extend_from_slice(encoded)
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

        let evm_block_number = self.block_num.checked_sub(block_delta).ok_or_else(|| {
            eyre!(
                "Native block {} is before EVM block delta {block_delta}",
                self.block_num
            )
        })?;
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
            number: u64::from(evm_block_number),
            gas_limit,
            gas_used: self.cumulative_gas_used as u128,
            timestamp: (((signed_block.header.header.timestamp as u64) * ANTELOPE_INTERVAL_MS)
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
        let payload_gas_limit = u64::try_from(header.gas_limit).map_err(|_| {
            eyre!(
                "gas limit {} exceeds Engine API u64 in native block {}",
                header.gas_limit,
                self.block_num
            )
        })?;

        let exec_payload = ExecutionPayloadV1 {
            parent_hash,
            fee_recipient: Default::default(),
            state_root: EMPTY_ROOT_HASH,
            receipts_root: receipts_root_hash,
            logs_bloom,
            prev_randao: B256::ZERO,
            block_number: header.number,
            gas_limit: payload_gas_limit,
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

#[cfg(test)]
mod tests {
    use super::*;
    use antelope::api::client::APIClient;

    struct InitTrace;

    impl BasicTrace for InitTrace {
        fn action_name(&self) -> u64 {
            INIT
        }

        fn action_account(&self) -> u64 {
            EOSIO_EVM
        }

        fn receiver(&self) -> u64 {
            EOSIO_EVM
        }

        fn console(&self) -> String {
            String::new()
        }

        fn data(&self) -> Vec<u8> {
            Vec::new()
        }
    }

    fn empty_block() -> ProcessingEVMBlock {
        ProcessingEVMBlock::new(ProcessingEVMBlockArgs {
            chain_id: 40,
            block_num: 100,
            block_hash: Checksum256::default(),
            prev_block_hash: Some(Checksum256::default()),
            lib_num: 99,
            lib_hash: Checksum256::default(),
            result: GetBlocksResultV0::default(),
            skip_events: false,
        })
    }

    #[test]
    fn malformed_antelope_binary_is_a_decode_error() {
        assert!(decode::<RawAction>(&[]).is_err());
    }

    #[tokio::test]
    async fn init_without_config_delta_is_an_error() {
        let mut block = empty_block();
        let cache = NameToAddressCache::new(APIClient::default());
        let error = block
            .handle_action(Box::new(InitTrace), &cache)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no EVM config table delta"));
    }
}
