//! Wave-4 org, account, approval, notification and app assistant action\n//! effects (`org.*`, `account.*`, `approval.*`, `notifications.*`,\n//! `service_account.*`, `developer_app.*`, `external_key.add_gcp_service_account`,\n//! `openclaw.connect`).
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
