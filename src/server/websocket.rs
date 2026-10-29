// ─────────────────────────────────────────────
// Imports
// ─────────────────────────────────────────────

use super::routes::AppState;
use crate::core::messages::ServerMessage;

use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;

// ─────────────────────────────────────────────
// WebSocket Upgrade
// ─────────────────────────────────────────────

pub(super) async fn websocket_upgrade(
    websocket: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let receiver = state.event_sender.subscribe();

    websocket.on_upgrade(move |socket| handle_websocket(socket, receiver))
}

// ─────────────────────────────────────────────
// WebSocket Connection Handler
// ─────────────────────────────────────────────

async fn handle_websocket(socket: WebSocket, mut receiver: broadcast::Receiver<ServerMessage>) {
    let (mut sender, mut incoming) = socket.split();

    // Send the initial connection message to the overlay.
    if sender
        .send(Message::Text(
            serde_json::to_string(&ServerMessage::Connected)
                .unwrap_or_else(|_| "{\"type\":\"connected\"}".to_owned())
                .into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    // Forward server events and monitor the client connection.
    loop {
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Ok(message) => {
                        let payload = match serde_json::to_string(&message) {
                            Ok(payload) => payload,
                            Err(error) => {
                                eprintln!(
                                    "Could not serialize overlay message: {error}"
                                );
                                continue;
                            }
                        };

                        if sender
                            .send(Message::Text(payload.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }

                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        eprintln!(
                            "Overlay client lagged; skipped {skipped} queued messages"
                        );
                    }

                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            message = incoming.next() => {
                match message {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    Some(Ok(Message::Ping(_))) |
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}
