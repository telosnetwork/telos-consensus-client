use std::sync::Arc;
use std::time::Instant;
use antelope::chain::abi::ABI;
use antelope::chain::{Decoder, Encoder};
use dashmap::DashMap;
use eyre::{Result};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use futures_util::{SinkExt, StreamExt};
use futures_util::stream::{SplitSink, SplitStream};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use crate::block_deserializer::block_deserializer;
use crate::block::Block;
use crate::evm_block_generator::evm_block_generator;
use crate::types::ship_types::{GetBlocksAckRequestV0, GetBlocksRequestV0, GetStatusRequestV0, ShipRequest, ShipResult};
use crate::types::ship_types::ShipRequest::{GetBlocksAck, GetStatus};
use crate::types::types::PriorityQueue;


const START_BLOCK: u32 = 300_000_000;
const STOP_BLOCK: u32 = 301_000_000;

pub async fn write_message(tx_stream: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, message: &ShipRequest) {
    let bytes = Encoder::pack(message);
    tx_stream.send(Message::Binary(bytes)).await.unwrap();
}

pub struct Translator {
    ship_endpoint: String,
    ship_abi: Option<ABI>,
    latest_status: Option<Arc<ShipResult>>,
    block_map: Arc<DashMap<u32, Block>>,
}

impl Translator {
    pub async fn new(ship_endpoint: String) -> Result<Self> {

        Ok(Self {
            ship_endpoint,
            ship_abi: None,
            latest_status: None,
            block_map: Arc::new(DashMap::new()),
        })
    }

    pub async fn launch(&mut self) -> Result<()> {
        let (ws_stream, _) = connect_async(self.ship_endpoint.as_str()).await?;
        let (mut ws_tx, mut ws_rx): (SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>) = ws_stream.split();

        let mut unackd_blocks = 0;
        let mut last_log = Instant::now();

        let block_queue = PriorityQueue::new();
        tokio::task::spawn(block_deserializer(block_queue.clone(), self.block_map.clone()));
        tokio::task::spawn(evm_block_generator(START_BLOCK, self.block_map.clone()));

        while let Some(msg_result) = ws_rx.next().await {
            match msg_result {
                Ok(msg) => {
                    // ABI is always the first message sent on connect
                    if self.ship_abi.is_none() {
                        let abi_string = msg.to_string();
                        let abi = ABI::from_string(abi_string.as_str()).unwrap();
                        self.ship_abi = Some(abi);

                        // Send GetStatus request after setting up the ABI
                        let request = GetStatus(GetStatusRequestV0);
                        write_message(&mut ws_tx, &request).await;
                    } else {
                        // Print received messages after ABI is set
                        let msg_data = msg.into_data();
                        //println!("Received message: {:?}", bytes_to_hex(&msg_data));
                        let mut decoder = Decoder::new(msg_data.as_slice());
                        let ship_result = &mut ShipResult::default();
                        decoder.unpack(ship_result);


                        match ship_result {
                            ShipResult::GetStatusResultV0(r) => {
                                println!("GetStatusResultV0: {:?}", r);
                                self.latest_status = Some(Arc::new(ShipResult::GetStatusResultV0(r.clone())));
                                write_message(&mut ws_tx, &ShipRequest::GetBlocks(GetBlocksRequestV0 {
                                    start_block_num: START_BLOCK,
                                    end_block_num: STOP_BLOCK,
                                    max_messages_in_flight: 1000,
                                    have_positions: vec![],
                                    irreversible_only: false,
                                    fetch_block: true,
                                    fetch_traces: true,
                                    fetch_deltas: true,
                                })).await;
                            }
                            ShipResult::GetBlocksResultV0(r) => {
                                unackd_blocks += 1;
                                if let Some(b) = &r.this_block {
                                    //println!("Got block: {}", b.block_num);
                                    let block = Block::new(b.block_num, r.clone());
                                    block_queue.push(block);
                                    // TODO: Better logic here, don't just ack every 10 blocks
                                    // TODO: Better threading so we don't block reading while we write?
                                    if b.block_num % 500 == 0 {
                                        println!("Processed {} blocks/sec", unackd_blocks as f64 / last_log.elapsed().as_secs_f64());
                                        last_log = Instant::now();
                                        write_message(&mut ws_tx, &GetBlocksAck(GetBlocksAckRequestV0 {
                                            num_messages: unackd_blocks
                                        })).await;
                                        unackd_blocks = 0;
                                    }
                                } else {
                                    // TODO: why would this happen?
                                    eprintln!("GetBlocksResultV0 without a block");
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    // Log the error and break the loop
                    println!("Error: {}", e);
                    break;
                },
            }
        }

        Ok(())
    }

}
