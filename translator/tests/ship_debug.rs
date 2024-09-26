use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::Translator;
use telos_translator_rs::translator::TranslatorConfig;
use telos_translator_rs::types::env::{MAINNET_GENESIS_CONFIG, TESTNET_GENESIS_CONFIG};
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::ContainerPort::Tcp;
use testcontainers::core::WaitFor;
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};
use tokio::sync::mpsc;
use tracing::info;

mod common;

use crate::common::test_utils::compare_block;
use common::test_utils::load_15_data;

#[tokio::test]
async fn test_ship() {
    let config = TranslatorConfig {
        http_endpoint: format!("http://video.jtbuice.com:8888",),
        ship_endpoint: format!("ws://video.jtbuice.com:19000",),
        validate_hash: None,
        evm_start_block: 182376723 - 36,
        evm_stop_block: Some(182377823),
        ..MAINNET_GENESIS_CONFIG.clone()
    };

    tracing_subscriber::fmt::init();

    let (tx, mut rx) = mpsc::channel::<TelosEVMBlock>(1000);

    let translator = Translator::new(config);

    let handle = tokio::spawn(async move {
        while let Some(block) = rx.recv().await {
            if block.block_num == 182376723 {
                info!("{block:?}");
            }
            info!("{}:{}", block.block_num, block.block_hash);
        }
    });

    match translator.launch(Some(tx)).await {
        Ok(_) => info!("Translator launched successfully"),
        Err(e) => panic!("Failed to launch translator: {:?}", e),
    }
}
