//! Wave-3 node and device assistant action effects (`node.register_token`,\n//! `node.rotate_token`, `node.delete`, `node.transfer`,\n//! `node.inject_credential`, `pending_credential.push`,\n//! `pending_credential.cancel`, `device.onboard`).
//!
//! Mounted empty by WS-0 so `routes.rs` stays single-writer while the owning
//! workstream fills this router. Receipts must use the shared
//! reserve-then-commit helper in `services/assistant_action_receipts.rs`;
//! destructive verbs confirm every time and are never remember-eligible.

use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
