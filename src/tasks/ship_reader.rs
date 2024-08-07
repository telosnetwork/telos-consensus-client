use crate::types::translator_types::RawMessage;
use eyre::Result;
use futures_util::stream::SplitStream;
use futures_util::StreamExt;
use log::debug;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::info;

pub async fn ship_reader(
    mut ws_rx: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    raw_ds_tx: mpsc::Sender<RawMessage>,
    mut stop_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let mut sequence: u64 = 0;

    loop {
        // Read the websocket
        let message = tokio::select! {
            message = ws_rx.next() => message,
            _ = &mut stop_rx => break
        };

        sequence += 1;
        match message {
            Some(Ok(msg)) => {
                debug!(
                    "Received message with sequence {}, sending to raw ds pool...",
                    sequence
                );
                // write to the channel
                if raw_ds_tx
                    .send(RawMessage::new(sequence, msg.into_data()))
                    .await
                    .is_err()
                {
                    println!("Receiver dropped");
                    break;
                }
                debug!("Sent message with sequence {} to raw ds pool...", sequence);
            }
            Some(Err(e)) => {
                println!("Error receiving message: {}", e);
                break;
            }
            None => {
                break;
            }
        }
    }
    info!("Exiting ship reader...");
    Ok(())
}
