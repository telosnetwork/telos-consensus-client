use crate::types::types::RawMessage;
use futures_util::stream::SplitStream;
use futures_util::StreamExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use log::debug;

pub async fn ship_reader(
    mut ws_rx: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    raw_ds_tx: mpsc::Sender<RawMessage>,
) {
    let mut sequence: u64 = 0;

    // Read the websocket
    while let Some(message) = ws_rx.next().await {
        sequence += 1;
        match message {
            Ok(msg) => {
                debug!("Received message with sequence {}, sending to raw ds pool...", sequence);
                // write to the channel
                if raw_ds_tx
                    .send(RawMessage::new(sequence, msg.into_data()))
                    .await
                    .is_err()
                {
                    println!("Receiver dropped");
                    return;
                }
                debug!("Sent message with sequence {} to raw ds pool...", sequence);
            }
            Err(e) => {
                println!("Error receiving message: {}", e);
                return;
            }
        }
    }
}
