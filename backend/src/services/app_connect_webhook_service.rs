//! App Connect Link terminal events use a bounded durable outbox on the session.
//! Reservation freezes safe metadata and one event ID. The expiry sweep recovers
//! both unreserved transitions and stale dispatches. Receivers must deduplicate
//! IDs: a crash after HTTP acceptance can still redeliver the same event.

use std::collections::BTreeMap;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::ReturnDocument;
use uuid::Uuid;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::app_connect_link::{
    AppConnectLink, AppConnectOrigin, AppConnectStatus, AppConnectWebhookData,
    AppConnectWebhookItem, AppConnectWebhookOrigin, COLLECTION_NAME as LINKS,
};
use crate::models::oauth_client::{COLLECTION_NAME as CLIENTS, OauthClient};
use crate::models::user_service::{COLLECTION_NAME as SERVICES, UserService};
use crate::services::connect_link_service::{
    WEBHOOK_MAX_DISPATCH_CYCLES, WEBHOOK_REDISPATCH_STALE_SECS,
};
use crate::services::webhook_delivery_service::DeliveryFailure;

const TERMINAL_STATUSES: &[&str] = &["completed", "cancelled", "expired", "failed"];

pub fn terminal_event_type(status: AppConnectStatus) -> Option<&'static str> {
    match status {
        AppConnectStatus::Completed => Some("app_connect_link.completed"),
        AppConnectStatus::Cancelled => Some("app_connect_link.cancelled"),
        AppConnectStatus::Expired => Some("app_connect_link.expired"),
        AppConnectStatus::Failed => Some("app_connect_link.failed"),
        AppConnectStatus::InProgress | AppConnectStatus::ReadyForConsent => None,
    }
}

async fn payload(state: &AppState, link: &AppConnectLink) -> AppResult<AppConnectWebhookData> {
    let ids: Vec<_> = link
        .items
        .iter()
        .filter_map(|i| i.user_service_id.as_ref())
        .collect();
    // Include disabled/tombstoned rows; these are session selections, not grants.
    let services: Vec<UserService> = state
        .db
        .collection::<UserService>(SERVICES)
        .find(doc! { "_id": { "$in": ids } })
        .await?
        .try_collect()
        .await?;
    let slugs: BTreeMap<_, _> = services.into_iter().map(|s| (s.id, s.slug)).collect();
    Ok(AppConnectWebhookData {
        user_id: link.user_id.clone(),
        app_connect_link_id: link.id.clone(),
        origin: match link.origin {
            AppConnectOrigin::App { .. } => AppConnectWebhookOrigin::App,
            AppConnectOrigin::Authorize { .. } => AppConnectWebhookOrigin::Authorize,
        },
        requirements_version: link.manifest_version,
        status: link.status,
        failure_reason: link.failure_reason.clone(),
        grant_update_required: link.grant_update_required,
        items: link
            .items
            .iter()
            .map(|i| AppConnectWebhookItem {
                requirement_id: i.requirement_id.clone(),
                state: i.state,
                user_service_id: i.user_service_id.clone(),
                slug: i
                    .user_service_id
                    .as_ref()
                    .and_then(|id| slugs.get(id))
                    .cloned(),
            })
            .collect(),
    })
}

async fn reserve(state: &AppState, id: &str) -> AppResult<Option<AppConnectLink>> {
    let collection = state.db.collection::<AppConnectLink>(LINKS);
    let Some(link) = collection
        .find_one(doc! {
            "_id": id, "status": { "$in": TERMINAL_STATUSES }, "webhook_event_reserved_at": null,
        })
        .await?
    else {
        return Ok(None);
    };
    let data = bson::to_bson(&payload(state, &link).await?)
        .map_err(|_| AppError::Internal("Could not encode app connect event".into()))?;
    Ok(collection
        .find_one_and_update(
            doc! {
                "_id": id, "revision": link.revision, "webhook_event_reserved_at": null,
                "status": { "$in": TERMINAL_STATUSES },
            },
            doc! { "$set": {
                "webhook_event_id": Uuid::new_v4().to_string(),
                "webhook_event_reserved_at": bson::DateTime::now(),
                "webhook_event_status": "pending", "webhook_event_attempts": 1_i32,
                "webhook_event_data": data,
            } },
        )
        .return_document(ReturnDocument::After)
        .await?)
}

/// Best effort after a successful terminal CAS/transaction. Failures leave the
/// terminal row recoverable by the sweep and never roll back code issuance.
pub async fn dispatch_terminal_webhook_if_needed(state: &AppState, id: &str) {
    match reserve(state, id).await {
        Ok(Some(link)) => spawn_delivery(state.clone(), link),
        Ok(None) => {}
        Err(error) => tracing::warn!(
            app_connect_link_id = id,
            error_code = error.error_code(),
            "Could not reserve app connect webhook"
        ),
    }
}

pub async fn redispatch_terminal_webhooks(state: &AppState) -> AppResult<()> {
    let collection = state.db.collection::<AppConnectLink>(LINKS);
    // Covers a crash after the terminal transition but before its first reservation.
    let mut unreserved = collection
        .find(doc! {
            "status": { "$in": TERMINAL_STATUSES }, "webhook_event_reserved_at": null,
        })
        .await?;
    while let Some(link) = unreserved.try_next().await? {
        dispatch_terminal_webhook_if_needed(state, &link.id).await;
    }
    let stale_before =
        bson::DateTime::from_chrono(Utc::now() - Duration::seconds(WEBHOOK_REDISPATCH_STALE_SECS));
    let mut stale = collection
        .find(doc! {
            "status": { "$in": TERMINAL_STATUSES }, "webhook_event_status": "pending",
            "webhook_event_reserved_at": { "$lte": stale_before },
            "webhook_event_attempts": { "$lt": i64::from(WEBHOOK_MAX_DISPATCH_CYCLES) },
        })
        .await?;
    while let Some(link) = stale.try_next().await? {
        let claimed = collection
            .find_one_and_update(
                doc! {
                    "_id": &link.id, "webhook_event_id": &link.webhook_event_id,
                    "webhook_event_status": "pending",
                    "webhook_event_attempts": i64::from(link.webhook_event_attempts),
                    "webhook_event_reserved_at": { "$lte": stale_before },
                },
                doc! {
                    "$set": { "webhook_event_reserved_at": bson::DateTime::now() },
                    "$inc": { "webhook_event_attempts": 1_i32 },
                },
            )
            .return_document(ReturnDocument::After)
            .await?;
        if let Some(link) = claimed {
            spawn_delivery(state.clone(), link);
        }
    }
    abandon_stale_exhausted_webhooks(state).await
}

async fn abandon_stale_exhausted_webhooks(state: &AppState) -> AppResult<()> {
    let mut exhausted = state
        .db
        .collection::<AppConnectLink>(LINKS)
        .find(doc! {
            "status": { "$in": TERMINAL_STATUSES }, "webhook_event_status": "pending",
            "webhook_event_attempts": { "$gte": i64::from(WEBHOOK_MAX_DISPATCH_CYCLES) },
            "webhook_event_reserved_at": { "$lte": bson::DateTime::from_chrono(
                Utc::now() - Duration::seconds(WEBHOOK_REDISPATCH_STALE_SECS)) },
        })
        .await?;
    while let Some(link) = exhausted.try_next().await? {
        abandon(state, &link, failure("dispatch_cycle_cap_reached")).await;
    }
    Ok(())
}

fn failure(reason: &'static str) -> DeliveryFailure {
    DeliveryFailure {
        attempts: 0,
        reason,
        last_status: None,
    }
}

async fn delivery_enabled(state: &AppState, app_id: &str) -> AppResult<bool> {
    let Some(client) = state
        .db
        .collection::<OauthClient>(CLIENTS)
        .find_one(doc! { "_id": app_id })
        .await?
    else {
        return Ok(false);
    };
    super::app_connect_rollout::is_enabled_for(state, &client).await
}

fn spawn_delivery(state: AppState, link: AppConnectLink) {
    tokio::spawn(async move {
        let (Some(event_id), Some(event_type), Some(data)) = (
            link.webhook_event_id.as_deref(),
            terminal_event_type(link.status),
            link.webhook_event_data.as_ref(),
        ) else {
            return;
        };
        // Check live policy for every cycle, including events reserved before a rollback.
        // Suppressed events are abandoned so enabling rollout does not replay them later.
        let result = match delivery_enabled(&state, &link.oauth_client_id).await {
            Ok(true) => match serde_json::to_value(data) {
                Ok(data) => {
                    state
                        .developer_webhook_dispatcher
                        .deliver_for_app(
                            &state.db,
                            &link.oauth_client_id,
                            event_id,
                            event_type,
                            data,
                        )
                        .await
                }
                Err(_) => Err(failure("serialization_failed")),
            },
            Ok(false) => {
                abandon(&state, &link, failure("app_connect_disabled")).await;
                return;
            }
            Err(_) => Err(failure("configuration_load_failed")),
        };
        match result {
            Ok(()) => {
                if let Err(error) = state
                    .db
                    .collection::<AppConnectLink>(LINKS)
                    .update_one(
                        doc! { "_id": &link.id, "webhook_event_id": event_id,
                        "webhook_event_status": "pending" },
                        doc! { "$set": { "webhook_event_status": "delivered",
                        "webhook_event_delivered_at": bson::DateTime::now() } },
                    )
                    .await
                {
                    tracing::warn!(app_connect_link_id = %link.id, %error,
                        "Could not mark app connect webhook delivered");
                }
            }
            Err(failure) if link.webhook_event_attempts >= WEBHOOK_MAX_DISPATCH_CYCLES => {
                abandon(&state, &link, failure).await;
            }
            Err(failure) => tracing::warn!(app_connect_link_id = %link.id, event_id,
                cycle = link.webhook_event_attempts, reason = failure.reason,
                "App connect webhook cycle failed; outbox remains pending"),
        }
    });
}

async fn abandon(state: &AppState, link: &AppConnectLink, failure: DeliveryFailure) {
    let (Some(event_id), Some(event_type)) = (
        link.webhook_event_id.as_deref(),
        terminal_event_type(link.status),
    ) else {
        return;
    };
    let updated = state
        .db
        .collection::<AppConnectLink>(LINKS)
        .update_one(
            doc! { "_id": &link.id, "webhook_event_id": event_id, "webhook_event_status": "pending",
            "webhook_event_attempts": i64::from(link.webhook_event_attempts) },
            doc! { "$set": { "webhook_event_status": "abandoned" } },
        )
        .await;
    if matches!(updated, Ok(result) if result.modified_count == 1) {
        super::developer_webhook_service::record_terminal_delivery_failure(
            &state.db,
            &link.oauth_client_id,
            event_id,
            event_type,
            failure,
        )
        .await;
    }
}
