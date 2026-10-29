// ─────────────────────────────────────────────
// Server Modules
// ─────────────────────────────────────────────

mod routes;
mod websocket;

// ─────────────────────────────────────────────
// Public Server Interface
// ─────────────────────────────────────────────

pub(crate) use routes::AppState;
pub use routes::build_router;
