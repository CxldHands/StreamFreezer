mod eventsub;
mod messages;
mod twitch_auth;

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use messages::ServerMessage;
use std::{
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;
use twitch_auth::{AuthStatus, TwitchAuth};

const HOST: [u8; 4] = [127, 0, 0, 1];
const PORT: u16 = 3001;
const CHANNEL_CAPACITY: usize = 64;

#[derive(Clone)]
struct AppState {
    event_sender: broadcast::Sender<ServerMessage>,
    twitch_auth: TwitchAuth,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (event_sender, _) = broadcast::channel(CHANNEL_CAPACITY);
    let twitch_auth = TwitchAuth::new();

    twitch_auth.restore_saved_session().await;

    // Clone both handles before moving the originals into AppState.
    let eventsub_auth = twitch_auth.clone();
    let eventsub_sender = event_sender.clone();

    let state = AppState {
        event_sender,
        twitch_auth,
    };

    tokio::spawn(async move {
        eventsub::run(eventsub_auth, eventsub_sender).await;
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/style.css", get(stylesheet))
        .route("/script.js", get(script))
        .route("/assets/avatar.svg", get(fallback_avatar))
        .route("/health", get(health))
        .route("/api/test/follow", post(trigger_test_follow))
        .route("/api/twitch/auth/start", post(start_twitch_auth))
        .route("/api/twitch/auth/status", get(twitch_auth_status))
        .route("/ws", get(websocket_upgrade))
        .with_state(state);

    let address = SocketAddr::from((HOST, PORT));
    let listener = tokio::net::TcpListener::bind(address).await?;

    println!("Twitch Follow Alerts service listening at http://{address}");
    println!("Overlay: http://{address}/");
    println!("Debug overlay: http://{address}/?debug=1");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../overlay/index.html"))
}

async fn stylesheet() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../overlay/style.css"),
    )
        .into_response()
}

async fn script() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../overlay/script.js"),
    )
        .into_response()
}

async fn fallback_avatar() -> Response {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        include_str!("../overlay/assets/avatar.svg"),
    )
        .into_response()
}

async fn health() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

async fn start_twitch_auth(State(state): State<AppState>) -> Response {
    match state.twitch_auth.start().await {
        Ok(status) => (StatusCode::OK, axum::Json(status)).into_response(),
        Err(message) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({ "error": message })),
        )
            .into_response(),
    }
}

async fn twitch_auth_status(State(state): State<AppState>) -> axum::Json<AuthStatus> {
    axum::Json(state.twitch_auth.status().await)
}

async fn trigger_test_follow(State(state): State<AppState>) -> impl IntoResponse {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();

    let event = ServerMessage::Follow {
        event_id: format!("synthetic-{timestamp}"),
        user: "TestFollower".to_owned(),
        avatar: "/assets/avatar.svg".to_owned(),
    };

    if state.event_sender.receiver_count() == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no overlay WebSocket clients are connected",
        );
    }

    match state.event_sender.send(event) {
        Ok(_) => (StatusCode::ACCEPTED, "synthetic follow event broadcast"),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "overlay WebSocket disconnected before the event was sent",
        ),
    }
}

async fn websocket_upgrade(
    websocket: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let receiver = state.event_sender.subscribe();
    websocket.on_upgrade(move |socket| handle_websocket(socket, receiver))
}

async fn handle_websocket(socket: WebSocket, mut receiver: broadcast::Receiver<ServerMessage>) {
    let (mut sender, mut incoming) = socket.split();

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

    loop {
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Ok(message) => {
                        let payload = match serde_json::to_string(&message) {
                            Ok(payload) => payload,
                            Err(error) => {
                                eprintln!("Could not serialize overlay message: {error}");
                                continue;
                            }
                        };

                        if sender.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        eprintln!("Overlay client lagged; skipped {skipped} queued messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            message = incoming.next() => {
                match message {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}
