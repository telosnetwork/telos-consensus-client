use alloy::primitives::FixedBytes;
use tokio::sync::mpsc;
use tracing::{debug, info};
use telos_translator_rs::{block::Block, translator::Translator, types::env::MAINNET_DEPLOY_CONFIG};

#[tokio::test]
async fn evm_deploy() {
    let config = MAINNET_DEPLOY_CONFIG.clone();

    tracing_subscriber::fmt::init();

    let (tx, mut rx) = mpsc::channel::<(FixedBytes<32>, Block)>(1000);

    let mut translator = Translator::new(config).await.unwrap();
    match translator.launch(Some(tx)).await {
        Ok(_) => info!("Translator launched successfully"),
        Err(e) => panic!("Failed to launch translator: {:?}", e)
    }

    while let Some((block_hash, block)) = rx.recv().await {
        debug!("{}:{}", block.block_num, hex::encode(block_hash));
    }
}
