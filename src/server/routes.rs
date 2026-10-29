// ─────────────────────────────────────────────
// Imports
// ─────────────────────────────────────────────

use crate::{
    core::messages::ServerMessage,
    twitch::auth::{AuthStatus, TwitchAuth},
};

use super::websocket;

use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};

use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::broadcast;

// ─────────────────────────────────────────────
// Application State
// ─────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) event_sender: broadcast::Sender<ServerMessage>,
    pub(crate) twitch_auth: TwitchAuth,
}

// ─────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/style.css", get(stylesheet))
        .route("/script.js", get(script))
        .route("/assets/avatar.svg", get(fallback_avatar))
        .route("/health", get(health))
        .route("/api/test/follow", post(trigger_test_follow))
        .route("/api/twitch/auth/start", post(start_twitch_auth))
        .route("/api/twitch/auth/status", get(twitch_auth_status))
        .route("/ws", get(websocket::websocket_upgrade))
        .with_state(state)
}

// ─────────────────────────────────────────────
// Overlay Assets
// ─────────────────────────────────────────────

async fn index() -> Html<&'static str> {
    Html(include_str!("../../overlay/index.html"))
}

async fn stylesheet() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../overlay/style.css"),
    )
        .into_response()
}

async fn script() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../overlay/script.js"),
    )
        .into_response()
}

async fn fallback_avatar() -> Response {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        include_str!("../../overlay/assets/avatar.svg"),
    )
        .into_response()
}

// ─────────────────────────────────────────────
// Health Check
// ─────────────────────────────────────────────

async fn health() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

// ─────────────────────────────────────────────
// Twitch Authentication
// ─────────────────────────────────────────────

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

// ─────────────────────────────────────────────
// Synthetic Follow Test
// ─────────────────────────────────────────────

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
