//! Wave-2 endpoint- and external-key-family assistant action effects
//! (`endpoint.update`, `endpoint.delete`, `external_key.rotate`,
//! `external_key.delete`).
//!
//! WS-0 lands this mount so `routes.rs` stays single-writer during the wave:
//! the owning workstream adds its effect handlers here and registers them in
//! [`router`] without touching shared files. Follow the reserve-then-commit
//! receipt discipline in `services/assistant_action_execution_service.rs` and
//! the evidence rules in `docs/chat/evidence-projection-conventions.md`.
//! Routes needing billing metering must attach a `BillingRoutePolicy`
//! extension; plain control-plane effects follow the existing
//! `/assistant/actions/key-create` precedent and carry none.

use axum::Router;

use crate::AppState;

/// Effect routes mounted at `/api/v1/assistant/actions/endpoints`.
pub fn router() -> Router<AppState> {
    Router::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_router_builds() {
        let _ = super::router();
    }
}
