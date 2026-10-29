// ─────────────────────────────────────────────
// Application Modules
// ─────────────────────────────────────────────

mod core;
mod server;
mod twitch;

// ─────────────────────────────────────────────
// Imports
// ─────────────────────────────────────────────

use crate::{
    server::{AppState, build_router},
    twitch::auth::TwitchAuth,
};

use std::net::SocketAddr;
use tokio::sync::broadcast;

// ─────────────────────────────────────────────
// Application Configuration
// ─────────────────────────────────────────────

const HOST: [u8; 4] = [127, 0, 0, 1];
const PORT: u16 = 3001;
const CHANNEL_CAPACITY: usize = 64;

// ─────────────────────────────────────────────
// Application Entry Point
// ─────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize shared event delivery.
    let (event_sender, _) = broadcast::channel(CHANNEL_CAPACITY);

    // Initialize Twitch authentication and restore the saved session.
    let twitch_auth = TwitchAuth::new();
    twitch_auth.restore_saved_session().await;

    // Prepare the EventSub worker before moving state into the server.
    let eventsub_auth = twitch_auth.clone();
    let eventsub_sender = event_sender.clone();

    tokio::spawn(async move {
        twitch::eventsub::run(eventsub_auth, eventsub_sender).await;
    });

    // Build shared server state and the HTTP router.
    let state = AppState {
        event_sender,
        twitch_auth,
    };

    let app = build_router(state);

    // Bind the local HTTP server.
    let address = SocketAddr::from((HOST, PORT));
    let listener = tokio::net::TcpListener::bind(address).await?;

    println!("StreamFreezer listening at http://{address}");
    println!("Follow overlay: http://{address}/");
    println!("Debug overlay: http://{address}/?debug=1");

    // Start serving requests.
    axum::serve(listener, app).await?;

    Ok(())
}
