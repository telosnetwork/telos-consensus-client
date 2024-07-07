use crate::block::Block;
use crate::tasks::{
    evm_block_processor, final_processor, order_preserving_queue, raw_deserializer, ship_reader,
};
use crate::types::ship_types::ShipRequest::{GetBlocksAck, GetStatus};
use crate::types::ship_types::{
    GetBlocksAckRequestV0, GetBlocksRequestV0, GetStatusRequestV0, ShipRequest, ShipResult,
};
use crate::types::types::{BlockOrSkip, PriorityQueue, RawMessage, WebsocketTransmitter};
use antelope::api::client::APIClient;
use antelope::api::default_provider::DefaultProvider;
use antelope::chain::{Decoder, Encoder};
use dashmap::DashMap;
use eyre::Result;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tracing::{error, info};

// Deposit block
//const START_BLOCK: u32 = 300000965;

// Withdraw block
// const START_BLOCK: u32 = 302247479;

// Short address in log
// const START_BLOCK: u32 = 300056989;

// Tx decode issue
pub const START_BLOCK: u32 = 300062700;
//const START_BLOCK: u32 = 300_000_000;
pub const STOP_BLOCK: u32 = 301_000_000;
pub const CHAIN_ID: u64 = 40;
pub const RAW_DS_THREADS: u8 = 4;
pub const BLOCK_PROCESS_THREADS: u8 = 4;

pub async fn write_message(
    tx_stream: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    message: &ShipRequest,
) {
    let bytes = Encoder::pack(message);
    tx_stream.lock().await.send(Message::Binary(bytes)).await.unwrap();
}

pub struct Translator {
    http_endpoint: String,
    ship_endpoint: String,
    ship_abi: Option<String>,
    latest_status: Option<Arc<ShipResult>>,
    block_map: Arc<DashMap<u32, Block>>,
}

impl Translator {
    pub async fn new(http_endpoint: String, ship_endpoint: String) -> Result<Self> {
        Ok(Self {
            http_endpoint,
            ship_endpoint,
            ship_abi: None,
            latest_status: None,
            block_map: Arc::new(DashMap::new()),
        })
    }

    pub async fn launch(&mut self) -> Result<()> {
        let api_client: APIClient<DefaultProvider> =
            APIClient::<DefaultProvider>::default_provider(self.http_endpoint.clone())
                .expect("Failed to create API client");

        let connect_result = connect_async(&self.ship_endpoint).await;
        if connect_result.is_err() {
            error!(
                "Failed to connect to ship at endpoint {}",
                &self.ship_endpoint
            );
            return Err(eyre::eyre!("Failed to connect to ship"));
        }

        let (ws_stream, _) = connect_result.unwrap();
        let (mut ws_tx, mut ws_rx): (
            SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
            SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        ) = ws_stream.split();

        // Buffer size here should be the readahead buffer size, in blocks.  This could get large if we are reading
        //  a block range with larges blocks/trxs, so this should be tuned based on the largest blocks we hit
        let (raw_ds_tx, raw_ds_rx) = mpsc::channel::<RawMessage>(10000);
        let (process_tx, process_rx) = mpsc::channel::<Block>(1000);
        let (order_tx, order_rx) = mpsc::channel::<BlockOrSkip>(1000);
        let (finalize_tx, finalize_rx) = mpsc::channel::<Block>(1000);

        tokio::task::spawn(ship_reader(ws_rx, raw_ds_tx));

        let raw_ds_rx = Arc::new(Mutex::new(raw_ds_rx));
        let ws_tx = Arc::new(Mutex::new(ws_tx));
        let process_rx = Arc::new(Mutex::new(process_rx));

        for thread_id in 0..RAW_DS_THREADS {
            let raw_ds_rx = raw_ds_rx.clone();
            let ws_tx = ws_tx.clone();
            tokio::task::spawn(raw_deserializer(thread_id, raw_ds_rx, ws_tx, process_tx.clone(), order_tx.clone()));
        }

        for _ in 0..BLOCK_PROCESS_THREADS {
            tokio::task::spawn(evm_block_processor(
                process_rx.clone(),
                order_tx.clone(),
                api_client.clone(),
            ));
        }

        // Shared queue for order preservation
        let queue = Arc::new(Mutex::new(BinaryHeap::new()));

        // Start the order-preserving queue task
        let queue_clone = queue.clone();
        tokio::spawn(order_preserving_queue(order_rx, finalize_tx, queue_clone));

        // Start the final processing task
        tokio::spawn(final_processor(finalize_rx));

        // Keep the main thread alive
        loop {
            tokio::task::yield_now().await;
        }

        Ok(())
    }
}
