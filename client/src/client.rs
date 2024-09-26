use crate::client::Error::ForkChoiceUpdated;
use crate::config::{AppConfig, CliArgs};
use crate::data::{self, Database, Lib};
use crate::execution_api_client::{ExecutionApiClient, ExecutionApiError, RpcRequest};
use crate::json_rpc::JsonResponseBody;
use eyre::{Context, Result};
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use reth_primitives::revm_primitives::db::{BlockHash, BlockHashRef};
use reth_primitives::B256;
use reth_rpc_types::engine::{ForkchoiceState, ForkchoiceUpdated};
use reth_rpc_types::Block;
use serde_json::json;
use std::hash::Hash;
use telos_translator_rs::block::TelosEVMBlock;
use tokio::sync::mpsc;
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
    pub latest_valid_executor_block: Option<Block>,
    //is_forked: bool,
    pub db: Database,
    shutdown_tx: mpsc::Sender<()>,
    shutdown_rx: mpsc::Receiver<()>,
}

impl ConsensusClient {
    pub async fn new(args: &CliArgs, config: AppConfig) -> Result<Self> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        let execution_api = ExecutionApiClient::new(&config.execution_endpoint, &config.jwt_secret)
            .wrap_err("Failed to create Execution API client")?;

        let db = match args.clean {
            false => Database::open(&config.data_path)?,
            true => Database::init(&config.data_path)?,
        };
        let latest_valid_executor_block = execution_api
            .get_latest_finalized_block()
            .await
            .wrap_err("Failed to get latest valid executor block")?;

        Ok(Self {
            config,
            execution_api,
            latest_valid_executor_block,
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
        let latest = self.latest_valid_executor_block.as_ref()?;
        let (number, hash) = (latest.header.number, latest.header.hash);
        Some((number.as_u32(), hash.to_string()))
    }

    pub fn is_in_start_stop_range(&self, num: u32) -> bool {
        match (self.config.evm_start_block, self.config.evm_stop_block) {
            (start_block, Some(stop_block)) => start_block <= num && num <= stop_block,
            (start_block, None) => start_block <= num,
        }
    }

    pub fn latest_evm_number(&self) -> Option<u32> {
        self.latest_valid_executor_block
            .as_ref()
            .map(|block| block.header.number.as_u32())
    }

    pub fn sync_range(&self) -> Option<u32> {
        self.latest_evm_number()?
            .checked_sub(self.config.evm_start_block)
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<TelosEVMBlock>) -> Result<(), Error> {
        let mut batch = vec![];
        let mut lib: Option<data::Block> = self.db.get_lib()?;
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

            let block_num = block.block_num.as_u64();
            let block_hash = block.block_hash;
            let lib_num = block.lib_num;
            if block_num % self.config.block_checkpoint_interval.as_u64() != 0 {
                self.db.put_block(From::from(&block))?;
                debug!("Block {} put in the database", block.block_num);
            }

            if block_num % self.config.block_checkpoint_interval.as_u64() == 0 {
                self.db.put_block(From::from(&block))?;
                debug!("Block {} put in the database", block.block_num);
            }

            let latest_start: u32 = block_num
                .saturating_sub(self.config.latest_blocks_in_db_num.into())
                .as_u32();

            if latest_start > 0 && latest_start % self.config.block_checkpoint_interval != 0 {
                self.db.delete_block(latest_start)?;
                debug!("Block {} delete from the database", latest_start);
            }

            let mut is_new_lib = false;
            if lib.as_ref().map(|lib| lib.number < lib_num).unwrap_or(true) {
                is_new_lib = true;
                lib = Some(From::from(Lib(&block)));
                self.db.put_lib(From::from(Lib(&block)))?;
                debug!("LIB {} put in the database", block.lib_num);
            }

            if let Some((latest_num, latest_hash)) = &self.latest_evm_block() {
                // Check fork
                if block_num == latest_num.as_u64() && &block_hash.to_string() != latest_hash {
                    error!("Fork detected! Latest executor block hash {latest_num:?} does not match consensus block hash {block_num:?}" );
                    return Err(Error::ExecutorHashMismatch);
                }

                // Skip synced blocks
                if block_num <= latest_num.as_u64() {
                    debug!("Block {block_num} skipped as its behind {latest_num} evm block");
                    continue;
                }
            }

            batch.push(block);

            // check if we caught up to head
            // if lib is greater than current block send in batches
            // if lib is less than current block batch size is 1
            // if lib is equal to the current block flush the batch
            let flush = match lib.as_ref() {
                Some(lib) if lib.number <= block_num.as_u32() => true,
                _ => batch.len() == self.config.batch_size,
            };

            if !flush {
                continue;
            }

            let lib_hash = lib.as_ref().map(|lib| lib.hash.parse::<B256>().unwrap());

            let finalized_hash = match lib_hash {
                // default finalized hash is the last one in the batch
                None => Some(block_hash),
                // if lib is less that current block, we caught to the head
                Some(_) if lib_num.as_u64() >= block_num => Some(block_hash),
                // if lib hash has been changed we should send finalized hash for fc update
                Some(lib_hash) if is_new_lib => Some(lib_hash),
                // head caught but no changes to the lib
                Some(_) => None,
            };

            debug!("Send batch fc {:?}", finalized_hash);
            self.send_batch(&batch, finalized_hash).await?;
            batch.clear();
        }

        // launch_handle.await.map_err(|_| Error::SpawnTranslator)?
        Ok(())
    }

    async fn send_batch(
        &self,
        batch: &[TelosEVMBlock],
        finalized_hash: Option<B256>,
    ) -> Result<(), Error> {
        let rpc_batch = batch
            .iter()
            .map(|block| {
                // TODO additional rpc call fields should be added.
                RpcRequest {
                    method: crate::execution_api_client::ExecutionApiMethod::NewPayloadV1,
                    params: vec![
                        json![block.execution_payload.clone()],
                        json![block.extra_fields.clone()],
                    ]
                    .into(),
                }
            })
            .collect::<Vec<RpcRequest>>();

        let new_payloadv1_result = self
            .execution_api
            .rpc_batch(rpc_batch)
            .await
            .map_err(|e| Error::NewPayloadV1(e.to_string()))?;
        // TODO: check for VALID status on new_payloadv1_result, and handle the failure case
        debug!("NewPayloadV1 result: {:?}", new_payloadv1_result);

        let last_block_sent = batch.last().unwrap();

        if let Some(finalized_hash_value) = finalized_hash {
            let fork_choice_updated_result = self
                .fork_choice_updated(
                    last_block_sent.block_hash,
                    last_block_sent.block_hash,
                    finalized_hash_value,
                )
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
                serde_json::from_value(fork_choice_updated.result).unwrap();
            debug!("fork_choice_updated_result {:?}", fork_choice_updated);

            // Valid, Invalid, Accepted, Syncing
            if fork_choice_updated.is_invalid() || fork_choice_updated.is_syncing() {
                debug!(
                    "Fork choice update status is {} ",
                    fork_choice_updated.payload_status.status
                );
                return Err(ForkChoiceUpdated(format!(
                    "Invalid status {}",
                    fork_choice_updated.payload_status.status
                )));
            }

            debug!(
                "Fork choice updated called with:\nhash {:?}\nparentHash {:?}\nnumber {:?}",
                last_block_sent.block_hash,
                last_block_sent.header.parent_hash,
                last_block_sent.block_num
            );
            debug!(
                "fork_choice_updated_result for block number {}: {:?}",
                last_block_sent.block_num, fork_choice_updated
            );
        } else {
            debug!(
                "Fork choice updated call skipped for block {}",
                last_block_sent.block_num
            );
        }

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
                params: json![vec![fork_choice_state]],
            })
            .await
    }
}
