use crate::block::{DecodedRow, TelosEVMBlock, WalletEvents};
use crate::types::env::TESTNET_DEPLOY_STATE;
use crate::types::execution_metadata::{
    ExecutionBranchEntry, TelosEngineAPIExtraFields, TelosExecutionContext,
    TelosExecutionMetadataV3,
};
use crate::types::translator_types::{generate_extra_fields_from_json, ChainId};
use crate::{
    block::ProcessingEVMBlock, translator::TranslatorConfig,
    types::translator_types::NameToAddressCache,
};
use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use alloy_rlp::Encodable;
use antelope::api::client::{APIClient, DefaultProvider};
use eyre::{eyre, Context, Result};
use hex::encode;
use reth_primitives::B256;
use reth_telos_rpc_engine_api::structs::{TelosAccountStateTableRow, TelosAccountTableRow};
use std::collections::HashMap;
use std::str::FromStr;
use tokio::{sync::mpsc, time::Instant};
use tracing::{debug, error, info};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockParent {
    native_block_number: u32,
    evm_hash: B256,
    child_context: TelosExecutionContext,
}

struct BlockMap {
    active_tip: Option<(String, BlockParent)>,
    map: HashMap<String, BlockParent>,
}

impl BlockMap {
    fn new(
        parent_hash: B256,
        starting_context: TelosExecutionContext,
        parent_native_hash: String,
        parent_native_block: u32,
        entries: Vec<ExecutionBranchEntry>,
    ) -> Result<Self> {
        let mut block_map = Self {
            active_tip: None,
            map: HashMap::new(),
        };
        for entry in entries {
            block_map.insert(
                entry.native_hash,
                BlockParent {
                    native_block_number: entry.native_block_number,
                    evm_hash: entry.evm_hash,
                    child_context: entry.child_context,
                },
            )?;
        }
        let parent = BlockParent {
            native_block_number: parent_native_block,
            evm_hash: parent_hash,
            child_context: starting_context,
        };
        block_map.insert(parent_native_hash.clone(), parent)?;
        block_map.active_tip = Some((parent_native_hash, parent));
        Ok(block_map)
    }

    fn parent(&self, block: &ProcessingEVMBlock) -> Result<BlockParent> {
        let native_parent_hash = block
            .prev_block_hash
            .map(|hash| hash.as_string())
            .ok_or_else(|| eyre!("native block {} has no parent hash", block.block_num))?;
        self.parent_for(block.block_num, &native_parent_hash)
    }

    fn parent_for(
        &self,
        native_block_number: u32,
        native_parent_hash: &str,
    ) -> Result<BlockParent> {
        let parent = self
            .active_tip
            .as_ref()
            .filter(|(hash, _)| hash == native_parent_hash)
            .map(|(_, parent)| *parent)
            .or_else(|| self.map.get(native_parent_hash).copied());

        if let Some(parent) = parent {
            let expected_block = parent.native_block_number.checked_add(1).ok_or_else(|| {
                eyre!("native parent block number overflow for {native_parent_hash}")
            })?;
            if expected_block != native_block_number {
                return Err(eyre!(
                    "native parent {native_parent_hash} is block {}, not the parent height of block {}",
                    parent.native_block_number,
                    native_block_number
                ));
            }
            return Ok(parent);
        }

        Err(eyre!(
            "execution parent and context for native parent {native_parent_hash} of block {} were not retained",
            native_block_number
        ))
    }

    fn next(
        &mut self,
        block: &TelosEVMBlock,
        chain_id: &ChainId,
        child_context: TelosExecutionContext,
    ) -> Result<()> {
        let native_block_number = block.block_num_with_delta(chain_id);
        let parent = BlockParent {
            native_block_number,
            evm_hash: block.block_hash,
            child_context,
        };
        self.insert(block.ship_hash.clone(), parent)?;
        self.active_tip = Some((block.ship_hash.clone(), parent));
        self.prune(block.lib_num, &block.lib_hash);
        Ok(())
    }

    fn insert(&mut self, native_hash: String, parent: BlockParent) -> Result<()> {
        if let Some(existing) = self.map.get(&native_hash) {
            if existing != &parent {
                return Err(eyre!(
                    "native block {native_hash} maps to conflicting execution branches"
                ));
            }
            return Ok(());
        }
        self.map.insert(native_hash, parent);
        Ok(())
    }

    fn prune(&mut self, irreversible_block: u32, irreversible_hash: &str) {
        let size_before = self.map.len();
        self.map.retain(|native_hash, parent| {
            parent.native_block_number > irreversible_block
                || (parent.native_block_number == irreversible_block
                    && native_hash == irreversible_hash)
        });
        debug!(
            "Removed {} irreversible branch entries from the map",
            size_before - self.map.len()
        );
    }
}

pub async fn final_processor(
    config: TranslatorConfig,
    api_client: APIClient<DefaultProvider>,
    mut rx: mpsc::Receiver<ProcessingEVMBlock>,
    tx: Option<mpsc::Sender<TelosEVMBlock>>,
    shutdown_tx: mpsc::Sender<()>,
) -> Result<()> {
    let mut last_log = Instant::now();
    let mut unlogged_blocks = 0;
    let mut unlogged_transactions = 0;
    let block_delta = config.chain_id.block_delta();

    let config_parent_hash = FixedBytes::from_str(&config.prev_hash)
        .wrap_err("Prev hash config is not a valid 32 byte hex string")?;
    let starting_context = TelosExecutionContext {
        gas_price: U256::from_str(&config.execution_context_starting_gas_price)
            .wrap_err("Execution context starting gas price is not a valid U256")?,
        revision: config.execution_context_starting_revision,
    };

    let validate_hash = match config.validate_hash.as_deref() {
        Some(hash) => Some(
            FixedBytes::from_str(hash)
                .wrap_err("Validate hash config is not a valid 32 byte hex string")?,
        ),
        None => None,
    };

    let mut validated = validate_hash.is_none();

    let native_to_evm_cache = NameToAddressCache::new(api_client);
    let stop_block = config
        .evm_stop_block
        .map(|block| {
            block
                .checked_add(block_delta)
                .ok_or_else(|| eyre!("Native stop block overflows u32"))
        })
        .transpose()?
        .unwrap_or(u32::MAX);

    let (parent_native_hash, parent_native_block) = config.effective_native_parent()?;
    let mut block_map = BlockMap::new(
        config_parent_hash,
        starting_context,
        parent_native_hash.to_string(),
        parent_native_block,
        config.execution_branch_entries.clone(),
    )?;

    while let Some(mut block) = rx.recv().await {
        let block_num = block.block_num;
        if block_num > stop_block {
            break;
        }
        debug!("Finalizing block #{block_num}");

        let parent = block_map.parent(&block)?;
        let parent_hash = parent.evm_hash;
        let block_starting_context = parent.child_context;

        let (header, exec_payload) = block
            .generate_evm_data(parent_hash, block_delta, &native_to_evm_cache)
            .await?;

        let block_hash = exec_payload.block_hash;

        debug!("Translator header: {:#?}", header);

        unlogged_blocks += 1;
        unlogged_transactions += block.transactions.len();

        let mut out = Vec::<u8>::new();
        header.encode(&mut out);
        debug!("Encoded header: 0x{}", hex::encode(out));
        debug!("Hash of header: {:?}", block_hash);

        if !validated {
            if let Some(validate_hash) = validate_hash {
                validated = validate_hash == block_hash;
                if !validated {
                    error!(
                        "Initial hash validation failed!, expected: \"{validate_hash}\" got: \"{block_hash}\"",
                    );
                    error!("Header: {:#?}", header);
                    return Err(eyre!("Initial hash validation failed!"));
                }
            }
        }

        if last_log.elapsed().as_secs_f64() > 1.0 {
            let blocks_sec = unlogged_blocks as f64 / last_log.elapsed().as_secs_f64();
            let trx_sec = unlogged_transactions as f64 / last_log.elapsed().as_secs_f64();
            info!(
                "Block #{} 0x{} - processed {:.1} blocks/sec and {:.1} tx/sec",
                block.block_num,
                encode(block_hash),
                blocks_sec,
                trx_sec
            );

            unlogged_blocks = 0;
            unlogged_transactions = 0;
            last_log = Instant::now();
        }

        let evm_block_num = header.number as u32;

        let mut statediffs_account = vec![];
        let mut statediffs_accountstate = vec![];

        let mut new_addresses_using_create = vec![];
        let mut new_addresses_using_openwallet = vec![];

        let mut receipts = Some(vec![]);

        let mut completed_block = TelosEVMBlock {
            block_num: evm_block_num,
            block_hash,
            ship_block_num: block.block_num,
            ship_hash: block.block_hash.as_string(),
            ship_parent_hash: block.prev_block_hash.map(|hash| hash.as_string()),
            lib_num: block.lib_num,
            lib_hash: block.lib_hash.as_string(),
            transactions: vec![],
            header,
            execution_payload: exec_payload,
            extra_fields: TelosEngineAPIExtraFields {
                statediffs_account: Some(vec![]),
                statediffs_accountstate: Some(vec![]),
                revision_changes: None,
                gasprice_changes: None,
                execution: None,
                new_addresses_using_create: Some(vec![]),
                new_addresses_using_openwallet: Some(vec![]),
                receipts,
            },
        };

        if evm_block_num > config.evm_deploy_block.unwrap_or_default() {
            for row in block.decoded_rows {
                match row {
                    DecodedRow::Account(removed, acc_diff) => {
                        statediffs_account.push(TelosAccountTableRow {
                            removed,
                            address: Address::from_slice(&acc_diff.address.data),
                            account: acc_diff.account.to_string(),
                            nonce: acc_diff.nonce,
                            code: Bytes::from(acc_diff.code.clone()),
                            balance: U256::from_be_slice(&acc_diff.balance.data),
                        })
                    }
                    DecodedRow::AccountState(removed, acc_state_diff, scope) => {
                        statediffs_accountstate.push(TelosAccountStateTableRow {
                            removed,
                            address: native_to_evm_cache.get_index(scope.n).await?,
                            key: U256::from_be_slice(&acc_state_diff.key.data),
                            value: U256::from_be_slice(&acc_state_diff.value.data),
                        });
                    }
                    _ => (),
                }
            }

            for new_wallet in block.new_wallets {
                match new_wallet {
                    WalletEvents::CreateWallet(trx_index, create_action) => {
                        new_addresses_using_create.push((
                            trx_index as u64,
                            U256::from_be_slice(
                                native_to_evm_cache
                                    .get(create_action.account.value())
                                    .await?
                                    .as_slice(),
                            ),
                        ))
                    }
                    WalletEvents::OpenWallet(trx_index, openwallet_action) => {
                        new_addresses_using_openwallet.push((
                            trx_index as u64,
                            U256::from_be_slice(&openwallet_action.address.data),
                        ))
                    }
                }
            }

            receipts = Some(
                block
                    .transactions
                    .iter()
                    .map(|(_trx, full_receipt)| full_receipt.receipt.clone())
                    .collect(),
            );

            completed_block.transactions = block.transactions.clone();

            completed_block.extra_fields = TelosEngineAPIExtraFields {
                statediffs_account: Some(statediffs_account),
                statediffs_accountstate: Some(statediffs_accountstate),
                revision_changes: None,
                gasprice_changes: None,
                execution: None,
                new_addresses_using_create: Some(new_addresses_using_create),
                new_addresses_using_openwallet: Some(new_addresses_using_openwallet),
                receipts,
            };
        } else if config
            .evm_deploy_block
            .is_some_and(|deploy_block| evm_block_num == deploy_block)
        {
            let (state_dump_block, state_dump_gas_price, extra_fields) =
                generate_extra_fields_from_json(TESTNET_DEPLOY_STATE);
            if state_dump_block != evm_block_num {
                return Err(eyre!(
                    "state dump block {state_dump_block} does not match configured deploy block {evm_block_num}"
                ));
            }
            completed_block.extra_fields = extra_fields;
            block.new_gas_prices.push((0, state_dump_gas_price));
        };

        let execution = TelosExecutionMetadataV3::new(
            completed_block.block_hash,
            completed_block.header.parent_hash,
            completed_block.execution_payload.transactions.len(),
            completed_block.execution_payload.base_fee_per_gas,
            block_starting_context,
            block.new_gas_prices,
            block.new_revisions,
        )?;
        let child_context = execution.child_context();
        completed_block.extra_fields.execution = Some(execution);

        block_map.next(&completed_block, &config.chain_id, child_context)?;

        let block_num = block.block_num;
        if let Some(tx) = tx.clone() {
            tx.send(completed_block)
                .await
                .map_err(|error| eyre!("finished-block consumer stopped: {error}"))?;
        }

        if block_num == stop_block {
            debug!("Processed stop block #{block_num}, exiting...");
            shutdown_tx
                .send(())
                .await
                .map_err(|_| eyre!("Can't send stop message"))?;
            break;
        }
    }
    while rx.recv().await.is_some() {}
    info!("Exiting final processor...");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(gas_price: u64, revision: u64) -> TelosExecutionContext {
        TelosExecutionContext {
            gas_price: U256::from(gas_price),
            revision,
        }
    }

    fn branch(
        native_block_number: u32,
        native_hash: &str,
        native_parent_hash: &str,
        evm_block_number: u32,
        evm_hash_byte: u8,
        child_context: TelosExecutionContext,
    ) -> ExecutionBranchEntry {
        ExecutionBranchEntry {
            native_block_number,
            native_hash: native_hash.to_string(),
            native_parent_hash: Some(native_parent_hash.to_string()),
            evm_block_number,
            evm_hash: B256::repeat_byte(evm_hash_byte),
            execution_base_fee: U256::from(7),
            child_context,
        }
    }

    #[test]
    fn forked_children_use_their_exact_native_parent() {
        let branch_a = branch(101, "a101", "root", 65, 0xa1, context(11, 1));
        let branch_b = branch(101, "b101", "root", 65, 0xb1, context(22, 2));
        let map = BlockMap::new(
            B256::ZERO,
            context(0, 0),
            "root".to_string(),
            100,
            vec![branch_a.clone(), branch_b.clone()],
        )
        .unwrap();

        assert_eq!(
            map.parent_for(102, "a101").unwrap().evm_hash,
            branch_a.evm_hash
        );
        assert_eq!(
            map.parent_for(102, "b101").unwrap().child_context,
            context(22, 2)
        );
    }

    #[test]
    fn restart_rebuilds_canonical_and_side_branch_parents() {
        let canonical_context = context(30, 3);
        let side = branch(200, "side200", "common199", 164, 0x55, context(40, 4));
        let map = BlockMap::new(
            B256::repeat_byte(0x44),
            canonical_context,
            "canonical200".to_string(),
            200,
            vec![side.clone()],
        )
        .unwrap();

        let canonical = map.parent_for(201, "canonical200").unwrap();
        assert_eq!(canonical.evm_hash, B256::repeat_byte(0x44));
        assert_eq!(canonical.child_context, canonical_context);
        assert_eq!(
            map.parent_for(201, "side200").unwrap().evm_hash,
            side.evm_hash
        );
    }

    #[test]
    fn fresh_bootstrap_accepts_only_the_configured_native_parent() {
        let map = BlockMap::new(
            B256::repeat_byte(0x44),
            context(30, 3),
            "native-anchor".to_string(),
            200,
            vec![],
        )
        .unwrap();

        assert_eq!(
            map.parent_for(201, "native-anchor").unwrap().evm_hash,
            B256::repeat_byte(0x44)
        );
        assert!(map.parent_for(201, "untrusted-parent").is_err());
    }

    #[test]
    fn deep_reorg_uses_retained_branch_context_at_each_parent() {
        let entries = vec![
            branch(301, "a301", "root300", 265, 0xa1, context(1, 1)),
            branch(302, "a302", "a301", 266, 0xa2, context(2, 2)),
            branch(301, "b301", "root300", 265, 0xb1, context(3, 3)),
            branch(302, "b302", "b301", 266, 0xb2, context(4, 4)),
        ];
        let map = BlockMap::new(
            B256::ZERO,
            context(0, 0),
            "root300".to_string(),
            300,
            entries,
        )
        .unwrap();

        assert_eq!(
            map.parent_for(303, "a302").unwrap().child_context,
            context(2, 2)
        );
        assert_eq!(
            map.parent_for(303, "b302").unwrap().child_context,
            context(4, 4)
        );
    }

    #[test]
    fn pruning_keeps_only_exact_lib_at_or_below_lib_and_all_newer_forks() {
        let mut map = BlockMap::new(
            B256::ZERO,
            context(0, 0),
            "older".to_string(),
            398,
            vec![
                branch(399, "old", "older", 363, 1, context(1, 0)),
                branch(400, "lib", "old", 364, 2, context(2, 0)),
                branch(400, "side-at-lib", "old", 364, 3, context(3, 0)),
                branch(401, "new-a", "lib", 365, 4, context(4, 0)),
                branch(401, "new-b", "lib", 365, 5, context(5, 0)),
            ],
        )
        .unwrap();

        map.prune(400, "lib");

        assert!(!map.map.contains_key("old"));
        assert!(map.map.contains_key("lib"));
        assert!(!map.map.contains_key("side-at-lib"));
        assert!(map.map.contains_key("new-a"));
        assert!(map.map.contains_key("new-b"));
    }
}
