use alloy::hex;
use alloy::primitives::{address, TxKind, B256, U256};
use antelope::api::client::{APIClient, DefaultProvider};
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::transaction::Transaction;
use telos_translator_rs::translator::Translator;
use telos_translator_rs::translator::TranslatorConfig;
use telos_translator_rs::types::env::TESTNET_GENESIS_CONFIG;
use testcontainers::core::ContainerPort::Tcp;
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};
use tokio::sync::mpsc;
use tracing::info;

#[tokio::test]
async fn evm_deploy() {
    // Change this container to a local image if using new ship data,
    //   then make sure to update the ship data in the testcontainer-nodeos-evm repo and build a new version

    // The tag for this image needs to come from the Github packages UI, under the "OS/Arch" tab
    //   and should be the tag for linux/amd64
    let container: ContainerAsync<GenericImage> = GenericImage::new(
        "ghcr.io/telosnetwork/testcontainer-nodeos-evm",
        "v0.1.4@sha256:a8dc857e46404d74b286f8c8d8646354ca6674daaaf9eb6f972966052c95eb4a",
    )
    .with_exposed_port(Tcp(8888))
    .with_exposed_port(Tcp(18999))
    .start()
    .await
    .unwrap();

    let port_8888 = container.get_host_port_ipv4(8888).await.unwrap();
    let port_18999 = container.get_host_port_ipv4(18999).await.unwrap();

    let api_base_url = format!("http://localhost:{port_8888}");
    let api_client = APIClient::<DefaultProvider>::default_provider(api_base_url).unwrap();

    let mut last_block = 0;

    loop {
        let Ok(info) = api_client.v1_chain.get_info().await else {
            println!("Waiting for telos node to produce blocks...");
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            continue;
        };
        if last_block != 0 && info.head_block_num > last_block {
            break;
        }
        last_block = info.head_block_num;
    }

    let config = TranslatorConfig {
        http_endpoint: format!("http://localhost:{port_8888}",),
        ship_endpoint: format!("ws://localhost:{port_18999}",),
        validate_hash: None,
        start_block: 0,
        stop_block: Some(54),
        block_delta: 0,
        ..TESTNET_GENESIS_CONFIG.clone()
    };

    tracing_subscriber::fmt::init();

    let (tx, mut rx) = mpsc::channel::<TelosEVMBlock>(1000);

    let mut translator = Translator::new(config);
    match translator.launch(Some(tx)).await {
        Ok(_) => info!("Translator launched successfully"),
        Err(e) => panic!("Failed to launch translator: {:?}", e),
    }

    while let Some(block) = rx.recv().await {
        info!("{}:{}", block.block_num, block.block_hash);

        match block.block_num {
            50 => {
                let tx = block.transactions[0].clone();
                assert_eq!(
                    tx.hash(),
                    &B256::new(hex!(
                        "ede91f8a618cd49907d9a90fe2bf0443848f5ff549369eac42d1978b4fb8eccc"
                    ))
                );
                match tx {
                    Transaction::LegacySigned(signed_legacy, _) => {
                        let trx = signed_legacy.clone().strip_signature();
                        assert_eq!(trx.value, U256::from(100190020000000000000000000u128));
                        match trx.to {
                            TxKind::Create => {
                                panic!("Block 50 trx[0] should be a call, not create");
                            }
                            TxKind::Call(addr) => {
                                assert_eq!(
                                    addr,
                                    address!("d80744e16d62c62c5fa2a04b92da3fe6b9efb523")
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        // Leaving this here to fetch data about transactions in future data sets
        // if !block.transactions.is_empty() {
        //     info!("Block has transactions");
        //     for t in block.transactions {
        //         info!("Transaction hash: {:?}", t.hash());
        //         match t { Transaction::LegacySigned(signed_legacy, receipt) => {
        //             info!("Legacy signed transaction value: {:?}", signed_legacy.clone().strip_signature().value);
        //             info!("Legacy signed transaction to: {:?}", signed_legacy.clone().strip_signature().to);
        //             info!("Done!");
        //         } }
        //     }
        // }
    }
}
