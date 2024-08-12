use antelope::api::client::{APIClient, DefaultProvider};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use reth_primitives::B256;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use telos_consensus_client::client::ConsensusClient;
use telos_consensus_client::config::AppConfig;
use telos_consensus_client::json_rpc::JsonRequestBody;
use telos_translator_rs::translator::TranslatorConfig;
use telos_translator_rs::types::env::TESTNET_GENESIS_CONFIG;
use testcontainers::core::ContainerPort::Tcp;
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage};
use tokio::sync::oneshot::Sender;
use tokio::sync::{oneshot, Mutex};
use tracing::info;

const MOCK_EXECUTION_API_PORT: u16 = 3000;

#[derive(Debug, Clone)]
struct MockExecutionApi {
    pub port: u16,
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

    (StatusCode::OK, "".to_string())
}

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
    };

    let shutdown_tx = mock_execution_api.start().await;

    let config = AppConfig {
        execution_endpoint: format!("http://localhost:{MOCK_EXECUTION_API_PORT}"),
        jwt_secret: "57ea261c64b8a871e4df3f0c790efd0d02f9846e5085483e3098f30118fd1520".to_string(),
        ship_endpoint: format!("ws://localhost:{port_18999}"),
        chain_endpoint: format!("http://localhost:{port_8888}"),
        batch_size: 5,
        block_delta: None,
        prev_hash: B256::ZERO.to_string(),
        start_block: 0,
        stop_block: Some(60),
    };

    let tconfig = TranslatorConfig {
        http_endpoint: format!("http://localhost:{port_8888}",),
        ship_endpoint: format!("ws://localhost:{port_18999}",),
        validate_hash: None,
        start_block: 30,
        stop_block: Some(54),
        block_delta: 0,
        ..TESTNET_GENESIS_CONFIG.clone()
    };

    let mut client_under_test = ConsensusClient::new(config).await;
    let _ = client_under_test.run().await;

    // Shutdown mock execution API
    shutdown_tx.send(()).unwrap();
}
