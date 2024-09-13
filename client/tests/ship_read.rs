use antelope::api::client::{APIClient, DefaultProvider};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use reth_primitives::B256;
use serde_json::Value;
use std::sync::Arc;
use telos_consensus_client::client::{ConsensusClient, Error};
use telos_consensus_client::config::{AppConfig, CliArgs};
use telos_consensus_client::json_rpc::JsonRequestBody;
use telos_translator_rs::block::TelosEVMBlock;
use telos_translator_rs::translator::Translator;
use testcontainers::core::ContainerPort::Tcp;
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};
use tokio::sync::oneshot::Sender;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::info;

const MOCK_EXECUTION_API_PORT: u16 = 3000;

#[derive(Debug, Clone)]
struct MockExecutionApi {
    pub port: u16,
    pub request_count: usize,
}

impl MockExecutionApi {
    pub async fn start(&mut self) -> Sender<()> {
        let state = Arc::new(Mutex::new(self.clone()));

        // build our application with a route
        let app = Router::new()
            .route(
                "/",
                post(
                    |state: State<Arc<Mutex<MockExecutionApi>>>, payload: Json<Value>| async move {
                        handle_post(state, payload).await
                    },
                ),
            )
            .with_state(state);

        // Create a TCP listener
        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", self.port))
            .await
            .unwrap();

        // Create a channel for shutdown signal
        let (tx, rx) = oneshot::channel::<()>();

        // Spawn the server task
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    rx.await.ok();
                })
                .await
                .unwrap();
        });

        // Return the shutdown signal sender
        tx
    }
}

async fn handle_post(
    state: State<Arc<Mutex<MockExecutionApi>>>,
    payload: Json<Value>,
) -> (StatusCode, String) {
    info!("Received payload: {:?}", payload);
    let payload_str = payload.to_string();
    let json_rpc_payload: JsonRequestBody = serde_json::from_str(&payload_str).unwrap();
    let request_number;
    {
        let mut state_unlocked = state.lock().await;
        request_number = state_unlocked.request_count;
        state_unlocked.request_count = request_number + 1;
    }
    match request_number {
        0 => {
            assert_eq!(
                json_rpc_payload.method, "eth_blockNumber",
                "Method is not eth_blockNumber"
            );
            assert!(json_rpc_payload.params.is_array(), "Params is not an array");
            assert_eq!(
                json_rpc_payload.params.as_array().unwrap().len(),
                0,
                "Params array is not of length 0"
            );
            (
                StatusCode::OK,
                r#"{
                "jsonrpc": "2.0",
                "error": null,
                "result": "0x0",
                "id": 1
            }"#
                    .to_string(),
            )
        }
        1 => {
            assert_eq!(
                json_rpc_payload.method, "eth_getBlockByNumber",
                "Method is not eth_getBlockByNumber"
            );
            assert!(json_rpc_payload.params.is_array(), "Params is not an array");
            let params = json_rpc_payload.params.as_array().unwrap();
            assert_eq!(params.len(), 2, "Params array is not of length 2");
            assert_eq!(params.first().unwrap(), "0x0", "First param is not 0x0");
            (StatusCode::OK, r#"{
                "jsonrpc": "2.0",
                "error": null,
                "result": {
                    "baseFeePerGas": "0x3b9aca00",
                    "blobGasUsed": "0x0",
                    "difficulty": "0x0",
                    "excessBlobGas": "0x0",
                    "extraData": "0x00",
                    "gasLimit": "0x1c9c380",
                    "gasUsed": "0x0",
                    "hash": "0xeb1b77e3581557e7c9ca99f7816a91545f90af91694db379eae408d13283c433",
                    "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                    "miner": "0x0000000000000000000000000000000000000000",
                    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "nonce": "0x0000000000000000",
                    "number": "0x0",
                    "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "receiptsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                    "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
                    "size": "0x243",
                    "stateRoot": "0xe1f1931c36e725ee55e8f6275fe3ba50c4fa93170db75fe4ee9ae49d7c0c7a16",
                    "timestamp": "0x0",
                    "totalDifficulty": "0x0",
                    "transactions": [],
                    "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                    "uncles": [],
                    "withdrawals": [],
                    "withdrawalsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
                    },
                "id": 1
            }"#.to_string())
        }
        2 => (
            StatusCode::OK,
            r#"{
                "jsonrpc": "2.0",
                "error": null,
                "result": "0x0",
                "id": 1
            }"#
                .to_string(),
        ),
        // TODO: handle more requests
        _ => {
            panic!("More requests than expected");
        }
    }
}

#[ignore]
#[tokio::test]
async fn evm_deploy() {
    tracing_subscriber::fmt::init();
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

    let mut mock_execution_api = MockExecutionApi {
        port: MOCK_EXECUTION_API_PORT,
        request_count: 0,
    };

    let shutdown_tx = mock_execution_api.start().await;

    let args = CliArgs {
        config: "config.toml".to_string(),
        clean: true,
    };

    let config = AppConfig {
        log_level: "debug".to_string(),
        chain_id: 41,
        execution_endpoint: format!("http://localhost:{MOCK_EXECUTION_API_PORT}"),
        jwt_secret: "57ea261c64b8a871e4df3f0c790efd0d02f9846e5085483e3098f30118fd1520".to_string(),
        ship_endpoint: format!("ws://localhost:{port_18999}"),
        chain_endpoint: format!("http://localhost:{port_8888}"),
        batch_size: 5,
        block_delta: None,
        prev_hash: B256::ZERO.to_string(),
        start_block: 0,
        validate_hash: None,
        stop_block: Some(60),
        data_path: "temp/db".to_string(),
        block_checkpoint_interval: 1000,
        maximum_sync_range: 100000,
        latest_blocks_in_db_num: 100,
        max_retry: None,
        retry_interval: None,
    };

    let mut client_under_test = ConsensusClient::new(&args, config.clone()).await.unwrap();

    let (tx, rx) = mpsc::channel::<TelosEVMBlock>(1000);
    let translator = Translator::new((&config.clone()).into());
    let tr_shutdown_tx = translator.shutdown_tx();

    let launch_handle = tokio::spawn(async move {
        translator
            .launch(Some(tx))
            .await
            .map_err(|_| Error::SpawnTranslator)
    });

    let (sender, receiver) = oneshot::channel();
    let _ = client_under_test.run(receiver, rx, None).await;

    // Shutdown
    shutdown_tx.send(()).unwrap();
    client_under_test
        .shutdown(sender, tr_shutdown_tx)
        .await
        .unwrap();
    launch_handle.await.unwrap().expect("Can't wait for launch");
}
