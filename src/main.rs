use crate::config::{AppConfig, CliArgs};
use clap::Parser;

mod auth;
mod block_reader;
mod config;
mod execution_api_client;
mod json_rpc;

#[tokio::main]
async fn main() {
    let cli_args = CliArgs::parse();
    let config_contents = std::fs::read_to_string(cli_args.config).unwrap();
    let config: AppConfig = toml::from_str(&config_contents).unwrap();

    let reader = block_reader::FileBlockReader::new(config.blocks_csv);
    let block_zero = reader.get_block(0).unwrap();
    let execution_api_client =
        execution_api_client::ExecutionApiClient::new(config.base_url, config.jwt_secret);
    let result = execution_api_client
        .rpc(
            execution_api_client::ExecutionApiMethod::NewPayloadV1,
            vec![block_zero],
        )
        .await;
    println!("result: {:?}", result);
    println!("block_zero: {:?}", block_zero);
}

#[cfg(test)]
mod tests {
    use crate::block_reader::FileBlockReader;

    #[test]
    fn block_reader() {
        let reader = FileBlockReader::new("blocks.csv".to_string());
        let block_zero = reader.get_block(0);
        assert!(block_zero.is_some());
    }
}
