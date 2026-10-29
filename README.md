# StreamFreezer

A lightweight Windows application for Twitch streaming alerts, built with Rust, Axum, HTML, CSS, and JavaScript.

## Current Status

**Implemented: Follow Alerts**

- Twitch Device Code OAuth authorization.
- Secure access and refresh token storage through Windows Credential Manager.
- Automatic session restoration and token refresh.
- Twitch EventSub WebSocket connection.
- `channel.follow` v2 subscription.
- Real follow notifications with follower usernames and profile pictures.
- Browser overlay with GSAP animation, FIFO alert queue, and duplicate-event protection.
- Synthetic follow testing.
- EventSub reconnect migration handling.

Reconnect migration has been implemented, but should be explicitly tested at runtime before being considered fully verified.

## Architecture

- `src/main.rs` — application startup, local Axum server, HTTP routes, WebSocket delivery, and embedded frontend assets.
- `src/messages.rs` — typed messages shared between the server and overlay.
- `src/twitch_auth.rs` — Twitch OAuth, token validation and refresh, account and scope validation, and credential storage.
- `src/eventsub.rs` — Twitch EventSub connection, follow subscription, event handling, profile image retrieval, and reconnect handling.
- `overlay/index.html` — alert markup and debug controls.
- `overlay/style.css` — overlay styling.
- `overlay/script.js` — WebSocket client, reconnect behavior, duplicate-message guard, FIFO queue, GSAP animation, and authorization UI.
- `overlay/assets/avatar.svg` — fallback avatar.

The frontend HTML, CSS, JavaScript, and SVG are embedded into the Rust executable at compile time. Rebuild the application after changing these files.

## Requirements

- Rust toolchain compatible with Edition 2024.
- Internet access for Twitch API and EventSub connections.
- Internet access for the existing GSAP CDN dependency.
- Windows for the intended OS credential-storage integration.

## Twitch Setup

The Twitch application uses a public Client ID registered in the Twitch Developer Console. The Client ID is an application identifier, not a client secret.

The current integration uses the `moderator:read:followers` scope and expects the broadcaster account `cxldhands`.

Authorization uses the OAuth Device Code flow. Tokens are stored through the operating system's credential store rather than in browser JavaScript or a plain-text configuration file. The application validates the saved session and attempts token refresh when necessary.

Never commit access tokens or refresh tokens to source control.

## Run

From the project root:

```powershell
cargo fmt
cargo test
cargo check
cargo run
```

Open the debug overlay:

`http://127.0.0.1:3001/?debug=1`

The normal OBS Browser Source URL is:

`http://127.0.0.1:3001/`

The debug page provides a synthetic follow test and Twitch account authorization controls.

## Endpoints

- `GET /health` — health check.
- `GET /ws` — local overlay WebSocket.
- `POST /api/test/follow` — broadcasts a synthetic follow event.
- `POST /api/twitch/auth/start` — starts Twitch Device Code authorization.
- `GET /api/twitch/auth/status` — returns non-secret authorization status.
- `GET /?debug=1` — debug overlay and controls.

The synthetic follow endpoint returns an error if no overlay WebSocket client is connected.

## Git and Local Files

- `target/` contains generated build output and should not be committed.
- Local logs and environment files should not be committed.
- OAuth tokens are stored in Windows Credential Manager.

## Roadmap

Future tools may include subscription alerts, raid alerts, shoutouts, and other streaming utilities. Shared Twitch authentication, event handling, and overlay delivery should be reused where appropriate.

The current priority is to preserve and verify Follow Alerts before expanding the application.

## Verification

Run `cargo fmt --check`, `cargo test`, and `cargo check` after code changes. Test real follow events and overlay behavior manually when modifying EventSub or WebSocket handling.
