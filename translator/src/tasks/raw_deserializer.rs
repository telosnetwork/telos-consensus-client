use crate::block::{ProcessingEVMBlock, ProcessingEVMBlockArgs};
use crate::translator::TranslatorConfig;
use crate::types::ship_types::ShipRequest::{GetBlocksAck, GetStatus};
use crate::types::ship_types::{
    GetBlocksAckRequestV0, GetBlocksRequestV0, GetStatusRequestV0, ShipRequest, ShipResult,
};
use antelope::chain::Decoder;
use eyre::{eyre, Result};
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info};

pub async fn raw_deserializer(
    config: TranslatorConfig,
    mut raw_ds_rx: Receiver<Vec<u8>>,
    mut ws_tx: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    block_deserializer_tx: Sender<ProcessingEVMBlock>,
) -> Result<()> {
    let mut unackd_blocks = 0;
    let mut last_log = Instant::now();
    let mut unlogged_blocks = 0;
    let block_delta = config.chain_id.block_delta();

    // TODO: maybe get this working as an ABI again?
    //   the problem is that the ABI from ship has invalid table names like `account_metadata`
    //   which cause from_string to fail, but if you change AbiTable.name to a String then
    //   when you use the ABI struct to pack for a contract deployment, it causes the table
    //   lookups via v1/chain/get_table_rows to fail because it doesn't like the string when
    //   it's trying to determine the index type of a table
    //let abi_string = msg.to_string();
    //let abi = ABI::from_string(abi_string.as_str()).unwrap();
    //self.ship_abi = Some(abi_string);
    let _ = raw_ds_rx.recv().await.ok_or(eyre!("cannot send"))?;

    // Send GetStatus request after setting up the ABI
    let request = &GetStatus(GetStatusRequestV0);
    ws_tx.send(request.into()).await?;

    debug!("Raw deserializer getting next message...");
    while let Some(msg) = raw_ds_rx.recv().await {
        debug!("Raw deserializer got message, decoding...");

        // Print received messages after ABI is set
        //info!("Received message: {:?}", bytes_to_hex(&msg_data));
        // TODO: Better threading so we don't block reading while deserialize?
        let mut decoder = Decoder::new(&msg);
        let ship_result = &mut ShipResult::default();
        decoder.unpack(ship_result);

        match ship_result {
            ShipResult::GetStatusResultV0(r) => {
                let start_block_num = config.evm_start_block + block_delta;
                let chain_begin_block = r.chain_state_begin_block;
                if start_block_num <= chain_begin_block {
                    return Err(eyre!("Start block {start_block_num} has to be greater than first chain block ({chain_begin_block})"));
                };
                info!(
                    "GetStatusResultV0 head: {:?} last_irreversible: {:?}",
                    r.head.block_num, r.last_irreversible.block_num
                );
                let request = &ShipRequest::GetBlocks(GetBlocksRequestV0 {
                    start_block_num,
                    // Increment stop block value by block delta + 1 as bound is exclusive
                    end_block_num: config
                        .evm_stop_block
                        .map(|n| n + block_delta + 1)
                        .unwrap_or(u32::MAX),
                    max_messages_in_flight: 10000,
                    have_positions: vec![],
                    irreversible_only: false, // TODO: Fork handling
                    fetch_block: true,
                    fetch_traces: true,
                    fetch_deltas: true,
                });
                ws_tx.send(request.into()).await?;
                debug!("GetBlocks request sent");
            }
            ShipResult::GetBlocksResultV0(r) => {
                unackd_blocks += 1;
                if let Some(b) = &r.this_block {
                    // Skip if current evm block <= evm deploy
                    let skip_events = (b.block_num - config.chain_id.block_delta()) <= config.evm_deploy_block.unwrap_or_default();
                    let block = ProcessingEVMBlock::new(ProcessingEVMBlockArgs {
                        chain_id: config.chain_id.0,
                        block_num: b.block_num,
                        block_hash: b.block_id,
                        prev_block_hash: r.prev_block.as_ref().map(|b| b.block_id),
                        lib_num: r.last_irreversible.block_num,
                        lib_hash: r.last_irreversible.block_id,
                        result: r.clone(),
                        skip_events,
                    });
                    debug!("Block #{} sending to block deserializer...", b.block_num);
                    block_deserializer_tx.send(block).await?;
                    debug!("Block #{} sent to block deserializer", b.block_num);
                    if last_log.elapsed().as_secs_f64() > 10.0 {
                        info!(
                            "Raw deserializer block #{} - processed {:.1} blocks/sec",
                            b.block_num,
                            (unlogged_blocks + unackd_blocks) as f64
                                / last_log.elapsed().as_secs_f64()
                        );
                        unlogged_blocks = 0;
                        last_log = Instant::now();
                    }

                    // TODO: Better logic here, don't just ack every N blocks, do this based on backpressure
                    if unackd_blocks > 10 {
                        //info!("Acking {} blocks", unackd_blocks);
                        // TODO: Better threading so we don't block reading while we write?
                        let request = &GetBlocksAck(GetBlocksAckRequestV0 {
                            num_messages: unackd_blocks,
                        });
                        ws_tx.send(request.into()).await?;

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
    info!("Exiting raw deserializer...");
    Ok(())
}
