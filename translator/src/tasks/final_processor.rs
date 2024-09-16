use crate::block::{DecodedRow, TelosEVMBlock, WalletEvents};
use crate::{
    block::ProcessingEVMBlock, translator::TranslatorConfig,
    types::translator_types::NameToAddressCache,
};
use alloy::primitives::{Address, Bytes, FixedBytes, U256};
use alloy_rlp::Encodable;
use antelope::api::client::{APIClient, DefaultProvider};
use eyre::{eyre, Context, Result};
use hex::encode;
use reth_telos_rpc_engine_api::structs::{
    TelosAccountStateTableRow, TelosAccountTableRow, TelosEngineAPIExtraFields,
};
use std::str::FromStr;
use tokio::{sync::mpsc, time::Instant};
use tracing::{debug, error, info};

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

    let mut parent_hash = FixedBytes::from_str(&config.prev_hash)
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

    while let Some(mut block) = rx.recv().await {
        if &block.block_num > stop_block {
            break;
        }
        debug!("Finalizing block #{}", block.block_num);

        let (header, exec_payload) = block
            .generate_evm_data(parent_hash, block_delta, &native_to_evm_cache)
            .await;

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
                "Block #{} 0x{} - processed {} blocks/sec and {} tx/sec",
                block.block_num,
                encode(block_hash),
                blocks_sec,
                trx_sec
            );
            //info!("Block map is {} long", block_map.len());
            unlogged_blocks = 0;
            unlogged_transactions = 0;
            last_log = Instant::now();
        }
        // TODO: Fork handling, hashing, all the things...

        let evm_block_num = header.number as u32;

        let mut statediffs_account = vec![];
        let mut statediffs_accountstate = vec![];

        for row in block.decoded_rows {
            match row {
                DecodedRow::Account(acc_diff) => statediffs_account.push(TelosAccountTableRow {
                    address: Address::from_slice(&acc_diff.address.data),
                    account: acc_diff.account.to_string(),
                    nonce: acc_diff.nonce,
                    code: Bytes::from(acc_diff.code.clone()),
                    balance: U256::from_be_slice(&acc_diff.balance.data),
                }),
                DecodedRow::AccountState(acc_state_diff) => {
                    statediffs_accountstate.push(TelosAccountStateTableRow {
                        address: native_to_evm_cache
                            .get_index(acc_state_diff.index)
                            .await
                            .unwrap(),
                        key: U256::from_be_slice(&acc_state_diff.key.data),
                        value: U256::from_be_slice(&acc_state_diff.value.data),
                    });
                }
                _ => (),
            }
        }

        let mut new_addresses_using_create = vec![];
        let mut new_addresses_using_openwallet = vec![];

        for new_wallet in block.new_wallets {
            match new_wallet {
                WalletEvents::CreateWallet(trx_index, create_action) => new_addresses_using_create
                    .push((
                        trx_index as u64,
                        U256::from_be_slice(
                            native_to_evm_cache
                                .get(create_action.account.value())
                                .await
                                .unwrap()
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

        let receipts = Some(
            block
                .transactions
                .iter()
                .map(|(_trx, full_receipt)| full_receipt.receipt.clone())
                .collect(),
        );

        let completed_block = TelosEVMBlock {
            block_num: evm_block_num,
            block_hash,
            lib_num: block.lib_num,
            lib_hash: FixedBytes::from_slice(&block.lib_hash.data),
            transactions: block.transactions,
            header,
            execution_payload: exec_payload,
            extra_fields: TelosEngineAPIExtraFields {
                statediffs_account: Some(statediffs_account),
                statediffs_accountstate: Some(statediffs_accountstate),
                revision_changes: block.new_revision,
                gasprice_changes: block.new_gas_price,
                new_addresses_using_create: Some(new_addresses_using_create),
                new_addresses_using_openwallet: Some(new_addresses_using_openwallet),
                receipts,
            },
        };

        let block_num = block.block_num;
        if let Some(tx) = tx.clone() {
            if let Err(error) = tx.send(completed_block).await {
                error!("Failed to send finished block to exit stream!! {error}.");
                break;
            }
        }
        parent_hash = block_hash;
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
