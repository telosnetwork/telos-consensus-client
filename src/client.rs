use crate::config::AppConfig;
use crate::execution_api_client::{ExecutionApiClient, ExecutionApiError, RpcRequest};
use crate::json_rpc::JsonResponseBody;
use alloy_rlp::encode;
use eyre::Result;
use log::{debug, error, info};
use reth_primitives::{Bytes, B256, U256};
use reth_primitives::revm_primitives::bitvec::macros::internal::funty::Fundamental;
use reth_rpc_types::engine::{ForkchoiceState, ForkchoiceUpdated};
use reth_rpc_types::{Block, ExecutionPayloadV1};
use serde_json::json;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::{Translator, TranslatorConfig};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use crate::client::Error::ForkChoiceUpdatedError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // #[error("Failed to sync block info.")]
    // BlockSyncInfo,
    // #[error("Executor block past config stop block.")]
    // ExecutorBlockPastStopBlock,
    // #[error("Latest block not found.")]
    // LatestBlockNotFound,
    #[error("Spawn translator error")]
    SpawnTranslatorError,
    #[error("Executor hash mismatch.")]
    ExecutorHashMismatch,
    #[error("Fork choice updated error")]
    ForkChoiceUpdatedError(String),
}

pub struct ConsensusClient {
    pub config: AppConfig,
    execution_api: ExecutionApiClient,
    //latest_consensus_block: ExecutionPayloadV1,
    latest_valid_executor_block: Option<Block>,
    //is_forked: bool,
}

impl ConsensusClient {
    pub async fn new(config: AppConfig) -> Self {
        let my_config = config.clone();

        let execution_api = ExecutionApiClient::new(config.execution_endpoint, config.jwt_secret);
        //let latest_consensus_block = ConsensusClient::get_latest_consensus_block(&translator).await;
        let latest_executor_block_response =
            ConsensusClient::get_latest_executor_block(&execution_api).await;


        let latest_executor_block = latest_executor_block_response.unwrap_or_else(|e| {
            panic!("Cannot fetch latest executor block: {}", e);
        });

        Self {
            config: my_config,
            execution_api,
            //latest_consensus_block,
            latest_valid_executor_block: latest_executor_block,
            // is_forked: true,
        }
    }

    async fn get_latest_executor_block(execution_api: &ExecutionApiClient) -> Result<Option<Block>, ExecutionApiError> {
        execution_api
            .block_by_number(None, false)
            .await
    }

    fn make_translator(config: &AppConfig) -> Translator {
        Translator::new(TranslatorConfig {
            chain_id: config.chain_id,
            start_block: config.start_block,
            stop_block: config.stop_block,
            block_delta: config.block_delta.unwrap_or(0u32),
            prev_hash: config.prev_hash.clone(),
            validate_hash: config.validate_hash.clone(),
            http_endpoint: config.chain_endpoint.clone(),
            ship_endpoint: config.ship_endpoint.clone(),
            raw_ds_threads: None,
            block_process_threads: None,
            raw_message_channel_size: None,
            block_message_channel_size: None,
            final_message_channel_size: None,
        })
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        let (tx, mut rx) = mpsc::channel::<TelosEVMBlock>(1000);

        let mut translator = Self::make_translator(&self.config);

        let launch_handle: JoinHandle<Result<(), Error>> = tokio::spawn(async move {
            translator
                .launch(Some(tx))
                .await
                .map_err(|_| Error::SpawnTranslatorError)
        });

        let mut batch = vec![];
        let mut send_to_executor = false;
        while let Some(block) = rx.recv().await {
            // Set this to true only once, if we are caught up from reader to executor's latest block

            if let Some(latest_block) = self.latest_valid_executor_block.clone() {
                if !send_to_executor {
                    if latest_block.header.number.unwrap() == block.block_num.as_u64()
                    {
                        // We've received the same block from transaltor as the latest executor block, check if hashes match
                        if latest_block.header.hash.unwrap() != block.block_hash {
                            error!("Fork detected! Latest executor block hash {:?} does not match consensus block hash {:?}",
                               latest_block.header.hash.unwrap(), block.block_hash);
                            return Err(Error::ExecutorHashMismatch);
                        }
                    }

                    // if this block is older than the latest valid executor block, skip it
                    send_to_executor = block.block_num
                        > latest_block.header.number.unwrap().as_u32();
                }
            } else {
                send_to_executor = true;
            }

            if !send_to_executor {
                continue;
            }

            // TODO: Check if we are caught up, if so do not batch anything

            batch.push(block);
            if self.config.batch_size >= batch.len() {
                self.send_batch(batch).await?;
                batch = vec![];
            }
        }

        if launch_handle.await.is_err() {
            return Err(Error::SpawnTranslatorError);
        }

        Ok(())
    }

    async fn send_batch(&self, batch: Vec<TelosEVMBlock>) -> Result<(), Error> {
        let rpc_batch = batch
            .iter()
            .map(|block| {
                // println!("block: {:?}", block);
                let base_fee_per_gas = if block.header.base_fee_per_gas.is_some() {
                    U256::from(block.header.base_fee_per_gas.unwrap())
                } else {
                    U256::ZERO
                };

                let mut transactions = vec![];
                for tx in block.transactions.iter() {
                    let encoded = encode(tx.envelope.clone());
                    transactions.push(Bytes::from(encoded));
                    // match tx.envelope {
                    //     TxEnvelope::Legacy(l) => {
                    //         let tx_envelope = TxEnvelope::from(l.clone());
                    //         let encoded = encode(tx_envelope);
                    //         transactions.push(Bytes::from(encoded));
                    //     }
                    //     TxEnvelope::Eip2930(_) => {}
                    //     TxEnvelope::Eip1559(_) => {}
                    //     TxEnvelope::Eip4844(_) => {}
                    // }
                }

                let execution_payload = ExecutionPayloadV1 {
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
                    base_fee_per_gas,
                    block_hash: block.block_hash,
                    transactions,
                };
                RpcRequest {
                    method: crate::execution_api_client::ExecutionApiMethod::NewPayloadV1,
                    params: json![execution_payload],
                }
            })
            .collect::<Vec<RpcRequest>>();

        let new_payloadv1_result = self.execution_api.rpc_batch(rpc_batch).await.unwrap();
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
            debug!("Fork choice error: {}", e);
            ForkChoiceUpdatedError(e.to_string())
        })?;


        if let Some(error) = fork_choice_updated.error {
            debug!("Fork choice error: {:?}", error);
            return Err(ForkChoiceUpdatedError(error.message));
        }

        let fork_choice_updated: ForkchoiceUpdated = serde_json::from_value(fork_choice_updated.result).unwrap();
        debug!("fork_choice_updated_result {:?}", fork_choice_updated);

        if fork_choice_updated.is_invalid() || fork_choice_updated.is_syncing() {
            debug!("Fork choice update status is {} ", fork_choice_updated.payload_status.status);
            return Err(ForkChoiceUpdatedError(format!("Invalid status {}", fork_choice_updated.payload_status.status)));
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

        let response = self.execution_api
            .rpc(RpcRequest {
                method: crate::execution_api_client::ExecutionApiMethod::ForkChoiceUpdatedV1,
                params: json![vec![fork_choice_state]],
            })
            .await;

        response
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
