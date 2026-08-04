use crate::block::{decode, ProcessingEVMBlock, ProcessingEVMBlockArgs};
use crate::translator::TranslatorConfig;
use crate::types::ship_types::ShipRequest::{GetBlocksAck, GetStatus};
use crate::types::ship_types::{
    BlockPosition, GetBlocksAckRequestV0, GetBlocksRequestV0, GetBlocksResultV0,
    GetStatusRequestV0, GetStatusResultV0, ShipRequest, ShipResult,
};
use antelope::chain::checksum::Checksum256;
use eyre::{eyre, Context, Result};
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, info};

pub async fn raw_deserializer(
    config: TranslatorConfig,
    mut raw_ds_rx: Receiver<Vec<u8>>,
    mut ws_tx: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    block_deserializer_tx: Sender<ProcessingEVMBlock>,
) -> Result<()> {
    let mut unackd_blocks: u32 = 0;
    let mut last_log = Instant::now();
    let mut unlogged_blocks = 0;
    let block_delta = config.chain_id.block_delta();
    let expected_native_chain_id = config.expected_native_chain_id()?;

    // TODO: maybe get this working as an ABI again?
    //   the problem is that the ABI from ship has invalid table names like `account_metadata`
    //   which cause from_string to fail, but if you change AbiTable.name to a String then
    //   when you use the ABI struct to pack for a contract deployment, it causes the table
    //   lookups via v1/chain/get_table_rows to fail because it doesn't like the string when
    //   it's trying to determine the index type of a table
    //let abi_string = msg.to_string();
    //let abi = ABI::from_string(abi_string.as_str()).unwrap();
    //self.ship_abi = Some(abi_string);
    let _validated_abi = raw_ds_rx
        .recv()
        .await
        .ok_or_else(|| eyre!("SHIP reader stopped before the validated ABI handshake"))?;

    // Send GetStatus request after setting up the ABI
    let request = &GetStatus(GetStatusRequestV0);
    ws_tx.send(request.into()).await?;

    debug!("Raw deserializer getting next message...");
    while let Some(msg) = raw_ds_rx.recv().await {
        debug!("Raw deserializer got message, decoding...");

        // Print received messages after ABI is set
        //info!("Received message: {:?}", bytes_to_hex(&msg_data));
        // TODO: Better threading so we don't block reading while deserialize?
        let ship_result: ShipResult =
            decode(&msg).wrap_err("failed to decode SHIP response payload")?;

        match &ship_result {
            ShipResult::GetStatusResultV0(r) => {
                let start_block_num = config
                    .evm_start_block
                    .checked_add(block_delta)
                    .ok_or_else(|| eyre!("Native start block overflows u32"))?;
                validate_ship_status(r, expected_native_chain_id, start_block_num)?;
                info!(
                    "GetStatusResultV0 head: {:?} last_irreversible: {:?}",
                    r.head.block_num, r.last_irreversible.block_num
                );
                let request = &ShipRequest::GetBlocks(GetBlocksRequestV0 {
                    start_block_num,
                    // Increment stop block value by block delta + 1 as bound is exclusive
                    end_block_num: config
                        .evm_stop_block
                        .map(|block| {
                            block
                                .checked_add(block_delta)
                                .and_then(|block| block.checked_add(1))
                                .ok_or_else(|| eyre!("Native stop block overflows u32"))
                        })
                        .transpose()?
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
                let b = require_complete_ship_block(r)?;
                unackd_blocks = unackd_blocks
                    .checked_add(1)
                    .ok_or_else(|| eyre!("SHIP acknowledgement counter overflow"))?;
                let evm_block_number = b
                    .block_num
                    .checked_sub(config.chain_id.block_delta())
                    .ok_or_else(|| {
                        eyre!(
                            "Native block {} is before the configured EVM block delta",
                            b.block_num
                        )
                    })?;
                let skip_events = evm_block_number <= config.evm_deploy_block.unwrap_or_default();
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
                if block_deserializer_tx.is_closed() {
                    return Err(eyre!("Block deserializer stopped while SHIP was active"));
                }
                debug!("Block #{} sending to block deserializer...", b.block_num);
                block_deserializer_tx.send(block).await?;
                debug!("Block #{} sent to block deserializer", b.block_num);
                if last_log.elapsed().as_secs_f64() > 10.0 {
                    info!(
                        "Raw deserializer block #{} - processed {:.1} blocks/sec",
                        b.block_num,
                        (unlogged_blocks + unackd_blocks) as f64 / last_log.elapsed().as_secs_f64()
                    );
                    unlogged_blocks = 0;
                    last_log = Instant::now();
                }

                // TODO: Better logic here, don't just ack every N blocks, do this based on backpressure
                if unackd_blocks > 10 {
                    let request = &GetBlocksAck(GetBlocksAckRequestV0 {
                        num_messages: unackd_blocks,
                    });
                    ws_tx.send(request.into()).await?;

                    unlogged_blocks += unackd_blocks;
                    unackd_blocks = 0;
                }
            }
        }
    }
    info!("Exiting raw deserializer...");
    Ok(())
}

fn validate_ship_status(
    status: &GetStatusResultV0,
    expected_chain_id: Checksum256,
    start_block: u32,
) -> Result<()> {
    if status.chain_id != expected_chain_id {
        return Err(eyre!(
            "SHIP chain id {} does not match configured native chain id {}",
            status.chain_id.as_string(),
            expected_chain_id.as_string()
        ));
    }
    if start_block < status.chain_state_begin_block {
        return Err(eyre!(
            "Native start block {start_block} predates available chain-state history beginning at {}",
            status.chain_state_begin_block
        ));
    }
    if start_block < status.trace_begin_block {
        return Err(eyre!(
            "Native start block {start_block} predates available trace history beginning at {}",
            status.trace_begin_block
        ));
    }
    if start_block > status.chain_state_end_block {
        return Err(eyre!(
            "Native start block {start_block} is beyond chain-state history ending at {}",
            status.chain_state_end_block
        ));
    }
    if start_block > status.trace_end_block {
        return Err(eyre!(
            "Native start block {start_block} is beyond trace history ending at {}",
            status.trace_end_block
        ));
    }
    Ok(())
}

fn require_complete_ship_block(result: &GetBlocksResultV0) -> Result<&BlockPosition> {
    let block_position = result
        .this_block
        .as_ref()
        .ok_or_else(|| eyre!("SHIP block result is missing this_block"))?;
    if result.prev_block.is_none() {
        return Err(eyre!(
            "SHIP block {} is missing its native parent",
            block_position.block_num
        ));
    }
    for (name, payload) in [
        ("signed block", result.block.as_ref()),
        ("transaction traces", result.traces.as_ref()),
        ("chain-state deltas", result.deltas.as_ref()),
    ] {
        if payload.is_none_or(Vec::is_empty) {
            return Err(eyre!(
                "SHIP block {} is missing {name}",
                block_position.block_num
            ));
        }
    }
    Ok(block_position)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checksum(byte: u8) -> Checksum256 {
        Checksum256::from_bytes(&[byte; 32]).unwrap()
    }

    fn status(chain_id: Checksum256) -> GetStatusResultV0 {
        GetStatusResultV0 {
            trace_begin_block: 100,
            trace_end_block: 200,
            chain_state_begin_block: 100,
            chain_state_end_block: 200,
            chain_id,
            ..Default::default()
        }
    }

    #[test]
    fn status_requires_the_same_chain_and_both_histories() {
        let expected = checksum(1);
        assert!(validate_ship_status(&status(expected), expected, 100).is_ok());
        assert!(validate_ship_status(&status(checksum(2)), expected, 100).is_err());

        let mut missing_traces = status(expected);
        missing_traces.trace_begin_block = 101;
        assert!(validate_ship_status(&missing_traces, expected, 100).is_err());

        let mut missing_state = status(expected);
        missing_state.chain_state_begin_block = 101;
        assert!(validate_ship_status(&missing_state, expected, 100).is_err());
    }

    #[test]
    fn block_results_require_block_traces_deltas_and_parent() {
        let mut result = GetBlocksResultV0 {
            this_block: Some(BlockPosition {
                block_num: 101,
                block_id: checksum(3),
            }),
            prev_block: Some(BlockPosition {
                block_num: 100,
                block_id: checksum(2),
            }),
            block: Some(vec![0]),
            traces: Some(vec![0]),
            deltas: Some(vec![0]),
            ..Default::default()
        };
        assert!(require_complete_ship_block(&result).is_ok());
        result.traces = None;
        assert!(require_complete_ship_block(&result).is_err());
    }
}
