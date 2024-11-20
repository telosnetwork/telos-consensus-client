use crate::block::{DecodedRow, TelosEVMBlock, WalletEvents};
use crate::types::translator_types::{ChainId, generate_extra_fields_from_json};
use crate::{
    block::ProcessingEVMBlock, translator::TranslatorConfig,
    types::translator_types::NameToAddressCache,
};
use alloy::primitives::{Address, Bytes, FixedBytes, U256};
use alloy_rlp::Encodable;
use antelope::api::client::{APIClient, DefaultProvider};
use eyre::{eyre, Context, Result};
use hex::encode;
use reth_primitives::B256;
use reth_telos_rpc_engine_api::structs::{
    TelosAccountStateTableRow, TelosAccountTableRow, TelosEngineAPIExtraFields,
};
use std::collections::HashMap;
use std::str::FromStr;
use tokio::{sync::mpsc, time::Instant};
use tracing::{debug, error, info};
use crate::types::env::TESTNET_DEPLOY_STATE;

struct BlockMap {
    parent_hash: B256,
    prev_block_num: Option<u32>,
    map: HashMap<String, (u32, B256)>,
}

impl BlockMap {
    fn new(parent_hash: B256) -> Self {
        Self {
            parent_hash,
            prev_block_num: None,
            map: HashMap::new(),
        }
    }

    fn parent_hash(&self, block: &ProcessingEVMBlock) -> Option<B256> {
        let Some(prev_block_num) = self.prev_block_num else {
            return Some(self.parent_hash);
        };

        let block_num = block.block_num;
        if block_num == prev_block_num + 1 {
            return Some(self.parent_hash);
        }

        debug!("Fork detected for block_num: {block_num}, prev_block_num = {prev_block_num}");
        block
            .prev_block_hash
            .map(|hash| hash.as_string())
            .as_ref()
            .and_then(|hash| self.map.get(hash).cloned())
            .map(|(_, hash)| hash)
    }

    fn next(&mut self, block: &TelosEVMBlock, chain_id: &ChainId) {
        let block_num = block.block_num_with_delta(chain_id);

        self.prev_block_num = Some(block_num);
        self.map
            .insert(block.ship_hash.clone(), (block_num, block.block_hash));
        self.parent_hash = block.block_hash;
        let size_before = self.map.len();
        self.map.retain(|_, &mut (num, _)| num >= block.lib_num);
        debug!(
            "Removed {} final blocks from the map",
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

    let validate_hash = match config.validate_hash {
        Some(hash) => Some(
            FixedBytes::from_str(&hash)
                .wrap_err("Validate hash config is not a valid 32 byte hex string")?,
        ),
        None => None,
    };

    let mut validated = validate_hash.is_none();

    let native_to_evm_cache = NameToAddressCache::new(api_client);
    let stop_block = &config
        .evm_stop_block
        .map(|n| n + block_delta)
        .unwrap_or(u32::MAX);

    let mut block_map = BlockMap::new(config_parent_hash);

    while let Some(mut block) = rx.recv().await {
        let block_num = block.block_num;
        if &block_num > stop_block {
            break;
        }
        debug!("Finalizing block #{block_num}");

        let parent_hash = block_map
            .parent_hash(&block)
            .expect("Block parent hash can be found");

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
            ship_hash: block.block_hash.as_string(),
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
                    WalletEvents::CreateWallet(trx_index, create_action) => new_addresses_using_create
                        .push((
                            trx_index as u64,
                            U256::from_be_slice(
                                native_to_evm_cache
                                    .get(create_action.account.value())
                                    .await?
                                    .as_slice(),
                            ),
                        )),
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
                revision_changes: block.new_revision,
                gasprice_changes: block.new_gas_price,
                new_addresses_using_create: Some(new_addresses_using_create),
                new_addresses_using_openwallet: Some(new_addresses_using_openwallet),
                receipts,
            };
        } else if config.evm_deploy_block.is_some_and(|deploy_block| evm_block_num == deploy_block) {
            let (state_dump_block, extra_fields) = generate_extra_fields_from_json(TESTNET_DEPLOY_STATE);
            assert_eq!(state_dump_block, evm_block_num, "State dump doesn\'t match configured deploy block");
            completed_block.extra_fields = extra_fields;
        };

        block_map.next(&completed_block, &config.chain_id);

        let block_num = block.block_num;
        if let Some(tx) = tx.clone() {
            if let Err(error) = tx.send(completed_block).await {
                error!("Failed to send finished block to exit stream!! {error}.");
                break;
            }
        }

        if &block_num == stop_block {
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
