//! WebSocket endpoint that streams Events from the EventBus.
//!
//! Protocol: client connects, server streams each Event as a JSON text frame.
//! No client-to-server messages are processed (read messages are discarded).

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use futures::StreamExt;
use tracing::warn;

use crate::server::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/events", get(upgrade))
}

async fn upgrade(ws: WebSocketUpgrade, State(app): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app))
}

async fn handle_socket(mut socket: WebSocket, app: AppState) {
    let mut rx = app.bus.subscribe();
    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok(event) => {
                        let payload = match serde_json::to_string(&event) {
                            Ok(s) => s,
                            Err(e) => {
                                warn!(error = %e, "serializing event");
                                continue;
                            }
                        };
                        if socket.send(Message::Text(payload)).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Drop lagged messages. Client should reconnect or replay via REST.
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
            client_msg = socket.next() => {
                match client_msg {
                    Some(Ok(_)) => continue,
                    _ => return,
                }
            }
        }
    }
}
