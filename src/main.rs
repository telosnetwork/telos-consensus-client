use clap::Parser;
use crate::config::{AppConfig, CliArgs};

mod block_reader;
mod execution_api_client;
mod config;

fn main() {
    let cli_args = CliArgs::parse();
    let config_contents = std::fs::read_to_string(cli_args.config).unwrap();
    let config: AppConfig = toml::from_str(&config_contents).unwrap();

    let reader = block_reader::FileBlockReader::new(config.blocks_csv);
    let block_zero = reader.get_block(0);
    println!("block_zero: {:?}", block_zero);
}

#[cfg(test)]
mod tests {
    use crate::block_reader::FileBlockReader;

    #[test]
    fn block_reader() {
        let reader = FileBlockReader::new();
        let block_zero = reader.get_block(0);
        assert!(block_zero.is_some());
    }
}
