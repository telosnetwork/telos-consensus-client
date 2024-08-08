use testcontainers::core::WaitFor;
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};
use testcontainers::core::ContainerPort::Tcp;

use tokio::sync::mpsc;

use tracing::info;

use alloy::hex;
use alloy::primitives::FixedBytes;

use telos_translator_rs::block::Block;
use telos_translator_rs::translator::{Translator, TranslatorConfig};
use telos_translator_rs::types::env::TESTNET_GENESIS_CONFIG;
use telos_translator_rs::test_utils::{LeapMockClient, SetJumpsParams};


#[tokio::test]
async fn mock_fork() {

    tracing_subscriber::fmt::init();

    let control_port = 6970;
    let chain_http_port = 8889;
    let chain_ship_port = 18998;

    let container: ContainerAsync<GenericImage> = GenericImage::new(
        "guilledk/leap-mock",
        "0.2.2@sha256:05ade342597fca9f14cbed6f22508789b1d8a1dc35f2aa64c46b1a8a96d5ae5d",
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
    // currently running this with the fork on will cause translator to stall
    // disable the mock_client related code to see translator reach end of fake chain
    let mock_client = LeapMockClient::new(&format!("http://localhost:{cntr_control_port}"));

    mock_client.set_jumps(SetJumpsParams{
        jumps: vec![(30, 25)]
    }).await.unwrap();

    let config = TranslatorConfig {
        http_endpoint: format!("http://localhost:{cntr_http_port}",),
        ship_endpoint: format!("ws://localhost:{cntr_ship_port}",),
        validate_hash: None,
        start_block: 1,
        stop_block: Some(99),
        block_delta: 0,
        ..TESTNET_GENESIS_CONFIG.clone()
    };

    let (tx, mut rx) = mpsc::channel::<(FixedBytes<32>, Block)>(1000);

    let mut translator = Translator::new(config);
    match translator.launch(Some(tx)).await {
        Ok(_) => info!("Translator launched successfully"),
        Err(e) => panic!("Failed to launch translator: {:?}", e),
    }

    while let Some((block_hash, block)) = rx.recv().await {
        info!("{}:{}", block.block_num, hex::encode(block_hash));
    }
}

