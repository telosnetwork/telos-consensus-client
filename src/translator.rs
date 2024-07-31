use crate::block::Block;
use crate::tasks::{
    evm_block_processor, final_processor, order_preserving_queue, raw_deserializer, ship_reader,
};
use crate::types::ship_types::ShipRequest::{GetBlocksAck, GetStatus};
use crate::types::ship_types::{
    GetBlocksAckRequestV0, GetBlocksRequestV0, GetStatusRequestV0, ShipRequest, ShipResult,
};
use crate::types::types::{BlockOrSkip, PriorityQueue, RawMessage, WebsocketTransmitter};
use alloy::primitives::FixedBytes;
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
use serde::{Deserialize, Serialize};


pub const DEFAULT_RAW_DS_THREADS: u8 = 4;
pub const DEFAULT_BLOCK_PROCESS_THREADS: u8 = 4;

pub const DEFAULT_RAW_MESSAGE_CHANNEL_SIZE: usize = 10000;
pub const DEFAULT_BLOCK_PROCESS_CHANNEL_SIZE: usize = 1000;
pub const DEFAULT_MESSAGE_ORDERER_CHANNEL_SIZE: usize = 1000;
pub const DEFAULT_MESSAGE_FINALIZER_CHANNEL_SIZE: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslatorConfig {
    pub chain_id: u64,
    pub start_block: u32,
    pub stop_block: Option<u32>,
    pub block_delta: u32,
    pub prev_hash: String,
    pub validate_hash: Option<String>,

    pub http_endpoint: String,
    pub ship_endpoint: String,

    pub raw_ds_threads: Option<u8>,
    pub block_process_threads: Option<u8>,

    pub raw_message_channel_size: Option<usize>,
    pub block_message_channel_size: Option<usize>,
    pub order_message_channel_size: Option<usize>,
    pub final_message_channel_size: Option<usize>,
}

pub async fn write_message(
    tx_stream: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    message: &ShipRequest,
) {
    let bytes = Encoder::pack(message);
    tx_stream.lock().await.send(Message::Binary(bytes)).await.unwrap();
}

pub struct Translator {
    config: TranslatorConfig,
    ship_abi: Option<String>,
    latest_status: Option<Arc<ShipResult>>,
    block_map: Arc<DashMap<u32, Block>>,
}

impl Translator {
    pub async fn new(config: TranslatorConfig) -> Result<Self> {
        Ok(Self {
            config,
            ship_abi: None,
            latest_status: None,
            block_map: Arc::new(DashMap::new()),
        })
    }

    pub async fn launch(&mut self, output_tx: Option<mpsc::Sender<(FixedBytes<32>, Block)>>) -> Result<()> {
        let api_client: APIClient<DefaultProvider> =
            APIClient::<DefaultProvider>::default_provider(self.config.http_endpoint.clone())
                .expect("Failed to create API client");

        let connect_result = connect_async(&self.config.ship_endpoint).await;
        if connect_result.is_err() {
            error!(
                "Failed to connect to ship at endpoint {}",
                &self.config.ship_endpoint
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
        let (raw_ds_tx, raw_ds_rx) = mpsc::channel::<RawMessage>(
            self.config.raw_message_channel_size.unwrap_or(DEFAULT_RAW_MESSAGE_CHANNEL_SIZE));

        let (process_tx, process_rx) = mpsc::channel::<Block>(
            self.config.block_message_channel_size.unwrap_or(DEFAULT_BLOCK_PROCESS_CHANNEL_SIZE));

        let (order_tx, order_rx) = mpsc::channel::<BlockOrSkip>(
            self.config.order_message_channel_size.unwrap_or(DEFAULT_MESSAGE_ORDERER_CHANNEL_SIZE));

        let (finalize_tx, finalize_rx) = mpsc::channel::<Block>(
            self.config.final_message_channel_size.unwrap_or(DEFAULT_MESSAGE_FINALIZER_CHANNEL_SIZE));

        tokio::task::spawn(ship_reader(ws_rx, raw_ds_tx));

        let raw_ds_rx = Arc::new(Mutex::new(raw_ds_rx));
        let ws_tx = Arc::new(Mutex::new(ws_tx));
        let process_rx = Arc::new(Mutex::new(process_rx));

        let ship_abi_received = Arc::new(Mutex::new(false));

        for thread_id in 0..self.config.raw_ds_threads.unwrap_or(DEFAULT_RAW_DS_THREADS) {
            let raw_ds_rx = raw_ds_rx.clone();
            let ws_tx = ws_tx.clone();
            tokio::task::spawn(raw_deserializer(thread_id, self.config.clone(), ship_abi_received.clone(), raw_ds_rx, ws_tx, process_tx.clone(), order_tx.clone()));
        }

        for _ in 0..self.config.block_process_threads.unwrap_or(DEFAULT_BLOCK_PROCESS_THREADS) {
            tokio::task::spawn(evm_block_processor(
                process_rx.clone(),
                order_tx.clone()
            ));
        }

        // Shared queue for order preservation
        let queue = Arc::new(Mutex::new(BinaryHeap::new()));

        // Start the order-preserving queue task
        let queue_clone = queue.clone();
        tokio::spawn(order_preserving_queue(order_rx, finalize_tx, queue_clone));

        // Start the final processing task
        tokio::spawn(final_processor(self.config.clone(), api_client, finalize_rx, output_tx));

        Ok(())
    }
}
