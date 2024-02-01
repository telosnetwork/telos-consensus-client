use crate::block_reader::FileBlockReader;
use crate::config::AppConfig;
use crate::execution_api_client::ExecutionApiClient;
use reth_rpc_types::{Block, ExecutionPayloadV1};
use serde_json::json;

pub struct ConsensusClient {
    pub config: AppConfig,
    reader: FileBlockReader,
    execution_api: ExecutionApiClient,
    latest_consensus_block: ExecutionPayloadV1,
    latest_valid_executor_block: Block,
    is_forked: bool,
}

impl ConsensusClient {
    pub async fn new(config: AppConfig) -> Self {
        let my_config = config.clone();
        let reader = crate::block_reader::FileBlockReader::new(my_config.blocks_csv.clone());
        let execution_api = ExecutionApiClient::new(config.base_url, config.jwt_secret);
        let latest_consensus_block = ConsensusClient::get_latest_consensus_block(&reader);
        let latest_executor_block =
            ConsensusClient::get_latest_executor_block(&execution_api).await;

        Self {
            config: my_config,
            reader,
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

        let next_block =
            self.reader.get_block(
                self.latest_valid_executor_block.header.number.unwrap().to::<u64>() + 1,
            ).unwrap();

        let result = self.execution_api.rpc(
                crate::execution_api_client::ExecutionApiMethod::NewPayloadV1,
                json![next_block],
            ).await;

        println!("result: {:?}", result);
        println!("next_block: {:?}", next_block);
    }

    fn get_latest_consensus_block(reader: &FileBlockReader) -> ExecutionPayloadV1 {
        reader.get_latest_block().unwrap().clone()
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
            self.reader.get_block(
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
                self.reader.get_block(
                    self.latest_valid_executor_block.header.number.unwrap().to::<u64>(),
                ).unwrap();

            self.is_forked = consensus_for_latest_executor_block.block_hash
                != self.latest_valid_executor_block.header.hash.unwrap();
        }

        Ok(true)
    }
}
