use testcontainers::core::ContainerPort::Tcp;
use testcontainers::core::WaitFor;
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};

use tokio::sync::mpsc;

use tracing::info;

use crate::common::test_utils::{ChainDescriptor, JumpInfo};
use common::test_utils::LeapMockClient;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::{Translator, TranslatorConfig};
use telos_translator_rs::types::env::TESTNET_GENESIS_CONFIG;

mod common;

#[tokio::test]
async fn mock_fork() {
    tracing_subscriber::fmt::init();

    let control_port = 6970;
    let chain_http_port = 8889;
    let chain_ship_port = 18998;

    let container: ContainerAsync<GenericImage> = GenericImage::new(
        "guilledk/leap-mock",
        "0.4.0@sha256:b5890c72a25c50c397d15b0ba9e1b6d3d8f0d9c504ab87a698e536d55cef3cf3",
    )
    .with_exposed_port(Tcp(chain_http_port))
    .with_exposed_port(Tcp(chain_ship_port))
    .with_exposed_port(Tcp(control_port))
    .with_wait_for(WaitFor::message_on_stdout("Control server running on"))
    .start()
    .await
    .unwrap();

    let cntr_control_port = container.get_host_port_ipv4(control_port).await.unwrap();
    let cntr_http_port = container.get_host_port_ipv4(chain_http_port).await.unwrap();
    let cntr_ship_port = container.get_host_port_ipv4(chain_ship_port).await.unwrap();

    // configure a fork from block 30 to 25
    let mock_client = LeapMockClient::new(&format!("http://localhost:{cntr_control_port}"));

    let _chain_info = mock_client
        .set_chain(ChainDescriptor {
            chain_id: None,
            start_time: None,
            jumps: vec![
                // JumpInfo {from: 13, to: 13},  // 0 delta fork
                JumpInfo { from: 35, to: 20 }, // normal fork
                JumpInfo { from: 80, to: 3 },  // long fork
            ],
            chain_start_block: 1,
            chain_end_block: 100,
            session_start_block: 2,
            session_stop_block: 100,
            http_port: Some(chain_http_port),
            ship_port: Some(chain_ship_port),
        })
        .await
        .unwrap();

    let config = TranslatorConfig {
        http_endpoint: format!("http://localhost:{cntr_http_port}",),
        ship_endpoint: format!("ws://localhost:{cntr_ship_port}",),
        validate_hash: None,
        evm_start_block: 2,
        evm_stop_block: Some(99),
        block_delta: 0,
        ..TESTNET_GENESIS_CONFIG.clone()
    };

    let (tx, mut rx) = mpsc::channel::<TelosEVMBlock>(1000);

    let translator = Translator::new(config);
    match translator.launch(Some(tx)).await {
        Ok(_) => info!("Translator launched successfully"),
        Err(e) => panic!("Failed to launch translator: {:?}", e),
    }

    while let Some(block) = rx.recv().await {
        info!("{}:{}", block.block_num, block.block_hash);
    }
}
