use std::cmp;

use crate::client::Error::{ConsensusClientShutdown, ForkChoiceUpdated};
use crate::config::{AppConfig, CliArgs};
use crate::data::{self, Database, Lib};
use crate::execution_api_client::{ExecutionApiClient, ExecutionApiError, RpcRequest};
use crate::json_rpc::JsonResponseBody;
use eyre::{Context, Result};
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use reth_primitives::B256;
use reth_rpc_types::engine::{ForkchoiceState, ForkchoiceUpdated};
use reth_rpc_types::Block;
use serde_json::json;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::Translator;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // #[error("Failed to sync block info.")]
    // BlockSyncInfo,
    // #[error("Executor block past config stop block.")]
    // ExecutorBlockPastStopBlock,
    // #[error("Latest block not found.")]
    // LatestBlockNotFound,
    #[error("Spawn translator error")]
    SpawnTranslator,
    #[error("Executor hash mismatch.")]
    ExecutorHashMismatch,
    #[error("Fork choice updated error")]
    ForkChoiceUpdated(String),
    #[error("New payload error")]
    NewPayloadV1(String),
    #[error("Consensus client shutdown error")]
    ConsensusClientShutdown(String),
    #[error("Database error: {0}")]
    Database(eyre::Report),
    #[error("Client is too many blocks ({0}) behind the executor, start from a more recent block or increase maximum range")]
    RangeAboveMaximum(u64),
}

pub struct ConsensusClient {
    pub config: AppConfig,
    execution_api: ExecutionApiClient,
    //latest_consensus_block: ExecutionPayloadV1,
    pub latest_valid_executor_block: Option<Block>,
    //is_forked: bool,
    db: Database,
}

impl ConsensusClient {
    pub async fn new(args: &CliArgs, config: AppConfig) -> Result<Self> {
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

        debug!("Latest valid executor block is: {latest_valid_executor_block:?}");

        Ok(Self {
            config,
            execution_api,
            latest_valid_executor_block,
            db,
        })
    }

    fn latest_evm_block(&self) -> Option<(u32, String)> {
        let latest = self.latest_valid_executor_block.as_ref()?;
        match (latest.header.number, latest.header.hash) {
            (Some(number), Some(hash)) => Some((number.as_u32(), hash.to_string())),
            _ => None,
        }
    }

    fn is_in_start_stop_range(&self, num: u32) -> bool {
        match (self.config.start_block, self.config.stop_block) {
            (start_block, Some(stop_block)) => start_block <= num && num <= stop_block,
            (start_block, None) => start_block <= num,
        }
    }

    fn latest_evm_number(&self) -> Option<u64> {
        self.latest_valid_executor_block.as_ref()?.header.number
    }

    fn min_latest_or_lib(&self, lib: Option<&data::Block>) -> Option<u32> {
        match (lib, self.latest_evm_block().as_ref()) {
            (Some(lib), Some(latest)) => Some(cmp::min(lib.number, latest.0)),
            (_, _) => None,
        }
    }

    fn sync_range(&self) -> Option<u64> {
        self.latest_evm_number()?
            .checked_sub(self.config.start_block.as_u64())
    }

    pub async fn run(&mut self, mut shutdown_rx: oneshot::Receiver<()>) -> Result<(), Error> {
        let (tx, mut rx) = mpsc::channel::<TelosEVMBlock>(1000);
        let (sender, receiver) = mpsc::channel::<()>(1);

        let mut lib = self.db.get_lib()?;

        let latest_number = self.min_latest_or_lib(lib.as_ref());

        let last_checked = match latest_number {
            Some(latest_number) => self.db.get_block_or_prev(latest_number)?,
            None => None,
        };

        if let Some(last_checked) = last_checked {
            if self.is_in_start_stop_range(last_checked.number + 1) {
                self.config.start_block = last_checked.number + 1;
                self.config.prev_hash = last_checked.hash
            }
        }

        if let Some(sync_range) = self.sync_range() {
            if sync_range > self.config.maximum_sync_range.as_u64() {
                return Err(Error::RangeAboveMaximum(sync_range));
            }
        }

        debug!("Starting translator from block {}", self.config.start_block);

        let mut translator = Translator::new((&self.config).into());
        let sender_tx = sender.clone();

        let launch_handle = tokio::spawn(async move {
            translator
                .launch(Some(tx), sender, receiver)
                .await
                .map_err(|_| Error::SpawnTranslator)
        });
        let mut batch = vec![];
        loop {
            let message = tokio::select! {
                message = rx.recv() => message,
                _ = &mut shutdown_rx => {
                    debug!("Shutdown signal received");
                    sender_tx.send(()).await.map_err(|e | ConsensusClientShutdown(e.to_string()))?;
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

            if lib.as_ref().map(|lib| lib.number < lib_num).unwrap_or(true) {
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

            // TODO: Check if we are caught up, if so do not batch anything

            batch.push(block);
            if batch.len() >= self.config.batch_size {
                self.send_batch(&batch).await?;
                batch = vec![];
            }
        }

        launch_handle.await.map_err(|_| Error::SpawnTranslator)?
    }

    async fn send_batch(&self, batch: &[TelosEVMBlock]) -> Result<(), Error> {
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
        let fork_choice_updated_result = self
            .fork_choice_updated(
                last_block_sent.block_hash,
                last_block_sent.block_hash,
                last_block_sent.block_hash,
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

        // TODO check for all invalid statuses, possible values are:
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

        // TODO: Check status of fork_choice_updated_result and handle the failure case
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

    // shutdown the consensus client
    #[allow(dead_code)]
    pub fn shutdown(&self, sender: oneshot::Sender<()>) -> Result<(), Error> {
        sender.send(()).map_err(|error| {
            error!("Failed to send shutdown signal: {error:?}");
            ConsensusClientShutdown(format!("Failed to send shutdown signal {error:?}"))
        })
    }

    /*
    pub async fn run(&mut self) {
        if !self.sync_block_info().await.unwrap_or(false) {
            error!("Failed to sync block info");
            return;
        }

        let mut next_block_number = self
            .latest_valid_executor_block
            .header
            .number
            .unwrap()
            + 1;
        let mut batch_count = 0;

        let latest_block_number_result = self.reader.get_latest_block().await;

        let last_block_number = match latest_block_number_result {
            None => {
                return Err(Error::LatestBlockNotFound);
            }
            Some(latest_block_number) => std::cmp::min(
                latest_block_number.payload.block_number,
                self.config.stop_block,
            ),
        };
        let last_block_number = std::cmp::min(
            self.translator
                .get_latest_block()
                .await
                .unwrap()
                .payload
                .block_number,
            self.config.stop_block,
        );

        if next_block_number > self.config.stop_block {
            return Err(Error::ExecutorBlockPastStopBlock);
        }

        let block_iter = self
            .reader
            .reader
            .iter(next_block_number, self.config.stop_block);

        let mut last_log_time = std::time::Instant::now();
        loop {
            let mut caught_up = false;
            let to_block = if last_block_number > next_block_number + self.config.batch_size {
                std::cmp::min(
                    next_block_number + self.config.batch_size,
                    self.config.stop_block,
                )
            } else {
                caught_up = true;
                last_block_number
            };
            self.do_batch(next_block_number, to_block, &block_iter)
                .await;
            if to_block == self.config.stop_block {
                return Ok(());
            }
            batch_count += 1;

            // do_batch is exclusive of to_block so we do NOT need to increment by 1
            next_block_number = to_block;
            if caught_up {
                info!(
                    "Caught up to latest block {}, sleeping for 5 seconds",
                    next_block_number
                );
                // TODO: make this more live & fork aware
                sleep(Duration::from_secs(5)).await;
            } else if last_log_time.elapsed().as_secs() > 5 {
                last_log_time = std::time::Instant::now();
                info!(
                    "Processed batch {}, up to block {}, sleeping for 1 second",
                    batch_count, next_block_number
                );
            }
        }
    }

    async fn do_batch(&self, from_block: u64, to_block: u64, reader: &ArrowBatchSequentialReader) {
        let mut next_block_number = from_block;
        let mut new_blocks: Vec<FullExecutionPayload>;

        while next_block_number < to_block {
            new_blocks = vec![];

            while new_blocks.len().as_u64() < (to_block - from_block + 1) {
                let next_row = reader.imut_next();

                if next_row.is_none() {
                    break;
                }

                let block = self.reader.decode_row(&next_row.unwrap()).await;

                next_block_number = block.payload.block_number + 1;
                new_blocks.push(block);
            }

            if new_blocks.is_empty() {
                break;
            }

            let rpc_batch = new_blocks
                .iter()
                .map(|block| {
                    // println!("block: {:?}", block);
                    RpcRequest {
                        method: ExecutionApiMethod::NewPayloadV1,
                        params: json![block],
                    }
                })
                .collect::<Vec<RpcRequest>>();

            let new_payloadv1_result = self.execution_api.rpc_batch(rpc_batch).await.unwrap();
            // TODO: check for VALID status on new_payloadv1_result, and handle the failure case
            debug!("NewPayloadV1 result: {:?}", new_payloadv1_result);

            let last_block_sent = new_blocks.last().unwrap();
            let fork_choice_updated_result = self
                .fork_choice_updated(
                    last_block_sent.payload.block_hash,
                    last_block_sent.payload.block_hash,
                    last_block_sent.payload.block_hash,
                )
                .await;

            // TODO: Check status of fork_choice_updated_result and handle the failure case
            debug!(
                "Fork choice updated called with:\nhash {:?}\nparentHash {:?}\nnumber {:?}",
                last_block_sent.payload.block_hash,
                last_block_sent.payload.parent_hash,
                last_block_sent.payload.block_number
            );
            debug!(
                "fork_choice_updated_result for block number {}: {:?}",
                last_block_sent.payload.block_number, fork_choice_updated_result
            );
        }
    }

    async fn get_latest_consensus_block(translator: &Translator) -> ExecutionPayloadV1 {
        translator.get_latest_block().await.unwrap().payload.clone()
    }

    async fn sync_block_info(&mut self) -> Result<bool, String> {
        self.latest_consensus_block =
            ConsensusClient::get_latest_consensus_block(&self.reader).await;
        self.latest_valid_executor_block =
            ConsensusClient::get_latest_executor_block(&self.execution_api).await;

        let consensus_for_latest_executor_block = self
            .reader
            .get_block(
                self.latest_valid_executor_block
                    .header
                    .number
                    .unwrap()
                    .to::<u64>(),
            )
            .await
            .unwrap();

        info!(
            "Consensus for latest executor block:\nhash {:?}\nparentHash {:?}\nnumber {:?}",
            consensus_for_latest_executor_block.payload.block_hash,
            consensus_for_latest_executor_block.payload.parent_hash,
            consensus_for_latest_executor_block.payload.block_number
        );
        info!(
            "Latest executor block:\nhash {:?}\nparentHash {:?}\nnumber {:?}",
            self.latest_valid_executor_block.header.hash.unwrap(),
            self.latest_valid_executor_block.header.parent_hash,
            self.latest_valid_executor_block
                .header
                .number
                .unwrap()
                .to_string()
        );
        let latest_executor_hash = self.latest_valid_executor_block.header.hash.unwrap();
        let consensus_for_latest_executor_hash =
            consensus_for_latest_executor_block.payload.block_hash;

        self.is_forked = latest_executor_hash != consensus_for_latest_executor_hash;

        while self.is_forked {
            let latest_valid_executor_block_number = self
                .latest_valid_executor_block
                .header
                .number
                .unwrap()
                .to::<u64>();

            warn!("Forked and latest executor block number is {}, executor hash is {} but consensus hash is {}",
                     latest_valid_executor_block_number,
                     self.latest_valid_executor_block.header.hash.unwrap(),
                     consensus_for_latest_executor_hash);

            if self
                .latest_valid_executor_block
                .header
                .number
                .unwrap()
                .to::<u64>()
                == 0
            {
                error!("Forked all the way back to execution block 0, cannot go back further");
                return Err(
                    "Forked all the way back to execution block 0, cannot go back further"
                        .to_string(),
                );
            }
            self.latest_valid_executor_block = self
                .execution_api
                .block_by_number(latest_valid_executor_block_number - 1, false)
                .await
                .unwrap();

            let consensus_for_latest_executor_block = self
                .reader
                .get_block(
                    self.latest_valid_executor_block
                        .header
                        .number
                        .unwrap()
                        .to::<u64>(),
                )
                .await
                .unwrap();

            self.is_forked = consensus_for_latest_executor_block.payload.block_hash
                != self.latest_valid_executor_block.header.hash.unwrap();
        }

        Ok(true)
    }
    */
}
