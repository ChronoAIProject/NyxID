//! Catalog-authoritative host moves. No redirect following and no arbitrary key scan.
use super::{nyxbot_gateway_release_service as releases, nyxbot_gateway_service as gateway};
use crate::models::{
    api_key::COLLECTION_NAME as KEYS,
    channel_whatsapp_native::COLLECTION_NAME as WHATSAPP,
    downstream_service::COLLECTION_NAME as SERVICES,
    nyxbot_channel::COLLECTION_NAME as CHANNELS,
    nyxbot_gateway_release::{COLLECTION_NAME as RELEASES, NyxbotGatewayRelease, ReleaseState},
    platform_entry_gateway::{COLLECTION_NAME as ENTRIES, PlatformEntryGateway},
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
};
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};
use serde::Deserialize;
use serde_json::json;
use std::time::{Duration, Instant};

const BATCH: i64 = 64;
const BACKSTOP: Duration = Duration::from_secs(3600);

// Reject before URL parsing: the URL parser normalizes dot segments, escaped
// dots and backslashes. None of those spellings may mint a new destination.
fn strict_url(value: &str, local_http: bool) -> Option<url::Url> {
    if !value.is_ascii()
        || value
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
        || value.contains(['%', '\\', '?', '#'])
    {
        return None;
    }
    let parsed = url::Url::parse(value).ok()?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(parsed.scheme() == "https" || (local_http && local && parsed.scheme() == "http"))
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    let authority = value.split_once("://")?.1;
    let (host, path) = authority.split_once('/').unwrap_or((authority, ""));
    if host.contains('@')
        || path.split('/').any(|s| s == "." || s == "..")
        || path.starts_with('/')
        || path.contains("//")
    {
        return None;
    }
    let expected = format!("/{path}");
    (parsed.path() == expected).then_some(parsed)
}

fn callback_id(value: &str) -> Option<&str> {
    strict_url(value, false)?;
    let (prefix, id) = value.rsplit_once('/')?;
    if !prefix.ends_with("/callbacks") {
        return None;
    }
    valid_callback_id(id).then_some(id)
}

pub(crate) fn valid_callback_id(id: &str) -> bool {
    // CMA cmaeg-core/src/seal.rs: Channel tokens have four components,
    // a canonical u8 key ID, and unpadded base64url ciphertext (nonce + tag).
    // MAX_TOKEN_LENGTH is 4096; real exported channel IDs exceed 200 bytes.
    // This checks syntax only, never decrypts or interprets the opaque value.
    use base64::Engine;
    if id.len() > 4096 {
        return false;
    }
    let Some((kid, sealed)) = id
        .strip_prefix("cmaeg1.ch.")
        .and_then(|s| s.split_once('.'))
    else {
        return false;
    };
    if !kid
        .parse::<u8>()
        .is_ok_and(|value| value.to_string() == kid)
    {
        return false;
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(sealed)
        .is_ok_and(|bytes| bytes.len() >= 28)
}

/// Only the administrator's current catalog base chooses the destination.
/// The stored URL contributes a validated opaque callback suffix, never a host.
pub(crate) fn rebase_callback(stored: &str, current_base: &str) -> Option<String> {
    let id = callback_id(stored)?;
    let base = current_base.trim_end_matches('/');
    strict_url(base, true)?;
    let next = format!("{base}/callbacks/{id}");
    (next != stored).then_some(next)
}

pub(crate) struct Catalog {
    id: String,
    raw_base: String,
    pub(crate) base: String,
}

impl Catalog {
    // Fence concurrent administrator edits in the same transaction as every
    // destination change, so a stale scan cannot overwrite a newer host move.
    pub(crate) async fn fence(&self, db: &Database, session: &mut ClientSession) -> AppResult<()> {
        if db.collection::<Document>(SERVICES).update_one(doc! {
            "_id":&self.id,"slug":"cmaeg","is_active":true,"base_url":&self.raw_base,"requires_user_credential":{"$ne":true},
        }, doc! {"$inc":{"gateway_host_revision":1_i64}}).session(session).await?.matched_count != 1 {
            return Err(AppError::Conflict("Gateway catalog changed".into()));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct Reconciler {
    completed: Option<(String, String, Instant)>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) nyxbot: u64,
    pub(crate) platform_entries: u64,
    pub(crate) releases: u64,
    pub(crate) deferred: u64,
    pub(crate) whatsapp_manual: u64,
    pub(crate) releases_review: u64,
}

impl Reconciler {
    pub(crate) async fn tick(&mut self, state: &AppState) -> AppResult<Option<Counts>> {
        let service =
            match super::assistant_service::resolve_admin_service_by_slug(&state.db, "cmaeg").await
            {
                Ok(row) => row,
                Err(AppError::Internal(_)) => {
                    self.completed = None;
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
        let base = gateway::validated_base(&service.base_url)?;
        if strict_url(&base, true).is_none() {
            return Err(AppError::ValidationError(
                "gateway_configuration_required".into(),
            ));
        }
        if self.completed.as_ref().is_some_and(|(id, previous, at)| {
            id == &service.id && previous == &base && at.elapsed() < BACKSTOP
        }) {
            return Ok(None);
        }
        let catalog = Catalog {
            id: service.id,
            raw_base: service.base_url,
            base,
        };
        let counts = reconcile(state, &catalog).await?;
        if counts.deferred == 0 {
            self.completed = Some((catalog.id, catalog.base, Instant::now()));
        }
        tracing::info!(
            nyxbot = counts.nyxbot,
            platform_entries = counts.platform_entries,
            releases = counts.releases,
            deferred = counts.deferred,
            whatsapp_manual = counts.whatsapp_manual,
            releases_review = counts.releases_review,
            "Gateway host reconciliation completed"
        );
        Ok(Some(counts))
    }
}

/// Audit only local row identity and hosts. Never include a callback path,
/// remote channel reference, credential, full URL, or provider response.
pub(crate) async fn audit(
    db: &Database,
    tx: &mut ClientSession,
    owner: &str,
    kind: &str,
    id: &str,
    old: &str,
    new: &str,
) -> AppResult<()> {
    releases::audit(
        db,
        tx,
        owner,
        "nyxbot_gateway_callback_rebased",
        json!({
            "row_kind":kind,"row_id":id,
            "old_host":url::Url::parse(old).ok().and_then(|u| u.host_str().map(str::to_owned)),
            "new_host":url::Url::parse(new).ok().and_then(|u| u.host_str().map(str::to_owned)),
        }),
    )
    .await
}

async fn batch<T: serde::de::DeserializeOwned + Send + Sync>(
    db: &Database,
    collection: &str,
    after: &str,
) -> AppResult<Vec<T>> {
    // _id pagination uses the built-in unique index. Filter eligibility after
    // the bounded read so sparse matches never require an unbounded scan.
    Ok(db
        .collection::<T>(collection)
        .find(doc! {"_id":{"$gt":after}})
        .sort(doc! {"_id":1})
        .limit(BATCH)
        .await?
        .try_collect()
        .await?)
}

#[derive(Deserialize)]
struct Route {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    #[serde(default)]
    bot_owner_id: Option<String>,
    #[serde(default)]
    route_api_key_id: String,
    #[serde(default)]
    transport: String,
    #[serde(default)]
    status: String,
}

async fn route_callback(db: &Database, key: &str, owner: &str) -> AppResult<Option<String>> {
    Ok(db
        .collection::<Document>(KEYS)
        .find_one(doc! {"_id":key,"user_id":owner})
        .projection(doc! {"callback_url":1})
        .await?
        .and_then(|d| d.get_str("callback_url").ok().map(str::to_owned)))
}

async fn rebase_route(
    state: &AppState,
    catalog: &Catalog,
    row: &Route,
    old: &str,
    next: &str,
) -> AppResult<bool> {
    super::transaction_service::run_db_only(&state.db, &(state, catalog, row, old, next), |tx,(state,catalog,row,old,next)| Box::pin(async move {
        let db = &state.db;
        let owner = row.bot_owner_id.as_deref().unwrap_or(&row.user_id);
        if db.collection::<Document>(KEYS).find_one(doc! {"_id":&row.route_api_key_id,"user_id":owner,"callback_url":*old})
            .projection(doc! {"_id":1}).session(&mut *tx).await?.is_none() {return Ok(false);}
        // The key must still be the dedicated route of this gateway link.
        if db.collection::<Document>(CHANNELS).update_one(doc! {"_id":&row.id,"user_id":&row.user_id,
            "bot_owner_id":&row.bot_owner_id,"route_api_key_id":&row.route_api_key_id,"transport":"gateway","status":{"$ne":"deleted"}},
            doc! {"$inc":{"gateway_host_revision":1_i64}}).session(&mut *tx).await?.matched_count != 1 { return Err(AppError::Conflict("Gateway route changed".into())); }
        catalog.fence(db, tx).await?;
        let changed = super::api_key_mutation_service::update_one(db,
            doc! {"_id":&row.route_api_key_id,"user_id":owner,"callback_url":*old},
            doc! {"$set":{"callback_url":*next}}, Some(&mut *tx)).await?.matched_count == 1;
        if !changed {return Err(AppError::Conflict("Gateway callback changed".into()));}
        audit(db,tx,&row.user_id,"nyxbot",&row.id,old,next).await?;
        Ok(changed)
    })).await
}

async fn rebase_release(
    state: &AppState,
    catalog: &Catalog,
    row: &NyxbotGatewayRelease,
) -> AppResult<bool> {
    super::transaction_service::run_db_only(&state.db, &(state,catalog,row), |tx,(state,catalog,row)| Box::pin(async move {
        let db = &state.db;
        catalog.fence(db,tx).await?;
        let mut set = doc! {"gateway_base_url":&catalog.base,"updated_at":bson::DateTime::now()};
        // Retry only reviews caused by this host mismatch. All remote identity,
        // version, live-reference and creator-issuer checks still run normally.
        if row.state == ReleaseState::ReviewRequired && row.last_error.as_deref() == Some("gateway_destination_changed") {
            set.insert("state", "due");
            set.insert("next_attempt_at", bson::DateTime::now());
            set.insert("last_error", bson::Bson::Null);
        }
        let changed = db.collection::<Document>(RELEASES).update_one(doc! {
            "_id":&row.id,"gateway_service_id":&catalog.id,"gateway_base_url":&row.gateway_base_url,
            "creator_issuer":&row.creator_issuer,"revision":row.revision,"state":bson::to_bson(&row.state).map_err(|_|AppError::Internal("Invalid release state".into()))?,
            "claim_id":&row.claim_id,"lease_until":row.lease_until.map(bson::DateTime::from_chrono),
            "$expr":{"$lte":[{"$ifNull":["$lease_until",bson::DateTime::from_millis(0)]},"$$NOW"]},
        }, doc! {"$set":set,"$inc":{"revision":1_i64}}).session(&mut *tx).await?.matched_count == 1;
        if !changed { return Err(AppError::Conflict("Gateway release changed".into())); }
        audit(db,tx,&row.owner_user_id,"release",&row.id,&row.gateway_base_url,&catalog.base).await?;
        Ok(true)
    })).await
}

async fn reconcile(state: &AppState, catalog: &Catalog) -> AppResult<Counts> {
    let mut counts = Counts::default();
    let mut after = String::new();
    loop {
        let rows: Vec<Route> = batch(&state.db, CHANNELS, &after).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            after = row.id.clone();
            if row.transport != "gateway" || row.status == "deleted" {
                continue;
            }
            let owner = row.bot_owner_id.as_deref().unwrap_or(&row.user_id);
            let Some(old) = route_callback(&state.db, &row.route_api_key_id, owner).await? else {
                continue;
            };
            let Some(next) = rebase_callback(&old, &catalog.base) else {
                continue;
            };
            match rebase_route(state, catalog, &row, &old, &next).await {
                Ok(true) => counts.nyxbot += 1,
                Ok(false) | Err(AppError::Conflict(_)) => counts.deferred += 1,
                Err(error) => return Err(error),
            }
        }
        tokio::task::yield_now().await;
    }
    after.clear();
    loop {
        let rows: Vec<PlatformEntryGateway> = batch(&state.db, ENTRIES, &after).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            after = row.id.clone();
            if !matches!(row.state.as_str(), "active" | "prepared")
                || (row.gateway_base == catalog.base
                    && rebase_callback(&row.callback_url, &catalog.base).is_none())
            {
                continue;
            }
            match super::platform_entry_gateway_service::rebase_host(state, catalog, &row).await {
                Ok(true) => counts.platform_entries += 1,
                Ok(false) => {}
                Err(AppError::Conflict(_) | AppError::Forbidden(_)) => counts.deferred += 1,
                Err(error) => return Err(error),
            }
        }
        tokio::task::yield_now().await;
    }
    after.clear();
    loop {
        let rows: Vec<NyxbotGatewayRelease> = batch(&state.db, RELEASES, &after).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            after = row.id.clone();
            if row.state == ReleaseState::Settled || row.gateway_base_url == catalog.base {
                continue;
            }
            if row.gateway_service_id != catalog.id
                || !state.config.accepts_jwt_issuer(&row.creator_issuer)
                || strict_url(&row.gateway_base_url, true).is_none()
            {
                counts.releases_review += 1;
                continue;
            }
            if row
                .lease_until
                .is_some_and(|until| until > chrono::Utc::now())
            {
                counts.deferred += 1;
                continue;
            }
            match rebase_release(state, catalog, &row).await {
                Ok(true) => counts.releases += 1,
                Ok(false) | Err(AppError::Conflict(_)) => counts.deferred += 1,
                Err(error) => return Err(error),
            }
        }
        tokio::task::yield_now().await;
    }
    // Activation is intentionally not reused for a live native WhatsApp bot:
    // it requires the relay phase and may initiate rollback on failure. Count
    // affected native callbacks for a separate, owner-managed Meta repair.
    after.clear();
    loop {
        let rows: Vec<Document> = state.db.collection::<Document>(WHATSAPP)
            .find(doc! {"_id":{"$gt":&after}}).sort(doc! {"_id":1}).limit(BATCH)
            .projection(doc! {"callback.phase":1,"callback.gateway_channel_id":1,"callback.gateway_webhook_url":1})
            .await?.try_collect().await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            after = row
                .get_str("_id")
                .map_err(|_| AppError::Internal("Invalid native metadata".into()))?
                .into();
            let Ok(callback) = row.get_document("callback") else {
                continue;
            };
            if !matches!(
                callback.get_str("phase").ok(),
                Some("gateway" | "awaiting_gateway" | "awaiting_relay")
            ) {
                continue;
            }
            if let (Ok(remote), Ok(stored)) = (
                callback.get_str("gateway_channel_id"),
                callback.get_str("gateway_webhook_url"),
            ) && stored != format!("{}/whatsapp/{remote}", catalog.base)
            {
                counts.whatsapp_manual += 1;
            }
        }
        tokio::task::yield_now().await;
    }
    Ok(counts)
}

#[cfg(test)]
#[path = "nyxbot_gateway_host_move_tests.rs"]
mod tests;
