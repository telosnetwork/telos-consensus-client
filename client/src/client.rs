use crate::client::Error::ForkChoiceUpdated;
use crate::config::{AppConfig, CliArgs};
use crate::data::{self, execution_branch_entry, Database, ExecutionCheckpoint, Lib};
use crate::execution_api_client::{ExecutionApiClient, ExecutionApiError, RpcRequest};
use crate::json_rpc::JsonResponseBody;
use alloy_rpc_types::Block;
use alloy_rpc_types_engine::{ForkchoiceState, ForkchoiceUpdated, PayloadStatus};
use eyre::{Context, Result};
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use reth_primitives::B256;
use serde_json::json;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use telos_translator_rs::block::TelosEVMBlock;
use tokio::sync::mpsc;
use tokio::task::JoinError;
use tracing::{debug, error, info};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // #[error("Failed to sync block info.")]
    // BlockSyncInfo,
    // #[error("Executor block past config stop block.")]
    // ExecutorBlockPastStopBlock,
    // #[error("Latest block not found.")]
    // LatestBlockNotFound,
    #[error("Cannot start consensus client {0}")]
    CannotStartConsensusClient(String),
    // #[error("Spawn translator error")]
    // SpawnTranslator,
    #[error("Executor hash mismatch.")]
    ExecutorHashMismatch,
    #[error("Invalid native irreversible block: {0}")]
    InvalidIrreversibleBlock(String),
    #[error("Fork choice updated error")]
    ForkChoiceUpdated(String),
    #[error("New payload error")]
    NewPayloadV1(String),
    #[error("Database error: {0}")]
    Database(eyre::Report),
    #[error("Client is too many blocks ({0}) behind the executor, start from a more recent block or increase maximum range"
    )]
    RangeAboveMaximum(u32),
    #[error("Cannot shutdown translator: {0}")]
    TranslatorShutdown(String),
    #[error("Translator error: {0}")]
    TranslatorError(String),
    #[error("Call to execution API failed: {0}")]
    ExecutionApiError(#[from] ExecutionApiError),
    #[error("Failed to run consensus client: {0}")]
    ConsensusClientRun(#[from] JoinError),
}

pub struct Shutdown(mpsc::Sender<()>);
impl Shutdown {
    #[allow(dead_code)]
    pub async fn shutdown(&self) -> Result<()> {
        Ok(self.0.send(()).await?)
    }
}

pub struct ConsensusClient {
    pub config: AppConfig,
    execution_api: ExecutionApiClient,
    //latest_consensus_block: ExecutionPayloadV1,
    pub latest_executor_block: Option<Block>,
    pub latest_finalized_executor_block: Option<Block>,
    //is_forked: bool,
    pub db: Database,
    shutdown_tx: mpsc::Sender<()>,
    shutdown_rx: mpsc::Receiver<()>,
}

impl ConsensusClient {
    pub async fn new(args: &CliArgs, config: AppConfig) -> Result<Self> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        let execution_api = ExecutionApiClient::new(
            &config.execution_endpoint,
            Path::new(&config.jwt_secret_path),
            Duration::from_millis(config.execution_connect_timeout_ms.unwrap_or(5_000)),
            Duration::from_millis(config.execution_request_timeout_ms.unwrap_or(30_000)),
            config
                .execution_max_response_bytes
                .unwrap_or(16 * 1024 * 1024),
        )
        .wrap_err("Failed to create Execution API client")?;
        let expected_anchor_hash = B256::from_str(&config.execution_anchor_block_hash)
            .wrap_err("Configured execution anchor hash is not a 32-byte hex value")?;
        execution_api
            .exchange_capabilities()
            .await
            .wrap_err("Execution endpoint capability negotiation failed")?;
        execution_api
            .verify_chain(
                config.chain_id.0,
                config.execution_anchor_block_number,
                expected_anchor_hash,
            )
            .await
            .wrap_err("Execution endpoint chain identity verification failed")?;

        let db = match args.clean {
            false => Database::open(&config.data_path)?,
            true => Database::init(&config.data_path)?,
        };
        let latest_executor_block = execution_api
            .get_latest_block(config.execution_anchor_block_number)
            .await
            .wrap_err("Failed to get latest executor block")?;
        let latest_finalized_executor_block = execution_api
            .get_latest_finalized_block(config.execution_anchor_block_number)
            .await
            .wrap_err("Failed to get latest valid executor block")?;

        Ok(Self {
            config,
            execution_api,
            latest_executor_block,
            latest_finalized_executor_block,
            db,
            shutdown_tx,
            shutdown_rx,
        })
    }

    #[allow(dead_code)]
    pub fn shutdown_handle(&self) -> Shutdown {
        Shutdown(self.shutdown_tx.clone())
    }

    fn latest_evm_block(&self) -> Option<(u32, String)> {
        let latest = self.latest_finalized_executor_block.as_ref()?;
        let (number, hash) = (latest.header.number, latest.header.hash);
        Some((number.as_u32(), hash.to_string()))
    }

    pub fn is_in_start_stop_range(&self, block: u32) -> bool {
        match (self.config.evm_start_block, self.config.evm_stop_block) {
            (start_block, Some(stop_block)) => start_block <= block && block <= stop_block,
            (start_block, None) => start_block <= block,
        }
    }

    pub fn is_in_check_range(&self, block: u64) -> bool {
        match (
            &self.latest_finalized_executor_block,
            &self.latest_executor_block,
        ) {
            (None, None) => false,
            (None, Some(latest)) => block < latest.header.number,
            (Some(_), None) => unreachable!(),
            (Some(valid), Some(latest)) => {
                valid.header.number <= block && block <= latest.header.number
            }
        }
    }

    pub fn latest_evm_number(&self) -> Option<u32> {
        self.latest_finalized_executor_block
            .as_ref()
            .map(|block| block.header.number.as_u32())
    }

    pub(crate) async fn canonical_block_hash(
        &self,
        block_number: u32,
    ) -> Result<Option<B256>, Error> {
        self.execution_api
            .get_block_by_number(block_number.into())
            .await
            .map(|block| block.map(|block| block.header.hash))
            .map_err(Error::ExecutionApiError)
    }

    pub fn sync_range(&self) -> Option<u32> {
        self.latest_evm_number()?
            .checked_sub(self.config.evm_start_block)
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<TelosEVMBlock>) -> Result<(), Error> {
        let mut batch = vec![];
        let chain_id = &self.config.chain_id;
        let mut lib: data::Block = self.db.get_lib()?.unwrap_or_default();
        let mut last_finalized_hash = self
            .latest_finalized_executor_block
            .as_ref()
            .map(|block| block.header.hash)
            .ok_or_else(|| {
                Error::InvalidIrreversibleBlock(
                    "execution endpoint did not return a finalized block or verified anchor"
                        .to_string(),
                )
            })?;

        loop {
            let message = tokio::select! {
                message = rx.recv() => message,
                _ = self.shutdown_rx.recv() => {
                    debug!("Shutdown signal received");
                    break;
                }
            };

            let Some(block) = message else {
                break;
            };

            let block_num = block.block_num;

            self.db.put_block(From::from(&block))?;
            debug!("Block {block_num} put in the database");

            let latest_start = block_num.saturating_sub(self.config.latest_blocks_in_db_num);

            // Keep latest blocks and every nth block
            if latest_start > 0 && latest_start % self.config.block_checkpoint_interval != 0 {
                self.db.delete_block(latest_start)?;
                debug!("Block {latest_start} deleted from the database");
            }

            let reported_lib_hash = block.lib_hash.clone();
            if block.lib_num < lib.number {
                return Err(Error::InvalidIrreversibleBlock(format!(
                    "reported LIB {} is behind stored LIB {}",
                    block.lib_num, lib.number
                )));
            }
            if block.lib_num == lib.number && lib.number != 0 && reported_lib_hash != lib.hash {
                return Err(Error::InvalidIrreversibleBlock(format!(
                    "reported LIB {} hash {} conflicts with stored hash {}",
                    block.lib_num, reported_lib_hash, lib.hash
                )));
            }
            let is_new_lib = block.lib_num > lib.number;

            if is_new_lib {
                let new_lib = Lib(&block);
                self.db.put_lib(Lib(&block).into())?;
                info!("LIB {new_lib:?} put in the database");
                lib = new_lib.into();
            }

            if let Some((latest_evm_num, latest_evm_hash)) = self.latest_evm_block() {
                // Check fork
                if block_num == latest_evm_num && block.block_hash.to_string() != latest_evm_hash {
                    error!("Fork detected! Latest executor block hash {latest_evm_num:?} does not match consensus block hash {block_num:?}");
                    return Err(Error::ExecutorHashMismatch);
                }

                // Skip synced blocks
                if block_num <= latest_evm_num {
                    debug!("Block {block_num} skipped as its behind {latest_evm_num} evm block");
                    continue;
                }
            }

            let block_hash = block.block_hash;

            if self.is_in_check_range(block_num.as_u64()) {
                debug!("Checking if block {block_num} exists...");
                let evm_block = self
                    .execution_api
                    .get_block_by_number(block_num.into())
                    .await?;

                if let Some(evm_block) = evm_block {
                    if evm_block.header.hash != block_hash {
                        return Err(Error::ExecutorHashMismatch);
                    }
                    continue;
                }
            }

            let block_is_final = block.is_final(chain_id);
            let block_is_lib = block.is_lib(chain_id);
            let lib_evm_num = block.lib_evm_num(chain_id);

            batch.push(block);

            // if LIB is less or equal than current block batch size is 1 or more blocks
            // if LIB is greater than current block send in batches
            let flush = !block_is_final || block_is_lib || batch.len() == self.config.batch_size;

            if !flush {
                continue;
            };

            let exact_lib_hash = if is_new_lib && !block_is_final {
                match self.db.get_execution_branch_entry(&lib.hash)? {
                    Some(entry) => {
                        if entry.native_block_number != lib.number
                            || entry.evm_block_number != lib_evm_num
                        {
                            return Err(Error::InvalidIrreversibleBlock(format!(
                                "native LIB {} ({}) maps to unexpected EVM block {}",
                                lib.number, lib.hash, entry.evm_block_number
                            )));
                        }
                        Some(entry.evm_hash)
                    }
                    None => {
                        let exact_lib = self.db.get_block(lib_evm_num)?.ok_or_else(|| {
                            Error::InvalidIrreversibleBlock(format!(
                                "cannot finalize native LIB {} ({}): exact EVM block {} is not retained",
                                lib.number, lib.hash, lib_evm_num
                            ))
                        })?;
                        Some(exact_lib.hash.parse().map_err(|error| {
                            Error::InvalidIrreversibleBlock(format!(
                                "stored LIB-mapped EVM block {} has an invalid hash: {error}",
                                exact_lib.number
                            ))
                        })?)
                    }
                }
            } else {
                None
            };
            let finalized_hash = select_finalized_hash(
                block_is_final,
                is_new_lib,
                block_hash,
                last_finalized_hash,
                exact_lib_hash,
            )?;
            last_finalized_hash = finalized_hash;
            // Telos exposes irreversibility but no distinct safe-head signal. Keeping these equal
            // prevents finalized from advancing beyond safe during historical catch-up.
            let safe_hash = finalized_hash;
            debug!("Send batch finalized hash: {last_finalized_hash}");
            self.send_batch(&batch, last_finalized_hash, safe_hash)
                .await?;
            batch.clear();
        }

        Ok(())
    }

    async fn send_batch(
        &self,
        batch: &[TelosEVMBlock],
        finalized_hash: B256,
        safe_hash: B256,
    ) -> Result<(), Error> {
        let Some(last_block_sent) = batch.last() else {
            return Err(Error::NewPayloadV1(
                "refusing to submit an empty payload batch".to_string(),
            ));
        };
        for (index, block) in batch.iter().enumerate() {
            if batch[index + 1..]
                .iter()
                .any(|candidate_parent| candidate_parent.block_hash == block.header.parent_hash)
            {
                return Err(Error::NewPayloadV1(format!(
                    "batch is not in parent order: block {} appears before parent {}",
                    block.block_num, block.header.parent_hash
                )));
            }
        }

        for block in batch {
            let response = self
                .execution_api
                .rpc(RpcRequest {
                    method: crate::execution_api_client::ExecutionApiMethod::NewPayloadV1,
                    params: json!([block.execution_payload.clone(), block.extra_fields.clone()]),
                })
                .await
                .map_err(|error| Error::NewPayloadV1(error.to_string()))?;
            let payload_status: PayloadStatus = serde_json::from_value(response.result)
                .map_err(|error| Error::NewPayloadV1(error.to_string()))?;
            validate_payload_status(block.block_num, block.block_hash, &payload_status)?;
            self.db
                .put_execution_branch_entry(&execution_branch_entry(block)?)?;
            self.db
                .prune_execution_branches(block.lib_num, &block.lib_hash)?;
            debug!(
                block_number = block.block_num,
                block_hash = %block.block_hash,
                native_hash = %block.ship_hash,
                "engine_newPayloadV1 accepted payload and persisted branch entry"
            );
        }

        let finalized_hash_value = finalized_hash;
        let fork_choice_updated_result = self
            .fork_choice_updated(last_block_sent.block_hash, safe_hash, finalized_hash_value)
            .await;

        let fork_choice_updated = fork_choice_updated_result.map_err(|e| {
            debug!("Fork choice update error: {}", e);
            ForkChoiceUpdated(e.to_string())
        })?;

        if let Some(error) = fork_choice_updated.error {
            debug!("Fork choice error: {:?}", error);
            return Err(ForkChoiceUpdated(error.message));
        }

        let fork_choice_updated: ForkchoiceUpdated =
            serde_json::from_value(fork_choice_updated.result)
                .map_err(|error| ForkChoiceUpdated(error.to_string()))?;
        info!("fork_choice_updated_result {:?}", fork_choice_updated);

        // Valid, Invalid, Accepted, Syncing
        if !fork_choice_updated.is_valid() {
            info!(
                "Fork choice update status is {} ",
                fork_choice_updated.payload_status.status
            );
            return Err(ForkChoiceUpdated(format!(
                "Invalid status {}",
                fork_choice_updated.payload_status.status
            )));
        }
        if fork_choice_updated.payload_status.latest_valid_hash != Some(last_block_sent.block_hash)
        {
            return Err(ForkChoiceUpdated(format!(
                "latestValidHash {:?} does not match head {}",
                fork_choice_updated.payload_status.latest_valid_hash, last_block_sent.block_hash
            )));
        }
        self.db
            .put_execution_checkpoint(ExecutionCheckpoint::try_from(last_block_sent)?)?;

        debug!(
            "Fork choice updated called with:\nhash {:?}\nparentHash {:?}\nnumber {:?}",
            last_block_sent.block_hash,
            last_block_sent.header.parent_hash,
            last_block_sent.block_num
        );
        info!(
            "fork_choice_updated_result for block number {}: {:?}",
            last_block_sent.block_num, fork_choice_updated
        );

        Ok(())
    }

    async fn fork_choice_updated(
        &self,
        head_hash: B256,
        safe_hash: B256,
        finalized_hash: B256,
    ) -> Result<JsonResponseBody, ExecutionApiError> {
        let fork_choice_state = ForkchoiceState {
            head_block_hash: head_hash,
            safe_block_hash: safe_hash,
            finalized_block_hash: finalized_hash,
        };

        self.execution_api
            .rpc(RpcRequest {
                method: crate::execution_api_client::ExecutionApiMethod::ForkChoiceUpdatedV1,
                params: json!([fork_choice_state, null]),
            })
            .await
    }
}

fn select_finalized_hash(
    block_is_final: bool,
    is_new_lib: bool,
    block_hash: B256,
    previous_finalized_hash: B256,
    exact_lib_hash: Option<B256>,
) -> Result<B256, Error> {
    if block_is_final {
        debug!("Current block is at or below the native LIB");
        return Ok(block_hash);
    }
    if is_new_lib {
        debug!("New native LIB detected");
        return exact_lib_hash.ok_or_else(|| {
            Error::InvalidIrreversibleBlock(
                "new native LIB has no exact EVM block hash".to_string(),
            )
        });
    }
    debug!("Native LIB is unchanged; retaining the verified finalized hash");
    Ok(previous_finalized_hash)
}

fn validate_payload_status(
    block_number: u32,
    block_hash: B256,
    payload_status: &PayloadStatus,
) -> Result<(), Error> {
    if !payload_status.is_valid() {
        return Err(Error::NewPayloadV1(format!(
            "block {block_number} returned status {}",
            payload_status.status
        )));
    }
    if payload_status.latest_valid_hash != Some(block_hash) {
        return Err(Error::NewPayloadV1(format!(
            "block {block_number} returned latestValidHash {:?}, expected {block_hash}",
            payload_status.latest_valid_hash
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_rpc_types_engine::PayloadStatusEnum;

    #[test]
    fn new_payload_requires_valid_status_for_the_exact_hash() {
        let block_hash = B256::repeat_byte(1);
        let valid = PayloadStatus::new(PayloadStatusEnum::Valid, Some(block_hash));
        validate_payload_status(10, block_hash, &valid).unwrap();

        let wrong_hash = PayloadStatus::new(PayloadStatusEnum::Valid, Some(B256::repeat_byte(2)));
        assert!(validate_payload_status(10, block_hash, &wrong_hash).is_err());

        let syncing = PayloadStatus::from_status(PayloadStatusEnum::Syncing);
        assert!(validate_payload_status(10, block_hash, &syncing).is_err());
    }

    #[test]
    fn restart_with_unchanged_lib_retains_a_finalized_hash_for_fcu() {
        let prior = B256::repeat_byte(7);
        assert_eq!(
            select_finalized_hash(false, false, B256::repeat_byte(8), prior, None).unwrap(),
            prior
        );
    }

    #[test]
    fn a_new_lib_requires_its_exact_evm_hash() {
        assert!(select_finalized_hash(
            false,
            true,
            B256::repeat_byte(8),
            B256::repeat_byte(7),
            None
        )
        .is_err());
    }
}
