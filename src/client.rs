use alloy_primitives::B256;
use crate::block_reader::FileBlockReader;
use crate::config::AppConfig;
use crate::execution_api_client::{ExecutionApiClient, RpcRequest};
use crate::sequential_block_reader::SequentialFileBlockReader;
use reth_rpc_types::{Block, ExecutionPayloadV1};
use reth_rpc_types::engine::ForkchoiceState;
use serde_json::json;
use crate::json_rpc::JsonResponseBody;

pub struct ConsensusClient {
    pub config: AppConfig,
    reader: FileBlockReader,
    sequential_reader: SequentialFileBlockReader,
    execution_api: ExecutionApiClient,
    latest_consensus_block: ExecutionPayloadV1,
    latest_valid_executor_block: Block,
    is_forked: bool,
}

impl ConsensusClient {
    pub async fn new(config: AppConfig) -> Self {
        let my_config = config.clone();
        let reader = crate::block_reader::FileBlockReader::new(my_config.blocks_csv.clone());
        let sequential_reader = crate::sequential_block_reader::SequentialFileBlockReader::new(my_config.blocks_bytes.clone());
        let execution_api = ExecutionApiClient::new(config.base_url, config.jwt_secret);
        let latest_consensus_block = ConsensusClient::get_latest_consensus_block(&reader);
        let latest_executor_block =
            ConsensusClient::get_latest_executor_block(&execution_api).await;

        Self {
            config: my_config,
            reader,
            sequential_reader,
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

        let mut last_block: Option<ExecutionPayloadV1> = None;
        let mut next_block_number = self.latest_valid_executor_block.header.number.unwrap().to::<u64>() + 1;

        const BATCH_SIZE: usize = 200;
        let mut new_blocks: Vec<ExecutionPayloadV1>;
        let mut batch_count = 0;

        last_block = self.sequential_reader.get_block(next_block_number);
        while last_block.as_ref().is_some() {
            batch_count += 1;
            new_blocks = vec![];

            while new_blocks.len() < BATCH_SIZE {
                last_block = self.sequential_reader.get_block(next_block_number);
                if last_block.as_ref().is_none() {
                    break;
                }
                next_block_number = last_block.as_ref().unwrap().block_number + 1;
                new_blocks.push(last_block.as_ref().unwrap().clone());
            }
            if last_block.as_ref().is_none() {
                break;
            }

            //new_blocks.reverse();

            let rpc_batch = new_blocks.iter().map(|block| {
                RpcRequest {
                    method: crate::execution_api_client::ExecutionApiMethod::NewPayloadV1,
                    params: json![vec![block]],
                }
            }).collect::<Vec<RpcRequest>>();

            let new_payloadv1_result = self.execution_api.rpc_batch(
               rpc_batch
            ).await.unwrap();
            println!("NewPayloadV1 result for batch {}: {:?}", batch_count, new_payloadv1_result);

            let last_block_sent = last_block.clone().unwrap();
            let fork_choice_updated_result = self.fork_choice_updated(
                last_block_sent.block_hash,
                last_block_sent.block_hash,
                last_block_sent.block_hash,
            ).await;

            println!("fork_choice_updated_result for block number {}: {:?}", last_block_sent.block_number, fork_choice_updated_result);

        }

    }

    fn get_latest_consensus_block(reader: &FileBlockReader) -> ExecutionPayloadV1 {
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
        self.latest_consensus_block = ConsensusClient::get_latest_consensus_block(&self.reader);
        self.latest_valid_executor_block =
            ConsensusClient::get_latest_executor_block(&self.execution_api).await;

        let mut consensus_for_latest_executor_block =
            self.sequential_reader.get_block(
                self.latest_valid_executor_block.header.number.unwrap().to::<u64>(),
            ).unwrap();

        println!("Consensus for latest executor block: {:?}", consensus_for_latest_executor_block);
        println!("Latest executor block: {:?}", self.latest_valid_executor_block);
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

            consensus_for_latest_executor_block =
                self.sequential_reader.get_block(
                    self.latest_valid_executor_block.header.number.unwrap().to::<u64>(),
                ).unwrap();

            self.is_forked = consensus_for_latest_executor_block.block_hash
                != self.latest_valid_executor_block.header.hash.unwrap();
        }

        Ok(true)
    }
}
