// ─────────────────────────────────────────────
// Imports
// ─────────────────────────────────────────────

use crate::{
    core::messages::ServerMessage,
    twitch::auth::{CLIENT_ID, TwitchAuth},
};

use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    time::Duration,
};
use tokio::sync::broadcast;
use tokio_tungstenite::{connect_async, tungstenite::Message};

// ─────────────────────────────────────────────
// EventSub Configuration
// ─────────────────────────────────────────────

const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const SUBSCRIPTIONS_URL: &str = "https://api.twitch.tv/helix/eventsub/subscriptions";

const MAX_RECENT_MESSAGE_IDS: usize = 512;
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

// ─────────────────────────────────────────────
// Connection Outcome
// ─────────────────────────────────────────────

enum ConnectionOutcome {
    Disconnected,
}

// ─────────────────────────────────────────────
// EventSub Worker
// ─────────────────────────────────────────────

pub async fn run(auth: TwitchAuth, event_sender: broadcast::Sender<ServerMessage>) {
    let mut recent_ids = HashSet::new();
    let mut recent_order = VecDeque::new();

    loop {
        // Wait for Twitch authorization before attempting EventSub.
        if auth.status().await.state != "connected" {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        match run_connection(&auth, &event_sender, &mut recent_ids, &mut recent_order).await {
            Ok(ConnectionOutcome::Disconnected) => {
                println!("Twitch EventSub disconnected; reconnecting.");
                tokio::time::sleep(RECONNECT_DELAY).await;
            }

            Err(error) => {
                eprintln!("Twitch EventSub error: {error}");
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        }
    }
}

// ─────────────────────────────────────────────
// EventSub Connection
// ─────────────────────────────────────────────

async fn run_connection(
    auth: &TwitchAuth,
    event_sender: &broadcast::Sender<ServerMessage>,
    recent_ids: &mut HashSet<String>,
    recent_order: &mut VecDeque<String>,
) -> Result<ConnectionOutcome, String> {
    let credentials = auth.eventsub_credentials().await?;
    let api_client = Client::new();

    println!("Connecting to Twitch EventSub.");

    let (socket, _) = connect_async(EVENTSUB_URL)
        .await
        .map_err(|error| format!("WebSocket connection failed: {error}"))?;

    let (mut writer, mut reader) = socket.split();

    // Twitch sends session_welcome immediately after connecting.
    let welcome_text = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = reader
                .next()
                .await
                .ok_or_else(|| "Twitch closed the connection before welcoming us.".to_owned())?
                .map_err(|error| format!("Could not read EventSub welcome: {error}"))?;

            match message {
                Message::Text(text) => break Ok(text.to_string()),

                Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                    Ok(text) => break Ok(text),
                    Err(_) => {
                        eprintln!("Ignoring non-text binary frame before EventSub welcome.");
                    }
                },

                Message::Ping(payload) => {
                    writer
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| format!("Could not reply to Twitch ping: {error}"))?;
                }

                Message::Pong(_) | Message::Frame(_) => {}

                Message::Close(_) => {
                    break Err("Twitch closed the connection before welcoming us.".to_owned());
                }
            }
        }
    })
    .await
    .map_err(|_| "Timed out waiting for Twitch EventSub welcome.".to_owned())??;

    let welcome: Value = serde_json::from_str(&welcome_text)
        .map_err(|error| format!("Invalid EventSub welcome JSON: {error}"))?;

    if welcome["metadata"]["message_type"] != "session_welcome" {
        return Err(format!(
            "Expected session_welcome, received: {}",
            welcome["metadata"]["message_type"]
                .as_str()
                .unwrap_or("unknown message type")
        ));
    }

    let welcome: Value = serde_json::from_str(welcome_text.as_str())
        .map_err(|error| format!("Invalid EventSub welcome JSON: {error}"))?;

    if welcome["metadata"]["message_type"] != "session_welcome" {
        return Err("Twitch did not send session_welcome.".to_owned());
    }

    let session_id = welcome["payload"]["session"]["id"]
        .as_str()
        .ok_or_else(|| "Welcome message has no session ID.".to_owned())?;

    println!("Twitch EventSub session established.");

    subscribe_to_follows(&credentials.access_token, &credentials.user_id, session_id).await?;

    // ─────────────────────────────────────────
    // Process EventSub Messages
    // ─────────────────────────────────────────

    while let Some(result) = reader.next().await {
        let message = result.map_err(|error| format!("EventSub WebSocket read failed: {error}"))?;

        match message {
            Message::Text(text) => {
                let value: Value = match serde_json::from_str(text.as_str()) {
                    Ok(value) => value,
                    Err(error) => {
                        eprintln!("Ignoring invalid EventSub JSON: {error}");
                        continue;
                    }
                };

                match value["metadata"]["message_type"].as_str() {
                    Some("session_keepalive") => {}

                    Some("notification") => {
                        publish_follow(
                            &value,
                            event_sender,
                            recent_ids,
                            recent_order,
                            &api_client,
                            &credentials.access_token,
                        )
                        .await;
                    }

                    // ─────────────────────────────────
                    // EventSub Reconnect Migration
                    // ─────────────────────────────────
                    Some("session_reconnect") => {
                        let reconnect_url = value["payload"]["session"]["reconnect_url"]
                            .as_str()
                            .ok_or_else(|| {
                            "Reconnect message has no reconnect URL.".to_owned()
                        })?;

                        if !reconnect_url.starts_with("wss://") {
                            return Err("Twitch provided an invalid reconnect URL.".to_owned());
                        }

                        println!("Twitch requested an EventSub reconnect.");

                        // Keep the current connection alive while establishing the replacement.
                        let (new_socket, _) =
                            connect_async(reconnect_url).await.map_err(|error| {
                                format!("EventSub migration connection failed: {error}")
                            })?;

                        let (mut new_writer, mut new_reader) = new_socket.split();

                        // Do not switch connections until Twitch welcomes the replacement session.
                        let welcome_text = tokio::time::timeout(
                            Duration::from_secs(10),
                            async {
                                loop {
                                    let message = new_reader
                                        .next()
                                        .await
                                        .ok_or_else(|| {
                                            "Replacement EventSub connection closed before welcome."
                                                .to_owned()
                                        })?
                                        .map_err(|error| {
                                            format!("Could not read replacement welcome: {error}")
                                        })?;

                                    match message {
                                        Message::Text(text) => break Ok(text.to_string()),

                                        Message::Binary(bytes) => {
                                            match String::from_utf8(bytes.to_vec()) {
                                                Ok(text) => break Ok(text),
                                                Err(_) => {
                                                    eprintln!(
                                                        "Ignoring non-text binary frame before replacement welcome."
                                                    );
                                                }
                                            }
                                        }

                                        Message::Ping(payload) => {
                                            new_writer
                                                .send(Message::Pong(payload))
                                                .await
                                                .map_err(|error| {
                                                    format!(
                                                        "Could not reply to replacement connection ping: {error}"
                                                    )
                                                })?;
                                        }

                                        Message::Pong(_) | Message::Frame(_) => {}

                                        Message::Close(_) => {
                                            break Err(
                                                "Twitch closed the replacement connection before welcome."
                                                    .to_owned(),
                                            );
                                        }
                                    }
                                }
                            },
                        )
                        .await
                        .map_err(|_| {
                            "Timed out waiting for replacement EventSub welcome.".to_owned()
                        })??;

                        let welcome: Value =
                            serde_json::from_str(&welcome_text).map_err(|error| {
                                format!("Invalid replacement EventSub welcome JSON: {error}")
                            })?;

                        if welcome["metadata"]["message_type"] != "session_welcome" {
                            return Err(
                                "Replacement EventSub connection did not receive session_welcome."
                                    .to_owned(),
                            );
                        }

                        welcome["payload"]["session"]["id"]
                            .as_str()
                            .ok_or_else(|| {
                                "Replacement EventSub welcome has no session ID.".to_owned()
                            })?;

                        // Twitch migrates subscriptions to the replacement session.
                        // Swap connections only after the replacement has been welcomed.
                        writer = new_writer;
                        reader = new_reader;

                        println!("EventSub reconnect migration completed.");
                    }

                    // ─────────────────────────────────
                    // Subscription Revocation
                    // ─────────────────────────────────
                    Some("revocation") => {
                        eprintln!(
                            "Twitch revoked an EventSub subscription: {}",
                            value["payload"]["subscription"]["status"]
                                .as_str()
                                .unwrap_or("unknown")
                        );
                    }

                    Some(other) => {
                        println!("Ignoring EventSub message type: {other}");
                    }

                    None => {
                        eprintln!("Ignoring EventSub message without a type.");
                    }
                }
            }

            Message::Ping(payload) => {
                writer
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|error| format!("Could not reply to Twitch ping: {error}"))?;
            }

            Message::Close(_) => {
                return Ok(ConnectionOutcome::Disconnected);
            }

            _ => {}
        }
    }

    Ok(ConnectionOutcome::Disconnected)
}

// ─────────────────────────────────────────────
// Follow Subscription
// ─────────────────────────────────────────────

async fn subscribe_to_follows(
    access_token: &str,
    user_id: &str,
    session_id: &str,
) -> Result<(), String> {
    let client = Client::new();

    let request_body = json!({
        "type": "channel.follow",
        "version": "2",
        "condition": {
            "broadcaster_user_id": user_id,
            "moderator_user_id": user_id
        },
        "transport": {
            "method": "websocket",
            "session_id": session_id
        }
    });

    let response = client
        .post(SUBSCRIPTIONS_URL)
        .header("Client-Id", CLIENT_ID)
        .bearer_auth(access_token)
        .json(&request_body)
        .send()
        .await
        .map_err(|error| format!("Could not create Twitch follow subscription: {error}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "Twitch rejected the follow subscription (HTTP {status}): {body}"
        ));
    }

    println!("Subscribed to channel.follow v2 for broadcaster ID {user_id}.");

    Ok(())
}

// ─────────────────────────────────────────────
// Follow Notification Processing
// ─────────────────────────────────────────────

async fn publish_follow(
    message: &Value,
    event_sender: &broadcast::Sender<ServerMessage>,
    recent_ids: &mut HashSet<String>,
    recent_order: &mut VecDeque<String>,
    client: &Client,
    access_token: &str,
) {
    if message["payload"]["subscription"]["type"] != "channel.follow" {
        return;
    }

    let Some(event_id) = message["metadata"]["message_id"].as_str() else {
        eprintln!("Ignoring follow notification without a message ID.");
        return;
    };

    if recent_ids.contains(event_id) {
        println!("Ignoring duplicate Twitch event: {event_id}");
        return;
    }

    let event = &message["payload"]["event"];

    let Some(user) = event["user_name"]
        .as_str()
        .or_else(|| event["user_login"].as_str())
        .filter(|user| !user.trim().is_empty())
    else {
        eprintln!("Ignoring follow notification without a username.");
        return;
    };

    // Record the event before awaiting the avatar lookup.
    remember_message_id(event_id, recent_ids, recent_order);

    let avatar = if let Some(user_id) = event["user_id"].as_str() {
        match fetch_avatar(client, access_token, user_id).await {
            Some(url) => url,
            None => {
                eprintln!("Could not retrieve Twitch avatar for @{user}.");
                "/assets/avatar.svg".to_owned()
            }
        }
    } else {
        "/assets/avatar.svg".to_owned()
    };

    let alert = ServerMessage::Follow {
        event_id: event_id.to_owned(),
        user: user.to_owned(),
        avatar,
    };

    if event_sender.send(alert).is_err() {
        eprintln!("Received follow from @{user}, but no overlay clients are connected.");
    } else {
        println!("Follow received: @{user}");
    }
}

// ─────────────────────────────────────────────
// Twitch Profile Image Lookup
// ─────────────────────────────────────────────

async fn fetch_avatar(client: &Client, access_token: &str, user_id: &str) -> Option<String> {
    let response = client
        .get("https://api.twitch.tv/helix/users")
        .query(&[("id", user_id)])
        .header("Client-Id", CLIENT_ID)
        .bearer_auth(access_token)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        eprintln!("Twitch user lookup failed with HTTP {}.", response.status());
        return None;
    }

    let body: Value = response.json().await.ok()?;
    let avatar = body["data"][0]["profile_image_url"].as_str()?;

    if avatar.starts_with("https://") {
        Some(avatar.to_owned())
    } else {
        None
    }
}

// ─────────────────────────────────────────────
// EventSub Message Deduplication
// ─────────────────────────────────────────────

fn remember_message_id(
    event_id: &str,
    recent_ids: &mut HashSet<String>,
    recent_order: &mut VecDeque<String>,
) {
    recent_ids.insert(event_id.to_owned());
    recent_order.push_back(event_id.to_owned());

    while recent_order.len() > MAX_RECENT_MESSAGE_IDS {
        if let Some(oldest_id) = recent_order.pop_front() {
            recent_ids.remove(&oldest_id);
        }
    }
}
