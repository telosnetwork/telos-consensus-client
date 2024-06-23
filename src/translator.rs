use std::sync::Arc;
use std::time::Instant;
use antelope::api::client::APIClient;
use antelope::api::default_provider::DefaultProvider;
use antelope::chain::{Decoder, Encoder};
use dashmap::DashMap;
use eyre::{Result};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use futures_util::{SinkExt, StreamExt};
use futures_util::stream::{SplitSink, SplitStream};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info};
use crate::tasks::{block_deserializer, evm_block_generator};
use crate::block::Block;
use crate::types::ship_types::{GetBlocksAckRequestV0, GetBlocksRequestV0, GetStatusRequestV0, ShipRequest, ShipResult};
use crate::types::ship_types::ShipRequest::{GetBlocksAck, GetStatus};
use crate::types::types::PriorityQueue;

// Deposit block
//const START_BLOCK: u32 = 300000965;

// Withdraw block
// const START_BLOCK: u32 = 302247479;

// Short address in log
// const START_BLOCK: u32 = 300056989;

// Tx decode issue
const START_BLOCK: u32 = 300062700;
//const START_BLOCK: u32 = 300_000_000;
const STOP_BLOCK: u32 = 301_000_000;
const CHAIN_ID: u64 = 40;

pub async fn write_message(tx_stream: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, message: &ShipRequest) {
    let bytes = Encoder::pack(message);
    tx_stream.send(Message::Binary(bytes)).await.unwrap();
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
        let connect_result = connect_async(self.ship_endpoint.as_str()).await;
        if connect_result.is_err() {
            error!("Failed to connect to ship at endpoint {}", self.ship_endpoint.as_str());
            return Err(eyre::eyre!("Failed to connect to ship"));
        }

        let (ws_stream, _) = connect_result.unwrap();
        let (mut ws_tx, mut ws_rx): (SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>) = ws_stream.split();
        let api_client: APIClient<DefaultProvider> = APIClient::<DefaultProvider>::default_provider(self.http_endpoint.clone()).expect("Failed to create API client");

        let mut unackd_blocks = 0;
        let mut last_log = Instant::now();
        let mut unlogged_blocks = 0;

        let block_queue = PriorityQueue::new();
        // TODO: Some better task management, if evm_block_generator crashes, we need to stop or slow down the reader/deserializer or the block_map grows forever
        tokio::task::spawn(block_deserializer(block_queue.clone(), self.block_map.clone()));
        tokio::task::spawn(evm_block_generator(START_BLOCK, self.block_map.clone(), api_client));

        while let Some(msg_result) = ws_rx.next().await {
            match msg_result {
                Ok(msg) => {
                    // ABI is always the first message sent on connect
                    if self.ship_abi.is_none() {
                        // TODO: maybe get this working as an ABI again?
                        //   the problem is that the ABI from ship has invalid table names like `account_metadata`
                        //   which cause from_string to fail, but if you change AbiTable.name to a String then
                        //   when you use the ABI struct to pack for a contract deployment, it causes the table
                        //   lookups via v1/chain/get_table_rows to fail because it doesn't like the string when
                        //   it's trying to determine the index type of a table
                        let abi_string = msg.to_string();
                        //let abi = ABI::from_string(abi_string.as_str()).unwrap();
                        self.ship_abi = Some(abi_string);

                        // Send GetStatus request after setting up the ABI
                        let request = GetStatus(GetStatusRequestV0);
                        write_message(&mut ws_tx, &request).await;
                    } else {
                        // Print received messages after ABI is set
                        let msg_data = msg.into_data();
                        //info!("Received message: {:?}", bytes_to_hex(&msg_data));
                        // TODO: Better threading so we don't block reading while deserialize?
                        let mut decoder = Decoder::new(msg_data.as_slice());
                        let ship_result = &mut ShipResult::default();
                        decoder.unpack(ship_result);

                        match ship_result {
                            ShipResult::GetStatusResultV0(r) => {
                                info!("GetStatusResultV0 head: {:?} last_irreversible: {:?}", r.head.block_num, r.last_irreversible.block_num);
                                self.latest_status = Some(Arc::new(ShipResult::GetStatusResultV0(r.clone())));
                                write_message(&mut ws_tx, &ShipRequest::GetBlocks(GetBlocksRequestV0 {
                                    start_block_num: START_BLOCK,
                                    end_block_num: STOP_BLOCK,
                                    max_messages_in_flight: 1000,
                                    have_positions: vec![],
                                    irreversible_only: true,  // TODO: Fork handling
                                    fetch_block: true,
                                    fetch_traces: true,
                                    fetch_deltas: true,
                                })).await;
                            }
                            ShipResult::GetBlocksResultV0(r) => {
                                unackd_blocks += 1;
                                if let Some(b) = &r.this_block {
                                    let block = Block::new(CHAIN_ID, b.block_num, b.block_id, r.clone());
                                    block_queue.push(block);
                                    if last_log.elapsed().as_secs_f64() > 10.0 {
                                        info!("Block #{} - rocessed {} blocks/sec", b.block_num, (unlogged_blocks + unackd_blocks) as f64 / last_log.elapsed().as_secs_f64());
                                        info!("Block queue size: {} with capacity: {}", block_queue.len(), block_queue.capacity());
                                        unlogged_blocks = 0;
                                        last_log = Instant::now();
                                    }

                                    // TODO: Better logic here, don't just ack every N blocks, do this based on backpressure
                                    if b.block_num % 200 == 0 {
                                        //info!("Acking {} blocks", unackd_blocks);
                                        // TODO: Better threading so we don't block reading while we write?
                                        write_message(&mut ws_tx, &GetBlocksAck(GetBlocksAckRequestV0 {
                                            num_messages: unackd_blocks
                                        })).await;
                                        //info!("Blocks acked");
                                        unlogged_blocks += unackd_blocks;
                                        unackd_blocks = 0;

                                    }
                                } else {
                                    // TODO: why would this happen?
                                    error!("GetBlocksResultV0 without a block");
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    // Log the error and break the loop
                    info!("Error: {}", e);
                    break;
                },
            }
        }

        Ok(())
    }

}
