use antelope::api::client::{APIClient, DefaultProvider};
use serde::Deserialize;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::Translator;
use telos_translator_rs::translator::TranslatorConfig;
use telos_translator_rs::types::env::TESTNET_GENESIS_CONFIG;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::ContainerPort::Tcp;
use testcontainers::core::WaitFor;
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};
use tokio::sync::mpsc;
use tracing::info;

mod common;

use crate::common::test_utils::compare_block;
use common::test_utils::load_15_data;

// TODO: Figure out the best end block for this test based on the container
const END_BLOCK: u32 = 35;

#[derive(Deserialize)]
struct NativeBlockIdentity {
    id: String,
    block_num: u32,
}

#[tokio::test]
async fn evm_deploy() {
    // Change this container to a local image if using new ship data,
    //   then make sure to update the ship data in the testcontainer-nodeos-evm repo and build a new version
    let valid_data = load_15_data().unwrap();

    // The tag for this image needs to come from the Github packages UI, under the "OS/Arch" tab
    //   and should be the tag for linux/amd64
    let container: ContainerAsync<GenericImage> = GenericImage::new(
        "ghcr.io/telosnetwork/testcontainer-nodeos-evm",
        "v0.1.7@sha256:54a6c1d9c75331f00115dae591d6abb6d0ba9568fb6c23402d1a9320dcf70be7",
    )
    .with_exposed_port(Tcp(8888))
    .with_exposed_port(Tcp(18999))
    .with_wait_for(WaitFor::Log(LogWaitStrategy::stderr("Produced")))
    .start()
    .await
    .unwrap();

    let port_8888 = container.get_host_port_ipv4(8888).await.unwrap();
    let port_18999 = container.get_host_port_ipv4(18999).await.unwrap();
    let api_client = APIClient::<DefaultProvider>::default_provider(
        format!("http://localhost:{port_8888}"),
        Some(10),
    )
    .unwrap();
    let native_info = api_client.v1_chain.get_info().await.unwrap();
    // antelope-rs' full block response still models `trx` as a string, while this pinned nodeos
    // fixture returns packed transaction objects. This test only needs the immutable block identity.
    let native_anchor = reqwest::Client::new()
        .post(format!("http://localhost:{port_8888}/v1/chain/get_block"))
        .json(&serde_json::json!({"block_num_or_id": "57"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<NativeBlockIdentity>()
        .await
        .unwrap();
    assert_eq!(native_anchor.block_num, 57);
    assert_eq!(hex::decode(&native_anchor.id).unwrap().len(), 32);

    let config = TranslatorConfig {
        http_endpoint: format!("http://localhost:{port_8888}",),
        ship_endpoint: format!("ws://localhost:{port_18999}",),
        native_chain_id: native_info.chain_id.as_string(),
        execution_anchor_native_block_number: 57,
        execution_anchor_native_block_hash: native_anchor.id,
        validate_hash: None,
        execution_context_anchor_block: 1,
        execution_context_starting_gas_price: "0".to_string(),
        execution_context_starting_revision: 0,
        evm_start_block: 1,
        evm_stop_block: Some(30),
        ..TESTNET_GENESIS_CONFIG.clone()
    };

    tracing_subscriber::fmt::init();

    let (tx, mut rx) = mpsc::channel::<TelosEVMBlock>(1000);

    let translator = Translator::new(config);
    let translator_shutdown = translator.shutdown_handle();

    let translator_handle = tokio::spawn(translator.launch(Some(tx)));

    while let Some(block) = rx.recv().await {
        if block.block_num > END_BLOCK {
            break;
        }
        info!("{}:{}", block.block_num, block.block_hash);
        // TODO: Some kind of assertion that all blocks in valid_data were seen

        if let Some(valid_block) = valid_data.get(&block.block_num) {
            compare_block(&block, valid_block);
        }
    }

    let _ = translator_shutdown.shutdown().await;
    let _ = translator_handle.await;
}
