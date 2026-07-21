use eyre::{eyre, Result};
use futures_util::stream::SplitStream;
use futures_util::StreamExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, info};

pub async fn ship_reader(
    mut ws_rx: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    raw_ds_tx: mpsc::Sender<Vec<u8>>,
    mut stop_rx: mpsc::Receiver<()>,
) -> Result<()> {
    let mut counter: u64 = 0;

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
                let Some(payload) = ship_message_payload(msg)? else {
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

fn ship_message_payload(message: Message) -> Result<Option<Vec<u8>>> {
    match message {
        Message::Binary(payload) => Ok(Some(payload)),
        Message::Ping(_) | Message::Pong(_) => Ok(None),
        Message::Close(frame) => Err(eyre!("SHIP websocket closed unexpectedly: {frame:?}")),
        Message::Text(_) | Message::Frame(_) => {
            Err(eyre!("SHIP websocket returned a non-binary data message"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_binary_ship_messages_are_forwarded() {
        assert_eq!(
            ship_message_payload(Message::Binary(vec![1, 2, 3])).unwrap(),
            Some(vec![1, 2, 3])
        );
        assert_eq!(ship_message_payload(Message::Ping(vec![])).unwrap(), None);
        assert!(ship_message_payload(Message::Close(None)).is_err());
        assert!(ship_message_payload(Message::Text("unexpected".to_string())).is_err());
    }
}
