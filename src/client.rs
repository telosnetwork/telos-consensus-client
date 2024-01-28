use crate::block_reader::FileBlockReader;
use crate::config::AppConfig;
use crate::execution_api_client::ExecutionApiClient;

pub struct ConsensusClient {
    pub config: AppConfig,
    reader: FileBlockReader,
    execution_api: ExecutionApiClient,
}

impl ConsensusClient {
    pub fn new(config: AppConfig) -> Self {
        let my_config = config.clone();
        let reader = crate::block_reader::FileBlockReader::new(my_config.blocks_csv.clone());
        let execution_api = ExecutionApiClient::new(config.base_url, config.jwt_secret);
        Self { config: my_config, reader, execution_api }
    }

    pub async fn run(&self) {
        let executor_latest_block = self.execution_api.block_number().await.unwrap();
        let next_block = self.reader.get_block(executor_latest_block + 1).unwrap();
        let result = self
            .execution_api
            .rpc(
                crate::execution_api_client::ExecutionApiMethod::NewPayloadV1,
                Some(vec![next_block]),
            )
            .await;
        println!("result: {:?}", result);
        println!("next_block: {:?}", next_block);
    }

}
