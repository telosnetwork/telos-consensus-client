use tracing::{error, info};
use telos_translator_rs::{translator::Translator, types::env::MAINNET_DEPLOY_CONFIG};

#[tokio::test]
async fn evm_deploy() {
    let config = MAINNET_DEPLOY_CONFIG.clone();

    let mut translator = Translator::new(config).await.unwrap();
    match translator.launch().await {
        Ok(_) => info!("Translator launched successfully"),
        Err(e) => error!("Failed to launch translator: {:?}", e)
    }
}
