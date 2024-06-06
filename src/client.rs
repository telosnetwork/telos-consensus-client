use std::rc::Weak;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use alloy_primitives::B256;
use arrowbatch::reader::ArrowBatchContext;
use crate::config::AppConfig;
use crate::execution_api_client::{ExecutionApiClient, RpcRequest};
use reth_rpc_types::{Block, ExecutionPayloadV1};
use reth_rpc_types::engine::ForkchoiceState;
use serde_json::json;
use tokio::time::sleep;
use crate::arrow_block_reader::ArrowFileBlockReader;
use crate::json_rpc::JsonResponseBody;

pub struct ConsensusClient {
    pub config: AppConfig,
    reader: ArrowFileBlockReader,
    context: Arc<Mutex<ArrowBatchContext>>,
    execution_api: ExecutionApiClient,
    latest_consensus_block: ExecutionPayloadV1,
    latest_valid_executor_block: Block,
    is_forked: bool,
}

// TODO: make this a config parameter 
pub const BATCH_SIZE: usize = 200;

impl ConsensusClient {
    pub async fn new(config: AppConfig, context: Arc<Mutex<ArrowBatchContext>>) -> Self {
        let my_config = config.clone();
        let reader_context = context.clone();
        
        let reader = ArrowFileBlockReader::new(reader_context, 36);
        let execution_api = ExecutionApiClient::new(config.base_url, config.jwt_secret);
        let latest_consensus_block = ConsensusClient::get_latest_consensus_block(&reader);
        let latest_executor_block =
            ConsensusClient::get_latest_executor_block(&execution_api).await;

        Self {
            config: my_config,
            reader,
            context,
            execution_api,
            latest_consensus_block,
            latest_valid_executor_block: latest_executor_block,
            is_forked: true,
        }
    }

    pub async fn run(&mut self) {
        if !self.sync_block_info().await.unwrap_or(false) {
            println!("Failed to sync block info");
            return;
        }

        let mut next_block_number = self.latest_valid_executor_block.header.number.unwrap().to::<u64>() + 1;
        let mut batch_count = 0;
        let mut last_block_number = self.reader.get_latest_block().unwrap().block_number;

        loop {
            let mut caught_up = false;
            let to_block = if last_block_number > next_block_number + BATCH_SIZE as u64 {
                next_block_number + BATCH_SIZE as u64
            } else {
                caught_up = true;
                last_block_number
            };
            self.do_batch(next_block_number, to_block).await;
            batch_count += 1;
            if caught_up {
                println!("Caught up to latest block {}, sleeping for 5 seconds", last_block_number);
                // TODO: make this more live
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
    
    async fn do_batch(&self, from_block: u64, to_block: u64) {
        let mut next_block_number = from_block;
        let mut new_blocks: Vec<ExecutionPayloadV1>;
        
        while next_block_number < to_block {
            new_blocks = vec![];

            while new_blocks.len() < BATCH_SIZE {
                if let Some(block) = self.reader.get_block(next_block_number) {
                    next_block_number = block.block_number + 1;
                    new_blocks.push(block);
                } else {
                    break;
                }
            }

            if new_blocks.is_empty() {
                break;
            }

            let rpc_batch = new_blocks.iter().map(|block| {
                RpcRequest {
                    method: crate::execution_api_client::ExecutionApiMethod::NewPayloadV1,
                    params: json![vec![block]],
                }
            }).collect::<Vec<RpcRequest>>();

            let new_payloadv1_result = self.execution_api.rpc_batch(rpc_batch).await.unwrap();
            println!("NewPayloadV1 result: {:?}", new_payloadv1_result);

            let last_block_sent = new_blocks.last().unwrap();
            let fork_choice_updated_result = self.fork_choice_updated(
                last_block_sent.block_hash,
                last_block_sent.block_hash,
                last_block_sent.block_hash,
            ).await;

            println!("Fork choice updated called with:\nhash {:?}\nparentHash {:?}\nnumber {:?}", last_block_sent.block_hash, last_block_sent.parent_hash, last_block_sent.block_number);
            println!("fork_choice_updated_result for block number {}: {:?}", last_block_sent.block_number, fork_choice_updated_result);
        }
    }

    fn get_latest_consensus_block(reader: &ArrowFileBlockReader) -> ExecutionPayloadV1 {
        reader.get_latest_block().unwrap().clone()
    }

    async fn fork_choice_updated(&self, head_hash: B256, safe_hash: B256, finalized_hash: B256) -> JsonResponseBody {
        let fork_choice_state = ForkchoiceState {
            head_block_hash: head_hash,
            safe_block_hash: safe_hash,
            finalized_block_hash: finalized_hash,
        };

        let fork_choice_updated_result = self.execution_api.rpc(RpcRequest {
            method: crate::execution_api_client::ExecutionApiMethod::ForkChoiceUpdatedV1,
            params: json![vec![fork_choice_state]],
        }).await.unwrap();

        fork_choice_updated_result
    }

    async fn get_latest_executor_block(execution_api: &ExecutionApiClient) -> Block {
        let executor_latest_block_number = execution_api.block_number().await.unwrap();
        execution_api.block_by_number(executor_latest_block_number, false).await.unwrap()
    }

    async fn sync_block_info(&mut self) -> Result<bool, String> {
        self.latest_consensus_block = ConsensusClient::get_latest_consensus_block(&mut self.reader);
        self.latest_valid_executor_block =
            ConsensusClient::get_latest_executor_block(&self.execution_api).await;

        let consensus_for_latest_executor_block =
            self.reader.get_block(
                self.latest_valid_executor_block.header.number.unwrap().to::<u64>(),
            ).unwrap();

        println!("Consensus for latest executor block:\nhash {:?}\nparentHash {:?}\nnumber {:?}", consensus_for_latest_executor_block.block_hash, consensus_for_latest_executor_block.parent_hash, consensus_for_latest_executor_block.block_number);
        println!("Latest executor block:\nhash {:?}\nparentHash {:?}\nnumber {:?}", self.latest_valid_executor_block.header.hash.unwrap(), self.latest_valid_executor_block.header.parent_hash, self.latest_valid_executor_block.header.number.unwrap().to_string());
        let mut count = 0u64;
        let latest_executor_hash = self.latest_valid_executor_block.header.hash.unwrap();
        let consensus_for_latest_executor_hash = consensus_for_latest_executor_block.block_hash;

        self.is_forked = latest_executor_hash != consensus_for_latest_executor_hash;

        while self.is_forked {
            let latest_valid_executor_block_number =
                self.latest_valid_executor_block.header.number.unwrap().to::<u64>();

            println!("Forked and latest executor block number is {}, executor hash is {} but consensus hash is {}",
                     latest_valid_executor_block_number,
                     self.latest_valid_executor_block.header.hash.unwrap(),
                     consensus_for_latest_executor_hash);

            if self.latest_valid_executor_block.header.number.unwrap().to::<u64>() == 0 {
                println!("Forked all the way back to execution block 0, cannot go back further");
                return Err(
                    "Forked all the way back to execution block 0, cannot go back further"
                        .to_string(),
                );
            }
            count += 1;
            self.latest_valid_executor_block =
                self.execution_api.block_by_number(
                    latest_valid_executor_block_number - 1, false
                ).await.unwrap();

            let consensus_for_latest_executor_block =
                self.reader.get_block(
                    self.latest_valid_executor_block.header.number.unwrap().to::<u64>(),
                ).unwrap();

            self.is_forked = consensus_for_latest_executor_block.block_hash
                != self.latest_valid_executor_block.header.hash.unwrap();
        }

        Ok(true)
    }
}