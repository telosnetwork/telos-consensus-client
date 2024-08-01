use alloy::primitives::FixedBytes;
use antelope::api::client::{APIClient, DefaultProvider};
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};
use testcontainers::core::ContainerPort::Tcp;
use tokio::sync::mpsc;
use tracing::{debug, info};
use telos_translator_rs::{block::Block, translator::Translator};
use telos_translator_rs::translator::TranslatorConfig;
use telos_translator_rs::types::env::TESTNET_GENESIS_CONFIG;

#[tokio::test]
async fn evm_deploy() {

    // Change this container to a local image if using new ship data,
    //   then make sure to update the ship data in the testcontainer-nodeos-evm repo and build a new version

    // The tag for this image needs to come from the Github packages UI, under the "OS/Arch" tab
    //   and should be the tag for linux/amd64
    let container: ContainerAsync<GenericImage> = GenericImage::new(
        "ghcr.io/telosnetwork/testcontainer-nodeos-evm",
        "v0.1.3@sha256:d9f198f0885498936bf731bf6d84a1e1b425d79d4ef8249f8bd2b6b6aa534314")
        .with_exposed_port(Tcp(8888))
        .with_exposed_port(Tcp(18999))
        .start()
        .await
        .unwrap();

    let api_client = APIClient::<DefaultProvider>::default_provider(
        format!("http://localhost:{}", container.get_host_port_ipv4(8888).await.unwrap())
    ).unwrap();

    let mut last_block = 0;

    loop {
        let get_info = api_client.v1_chain.get_info().await;
        if let Ok(info) = get_info {
            if last_block != 0 && info.head_block_num > last_block {
                break;
            }
            last_block = info.head_block_num;
        }
        info!("Waiting for telos node to produce blocks...");
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    let config = TranslatorConfig {
        http_endpoint: format!("http://localhost:{}", container.get_host_port_ipv4(8888).await.unwrap()),
        ship_endpoint: format!("ws://localhost:{}", container.get_host_port_ipv4(18999).await.unwrap()),
        validate_hash: None,
        start_block: 30,
        stop_block: Some(75),
        block_delta: 0,
        ..TESTNET_GENESIS_CONFIG.clone()
    };

    tracing_subscriber::fmt::init();

    let (tx, mut rx) = mpsc::channel::<(FixedBytes<32>, Block)>(1000);

    let mut translator = Translator::new(config).await.unwrap();
    match translator.launch(Some(tx)).await {
        Ok(_) => info!("Translator launched successfully"),
        Err(e) => panic!("Failed to launch translator: {:?}", e)
    }

    while let Some((block_hash, block)) = rx.recv().await {
        // TODO: Make logging work
        // TODO: Add some example assertions against blocks/transactions
        info!("{}:{}", block.block_num, hex::encode(block_hash));
        if !block.transactions.is_empty() {
            info!("Block has transactions");
        }
    }
}
