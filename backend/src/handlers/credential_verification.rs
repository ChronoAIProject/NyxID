//! Human checks and best-effort post-save verification through the normal proxy.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{credential_verification as verification, org_service},
    telemetry::TelemetryContext,
};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::Request,
};
use chrono::Utc;
use mongodb::bson::{self, doc};
use std::sync::{Arc, Mutex};
use verification::{CheckContext, VerificationView};

pub async fn check(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<VerificationView>> {
    super::login_client_context::require_first_party_human(&user)?;
    let snapshot = authorized_snapshot(&state, &user, &id).await?;
    let Some(view) = snapshot.view() else {
        return Err(AppError::BadRequest(
            "This connection does not support server credential checks".into(),
        ));
    };
    if !view.can_check {
        return Ok(Json(view));
    }
    Ok(Json(run(&state, &user, snapshot).await.unwrap_or_else(
        |_| VerificationView::unverified("check_unavailable", true),
    )))
}

async fn authorized_snapshot(
    state: &AppState,
    user: &AuthUser,
    id: &str,
) -> AppResult<verification::Snapshot> {
    let snapshot = verification::snapshot(&state.db, id)
        .await?
        .ok_or_else(|| AppError::NotFound("Connection not found".into()))?;
    let access = org_service::resolve_owner_access(
        &state.db,
        &user.user_id.to_string(),
        &snapshot.service.user_id,
    )
    .await?;
    if !access.can_write()
        || !access.allows_resource(id)
        || (!user.allow_all_services && !user.allowed_service_ids.iter().any(|v| v == id))
    {
        return Err(AppError::Forbidden(
            "Connection owner or organization admin access is required".into(),
        ));
    }
    Ok(snapshot)
}

/// The save has already committed. Failure can never turn it into an HTTP failure.
pub async fn after_save(state: &AppState, user: &AuthUser, id: &str) -> Option<VerificationView> {
    let snapshot = authorized_snapshot(state, user, id).await.ok()?;
    let view = snapshot.view()?;
    if !view.can_check {
        return Some(view);
    }
    if super::login_client_context::require_first_party_human(user).is_err() {
        return Some(VerificationView::unverified("check_not_authorized", true));
    }
    Some(
        run(state, user, snapshot)
            .await
            .unwrap_or_else(|_| VerificationView::unverified("check_unavailable", true)),
    )
}

async fn run(
    state: &AppState,
    user: &AuthUser,
    snapshot: verification::Snapshot,
) -> AppResult<VerificationView> {
    let Some(operation) = verification::operation(&state.db, &snapshot).await? else {
        return Ok(VerificationView::unverified("check_unavailable", false));
    };
    let catalog = snapshot
        .catalog
        .as_ref()
        .expect("operation requires catalog");
    let declaration = catalog
        .credential_verification
        .as_ref()
        .expect("operation requires declaration");
    let started = Utc::now();
    if !verification::claim(&state.db, &snapshot, started).await? {
        return Ok(VerificationView::unverified("check_rate_limited", true));
    }
    let outcome = Arc::new(Mutex::new(None));
    let query = {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("_nyxid_via", &snapshot.service.id);
        for (key, value) in &declaration.query {
            query.append_pair(key, value);
        }
        query.finish()
    };
    let path = operation.path.trim_start_matches('/');
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/proxy/{}/{}?{}", catalog.id, path, query))
        .extension(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        )
        .body(Body::empty())
        .map_err(|_| AppError::Internal("Verification request construction failed".into()))?;
    let mut proxy_state = state.clone();
    proxy_state.http_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(verification::TIMEOUT)
        .build()
        .map_err(|_| AppError::Internal("Verification transport unavailable".into()))?;
    let result = tokio::time::timeout(
        verification::TIMEOUT,
        verification::CHECK.scope(
            CheckContext {
                db: state.db.clone(),
                actor_id: user.user_id.to_string(),
                service_id: snapshot.service.id.clone(),
                key_id: snapshot.key.id.clone(),
                epoch: snapshot.key.credential_epoch,
                binding: snapshot.binding(),
                outcome: outcome.clone(),
            },
            Box::pin(super::proxy::proxy_request(
                State(proxy_state),
                user.clone(),
                TelemetryContext::default(),
                Path((catalog.id.clone(), path.into())),
                request,
            )),
        ),
    )
    .await;
    let fallback = if result.is_err() {
        ("not_verified", "check_timeout")
    } else {
        ("not_verified", "check_unavailable")
    };
    let (status, reason) = outcome
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or(fallback);
    let current = authorized_snapshot(state, user, &snapshot.service.id)
        .await
        .ok();
    let Some(current) = current.filter(|v| v.binding() == snapshot.binding()) else {
        return Ok(VerificationView::unverified("binding_changed", true));
    };
    let current_operation = verification::operation(&state.db, &current).await?;
    if current_operation.is_none_or(|op| {
        op.id != operation.id
            || op.operation_generation != operation.operation_generation
            || op.updated_at != operation.updated_at
    }) {
        return Ok(VerificationView::unverified("binding_changed", true));
    }
    let observed = Utc::now();
    let stored=state.db.collection::<bson::Document>("user_services").update_one(doc!{
        "_id":&snapshot.service.id,"user_id":&snapshot.service.user_id,
        "updated_at":bson::DateTime::from_chrono(snapshot.service.updated_at),
        "credential_verification.binding":snapshot.binding(),"credential_verification.started_at":bson::DateTime::from_chrono(started),
    },doc!{"$set":{"credential_verification.state":status,"credential_verification.reason_code":reason,"credential_verification.observed_at":bson::DateTime::from_chrono(observed)}}).await?;
    if stored.matched_count != 1 {
        return Ok(VerificationView::unverified("binding_changed", true));
    }
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        user,
        "credential_verification_checked",
        Some(
            serde_json::json!({"user_service_id":snapshot.service.id,"state":status,"reason_code":reason}),
        ),
    );
    // Like the Codex verifier, return a fresh binding-aware projection rather
    // than echoing a provider result after a concurrent replacement.
    Ok(authorized_snapshot(state, user, &snapshot.service.id)
        .await?
        .view()
        .unwrap_or_else(|| VerificationView::unverified("binding_changed", false)))
}

#[cfg(test)]
#[path = "credential_verification_tests.rs"]
mod tests;
