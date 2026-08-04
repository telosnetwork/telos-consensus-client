use eyre::{eyre, Result};
use futures_util::stream::SplitStream;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, info};

const MAX_SHIP_ABI_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ShipSessionState {
    #[default]
    AwaitingAbi,
    Streaming,
}

#[derive(Debug, Deserialize)]
struct ShipAbi {
    version: String,
    structs: Vec<AbiStruct>,
    variants: Vec<AbiVariant>,
}

#[derive(Debug, Deserialize)]
struct AbiStruct {
    name: String,
    #[serde(default)]
    base: Option<String>,
    fields: Vec<AbiField>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct AbiField {
    name: String,
    #[serde(rename = "type")]
    field_type: String,
}

#[derive(Debug, Deserialize)]
struct AbiVariant {
    name: String,
    types: Vec<String>,
}

pub async fn ship_reader(
    mut ws_rx: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    raw_ds_tx: mpsc::Sender<Vec<u8>>,
    mut stop_rx: mpsc::Receiver<()>,
) -> Result<()> {
    let mut counter: u64 = 0;
    let mut state = ShipSessionState::default();

    loop {
        // Read the websocket
        let message = tokio::select! {
            message = ws_rx.next() => message,
            stop = stop_rx.recv() => {
                return if stop.is_some() {
                    info!("SHIP reader received shutdown signal");
                    Ok(())
                } else {
                    Err(eyre!("SHIP shutdown channel closed unexpectedly"))
                };
            }
        };

        counter += 1;
        match message {
            Some(Ok(msg)) => {
                let Some(payload) = ship_message_payload(msg, &mut state)? else {
                    continue;
                };
                debug!("Received message {counter}, sending to raw ds pool...",);
                raw_ds_tx
                    .send(payload)
                    .await
                    .map_err(|_| eyre!("SHIP deserializer stopped while websocket was active"))?;
                debug!("Sent message {counter} to raw ds pool...");
            }
            Some(Err(e)) => {
                return Err(eyre!("SHIP websocket receive failed: {e}"));
            }
            None => {
                return Err(eyre!("SHIP websocket reached unexpected EOF"));
            }
        }
    }
}

fn ship_message_payload(message: Message, state: &mut ShipSessionState) -> Result<Option<Vec<u8>>> {
    match message {
        Message::Text(payload) => {
            if *state != ShipSessionState::AwaitingAbi {
                return Err(eyre!("SHIP websocket sent text after the ABI handshake"));
            }
            validate_ship_abi(&payload)?;
            *state = ShipSessionState::Streaming;
            Ok(Some(payload.into_bytes()))
        }
        Message::Binary(payload) => {
            if *state != ShipSessionState::Streaming {
                return Err(eyre!(
                    "SHIP websocket sent binary data before the ABI handshake"
                ));
            }
            Ok(Some(payload))
        }
        // Tungstenite queues the protocol-required Pong while reading Ping frames. Neither control
        // frame is SHIP application data, and neither is allowed to advance the handshake state.
        Message::Ping(_) | Message::Pong(_) => Ok(None),
        Message::Close(frame) => Err(eyre!("SHIP websocket closed unexpectedly: {frame:?}")),
        Message::Frame(_) => Err(eyre!(
            "SHIP websocket returned an unexpected raw websocket frame"
        )),
    }
}

fn validate_ship_abi(payload: &str) -> Result<()> {
    if payload.len() > MAX_SHIP_ABI_BYTES {
        return Err(eyre!(
            "SHIP ABI handshake exceeds the {MAX_SHIP_ABI_BYTES}-byte limit"
        ));
    }

    let abi: ShipAbi = serde_json::from_str(payload)
        .map_err(|error| eyre!("Invalid SHIP ABI handshake: {error}"))?;
    if abi.version != "eosio::abi/1.1" {
        return Err(eyre!(
            "Unsupported SHIP ABI version {}; expected eosio::abi/1.1",
            abi.version
        ));
    }

    for (name, fields) in [
        ("get_status_request_v0", &[][..]),
        (
            "block_position",
            &[("block_num", "uint32"), ("block_id", "checksum256")][..],
        ),
        (
            "get_status_result_v0",
            &[
                ("head", "block_position"),
                ("last_irreversible", "block_position"),
                ("trace_begin_block", "uint32"),
                ("trace_end_block", "uint32"),
                ("chain_state_begin_block", "uint32"),
                ("chain_state_end_block", "uint32"),
                ("chain_id", "checksum256$"),
            ][..],
        ),
        (
            "get_blocks_request_v0",
            &[
                ("start_block_num", "uint32"),
                ("end_block_num", "uint32"),
                ("max_messages_in_flight", "uint32"),
                ("have_positions", "block_position[]"),
                ("irreversible_only", "bool"),
                ("fetch_block", "bool"),
                ("fetch_traces", "bool"),
                ("fetch_deltas", "bool"),
            ][..],
        ),
        (
            "get_blocks_ack_request_v0",
            &[("num_messages", "uint32")][..],
        ),
        (
            "get_blocks_result_v0",
            &[
                ("head", "block_position"),
                ("last_irreversible", "block_position"),
                ("this_block", "block_position?"),
                ("prev_block", "block_position?"),
                ("block", "bytes?"),
                ("traces", "bytes?"),
                ("deltas", "bytes?"),
            ][..],
        ),
    ] {
        validate_abi_struct(&abi.structs, name, fields)?;
    }

    validate_abi_variant(
        &abi.variants,
        "request",
        &[
            "get_status_request_v0",
            "get_blocks_request_v0",
            "get_blocks_ack_request_v0",
        ],
    )?;
    validate_abi_variant(
        &abi.variants,
        "result",
        &["get_status_result_v0", "get_blocks_result_v0"],
    )?;
    Ok(())
}

fn validate_abi_struct(
    structs: &[AbiStruct],
    name: &str,
    expected_fields: &[(&str, &str)],
) -> Result<()> {
    let mut matching = structs.iter().filter(|item| item.name == name);
    let item = matching
        .next()
        .ok_or_else(|| eyre!("SHIP ABI is missing required struct {name}"))?;
    if matching.next().is_some() {
        return Err(eyre!("SHIP ABI contains duplicate struct {name}"));
    }
    if item.base.as_deref().is_some_and(|base| !base.is_empty()) {
        return Err(eyre!("SHIP ABI struct {name} has an unsupported base type"));
    }
    let actual_fields: Vec<_> = item
        .fields
        .iter()
        .map(|field| (field.name.as_str(), field.field_type.as_str()))
        .collect();
    if actual_fields != expected_fields {
        return Err(eyre!(
            "SHIP ABI struct {name} does not match the supported layout"
        ));
    }
    Ok(())
}

fn validate_abi_variant(variants: &[AbiVariant], name: &str, expected: &[&str]) -> Result<()> {
    let mut matching = variants.iter().filter(|item| item.name == name);
    let item = matching
        .next()
        .ok_or_else(|| eyre!("SHIP ABI is missing required variant {name}"))?;
    if matching.next().is_some() {
        return Err(eyre!("SHIP ABI contains duplicate variant {name}"));
    }
    if item.types.len() < expected.len()
        || item
            .types
            .iter()
            .take(expected.len())
            .map(String::as_str)
            .ne(expected.iter().copied())
    {
        return Err(eyre!(
            "SHIP ABI variant {name} does not preserve the supported v0 layout"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn supported_abi() -> String {
        json!({
            "version": "eosio::abi/1.1",
            "structs": [
                {"name": "get_status_request_v0", "fields": []},
                {"name": "block_position", "fields": [
                    {"name": "block_num", "type": "uint32"},
                    {"name": "block_id", "type": "checksum256"}
                ]},
                {"name": "get_status_result_v0", "fields": [
                    {"name": "head", "type": "block_position"},
                    {"name": "last_irreversible", "type": "block_position"},
                    {"name": "trace_begin_block", "type": "uint32"},
                    {"name": "trace_end_block", "type": "uint32"},
                    {"name": "chain_state_begin_block", "type": "uint32"},
                    {"name": "chain_state_end_block", "type": "uint32"},
                    {"name": "chain_id", "type": "checksum256$"}
                ]},
                {"name": "get_blocks_request_v0", "fields": [
                    {"name": "start_block_num", "type": "uint32"},
                    {"name": "end_block_num", "type": "uint32"},
                    {"name": "max_messages_in_flight", "type": "uint32"},
                    {"name": "have_positions", "type": "block_position[]"},
                    {"name": "irreversible_only", "type": "bool"},
                    {"name": "fetch_block", "type": "bool"},
                    {"name": "fetch_traces", "type": "bool"},
                    {"name": "fetch_deltas", "type": "bool"}
                ]},
                {"name": "get_blocks_ack_request_v0", "fields": [
                    {"name": "num_messages", "type": "uint32"}
                ]},
                {"name": "get_blocks_result_v0", "fields": [
                    {"name": "head", "type": "block_position"},
                    {"name": "last_irreversible", "type": "block_position"},
                    {"name": "this_block", "type": "block_position?"},
                    {"name": "prev_block", "type": "block_position?"},
                    {"name": "block", "type": "bytes?"},
                    {"name": "traces", "type": "bytes?"},
                    {"name": "deltas", "type": "bytes?"}
                ]}
            ],
            "variants": [
                {"name": "request", "types": [
                    "get_status_request_v0",
                    "get_blocks_request_v0",
                    "get_blocks_ack_request_v0"
                ]},
                {"name": "result", "types": [
                    "get_status_result_v0",
                    "get_blocks_result_v0"
                ]}
            ]
        })
        .to_string()
    }

    #[test]
    fn supported_abi_is_required_before_binary_messages() {
        let abi = supported_abi();
        let mut state = ShipSessionState::default();
        assert_eq!(
            ship_message_payload(Message::Text(abi.clone()), &mut state).unwrap(),
            Some(abi.into_bytes())
        );
        assert_eq!(state, ShipSessionState::Streaming);
        assert_eq!(
            ship_message_payload(Message::Binary(vec![1, 2, 3]), &mut state).unwrap(),
            Some(vec![1, 2, 3])
        );
        assert!(ship_message_payload(Message::Text(supported_abi()), &mut state).is_err());
    }

    #[test]
    fn malformed_or_incompatible_abi_is_rejected() {
        let mut state = ShipSessionState::default();
        assert!(ship_message_payload(Message::Text("not json".to_string()), &mut state).is_err());
        assert_eq!(state, ShipSessionState::AwaitingAbi);

        let mut abi: Value = serde_json::from_str(&supported_abi()).unwrap();
        abi["version"] = json!("eosio::abi/1.2");
        assert!(ship_message_payload(Message::Text(abi.to_string()), &mut state).is_err());

        let oversized = " ".repeat(MAX_SHIP_ABI_BYTES + 1);
        assert!(ship_message_payload(Message::Text(oversized), &mut state).is_err());
        assert!(ship_message_payload(Message::Binary(vec![1]), &mut state).is_err());
    }

    #[test]
    fn appended_ship_protocol_versions_preserve_v0_variant_indices() {
        let mut abi: Value = serde_json::from_str(&supported_abi()).unwrap();
        abi["variants"][0]["types"]
            .as_array_mut()
            .unwrap()
            .push(json!("get_blocks_request_v1"));
        abi["variants"][1]["types"]
            .as_array_mut()
            .unwrap()
            .push(json!("get_blocks_result_v1"));

        let mut state = ShipSessionState::default();
        assert!(ship_message_payload(Message::Text(abi.to_string()), &mut state).is_ok());
        assert_eq!(state, ShipSessionState::Streaming);

        let mut reordered: Value = serde_json::from_str(&supported_abi()).unwrap();
        reordered["variants"][0]["types"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        let mut state = ShipSessionState::default();
        assert!(ship_message_payload(Message::Text(reordered.to_string()), &mut state).is_err());
    }

    #[test]
    fn websocket_control_frames_do_not_advance_the_handshake() {
        let mut state = ShipSessionState::default();
        assert_eq!(
            ship_message_payload(Message::Ping(vec![1]), &mut state).unwrap(),
            None
        );
        assert_eq!(
            ship_message_payload(Message::Pong(vec![1]), &mut state).unwrap(),
            None
        );
        assert_eq!(state, ShipSessionState::AwaitingAbi);
        assert!(ship_message_payload(Message::Close(None), &mut state).is_err());
        assert_eq!(state, ShipSessionState::AwaitingAbi);
    }
}
