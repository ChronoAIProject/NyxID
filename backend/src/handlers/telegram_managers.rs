use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::platform_credential::{COLLECTION_NAME, PlatformCredential},
    mw::auth::AuthUser,
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, header},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;
#[derive(Serialize)]
pub struct Manager {
    manager_bot_id: String,
    username: String,
    ready: bool,
    legacy_default: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    token: Zeroizing<String>,
}
fn headers() -> HeaderMap {
    HeaderMap::from_iter([(header::CACHE_CONTROL, "no-store".parse().unwrap())])
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<(HeaderMap, Json<Vec<Manager>>)> {
    super::platform_bots::admin(&state, &auth).await?;
    let rows: Vec<_> = state
        .db
        .collection::<PlatformCredential>(COLLECTION_NAME)
        .find(doc! {"provider":{"$regex":"^telegram-new(?:/[0-9]+)?$"}})
        .sort(doc! {"_id":1})
        .limit(101)
        .await?
        .try_collect()
        .await?;
    Ok((
        headers(),
        Json(
            rows.into_iter()
                .filter_map(|r| {
                    Some(Manager {
                        manager_bot_id: r.fields.get("manager_bot_id")?.clone(),
                        username: r
                            .fields
                            .get("manager_username")
                            .cloned()
                            .unwrap_or_default(),
                        ready: r.fields.get("webhook_ready").is_some_and(|v| v == "true"),
                        legacy_default: r.provider == "telegram-new",
                    })
                })
                .collect(),
        ),
    ))
}
pub async fn configure(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<Configure>,
) -> AppResult<(HeaderMap, Json<Manager>)> {
    super::platform_bots::admin(&state, &auth).await?;
    let id = body
        .token
        .split(':')
        .next()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|id| *id > 0)
        .ok_or_else(|| AppError::ValidationError("Invalid manager token".into()))?;
    let mut service = super::telegram_new::service(&state);
    service.manager_id = Some(id);
    let fields = std::collections::BTreeMap::from([(
        crate::services::channel_adapters::telegram_new::MANAGER_TOKEN.into(),
        Some(body.token),
    )]);
    service
        .configure_manager(&auth.user_id.to_string(), &fields, false)
        .await?;
    let configured = stored_manager(&state.db, &service.manager_provider()).await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "telegram_manager_configured",
        Some(serde_json::json!({"manager_bot_id":id.to_string()})),
    );
    Ok((headers(), Json(configured)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clear {
    manager_bot_id: String,
}

pub async fn clear(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<Clear>,
) -> AppResult<axum::http::StatusCode> {
    super::platform_bots::admin(&state, &auth).await?;
    let service = super::telegram_new::service(&state)
        .for_manager(
            body.manager_bot_id
                .parse::<i64>()
                .map_err(|_| AppError::ValidationError("Invalid manager identity".into()))?,
        )
        .await?;
    if service.manager_id.is_none() {
        return Err(AppError::Conflict(
            "Manage the default manager in Platform Credentials.".into(),
        ));
    }
    service.clear_manager_as(&auth.user_id.to_string()).await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "telegram_manager_cleared",
        Some(serde_json::json!({"manager_bot_id":body.manager_bot_id.to_string()})),
    );
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Read the same persisted metadata as the manager list after configuration.
async fn stored_manager(db: &mongodb::Database, provider: &str) -> AppResult<Manager> {
    let row = db
        .collection::<PlatformCredential>(COLLECTION_NAME)
        .find_one(doc! {"provider":provider})
        .await?
        .ok_or_else(|| AppError::NotFound("Telegram manager unavailable".into()))?;
    Ok(Manager {
        manager_bot_id: row
            .fields
            .get("manager_bot_id")
            .cloned()
            .ok_or_else(|| AppError::Conflict("Manager identity unavailable".into()))?,
        username: row
            .fields
            .get("manager_username")
            .cloned()
            .unwrap_or_default(),
        ready: row.fields.get("webhook_ready").is_some_and(|v| v == "true"),
        legacy_default: row.provider == "telegram-new",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn configured_manager_response_uses_stored_readiness_and_provider_identity() {
        let (state, _, _) = crate::services::telegram_new_tests::fixture().await;
        let rows = state.db.collection::<PlatformCredential>(COLLECTION_NAME);
        for provider in ["telegram-new", "telegram-new/200"] {
            if provider != "telegram-new" {
                let mut row = rows
                    .find_one(doc! {"provider":"telegram-new"})
                    .await
                    .unwrap()
                    .unwrap();
                row.id = uuid::Uuid::new_v4().to_string();
                row.provider = provider.into();
                row.fields.insert("manager_bot_id".into(), "200".into());
                rows.insert_one(row).await.unwrap();
            }
            for ready in [None, Some("false"), Some("true")] {
                let update = ready.map_or_else(
                    || doc! {"$unset":{"fields.webhook_ready":""}},
                    |value| doc! {"$set":{"fields.webhook_ready":value}},
                );
                rows.update_one(doc! {"provider":provider}, update)
                    .await
                    .unwrap();
                let response = stored_manager(&state.db, provider).await.unwrap();
                let json = serde_json::to_value(response).unwrap();
                assert_eq!(json["ready"], ready == Some("true"));
                assert_eq!(json["legacy_default"], provider == "telegram-new");
                assert_eq!(
                    json["manager_bot_id"],
                    if provider == "telegram-new" {
                        "100"
                    } else {
                        "200"
                    }
                );
            }
        }
        state.db.drop().await.unwrap();
    }
}
