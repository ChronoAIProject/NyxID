#![allow(
    clippy::large_enum_variant,
    clippy::collapsible_if,
    clippy::too_many_arguments,
    clippy::needless_borrow,
    clippy::items_after_test_module
)]

//! Purpose-confined native public inference. This does not change ordinary
//! platform-key payer resolution or consult a person's connections or agents.
use super::{
    api_key_mutation_service::{map_transaction_error, transaction_result},
    billing,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        credits::Credits,
        downstream_service::{DownstreamService, InferenceWireProtocol},
        platform_binding::PlatformBinding,
        platform_bot_config::PlatformBotConfig,
        service_billing::BillingMetric,
        usage_meter::CredentialClass,
    },
};
use futures::{StreamExt, TryStreamExt};
use mongodb::{
    Database,
    bson::{self, Document, doc},
};

const TURNS: &str = "platform_public_turns";
const OLD_LEGACY_TURN_INDEX: &str = "source_id_1_sender_1_update_id_1";
const LEGACY_TURN_INDEX: &str = "platform_legacy_public_turn_unique";
const QUOTAS: &str = "platform_public_quotas";
const RESPONSE_BYTES: usize = 64 * 1024;
fn denied() -> AppError {
    AppError::Conflict(
        "Public inference is unavailable; review platform funding and configuration".into(),
    )
}

#[derive(Clone)]
pub(crate) enum PublicPurpose {
    Guide,
    Readiness,
    Customer {
        installation: crate::models::platform_installation::PlatformInstallation,
        setup: crate::models::platform_setup::PlatformSetup,
        test_only: bool,
    },
}

#[derive(Clone)]
pub(crate) struct GuideAuthority {
    pub profile: PlatformBotConfig,
    pub binding: Option<PlatformBinding>,
    pub sender: String,
    pub update_id: i64,
    pub purpose: PublicPurpose,
    pub receipt: Option<super::platform_native_receipt_service::ReceiptClaim>,
}

fn scope(a: &GuideAuthority) -> String {
    match &a.purpose {
        PublicPurpose::Guide => format!("guide:{}", a.profile.id),
        PublicPurpose::Readiness => format!("readiness:{}", a.profile.id),
        PublicPurpose::Customer { installation, .. } => format!("child:{}", installation.id),
    }
}

pub(crate) async fn fence_public(
    db: &Database,
    a: &GuideAuthority,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    if let Some(claim) = &a.receipt {
        super::platform_native_receipt_service::fence(db, claim, session).await?;
    }
    if matches!(a.purpose, PublicPurpose::Readiness) {
        super::platform_bot_config_service::fence_admin(db, &a.profile.configured_by, session)
            .await?;
        if db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(doc! {"_id": &a.profile.id, "generation": a.profile.generation, "deleted_at": bson::Bson::Null},
            doc! {"$inc": {"readiness_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(denied()); }
    }
    if matches!(a.purpose, PublicPurpose::Guide) {
        if a.binding.as_ref().is_some_and(|binding| {
            binding.telegram_sender_id != a.sender || binding.telegram_chat_id != a.sender
        }) {
            return Err(AppError::Forbidden("Guide sender authority changed".into()));
        }
        super::platform_binding_service::fence_reply(db, &a.profile, a.binding.as_ref(), session)
            .await?;
    }
    if let PublicPurpose::Customer {
        installation,
        setup,
        test_only,
    } = &a.purpose
    {
        if !*test_only {
            super::platform_provisioning_service::assess_owned_runtime(
                db,
                &installation.authority,
                session,
            )
            .await?;
        }
        let expected = if *test_only {
            "ready_for_test"
        } else {
            "active"
        };
        if db.collection::<Document>(crate::models::platform_installation::COLLECTION_NAME).update_one(
            doc! {"_id": &installation.id, "generation": installation.generation, "owner_user_id": &setup.owner_user_id, "state": expected,
                "authority": bson::to_bson(&installation.authority).map_err(|_| denied())?}, doc! {"$inc": {"public_effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(denied()); }
        if setup.resources.agent_id.as_deref() != Some(installation.agent_id.as_str()) || db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME).update_one(
            doc! {"_id": &installation.agent_id, "user_id": &installation.owner_user_id, "platform_setup_id": &installation.setup_id, "destroyed_at": bson::Bson::Null},
            doc! {"$inc": {"public_effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(denied()); }
        let live = if *test_only {
            super::platform_provisioning_service::fence_authority(
                db,
                &installation.authority,
                session,
            )
            .await?
        } else {
            super::platform_provisioning_service::fence_owned_runtime(
                db,
                &installation.authority,
                session,
            )
            .await?
        };
        if (if *test_only {
            live.state != crate::models::platform_setup::SetupState::ReadyForTest
        } else {
            live.state != crate::models::platform_setup::SetupState::Active
        }) || live.resources.channel_bot_id.as_deref()
            != Some(installation.channel_bot_id.as_str())
        {
            return Err(denied());
        }
        if db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(
            doc! {"_id": &installation.channel_bot_id, "platform_installation_id": &installation.id, "is_active": true, "status": "active", "webhook_registered": true}, doc! {"$inc": {"public_effect_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(denied()); }
    }
    Ok(())
}

pub(crate) async fn admit_effect(state: &AppState, a: &GuideAuthority) -> AppResult<()> {
    let db = state.db.clone();
    let a = a.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| transaction_result(fence_public(&db, &a, session).await))
        .await
        .map_err(map_transaction_error)
}

fn legacy_turn_keys() -> Document {
    doc! {"source_id": 1, "sender": 1, "update_id": 1}
}

fn legacy_turn_partial() -> Document {
    // Runtime transcripts share this collection but never carry source_id.
    // Include even null/malformed legacy identities rather than silently
    // removing their existing deduplication constraint.
    doc! {"source_id": {"$exists": true}}
}

fn legacy_turn_index_matches(
    index: &mongodb::IndexModel,
    name: &str,
    partial: Option<Document>,
) -> bool {
    if !index.keys.iter().eq(legacy_turn_keys().iter()) {
        return false;
    }
    let Some(options) = &index.options else {
        return false;
    };
    let Ok(mut actual) = bson::to_document(options) else {
        return false;
    };
    // Index format version is server metadata, not a uniqueness/filter option.
    actual.remove("v");
    let mut expected = doc! {"name": name, "unique": true};
    if let Some(partial) = partial {
        expected.insert("partialFilterExpression", partial);
    }
    actual == expected
}

async fn ensure_legacy_turn_index(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    let review = || mongodb::error::Error::custom("Public turn identity indexes require review");
    let collection = db.collection::<Document>(TURNS);
    collection
        .create_index(
            IndexModel::builder()
                .keys(legacy_turn_keys())
                .options(
                    IndexOptions::builder()
                        .name(LEGACY_TURN_INDEX.to_owned())
                        .unique(true)
                        .partial_filter_expression(legacy_turn_partial())
                        .build(),
                )
                .build(),
        )
        .await?;
    let indexes = collection
        .list_indexes()
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let replacement_exists = |indexes: &[IndexModel]| {
        indexes.iter().any(|index| {
            legacy_turn_index_matches(index, LEGACY_TURN_INDEX, Some(legacy_turn_partial()))
        })
    };
    if !replacement_exists(&indexes) {
        return Err(review());
    }
    let mut retire_old = false;
    for index in &indexes {
        let name = index
            .options
            .as_ref()
            .and_then(|options| options.name.as_deref());
        if name == Some(OLD_LEGACY_TURN_INDEX) {
            if !legacy_turn_index_matches(index, OLD_LEGACY_TURN_INDEX, None) {
                return Err(review());
            }
            retire_old = true;
        } else if index.keys.len() == legacy_turn_keys().len()
            && legacy_turn_keys()
                .keys()
                .all(|field| index.keys.contains_key(field))
            && index.options.as_ref().is_some_and(|options| {
                options.unique == Some(true)
                    && options.partial_filter_expression.as_ref() != Some(&legacy_turn_partial())
            })
        {
            // Unknown constraints require operator review; never guess a drop.
            return Err(review());
        }
    }
    // Every legacy identity is protected by the verified replacement before
    // retiring the recognized constraint. No stored rows are rewritten.
    if retire_old && let Err(error) = collection.drop_index(OLD_LEGACY_TURN_INDEX).await {
        let missing = matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 27);
        if !missing {
            return Err(error);
        }
        let remaining = collection
            .list_indexes()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        if !replacement_exists(&remaining)
            || remaining.iter().any(|index| {
                index
                    .options
                    .as_ref()
                    .and_then(|options| options.name.as_deref())
                    == Some(OLD_LEGACY_TURN_INDEX)
            })
        {
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<Document>(QUOTAS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"scope": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Document>(TURNS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"state": 1, "lease_expires_at": 1})
                .build(),
        )
        .await?;
    ensure_legacy_turn_index(db).await?;
    db.collection::<Document>(QUOTAS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"purge_after": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(TURNS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"purge_after": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub(crate) async fn sponsor_owner(
    state: &AppState,
    profile: &PlatformBotConfig,
) -> AppResult<String> {
    if state
        .db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(
            doc! {"_id": &profile.funding_owner_user_id, "user_type": "org", "is_active": true},
        )
        .await?
        .is_none()
    {
        return Err(denied());
    }
    let resolved = state
        .billing
        .owner_resolver()
        .resolve_for_wallet_management(&profile.configured_by, Some(&profile.funding_owner_user_id))
        .await?;
    if !matches!(
        resolved.pays,
        billing::owner_resolver::PaysFrom::OrgWallet { .. }
    ) || resolved.owner_id != profile.funding_owner_user_id
    {
        return Err(denied());
    }
    Ok(resolved.owner_id)
}

pub(crate) fn readiness_projection(service: &DownstreamService) -> serde_json::Value {
    serde_json::json!({"id": service.id, "slug": service.slug, "base_url": service.base_url,
        "service_type": service.service_type, "is_active": service.is_active, "visibility": service.visibility,
        "auth_method": service.auth_method, "auth_key_name": service.auth_key_name,
        "platform_key": service.platform_key, "inference": service.inference, "billing": service.billing,
        "provider_config_id": service.provider_config_id, "requires_user_credential": service.requires_user_credential,
        "updated_at": service.updated_at})
}

async fn revalidate_prepared_target(
    state: &AppState,
    profile: &PlatformBotConfig,
    captured: &DownstreamService,
    target: &super::proxy_service::ProxyTarget,
) -> AppResult<()> {
    let live = selected_service(state, profile).await?;
    let (auth_method, auth_key_name) =
        super::platform_key_service::effective_auth(&state.db, &live).await?;
    // Material is compared only in memory: never serialized into a filter, log or diagnostic.
    if readiness_projection(captured) != readiness_projection(&live)
        || captured.credential_encrypted != live.credential_encrypted
        || captured.default_request_headers != live.default_request_headers
        || auth_method != target.auth_method
        || auth_key_name != target.auth_key_name
    {
        return Err(denied());
    }
    Ok(())
}

pub(crate) async fn selected_service(
    state: &AppState,
    profile: &PlatformBotConfig,
) -> AppResult<DownstreamService> {
    let config = profile.guide.as_ref().ok_or_else(denied)?;
    super::platform_setup_service::validate_limits(&config.limits)?;
    let service = state.db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id": &config.model_service_id, "is_active": true, "service_type": "http", "inference.model_list": true, "slug": {"$ne": "llm-nyx"}}).await?.ok_or_else(denied)?;
    if config.model_id.is_empty()
        || config.model_id.len() > 200
        || config.model_id.chars().any(char::is_control)
        || !super::platform_key_service::available(
            &state.db,
            &service,
            &profile.funding_owner_user_id,
        )
        .await?
    {
        return Err(denied());
    }
    Ok(service)
}

fn live_leases() -> Document {
    doc! {"$filter": {"input": {"$ifNull": ["$leases", []]}, "as": "lease", "cond": {"$gt": ["$$lease.expires_at", "$$NOW"]}}}
}
fn window(unit: &str) -> Document {
    doc! {"$dateTrunc": {"date": "$$NOW", "unit": unit, "timezone": "UTC"}}
}

async fn seed(db: &Database, scope: &str) -> AppResult<()> {
    // Only the bounded, configured profile bucket is seeded outside admission.
    match db
        .collection::<Document>(QUOTAS)
        .update_one(
            doc! {"scope": scope},
            doc! {"$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "scope": scope}},
        )
        .upsert(true)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if super::telegram_new_service::is_duplicate_key_error(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn admit(state: &AppState, a: &GuideAuthority) -> AppResult<Option<String>> {
    let guide = a.profile.guide.as_ref().ok_or_else(denied)?;
    let global = scope(&a);
    seed(&state.db, &global).await?;
    let db = state.db.clone();
    let a = a.clone();
    let global = global.clone();
    let turn = uuid::Uuid::new_v4().to_string();
    let max_global = guide.platform_turns_per_hour;
    let concurrent_global = guide.platform_concurrent_turns;
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            fence_public(&db, &a, session).await?;
            if db.collection::<Document>(TURNS).find_one(doc! {"source_id": &global, "sender": &a.sender, "update_id": a.update_id}).session(&mut *session).await?.is_some() { return Ok(None); }
            let guide = a.profile.guide.as_ref().ok_or_else(denied)?;
            let sender_scope = format!("{global}:sender:{}", a.sender);
            db.collection::<Document>(QUOTAS).update_one(doc! {"scope": &sender_scope}, doc! {"$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "scope": &sender_scope}}).upsert(true).session(&mut *session).await?;
            for (scope, hourly, concurrency) in [(&global, max_global, concurrent_global), (&sender_scope, guide.limits.turns_per_hour, guide.limits.concurrent_turns)] {
                let hour_count = doc! {"$cond": [{"$eq": ["$hour_start", window("hour")]}, {"$ifNull": ["$hour_count", 0_i64]}, 0_i64]};
                let result = db.collection::<Document>(QUOTAS).update_one(doc! {"scope": scope,
                    "$expr": {"$and": [{"$lt": [hour_count.clone(), i64::from(hourly)]}, {"$lt": [{"$size": live_leases()}, i64::from(concurrency)]}]}},
                    vec![doc! {"$set": {"purge_after": {"$dateAdd": {"startDate": "$$NOW", "unit": "day", "amount": 7_i64}}, "hour_start": window("hour"), "hour_count": {"$add": [hour_count, 1_i64]},
                        "leases": {"$concatArrays": [live_leases(), [{"turn_id": &turn, "expires_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 120_i64}}}]]}}}])
                    .session(&mut *session).await?;
                if result.matched_count != 1 { return Err(AppError::RateLimited); }
            }
            db.collection::<Document>(TURNS).insert_one(doc! {"_id": &turn, "source_id": &global, "profile_id": &a.profile.id, "profile_generation": a.profile.generation,
                "sender": &a.sender, "update_id": a.update_id, "scopes": [&global, &sender_scope], "state": "admitted", "reserved_credits": Credits::ZERO.decimal(),
                "purge_after": bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::days(1))}).session(&mut *session).await?;
            db.collection::<Document>(TURNS).update_one(doc! {"_id": &turn},
                vec![doc! {"$set": {"lease_expires_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 120_i64}}}}]).session(&mut *session).await?;
            Ok(Some(turn.clone()))
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

async fn reserve_budget(
    state: &AppState,
    a: &GuideAuthority,
    turn: &str,
    cost: Credits,
) -> AppResult<()> {
    let db = state.db.clone();
    let a = a.clone();
    let turn = turn.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            fence_public(&db, &a, session).await?;
            let guide = a.profile.guide.as_ref().ok_or_else(denied)?;
            let global = scope(&a);
            for (scope, cap) in [(global.clone(), guide.limits.daily_credits), (format!("{global}:sender:{}", a.sender), if a.binding.is_none() {guide.anonymous_daily_credits} else {guide.limits.daily_credits})] {
                let spent = doc! {"$cond": [{"$eq": ["$day_start", window("day")]}, {"$ifNull": ["$spent", Credits::ZERO.decimal()]}, Credits::ZERO.decimal()]};
                let next = doc! {"$add": [spent, cost.decimal()]};
                if db.collection::<Document>(QUOTAS).update_one(doc! {"scope": &scope, "leases": {"$elemMatch": {"turn_id": &turn}},
                    "$expr": {"$and": [{"$lte": [next.clone(), cap.decimal()]}, {"$in": [&turn, {"$map": {"input": live_leases(), "as": "lease", "in": "$$lease.turn_id"}}]}]}},
                    vec![doc! {"$set": {"day_start": window("day"), "spent": next}}]).session(&mut *session).await?.matched_count != 1 { return Err(AppError::RateLimited); }
            }
            if db.collection::<Document>(TURNS).update_one(doc! {"_id": &turn, "state": "admitted", "profile_generation": a.profile.generation},
                vec![doc! {"$set": {"state": "funded", "reserved_credits": cost.decimal(), "reserved_day": window("day")}}]).session(&mut *session).await?.matched_count != 1 { return Err(denied()); }
            Ok(())
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

async fn release(db: &Database, turn: &str, forwarded: bool) -> AppResult<()> {
    release_inner(db, turn, forwarded, false).await
}

async fn release_inner(
    db: &Database,
    turn: &str,
    forwarded: bool,
    expired_only: bool,
) -> AppResult<()> {
    let db = db.clone();
    let turn = turn.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let mut filter = doc! {"_id": &turn, "state": {"$ne": "completed"}};
            if expired_only { filter.insert("$expr", doc! {"$lte": ["$lease_expires_at", "$$NOW"]}); }
            let Some(row) = db.collection::<Document>(TURNS).find_one_and_update(filter,
                doc! {"$set": {"state": "completed", "forwarded": forwarded}}).session(&mut *session).await? else { return Ok(()) };
            // Durable dispatch admission is conservative even if a stale Drop says false.
            let forwarded = forwarded || row.get_str("state").ok() == Some("dispatch_admitted");
            db.collection::<Document>(TURNS).update_one(doc! {"_id": &turn}, doc! {"$set": {"forwarded": forwarded}}).session(&mut *session).await?;
            if let Ok(scopes) = row.get_array("scopes") {
                for scope in scopes.iter().filter_map(bson::Bson::as_str) {
                    db.collection::<Document>(QUOTAS).update_one(doc! {"scope": scope}, doc! {"$pull": {"leases": {"turn_id": &turn}}}).session(&mut *session).await?;
                    if !forwarded && let Some(day) = row.get("reserved_day") {
                        db.collection::<Document>(QUOTAS).update_one(doc! {"scope": scope, "day_start": day.clone()},
                            vec![doc! {"$set": {"spent": {"$subtract": ["$spent", row.get("reserved_credits").cloned().unwrap_or_else(|| Credits::ZERO.decimal().into())]}}}]).session(&mut *session).await?;
                    }
                }
            }
            Ok(())
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn sweep_abandoned(db: &Database) -> AppResult<()> {
    let mut rows = db
        .collection::<Document>(TURNS)
        .find(doc! {
            "state": {"$in": ["admitted", "funded", "dispatch_admitted"]},
            "$expr": {"$lte": ["$lease_expires_at", "$$NOW"]}
        })
        .limit(100)
        .await?;
    while let Some(row) = rows.next().await {
        let row = row?;
        let id = row.get_str("_id").map_err(|_| denied())?;
        release_inner(db, id, false, true).await?;
    }
    Ok(())
}

struct TurnGuard {
    db: Database,
    id: String,
    forwarded: bool,
    armed: bool,
}
impl TurnGuard {
    async fn finish(&mut self) -> AppResult<()> {
        release(&self.db, &self.id, self.forwarded).await?;
        self.armed = false;
        Ok(())
    }
}
impl Drop for TurnGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let db = self.db.clone();
        let id = self.id.clone();
        let forwarded = self.forwarded;
        tokio::spawn(async move {
            let _ = release(&db, &id, forwarded).await;
        });
    }
}

async fn admit_forward(state: &AppState, a: &GuideAuthority, turn: &str) -> AppResult<()> {
    let db = state.db.clone();
    let a = a.clone();
    let turn = turn.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            fence_public(&db, &a, session).await?;
            let row = db.collection::<Document>(TURNS).find_one_and_update(doc! {"_id": &turn, "profile_generation": a.profile.generation, "state": "funded"},
                doc! {"$set": {"state": "dispatch_admitted"}}).session(&mut *session).await?.ok_or_else(denied)?;
            for scope in row.get_array("scopes").map_err(|_| denied())?.iter().filter_map(bson::Bson::as_str) {
                if db.collection::<Document>(QUOTAS).update_one(doc! {"scope": scope, "$expr": {"$in": [&turn, {"$map": {"input": live_leases(), "as": "lease", "in": "$$lease.turn_id"}}]}},
                    doc! {"$inc": {"dispatch_fence": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(denied()); }
            }
            Ok(())
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

/// The only sponsor entry point: profile, verified sender and exact live binding.
/// No caller-supplied payer, credential, destination, session or tool definition.
pub(crate) async fn guide_text(
    state: &AppState,
    a: &GuideAuthority,
    prompt: &str,
    input: &str,
) -> AppResult<Option<String>> {
    let work = async {
        tokio::select! {
            biased;
            error = watch_authority(state, a) => Err(error),
            result = Box::pin(guide_text_inner(state, a, prompt, input)) => result,
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(90), work)
        .await
        .map_err(|_| denied())?
}

async fn watch_authority(state: &AppState, a: &GuideAuthority) -> AppError {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Err(error) = admit_effect(state, a).await {
            return error;
        }
    }
}

async fn guide_text_inner(
    state: &AppState,
    a: &GuideAuthority,
    prompt: &str,
    input: &str,
) -> AppResult<Option<String>> {
    admit_effect(state, a).await?;
    let config = a.profile.guide.as_ref().ok_or_else(denied)?;
    if input.chars().count() > config.limits.max_input_chars as usize || prompt.len() > 48 * 1024 {
        return Err(AppError::ValidationError(
            "Public message is too long".into(),
        ));
    }
    if !state.billing.billing_enabled() {
        return Err(denied());
    }
    let owner = match &a.purpose {
        PublicPurpose::Guide | PublicPurpose::Readiness => sponsor_owner(state, &a.profile).await?,
        PublicPurpose::Customer { installation, .. } => {
            state
                .billing
                .owner_resolver()
                .resolve_for_execution(
                    &installation.owner_user_id,
                    &installation.owner_user_id,
                    CredentialClass::NyxidManagedMaster,
                )
                .await?
                .owner_id
        }
    };
    let mut catalog_principal = a.profile.clone();
    catalog_principal.funding_owner_user_id = owner.clone();
    let service = selected_service(state, &catalog_principal).await?;
    let Some(turn) = admit(state, a).await? else {
        return Ok(None);
    };
    let mut turn_guard = TurnGuard {
        db: state.db.clone(),
        id: turn.clone(),
        forwarded: false,
        armed: true,
    };
    let mut target = super::proxy_service::resolve_catalog_platform_target(
        &state.db,
        &state.encryption_keys,
        &owner,
        service.clone(),
        state.platform_user_rate_limit,
    )
    .await?;
    close_public_identity(&mut target);
    let protocol = service.inference.as_ref().ok_or_else(denied)?.wire_protocol;
    let (path, body) = match protocol {
        InferenceWireProtocol::OpenaiCompletions => (
            "chat/completions",
            serde_json::json!({"model": config.model_id, "messages": [{"role": "system", "content": prompt}, {"role": "user", "content": input}], "stream": false, "max_completion_tokens": config.limits.max_output_tokens}),
        ),
        InferenceWireProtocol::OpenaiResponses => (
            "responses",
            serde_json::json!({"model": config.model_id, "instructions": prompt, "input": input, "stream": false, "store": false, "max_output_tokens": config.limits.max_output_tokens}),
        ),
        InferenceWireProtocol::AnthropicMessages => (
            "messages",
            serde_json::json!({"model": config.model_id, "system": prompt, "messages": [{"role": "user", "content": input}], "stream": false, "max_tokens": config.limits.max_output_tokens}),
        ),
    };
    let body = bytes::Bytes::from(serde_json::to_vec(&body).map_err(|_| denied())?);
    let request_len = i64::try_from(body.len()).map_err(|_| denied())?;
    // This sponsor context is deliberately distinct from resolve_for_execution:
    // configured_by establishes live wallet consent, never becomes the payer.
    let ctx = billing::BillingRouteContext::new(
        billing::BillingIngress::LlmProvider,
        uuid::Uuid::new_v4().to_string(),
        owner.clone(),
        owner,
        None,
        None,
        Some(service.id.clone()),
        Some(service.slug.clone()),
        billing::NodeIntent::Direct,
        target.auth_method.clone(),
        CredentialClass::NyxidManagedMaster,
        BillingMetric::Tokens,
        service.billing.as_ref(),
        state.billing.resale_enabled(),
    );
    let operation = state.billing.http_operation(&ctx, "POST", path).await?;
    let ctx = ctx
        .with_operation(operation.as_deref())
        .with_request_body(Some(&body))
        .with_native_text_bound(
            request_len,
            i64::from(config.limits.max_output_tokens),
            RESPONSE_BYTES as i64,
        );
    if ctx
        .platform_specs()
        .any(|(metric, _)| matches!(metric, BillingMetric::Images | BillingMetric::VoiceSeconds))
    {
        return Err(denied());
    }
    let metered = Box::pin(state.billing.open_required(&ctx)).await?;
    let mut guard = super::assistant_oneshot_inference::MeterGuard {
        billing: state.billing.clone(),
        metered,
        forwarded: false,
        usage: super::llm_usage_service::platform_usage(None, request_len, true),
        model: Some(config.model_id.clone()),
        armed: true,
    };
    if guard
        .metered
        .route
        .as_ref()
        .is_none_or(|route| !route.has_billable_layers())
    {
        return Err(denied());
    }
    use futures::TryStreamExt;
    let rows: Vec<crate::models::usage_meter::UsageMeterRow> = state
        .db
        .collection(crate::models::usage_meter::COLLECTION_NAME)
        .find(doc! {"billing_request_id": &ctx.billing_request_id})
        .await?
        .try_collect()
        .await?;
    if rows.is_empty()
        || rows
            .iter()
            .any(|row| row.billing_owner_id != ctx.billing_owner_id)
    {
        return Err(denied());
    }
    let cost =
        Credits::checked_sum(rows.iter().map(|row| row.reserved_credits)).map_err(|_| denied())?;
    reserve_budget(state, a, &turn, cost).await?;
    // Revalidate the same funding consent and recipient after all preparation.
    if matches!(a.purpose, PublicPurpose::Guide | PublicPurpose::Readiness) {
        sponsor_owner(state, &a.profile).await?;
    }
    revalidate_prepared_target(state, &catalog_principal, &service, &target).await?;
    admit_effect(state, a).await?;
    admit_forward(state, a, &turn).await?;
    state.billing.mark_forwarded(&guard.metered).await?;
    guard.forwarded = true;
    turn_guard.forwarded = true;
    let permit = billing::route_inventory::enforce_billing_egress_classification(
        Some(billing::route_inventory::BillingRoutePolicy::Metered(
            billing::BillingIngress::LlmProvider,
        )),
        billing::BillingIngress::LlmProvider,
    )?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    let extra = if protocol == InferenceWireProtocol::AnthropicMessages {
        vec![("anthropic-version".into(), "2023-06-01".into())]
    } else {
        vec![]
    };
    let response = Box::pin(
        super::proxy_service::forward_request_with_extra_outbound_headers(
            &state.http_client,
            &target,
            reqwest::Method::POST,
            path,
            None,
            headers,
            super::proxy_service::ProxyBody::Buffered(Some(body)),
            vec![],
            vec![],
            None,
            &state.token_exchange_cache,
            &state.cloud_response_cache,
            extra,
            permit,
        ),
    )
    .await
    .map_err(|_| denied())?;
    let status = response.status();
    let success = status.is_success();
    let mut stream = response.bytes_stream();
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| denied())?;
        if bytes.len() + chunk.len() > RESPONSE_BYTES {
            return Err(denied());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Option<serde_json::Value> = serde_json::from_slice(&bytes).ok();
    let generated = value.as_ref().is_some_and(|value| {
        value.get("output").is_some()
            || value.get("choices").is_some()
            || value.get("content").is_some()
    });
    if matches!(
        status.as_u16(),
        400 | 401 | 403 | 404 | 405 | 406 | 415 | 422 | 429
    ) && !generated
    {
        guard.usage = crate::models::service_billing::PlatformUsage::default();
        Box::pin(guard.finish()).await?;
        guard.armed = false;
        turn_guard.finish().await?;
        return Err(denied());
    }
    let usage = super::llm_usage_service::usage_from_body(&bytes, path, success);
    guard.usage = super::llm_usage_service::platform_usage(
        usage.as_ref(),
        request_len + bytes.len() as i64,
        true,
    );
    Box::pin(guard.finish()).await?;
    guard.armed = false;
    if !success {
        return Err(denied());
    }
    let text = extract_text(protocol, &config.model_id, &bytes)?;
    admit_effect(state, a).await?;
    turn_guard.finish().await?;
    Ok(Some(text))
}

pub(crate) async fn admit_public_reply(state: &AppState, a: &GuideAuthority) -> AppResult<bool> {
    let db = state.db.clone();
    let a = a.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<bool> = async {
                fence_public(&db, &a, session).await?;
                super::platform_binding_service::admit_native_reply_in_session(
                    db.clone(),
                    a.profile.clone(),
                    if matches!(
                        a.purpose,
                        PublicPurpose::Customer {
                            test_only: false,
                            ..
                        }
                    ) {
                        None
                    } else {
                        a.binding.clone()
                    },
                    scope(&a),
                    a.sender.clone(),
                    a.update_id,
                    "public",
                    false,
                    session,
                )
                .await
            }
            .await;
            transaction_result(result)
        })
        .await
        .map_err(map_transaction_error)
}

fn extract_text(
    protocol: InferenceWireProtocol,
    expected_model: &str,
    bytes: &[u8],
) -> AppResult<String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| denied())?;
    if value["model"].as_str() != Some(expected_model) {
        return Err(denied());
    }
    let text = match protocol {
        InferenceWireProtocol::OpenaiCompletions => value["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned),
        InferenceWireProtocol::OpenaiResponses => value["output"].as_array().map(|items| {
            items
                .iter()
                .filter_map(|item| item["content"].as_array())
                .flatten()
                .filter_map(|part| part["text"].as_str())
                .collect::<String>()
        }),
        InferenceWireProtocol::AnthropicMessages => value["content"].as_array().map(|items| {
            items
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<String>()
        }),
    }
    .filter(|text| !text.is_empty() && text.len() <= RESPONSE_BYTES)
    .ok_or_else(denied)?;
    Ok(text)
}

/// Funding readiness uses the same frozen rate and wallet reservation machinery
/// as execution, then releases it without provider egress or a charge.
pub(crate) async fn probe_funding(
    state: &AppState,
    profile: &PlatformBotConfig,
    owner: &str,
    limits: &crate::models::platform_setup::RuntimeLimits,
) -> AppResult<()> {
    if !state.billing.billing_enabled() {
        return Err(denied());
    }
    let mut principal = profile.clone();
    principal.funding_owner_user_id = owner.to_owned();
    let service = selected_service(state, &principal).await?;
    let target = super::proxy_service::resolve_catalog_platform_target(
        &state.db,
        &state.encryption_keys,
        owner,
        service.clone(),
        state.platform_user_rate_limit,
    )
    .await?;
    let path = match service.inference.as_ref().ok_or_else(denied)?.wire_protocol {
        InferenceWireProtocol::OpenaiCompletions => "chat/completions",
        InferenceWireProtocol::OpenaiResponses => "responses",
        InferenceWireProtocol::AnthropicMessages => "messages",
    };
    let ctx = billing::BillingRouteContext::new(
        billing::BillingIngress::LlmProvider,
        uuid::Uuid::new_v4().to_string(),
        owner.to_owned(),
        owner.to_owned(),
        None,
        None,
        Some(service.id.clone()),
        Some(service.slug.clone()),
        billing::NodeIntent::Direct,
        target.auth_method.clone(),
        CredentialClass::NyxidManagedMaster,
        BillingMetric::Tokens,
        service.billing.as_ref(),
        state.billing.resale_enabled(),
    );
    let operation = state.billing.http_operation(&ctx, "POST", path).await?;
    let ctx = ctx
        .with_operation(operation.as_deref())
        .with_request_body(Some(b"{}"))
        .with_native_text_bound(
            i64::from(limits.max_input_chars) + 48 * 1024,
            i64::from(limits.max_output_tokens),
            RESPONSE_BYTES as i64,
        );
    let metered = Box::pin(state.billing.open_required(&ctx)).await?;
    let funded = metered
        .route
        .as_ref()
        .is_some_and(|route| route.has_billable_layers());
    state
        .billing
        .fail(&metered, "platform_funding_readiness_probe")
        .await?;
    if !funded {
        return Err(denied());
    }
    Ok(())
}

pub(crate) async fn discover_models(
    state: &AppState,
    profile: &PlatformBotConfig,
) -> AppResult<Vec<String>> {
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        Box::pin(discover_models_inner(state, profile)),
    )
    .await
    .map_err(|_| denied())?
}

async fn discover_models_inner(
    state: &AppState,
    profile: &PlatformBotConfig,
) -> AppResult<Vec<String>> {
    let mut a = GuideAuthority {
        profile: profile.clone(),
        binding: None,
        sender: profile.configured_by.clone(),
        update_id: rand::random::<u32>().into(),
        purpose: PublicPurpose::Readiness,
        receipt: None,
    };
    let config = a.profile.guide.as_mut().ok_or_else(denied)?;
    config.anonymous_daily_credits = config.limits.daily_credits;
    config.limits.turns_per_hour = 20;
    config.limits.concurrent_turns = 1;
    config.platform_turns_per_hour = 20;
    config.platform_concurrent_turns = 1;
    admit_effect(state, &a).await?;
    let owner = sponsor_owner(state, profile).await?;
    let service = selected_service(state, profile).await?;
    let Some(turn) = admit(state, &a).await? else {
        return Err(denied());
    };
    let mut turn_guard = TurnGuard {
        db: state.db.clone(),
        id: turn.clone(),
        forwarded: false,
        armed: true,
    };
    let mut target = super::proxy_service::resolve_catalog_platform_target(
        &state.db,
        &state.encryption_keys,
        &owner,
        service.clone(),
        state.platform_user_rate_limit,
    )
    .await?;
    close_public_identity(&mut target);
    let ctx = billing::BillingRouteContext::new(
        billing::BillingIngress::LlmProvider,
        uuid::Uuid::new_v4().to_string(),
        owner.clone(),
        owner,
        None,
        None,
        Some(service.id.clone()),
        Some(service.slug.clone()),
        billing::NodeIntent::Direct,
        target.auth_method.clone(),
        CredentialClass::NyxidManagedMaster,
        BillingMetric::Requests,
        service.billing.as_ref(),
        state.billing.resale_enabled(),
    )
    .with_native_text_bound(1, 1, RESPONSE_BYTES as i64);
    let operation = state.billing.http_operation(&ctx, "GET", "models").await?;
    let ctx = ctx.with_operation(operation.as_deref());
    let metered = Box::pin(state.billing.open_required(&ctx)).await?;
    let mut guard = super::assistant_oneshot_inference::MeterGuard {
        billing: state.billing.clone(),
        metered,
        forwarded: false,
        usage: super::llm_usage_service::platform_usage(None, 0, false),
        model: None,
        armed: true,
    };
    if guard
        .metered
        .route
        .as_ref()
        .is_none_or(|route| !route.has_billable_layers())
    {
        return Err(denied());
    }
    use futures::TryStreamExt;
    let rows: Vec<crate::models::usage_meter::UsageMeterRow> = state
        .db
        .collection(crate::models::usage_meter::COLLECTION_NAME)
        .find(doc! {"billing_request_id": &ctx.billing_request_id})
        .await?
        .try_collect()
        .await?;
    if rows.is_empty()
        || rows
            .iter()
            .any(|row| row.billing_owner_id != ctx.billing_owner_id)
    {
        return Err(denied());
    }
    let cost =
        Credits::checked_sum(rows.iter().map(|row| row.reserved_credits)).map_err(|_| denied())?;
    reserve_budget(state, &a, &turn, cost).await?;
    sponsor_owner(state, profile).await?;
    revalidate_prepared_target(state, profile, &service, &target).await?;
    admit_forward(state, &a, &turn).await?;
    state.billing.mark_forwarded(&guard.metered).await?;
    guard.forwarded = true;
    turn_guard.forwarded = true;
    let permit = billing::route_inventory::enforce_billing_egress_classification(
        Some(billing::route_inventory::BillingRoutePolicy::Metered(
            billing::BillingIngress::LlmProvider,
        )),
        billing::BillingIngress::LlmProvider,
    )?;
    let extra = if service.inference.as_ref().ok_or_else(denied)?.wire_protocol
        == InferenceWireProtocol::AnthropicMessages
    {
        vec![("anthropic-version".into(), "2023-06-01".into())]
    } else {
        vec![]
    };
    let response = Box::pin(
        super::proxy_service::forward_request_with_extra_outbound_headers(
            &state.http_client,
            &target,
            reqwest::Method::GET,
            "models",
            None,
            reqwest::header::HeaderMap::new(),
            super::proxy_service::ProxyBody::Buffered(None),
            vec![],
            vec![],
            None,
            &state.token_exchange_cache,
            &state.cloud_response_cache,
            extra,
            permit,
        ),
    )
    .await
    .map_err(|_| denied())?;
    let success = response.status().is_success();
    let mut stream = response.bytes_stream();
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| denied())?;
        if bytes.len() + chunk.len() > RESPONSE_BYTES {
            return Err(denied());
        }
        bytes.extend_from_slice(&chunk);
    }
    guard.usage = super::llm_usage_service::platform_usage(None, bytes.len() as i64, false);
    Box::pin(guard.finish()).await?;
    guard.armed = false;
    if !success {
        return Err(denied());
    }
    let models = parse_models(&bytes)?;
    admit_effect(state, &a).await?;
    turn_guard.finish().await?;
    Ok(models)
}

fn close_public_identity(target: &mut super::proxy_service::ProxyTarget) {
    target.service.identity_propagation_mode = "none".into();
    target.service.forward_access_token = false;
    target.service.inject_delegation_token = false;
}

fn parse_models(bytes: &[u8]) -> AppResult<Vec<String>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| denied())?;
    let mut models: Vec<String> = value["data"]
        .as_array()
        .ok_or_else(denied)?
        .iter()
        .filter_map(|row| row["id"].as_str())
        .filter(|id| !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control))
        .take(1000)
        .map(str::to_owned)
        .collect();
    models.sort();
    models.dedup();
    if models.is_empty() {
        return Err(denied());
    }
    Ok(models)
}

pub(crate) async fn validate_model_contract(
    state: &AppState,
    profile: &PlatformBotConfig,
) -> AppResult<()> {
    let model = profile.guide.as_ref().ok_or_else(denied)?.model_id.clone();
    if !discover_models(state, profile).await?.contains(&model) {
        return Err(AppError::ValidationError(
            "Select a currently advertised platform model".into(),
        ));
    }
    let mut profile = profile.clone();
    let config = profile.guide.as_mut().ok_or_else(denied)?;
    config.limits.max_output_tokens = 128;
    config.limits.turns_per_hour = 20;
    config.limits.concurrent_turns = 1;
    config.platform_turns_per_hour = 20;
    config.platform_concurrent_turns = 1;
    config.anonymous_daily_credits = config.limits.daily_credits;
    let a = GuideAuthority {
        sender: profile.configured_by.clone(),
        profile,
        binding: None,
        update_id: rand::random::<u32>().into(),
        purpose: PublicPurpose::Readiness,
        receipt: None,
    };
    guide_text(
        state,
        &a,
        "Return only the plain text OK. No tools or external actions.",
        "Model readiness check",
    )
    .await?
    .ok_or_else(denied)?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn funded_test_fixture() -> super::platform_onboarding_test_support::Fixture {
    tests::funded_fixture().await
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn startup_preserves_legacy_dedupe_and_multiple_runtime_turns() {
        use crate::models::platform_public_turn::PlatformPublicTurn;
        use mongodb::{IndexModel, options::IndexOptions};

        let db =
            crate::test_utils::connect_transaction_test_database("shared_public_turn_index").await;
        let turns = db.collection::<Document>(TURNS);
        turns
            .create_index(
                IndexModel::builder()
                    .keys(legacy_turn_keys())
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .unwrap();
        let legacy = doc! {
            "_id": uuid::Uuid::new_v4().to_string(),
            "source_id": "guide:retained-profile", "sender": "retained-sender",
            "update_id": 42_i64, "profile_id": "retained-profile",
            "profile_generation": 1_i64, "scopes": ["guide:retained-profile"],
            "state": "settled", "reserved_credits": Credits::ZERO.decimal(),
        };
        turns.insert_one(&legacy).await.unwrap();
        let mut runtime = PlatformPublicTurn {
            proof_record_id: uuid::Uuid::new_v4().to_string(),
            id: uuid::Uuid::new_v4().to_string(),
            public_session_id: uuid::Uuid::new_v4().to_string(),
            conversation_id: uuid::Uuid::new_v4().to_string(),
            receipt_id: uuid::Uuid::new_v4().to_string(),
            claim_id: uuid::Uuid::new_v4().to_string(),
            sponsor_owner_user_id: uuid::Uuid::new_v4().to_string(),
            policy_digest: "test-policy".into(),
            credential_api_key_id: uuid::Uuid::new_v4().to_string(),
            input_ciphertext: vec![1, 2],
            reply_ciphertext: vec![3, 4],
            reply_digest: "test-reply".into(),
            reply_id: Some(uuid::Uuid::new_v4().to_string()),
            outcome: Some("settled".into()),
            max_output_bytes: 512,
            nyxagent_service_id: uuid::Uuid::new_v4().to_string(),
            nyxagent_catalog_digest: "test-catalog".into(),
            outer_dispatch_admitted: true,
            expires_at: chrono::Utc::now() + chrono::Duration::days(1),
        };
        let first_runtime = bson::to_document(&runtime).unwrap();
        assert!(!first_runtime.contains_key("source_id"));
        turns.insert_one(&first_runtime).await.unwrap();

        crate::db::ensure_indexes(&db).await.unwrap();
        // Recreate the recognized old constraint while there is still only one
        // missing-key row, then exercise competing startup retirement attempts.
        turns
            .create_index(
                IndexModel::builder()
                    .keys(legacy_turn_keys())
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .unwrap();
        let (first, second) =
            tokio::join!(ensure_legacy_turn_index(&db), ensure_legacy_turn_index(&db));
        first.unwrap();
        second.unwrap();
        runtime.id = uuid::Uuid::new_v4().to_string();
        runtime.receipt_id = uuid::Uuid::new_v4().to_string();
        runtime.claim_id = uuid::Uuid::new_v4().to_string();
        runtime.reply_id = Some(uuid::Uuid::new_v4().to_string());
        let second_runtime = bson::to_document(&runtime).unwrap();
        turns.insert_one(&second_runtime).await.unwrap();
        crate::db::ensure_indexes(&db).await.unwrap();

        for original in [&legacy, &first_runtime, &second_runtime] {
            assert!(
                turns
                    .find_one(doc! {"_id": original.get_str("_id").unwrap()})
                    .await
                    .unwrap()
                    .as_ref()
                    == Some(original),
                "startup must preserve complete retained rows"
            );
        }
        assert_eq!(turns.count_documents(doc! {}).await.unwrap(), 3);
        let mut duplicate = legacy.clone();
        duplicate.insert("_id", uuid::Uuid::new_v4().to_string());
        let error = turns.insert_one(duplicate).await.unwrap_err();
        assert!(super::super::telegram_new_service::is_duplicate_key_error(
            &error
        ));
        // A present-but-null legacy source remains constrained as before.
        let malformed =
            doc! {"source_id": bson::Bson::Null, "sender": "legacy", "update_id": 1_i64};
        turns.insert_one(malformed.clone()).await.unwrap();
        let error = turns.insert_one(malformed).await.unwrap_err();
        assert!(super::super::telegram_new_service::is_duplicate_key_error(
            &error
        ));
        let indexes = turns
            .list_indexes()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert!(indexes.iter().any(|index| legacy_turn_index_matches(
            index,
            LEGACY_TURN_INDEX,
            Some(legacy_turn_partial())
        )));
        assert!(!indexes.iter().any(|index| {
            index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                == Some(OLD_LEGACY_TURN_INDEX)
        }));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_turn_index_refuses_unrecognized_definitions_without_retiring_them() {
        use mongodb::{IndexModel, options::IndexOptions};

        for (name, keys, partial, sparse) in [
            (OLD_LEGACY_TURN_INDEX, legacy_turn_keys(), None, Some(true)),
            ("operator_legacy_unique", legacy_turn_keys(), None, None),
            (
                "operator_reordered_legacy_unique",
                doc! {"sender": 1, "update_id": 1, "source_id": 1},
                None,
                None,
            ),
            (
                LEGACY_TURN_INDEX,
                legacy_turn_keys(),
                Some(doc! {"sender": {"$exists": true}}),
                None,
            ),
        ] {
            let db =
                crate::test_utils::connect_transaction_test_database("shared_turn_index_refusal")
                    .await;
            let turns = db.collection::<Document>(TURNS);
            let options = IndexOptions::builder()
                .name(name.to_owned())
                .unique(true)
                .partial_filter_expression(partial)
                .sparse(sparse)
                .build();
            turns
                .create_index(IndexModel::builder().keys(keys).options(options).build())
                .await
                .unwrap();
            let before = turns
                .list_indexes()
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            let retained = doc! {"_id":uuid::Uuid::new_v4().to_string(),"source_id":"retained","sender":"sender","update_id":1_i64};
            turns.insert_one(&retained).await.unwrap();
            assert!(ensure_legacy_turn_index(&db).await.is_err());
            let after = turns
                .list_indexes()
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            let named = |index: &&IndexModel| {
                index
                    .options
                    .as_ref()
                    .and_then(|options| options.name.as_deref())
                    == Some(name)
            };
            assert_eq!(
                bson::to_document(after.iter().find(named).unwrap()).unwrap(),
                bson::to_document(before.iter().find(named).unwrap()).unwrap(),
                "unexpected indexes must remain intact"
            );
            assert_eq!(turns.find_one(doc! {}).await.unwrap(), Some(retained));
            db.drop().await.unwrap();
        }
    }

    async fn boxed_guide_text(
        state: &AppState,
        a: &GuideAuthority,
        prompt: &str,
        input: &str,
    ) -> AppResult<Option<String>> {
        Box::pin(super::guide_text(state, a, prompt, input)).await
    }

    fn exact_money(row: &Document, field: &str) -> Credits {
        let value = row.get(field).unwrap().clone();
        assert!(
            matches!(value, bson::Bson::Decimal128(_)),
            "money must remain Decimal128"
        );
        Credits::from_bson(value, crate::models::credits::SCALE).unwrap()
    }

    async fn drain_settlements(db: &Database) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let stats = super::super::billing::reservation::recover_retryable_settlements(db)
                    .await
                    .unwrap();
                assert_eq!(stats.dead_lettered, 0);
                if db
                    .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                    .count_documents(doc! {"forwarded": true, "released": false})
                    .await
                    .unwrap()
                    == 0
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("production settlement worker must drain admitted test usage");
    }

    pub(super) async fn funded_fixture() -> super::super::platform_onboarding_test_support::Fixture
    {
        use crate::models::{downstream_service::*, service_billing::*};
        let mut f = super::super::platform_onboarding_test_support::distinct_fixture().await;
        let build = f
            .state
            .db
            .run_command(doc! {"buildInfo": 1_i64})
            .await
            .expect("funded tests require readable MongoDB version");
        let major = build.get_array("versionArray").unwrap()[0]
            .as_i32()
            .unwrap();
        assert!(
            major >= 8,
            "funded platform tests require MongoDB >=8.0 for production billing bulk writes; use an isolated Mongo8 replica-set NYXID_TEST_DATABASE_URL"
        );

        f.state.billing = std::sync::Arc::new(crate::billing_integration_tests::billing_service(
            &f.state.db,
            std::sync::Arc::new(crate::billing_integration_tests::FakeLago::default()),
            0,
        ));
        crate::billing_integration_tests::create_usage_index(&f.state.db).await;
        let mut service = test_helpers::dummy_service();
        service.id = f.profile.guide.as_ref().unwrap().model_service_id.clone();
        service.slug = format!("public-guide-{}", uuid::Uuid::new_v4());
        service.name = "Funded public test model".into();
        service.base_url = f.server.uri();
        service.auth_method = "bearer".into();
        service.credential_encrypted = f
            .state
            .encryption_keys
            .encrypt(b"fake-catalog-credential")
            .await
            .unwrap();
        service.platform_key = Some(PlatformKeyConfig {
            enabled: true,
            audience: PlatformKeyAudience::Public,
            allowed_owner_ids: vec![],
        });
        service.inference = Some(ServiceInference {
            wire_protocol: InferenceWireProtocol::OpenaiCompletions,
            model_list: true,
            realtime: false,
            voice: None,
        });
        let code = format!("platform_svc_{}_pk", service.slug);
        service.billing = Some(ServiceBilling {
            platform_key_pricing: Some(LanePricing {
                metric: BillingMetric::Requests,
                credits_per_unit: "0.001".into(),
                lago_metric_code: code.clone(),
                sync_status: PricingSyncStatus::Synced,
                sync_error: None,
                components: vec![],
                operations: vec![],
            }),
            ..Default::default()
        });
        let mut rate = crate::billing_integration_tests::rate(&code, chrono::Utc::now());
        rate.credits_per_unit_micros = 1000;
        f.state
            .db
            .collection::<crate::models::billing_rate_cache::BillingRateCache>(
                crate::models::billing_rate_cache::COLLECTION_NAME,
            )
            .insert_one(rate)
            .await
            .unwrap();
        f.state
            .db
            .collection::<DownstreamService>(COLLECTION_NAME)
            .insert_one(service)
            .await
            .unwrap();
        f
    }

    #[tokio::test]
    async fn platform_onboarding_token_pricing_reserves_bounded_text_and_cap_denies_before_provider()
     {
        use crate::models::{downstream_service, service_billing::BillingMetric, usage_meter};
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let mut f = Box::pin(funded_fixture()).await;
        let service_id = f.profile.guide.as_ref().unwrap().model_service_id.clone();
        f.state.db.collection::<Document>(downstream_service::COLLECTION_NAME)
            .update_one(doc! {"_id": &service_id}, doc! {"$set": {"billing.platform_key_pricing.metric": bson::to_bson(&BillingMetric::Tokens).unwrap()}})
            .await.unwrap();
        f.profile.guide.as_mut().unwrap().limits.max_output_tokens = 128;
        f.profile.guide.as_mut().unwrap().limits.daily_credits = "1000".parse().unwrap();
        f.state
            .db
            .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.profile.id},
                doc! {"$set": {"guide": bson::to_bson(&f.profile.guide).unwrap()}},
            )
            .await
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "test-model", "choices": [{"message": {"content": "Bounded reply"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            })))
            .expect(1)
            .mount(&f.server)
            .await;
        let mut a = GuideAuthority {
            profile: f.profile.clone(),
            binding: Some(f.binding.clone()),
            sender: "700".into(),
            update_id: 18900,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        assert_eq!(
            boxed_guide_text(&f.state, &a, "Public setup guide", "Help")
                .await
                .unwrap()
                .as_deref(),
            Some("Bounded reply")
        );
        drain_settlements(&f.state.db).await;
        let row = f
            .state
            .db
            .collection::<usage_meter::UsageMeterRow>(usage_meter::COLLECTION_NAME)
            .find_one(
                doc! {"billing_owner_id": &f.profile.funding_owner_user_id, "forwarded": true},
            )
            .await
            .unwrap()
            .unwrap();
        assert!(row.reserved_credits > "0.128".parse::<Credits>().unwrap());
        assert_eq!(
            row.funding.unwrap().wallet_charge_credits,
            Some("0.015".parse::<Credits>().unwrap())
        );
        a.update_id += 1;
        a.profile.guide.as_mut().unwrap().limits.daily_credits = "0.001".parse().unwrap();
        f.state
            .db
            .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.profile.id},
                doc! {"$set": {"guide": bson::to_bson(&a.profile.guide).unwrap()}},
            )
            .await
            .unwrap();
        assert!(
            boxed_guide_text(&f.state, &a, "Public setup guide", "Help")
                .await
                .is_err()
        );
        assert_eq!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.url.path() == "/chat/completions")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn platform_onboarding_preparation_drift_blocks_text_and_models_before_provider() {
        for models in [false, true] {
            let mut f = Box::pin(funded_fixture()).await;
            let entered = std::sync::Arc::new(tokio::sync::Notify::new());
            let resume = std::sync::Arc::new(tokio::sync::Notify::new());
            f.state.billing =
                std::sync::Arc::new(crate::billing_integration_tests::billing_service(
                    &f.state.db,
                    std::sync::Arc::new(
                        crate::billing_integration_tests::FakeLago::with_wallet_gate(
                            entered.clone(),
                            resume.clone(),
                        ),
                    ),
                    0,
                ));
            let a = GuideAuthority {
                profile: f.profile.clone(),
                binding: Some(f.binding.clone()),
                sender: "700".into(),
                update_id: 19600,
                purpose: PublicPurpose::Guide,
                receipt: None,
            };
            let work = async {
                if models {
                    Box::pin(discover_models(&f.state, &f.profile))
                        .await
                        .map(|_| ())
                } else {
                    Box::pin(boxed_guide_text(&f.state, &a, "Public help", "Hello"))
                        .await
                        .map(|_| ())
                }
            };
            let change = async {
                tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
                    .await
                    .unwrap();
                let update = if models {
                    doc! {"$set": {"credential_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic,
                    bytes: f.state.encryption_keys.encrypt(b"replacement-catalog-credential").await.unwrap()}}}
                } else {
                    doc! {"$set": {"base_url": format!("{}/changed", f.server.uri())}}
                };
                f.state
                    .db
                    .collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id": &f.profile.guide.as_ref().unwrap().model_service_id},
                        update,
                    )
                    .await
                    .unwrap();
                resume.notify_one();
            };
            let (result, ()) = tokio::join!(work, change);
            assert!(result.is_err());
            assert!(
                f.server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| !matches!(
                        r.url.path(),
                        "/chat/completions"
                            | "/models"
                            | "/changed/chat/completions"
                            | "/changed/models"
                    ))
            );
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                    .count_documents(doc! {"forwarded": true})
                    .await
                    .unwrap(),
                0
            );
        }
    }

    #[tokio::test]
    async fn platform_onboarding_token_pricing_empty_sponsor_wallet_denies_before_provider() {
        let mut f = Box::pin(funded_fixture()).await;
        f.state.billing = std::sync::Arc::new(crate::billing_integration_tests::billing_service(
            &f.state.db,
            std::sync::Arc::new(
                crate::billing_integration_tests::FakeLago::with_wallet_balance(Credits::ZERO),
            ),
            0,
        ));
        f.state.db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
            .update_one(doc! {"_id": &f.profile.guide.as_ref().unwrap().model_service_id},
                doc! {"$set": {"billing.platform_key_pricing.metric": bson::to_bson(&crate::models::service_billing::BillingMetric::Tokens).unwrap()}}).await.unwrap();
        let a = GuideAuthority {
            profile: f.profile.clone(),
            binding: Some(f.binding.clone()),
            sender: "700".into(),
            update_id: 19700,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        assert!(matches!(
            Box::pin(boxed_guide_text(&f.state, &a, "Public guide", "Hello")).await,
            Err(AppError::InsufficientCredits)
        ));
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(doc! {"forwarded": true})
                .await
                .unwrap(),
            0
        );
        assert!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.url.path() != "/chat/completions")
        );
    }

    #[tokio::test]
    async fn platform_onboarding_two_replica_budget_and_concurrency_admission_are_atomic() {
        let mut f = Box::pin(funded_fixture()).await;
        let guide = f.profile.guide.as_mut().unwrap();
        guide.limits.daily_credits = "0.001".parse().unwrap();
        guide.limits.concurrent_turns = 2;
        guide.platform_concurrent_turns = 2;
        f.state
            .db
            .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.profile.id},
                doc! {"$set": {"guide": bson::to_bson(&f.profile.guide).unwrap()}},
            )
            .await
            .unwrap();
        let a = GuideAuthority {
            profile: f.profile.clone(),
            binding: Some(f.binding.clone()),
            sender: "700".into(),
            update_id: 19800,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        let mut b = a.clone();
        b.update_id += 1;
        let (left, right) = tokio::join!(admit(&f.state, &a), admit(&f.state, &b));
        let left = left.unwrap().unwrap();
        let right = right.unwrap().unwrap();
        let cost = "0.001".parse::<Credits>().unwrap();
        let (first, second) = tokio::join!(
            reserve_budget(&f.state, &a, &left, cost),
            reserve_budget(&f.state, &b, &right, cost)
        );
        assert_ne!(first.is_ok(), second.is_ok());
        assert!(matches!(first, Ok(()) | Err(AppError::RateLimited)));
        assert!(matches!(second, Ok(()) | Err(AppError::RateLimited)));
        let quota = f
            .state
            .db
            .collection::<Document>(QUOTAS)
            .find_one(doc! {"scope": scope(&a)})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(exact_money(&quota, "spent"), cost);
        release(&f.state.db, &left, false).await.unwrap();
        release(&f.state.db, &right, false).await.unwrap();
        let quota = f
            .state
            .db
            .collection::<Document>(QUOTAS)
            .find_one(doc! {"scope": scope(&a)})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(exact_money(&quota, "spent"), Credits::ZERO);
        let mut a = a;
        a.profile.guide.as_mut().unwrap().platform_concurrent_turns = 1;
        a.update_id += 10;
        f.state
            .db
            .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.profile.id},
                doc! {"$set": {"guide": bson::to_bson(&a.profile.guide).unwrap()}},
            )
            .await
            .unwrap();
        let mut b = a.clone();
        b.update_id += 1;
        let (first, second) = tokio::join!(admit(&f.state, &a), admit(&f.state, &b));
        assert_ne!(first.is_ok(), second.is_ok());
        for result in [first, second] {
            match result {
                Ok(Some(turn)) => release(&f.state.db, &turn, false).await.unwrap(),
                Err(AppError::RateLimited) => (),
                other => panic!("unexpected admission: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn platform_onboarding_slow_headers_and_profile_pause_cancel_in_flight_inference() {
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let f = Box::pin(funded_fixture()).await;
        Mock::given(method("POST")).and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(3)).set_body_json(serde_json::json!({
                "model": "test-model", "choices": [{"message": {"content": "Late response must not be delivered"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            }))).expect(1).mount(&f.server).await;
        let a = GuideAuthority {
            profile: f.profile.clone(),
            binding: Some(f.binding.clone()),
            sender: "700".into(),
            update_id: 19900,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        let work = boxed_guide_text(&f.state, &a, "Public help", "Hello");
        let pause = async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if f.server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|r| r.url.path() == "/chat/completions")
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            f.state
                .db
                .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &f.profile.id},
                    doc! {"$set": {"enabled": false}, "$inc": {"generation": 1_i64}},
                )
                .await
                .unwrap();
        };
        let (result, ()) = tokio::join!(work, pause);
        assert!(result.is_err());
        let quota = f
            .state
            .db
            .collection::<Document>(QUOTAS)
            .find_one(doc! {"scope": scope(&a)})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            exact_money(&quota, "spent"),
            "0.001".parse::<Credits>().unwrap()
        );
        assert_eq!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.url.path() == "/chat/completions")
                .count(),
            1
        );
        assert!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| !r.url.path().ends_with("/sendMessage"))
        );
    }

    async fn prepare_second_client(
        f: &super::super::platform_onboarding_test_support::Fixture,
    ) -> (String, crate::models::platform_setup::PlatformSetup) {
        use super::super::{
            platform_provisioning_service as p, platform_setup_service as setups,
            telegram_new_tests as telegram,
        };
        use crate::models::{platform_binding, platform_setup::*};
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{body_partial_json, method, path},
        };
        let owner = uuid::Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let mut binding = f.binding.clone();
        binding.id = uuid::Uuid::new_v4().to_string();
        binding.user_id = owner.clone();
        binding.telegram_sender_id = "701".into();
        binding.telegram_chat_id = "701".into();
        f.state
            .db
            .collection::<platform_binding::PlatformBinding>(platform_binding::COLLECTION_NAME)
            .insert_one(&binding)
            .await
            .unwrap();
        let facts = SetupFacts {
            name: "D restaurant publication".into(),
            timezone: "UTC".into(),
            hours: "12:00–20:00".into(),
            address: "D public address".into(),
            contact: "D public contact".into(),
            ..Default::default()
        };
        let row = setups::create_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &owner,
            &uuid::Uuid::new_v4().to_string(),
            SetupTemplate::RestaurantStarter,
            &facts,
        )
        .await
        .unwrap();
        let row = setups::attach_binding(
            &f.state.db,
            &owner,
            &row.id,
            row.revision,
            &binding.id,
            binding.generation,
        )
        .await
        .unwrap();
        let row = setups::publish_reviewed(
            &f.state.db,
            &f.state.encryption_keys,
            &owner,
            &row.id,
            row.revision,
        )
        .await
        .unwrap();
        let row = setups::request_review(
            &f.state.db,
            &f.state.encryption_keys,
            &owner,
            &row.id,
            row.revision,
        )
        .await
        .unwrap();
        let digest = setups::approval_digest(&row, &facts).unwrap();
        let approved = p::approve_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &owner,
            &row.id,
            row.revision,
            &digest,
        )
        .await
        .unwrap();
        let job = p::claim_step(&f.state.db).await.unwrap().unwrap();
        assert_eq!(job.setup_id, approved.id);
        let base = f.server.uri();
        let service = telegram::service(&f.state, &base);
        let (_, url) = service.begin_approved(&job, "D restaurant").await.unwrap();
        let challenge = reqwest::Url::parse(&url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "start")
            .unwrap()
            .1
            .to_string();
        let headers = telegram::headers(&f.state).await;
        let start = serde_json::json!({"update_id": 22000, "message": {"date": chrono::Utc::now().timestamp(), "message_id": 22000,
            "from": {"id": 701, "is_bot": false}, "chat": {"id": 701, "type": "private"}, "text": format!("/start {challenge}")}});
        service
            .webhook(&headers, &serde_json::to_vec(&start).unwrap())
            .await
            .unwrap();
        let child = serde_json::json!({"id": 901, "is_bot": true, "username": "OtherCustomerBot"});
        service.webhook(&headers, &serde_json::to_vec(&serde_json::json!({"update_id": 22001, "managed_bot": {"user": {"id": 701, "is_bot": false}, "bot": child}})).unwrap()).await.unwrap();
        let created = serde_json::json!({"update_id": 22002, "message": {"date": chrono::Utc::now().timestamp(), "message_id": 22002,
            "from": {"id": 701, "is_bot": false}, "chat": {"id": 701, "type": "private"}, "managed_bot_created": {"bot": child}}});
        service
            .webhook(&headers, &serde_json::to_vec(&created).unwrap())
            .await
            .unwrap();
        let (candidate, native, digest) =
            p::child_candidate(&f.state.db, &f.state.encryption_keys, &owner, &approved.id)
                .await
                .unwrap();
        let row = p::approve_child_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &owner,
            &approved.id,
            candidate.revision - 1,
            native.revision,
            901,
            native.manager_revision.unwrap(),
            &digest,
        )
        .await
        .unwrap();
        let job = p::claim_step(&f.state.db).await.unwrap().unwrap();
        assert_eq!(job.setup_id, row.id);
        Box::pin(p::execute_step(&f.state, &job)).await.unwrap();
        Mock::given(method("POST"))
            .and(path("/bot100:manager-test-secret/getManagedBotToken"))
            .and(body_partial_json(serde_json::json!({"user_id": 901})))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"ok": true, "result": "901:other-child-test-secret"}),
            ))
            .with_priority(1)
            .mount(&f.server)
            .await;
        let callback = format!(
            "{}/api/v1/webhooks/channel/telegram-new/{}",
            f.state.config.base_url.trim_end_matches('/'),
            row.resources.channel_bot_id.as_ref().unwrap()
        );
        for (endpoint, result) in [
            ("getMe", child),
            ("getWebhookInfo", serde_json::json!({"url": callback})),
            ("setWebhook", serde_json::json!(true)),
            ("deleteWebhook", serde_json::json!(true)),
            ("sendMessage", serde_json::json!({"message_id": 333})),
        ] {
            Mock::given(method("POST"))
                .and(path(format!("/bot901:other-child-test-secret/{endpoint}")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"ok": true, "result": result})),
                )
                .mount(&f.server)
                .await;
        }
        Box::pin(service.complete_pending_creations())
            .await
            .unwrap();
        let ready = setups::get_owned(&f.state.db, &owner, &row.id)
            .await
            .unwrap();
        assert_eq!(ready.state, SetupState::ReadyForTest);
        (owner, ready)
    }

    async fn activate_second_client(
        f: &super::super::platform_onboarding_test_support::Fixture,
    ) -> (
        String,
        crate::models::platform_setup::PlatformSetup,
        axum::http::HeaderMap,
    ) {
        use super::super::{platform_runtime_service as runtime, platform_setup_service as setups};
        use sha2::Digest;
        let (owner, ready) = Box::pin(prepare_second_client(f)).await;
        let bot = ready.resources.channel_bot_id.as_deref().unwrap();
        let secret = "second-local-customer-webhook";
        f.state.db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(doc! {"_id": bot}, doc! {"$set": {"webhook_secret_hash": hex::encode(sha2::Sha256::digest(secret.as_bytes()))}}).await.unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-telegram-bot-api-secret-token", secret.parse().unwrap());
        let base = f.server.uri();
        let update = serde_json::json!({"update_id": 9006, "message": {"from": {"id": 800, "is_bot": false}, "chat": {"id": 800, "type": "private"}, "text": "/test"}});
        let response = Box::pin(crate::handlers::channel_webhooks::channel_webhook_with_api(
            axum::extract::State(f.state.clone()),
            axum::extract::Path(("telegram-new".into(), bot.to_owned())),
            "/".parse().unwrap(),
            headers.clone(),
            bytes::Bytes::from(serde_json::to_vec(&update).unwrap()),
            &base,
        ))
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let installation = f
            .state
            .db
            .collection::<Document>(crate::models::platform_installation::COLLECTION_NAME)
            .find_one(doc! {"setup_id": &ready.id})
            .await
            .unwrap()
            .unwrap();
        let installation_id = installation.get_str("_id").unwrap().to_owned();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if f.state.db.collection::<Document>(crate::models::platform_installation::TESTS)
                    .find_one(doc! {"setup_revision": ready.revision, "customer_sender_id": "800", "state": "awaiting_customer_reply", "installation_id": &installation_id})
                    .await.unwrap().is_some() {break;}
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        let requests = f.server.received_requests().await.unwrap();
        let sent: serde_json::Value = requests
            .iter()
            .filter(|r| r.url.path() == "/bot901:other-child-test-secret/sendMessage")
            .map(|r| serde_json::from_slice::<serde_json::Value>(&r.body).unwrap())
            .find(|body| {
                body["text"]
                    .as_str()
                    .is_some_and(|s| s.contains("with TEST "))
            })
            .unwrap();
        let code = zeroize::Zeroizing::new(
            sent["text"]
                .as_str()
                .unwrap()
                .split("with TEST ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .to_owned(),
        );
        let proof = serde_json::json!({"update_id": 9008, "message": {"from": {"id": 800, "is_bot": false}, "chat": {"id": 800, "type": "private"},
            "reply_to_message": {"message_id": 333}, "text": format!("TEST {}", code.as_str())}});
        let response = Box::pin(crate::handlers::channel_webhooks::channel_webhook_with_api(
            axum::extract::State(f.state.clone()),
            axum::extract::Path(("telegram-new".into(), bot.to_owned())),
            "/".parse().unwrap(),
            headers.clone(),
            bytes::Bytes::from(serde_json::to_vec(&proof).unwrap()),
            &base,
        ))
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let tested = setups::get_owned(&f.state.db, &owner, &ready.id)
            .await
            .unwrap();
        assert!(tested.delivery_tested);
        let active = Box::pin(runtime::activate_owned(
            &f.state,
            &owner,
            &tested.id,
            tested.revision,
        ))
        .await
        .unwrap();
        (owner, active, headers)
    }

    #[tokio::test]
    async fn platform_onboarding_signed_admin_chat_has_no_private_inventory_or_tool_authority() {
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let f = Box::pin(funded_fixture()).await;
        f.state.db.collection::<Document>(crate::models::platform_binding::COLLECTION_NAME)
            .update_one(doc! {"_id": &f.binding.id}, doc! {"$set": {"user_id": &f.operator}, "$inc": {"generation": 1_i64, "revocation_generation": 1_i64}}).await.unwrap();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<crate::models::user_endpoint::UserEndpoint>(
                crate::models::user_endpoint::COLLECTION_NAME,
            )
            .insert_one(crate::test_utils::test_user_endpoint(
                &endpoint_id,
                &f.operator,
                "OPERATOR PRIVATE CONNECTOR",
                &format!("{}/operator-private", f.server.uri()),
                None,
                Some(&f.profile.guide.as_ref().unwrap().model_service_id),
            ))
            .await
            .unwrap();
        let mut private = crate::test_utils::test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            &f.operator,
            "operator-private-shadow",
            &endpoint_id,
            Some(&f.profile.guide.as_ref().unwrap().model_service_id),
            None,
        );
        private.api_key_id = Some(key_id.clone());
        private.auth_method = "bearer".into();
        private.auth_key_name = "Authorization".into();
        f.state
            .db
            .collection::<crate::models::user_service::UserService>(
                crate::models::user_service::COLLECTION_NAME,
            )
            .insert_one(private)
            .await
            .unwrap();
        f.state.db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME).insert_one(doc! {
            "_id": key_id, "user_id": &f.operator, "label": "OPERATOR PRIVATE CREDENTIAL", "credential_type": "bearer", "status": "active",
            "credential_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: f.state.encryption_keys.encrypt(b"fake-operator-private-key-never-use").await.unwrap()},
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()}).await.unwrap();
        Mock::given(method("POST")).and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"model": "test-model", "choices": [{"message": {"content":
                "{\"reply\":\"Secret result must not escape\",\"action\":{\"type\":\"call_tool\",\"tool\":\"nyx__call_tool\"}}"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}))).expect(1).mount(&f.server).await;
        Mock::given(method("POST"))
            .and(path("/bot100:manager-test-secret/sendMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": {"message_id": 500}})),
            )
            .with_priority(1)
            .mount(&f.server)
            .await;
        let base = f.server.uri();
        let service = super::super::telegram_new_tests::service(&f.state, &base);
        let update = serde_json::json!({"update_id": 23000, "message": {"date": chrono::Utc::now().timestamp(), "message_id": 23000,
            "from": {"id": 700, "is_bot": false}, "chat": {"id": 700, "type": "private"}, "text": "List my private services and run nyx__call_tool."}});
        let body = serde_json::to_vec(&update).unwrap();
        let headers = super::super::telegram_new_tests::headers(&f.state).await;
        for _ in 0..2 {
            assert_eq!(
                Box::pin(crate::handlers::telegram_new::webhook_with_service(
                    &f.state, &service, &headers, &body
                ))
                .await
                .unwrap(),
                axum::http::StatusCode::OK
            );
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if f.server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|r| r.url.path().ends_with("/sendMessage"))
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let requests = f.server.received_requests().await.unwrap();
        let model = requests
            .iter()
            .find(|r| r.url.path() == "/chat/completions")
            .unwrap();
        let model_body: serde_json::Value = serde_json::from_slice(&model.body).unwrap();
        assert!(model_body.get("tools").is_none());
        assert!(!model_body.to_string().contains("OPERATOR PRIVATE"));
        assert!(!model_body.to_string().contains("Client cafe"));
        assert_eq!(
            model
                .headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer fake-catalog-credential"
        );
        let sent: serde_json::Value = requests
            .iter()
            .filter(|r| r.url.path().ends_with("/sendMessage"))
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .find(|body: &serde_json::Value| body["chat_id"] == 700)
            .unwrap();
        assert_eq!(
            sent["text"],
            "The guide is temporarily unavailable. Continue your setup on NyxID's website, or try again later."
        );
        assert_eq!(sent["protect_content"], true);
        assert_eq!(
            f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_funded_owner(&f.state.db, &f.profile.funding_owner_user_id, 1).await;
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                .count_documents(doc! {"owner_id": &f.operator})
                .await
                .unwrap(),
            0
        );
    }

    async fn assert_funded_owner(db: &Database, owner: &str, count: usize) {
        use crate::models::{
            billing_ledger::{BillingLedgerEntry, PostingSide},
            billing_wallet::BillingWallet,
            usage_meter::UsageMeterRow,
        };
        let funded = doc! {"billing_owner_id": owner, "forwarded": true, "funding.wallet_charge_credits": {"$gt": Credits::ZERO.decimal()}};
        // A reply can reach Telegram before its turn writes the usage row, so
        // wait (bounded) for the expected settled charges before inspecting them.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            drain_settlements(db).await;
            let settled = db
                .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(funded.clone())
                .await
                .unwrap();
            if settled as usize >= count || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let rows = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find(funded)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert_eq!(rows.len(), count);
        let cost = Credits::from_micros(1000);
        for row in rows {
            let row = row.unwrap();
            let funding = row.funding.as_ref().unwrap();
            assert!(funding.settled);
            assert_eq!(funding.wallet_charge_credits, Some(cost));
            let ledger = db
                .collection::<BillingLedgerEntry>(crate::models::billing_ledger::COLLECTION_NAME)
                .find_one(
                    doc! {"owner_id": owner, "reference_id": &row.id, "movement": "usage_settled"},
                )
                .await
                .unwrap()
                .unwrap();
            assert!(
                ledger
                    .postings
                    .iter()
                    .any(|posting| posting.account == format!("wallet:{owner}")
                        && posting.side == PostingSide::Credit
                        && posting.amount == cost)
            );
        }
        let opening = db
            .collection::<BillingLedgerEntry>(crate::models::billing_ledger::COLLECTION_NAME)
            .find_one(doc! {"owner_id": owner, "movement": "wallet_funded"})
            .await
            .unwrap()
            .unwrap();
        assert!(
            opening
                .postings
                .iter()
                .any(|posting| posting.account == format!("wallet:{owner}")
                    && posting.side == PostingSide::Debit
                    && posting.amount == Credits::from_whole(10000))
        );
        let total = cost.checked_mul(count as i64).unwrap();
        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! {"owner_id": owner})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            wallet.available_credits().unwrap(),
            Credits::from_whole(10000).checked_sub(total).unwrap()
        );
        let checkpoint = db
            .collection::<Document>("billing_account_balances")
            .find_one(doc! {"_id": format!("wallet:{owner}")})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            exact_money(&checkpoint, "balance"),
            Credits::from_whole(10000).checked_sub(total).unwrap()
        );
    }

    #[tokio::test]
    async fn platform_onboarding_distinct_guide_payer_is_sponsor_never_operator_or_client() {
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let f = Box::pin(funded_fixture()).await;
        assert_ne!(f.operator, f.owner);
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "test-model", "choices": [{"message": {"content": "Public guide reply"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            })))
            .mount(&f.server)
            .await;
        let authority = GuideAuthority {
            profile: f.profile.clone(),
            binding: Some(f.binding.clone()),
            sender: "700".into(),
            update_id: 8000,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        assert_eq!(
            boxed_guide_text(
                &f.state,
                &authority,
                "Closed public setup guide",
                "Help with public setup"
            )
            .await
            .unwrap()
            .as_deref(),
            Some("Public guide reply")
        );
        drain_settlements(&f.state.db).await;
        let rows = f
            .state
            .db
            .collection::<crate::models::usage_meter::UsageMeterRow>(
                crate::models::usage_meter::COLLECTION_NAME,
            )
            .find(doc! {"billing_owner_id": &f.profile.funding_owner_user_id})
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert_eq!(rows.len(), 1);
        let row = rows.into_iter().next().unwrap().unwrap();
        assert_eq!(row.billing_owner_id, f.profile.funding_owner_user_id);
        assert_funded_owner(&f.state.db, &f.profile.funding_owner_user_id, 1).await;
        for owner in [&f.operator, &f.owner] {
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                    .count_documents(doc! {"owner_id": owner})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                    .count_documents(doc! {"billing_owner_id": owner})
                    .await
                    .unwrap(),
                0
            );
        }
        for owner in [&f.operator, &f.owner] {
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
                    .count_documents(doc! {"owner_id": owner, "movement": "usage_settled"})
                    .await
                    .unwrap(),
                0
            );
        }
        let requests = f.server.received_requests().await.unwrap();
        let request = requests
            .iter()
            .find(|request| request.url.path() == "/chat/completions")
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert!(body.get("tools").is_none());
        assert_eq!(body["messages"][0]["content"], "Closed public setup guide");
        // Telegram customer C is not used as a NyxID payer or credential principal.
        assert!(
            f.state
                .db
                .collection::<Document>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! {"_id": "800"})
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn platform_onboarding_distinct_customer_test_activation_and_personal_payer() {
        // The full journey's debug-build futures exceed the default test stack.
        std::thread::Builder::new()
            .name("platform-distinct-customer".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(Box::pin(
                        distinct_customer_test_activation_and_personal_payer(),
                    ));
            })
            .expect("distinct customer journey thread")
            .join()
            .expect("distinct customer journey");
    }

    async fn distinct_customer_test_activation_and_personal_payer() {
        use super::super::{
            platform_provisioning_service as p, platform_runtime_service as r,
            platform_setup_service as setups, telegram_new_tests as telegram,
        };
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let f = Box::pin(funded_fixture()).await;
        Box::pin(f.observe()).await;
        let (candidate, native, digest) =
            p::child_candidate(&f.state.db, &f.state.encryption_keys, &f.owner, &f.setup.id)
                .await
                .unwrap();
        let approved = p::approve_child_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &f.setup.id,
            candidate.revision - 1,
            native.revision,
            900,
            native.manager_revision.unwrap(),
            &digest,
        )
        .await
        .unwrap();
        let job = f.claim().await;
        p::execute_step(&f.state, &job).await.unwrap();
        telegram::provider_connection(
            &f.server,
            approved.resources.channel_bot_id.as_ref().unwrap(),
            200,
        )
        .await;
        let base = f.server.uri();
        let service = telegram::service(&f.state, &base);
        service.complete_pending_creations().await.unwrap();
        let ready = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
            .await
            .unwrap();
        assert_eq!(
            ready.state,
            crate::models::platform_setup::SetupState::ReadyForTest
        );
        let bot = ready.resources.channel_bot_id.as_deref().unwrap();
        let personal_key = super::super::key_service::create_api_key(
            &f.state.db,
            &f.owner,
            "Personal callback",
            "read",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("https://callback.nyxid.test/personal"),
        )
        .await
        .unwrap();
        // The durable child marker closes ordinary routes in every conversation class.
        for kind in ["private", "group", "supergroup", "channel", "topic"] {
            assert!(matches!(
                super::super::channel_routing_service::create_conversation(
                    &f.state.db,
                    &f.owner,
                    Some(bot),
                    "telegram-new",
                    "test-chat",
                    kind,
                    Some("700"),
                    &personal_key.id,
                    true,
                    true
                )
                .await,
                Err(AppError::Forbidden(_))
            ));
            assert!(
                super::super::channel_routing_service::resolve_agent(
                    &f.state.db,
                    bot,
                    "test-chat",
                    Some("700"),
                    &f.owner
                )
                .await
                .unwrap()
                .is_none()
            );
        }
        assert!(
            super::super::channel_bot_service::get_bot_for_user(&f.state.db, bot, &f.owner)
                .await
                .is_err()
        );
        // The owner is rejected before a wallet or inference admission is created.
        assert!(r::capture(&f.state, bot, 700, 9000, true).await.is_err());
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                .count_documents(doc! {"owner_id": &f.owner})
                .await
                .unwrap(),
            0
        );
        Mock::given(method("POST")).and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "test-model", "choices": [{"message": {"content": "Customer public reply"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            }))).mount(&f.server).await;
        let customer = r::capture(&f.state, bot, 800, 9001, true).await.unwrap();
        assert_eq!(
            boxed_guide_text(
                &f.state,
                &customer,
                "Published restaurant facts only",
                "What are your hours?"
            )
            .await
            .unwrap()
            .as_deref(),
            Some("Customer public reply")
        );
        let (test, code) = r::begin_test(&f.state, &customer).await.unwrap();
        Mock::given(method("POST"))
            .and(path("/bot900:child-test-secret/sendMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": {"message_id": 222}})),
            )
            .mount(&f.server)
            .await;
        let sent = service.api.call("900:child-test-secret", "sendMessage", serde_json::json!({"chat_id": 800, "text": format!("Customer public reply\nTEST {}", code.as_str()), "protect_content": true})).await.unwrap();
        r::observed_test_send(
            &f.state,
            &customer,
            &test,
            sent["message_id"].as_i64().unwrap(),
        )
        .await
        .unwrap();
        assert!(
            !r::confirm_test(&f.state, bot, 800, 222, "wrong", 9002)
                .await
                .unwrap()
        );
        assert!(
            r::confirm_test(&f.state, bot, 800, 222, code.as_str(), 9003)
                .await
                .unwrap()
        );
        assert!(
            !r::confirm_test(&f.state, bot, 801, 222, code.as_str(), 9004)
                .await
                .unwrap()
        );
        let live = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
            .await
            .unwrap();
        assert!(live.delivery_tested);
        let active = r::activate_owned(&f.state, &f.owner, &live.id, live.revision)
            .await
            .unwrap();
        assert!(active.expires_at.is_none());
        let public = r::capture(&f.state, bot, 800, 9005, false).await.unwrap();
        boxed_guide_text(
            &f.state,
            &public,
            "Published restaurant facts only",
            "Public question",
        )
        .await
        .unwrap();
        drain_settlements(&f.state.db).await;
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(doc! {"billing_owner_id": &f.owner, "forwarded": true})
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                .count_documents(doc! {"owner_id": &f.operator})
                .await
                .unwrap(),
            0
        );
        assert_funded_owner(&f.state.db, &f.owner, 2).await;
        for owner in [&f.operator, &f.profile.funding_owner_user_id] {
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
                    .count_documents(doc! {"owner_id": owner, "movement": "usage_settled"})
                    .await
                    .unwrap(),
                0
            );
        }
        let (other_owner, other_active, other_headers) = Box::pin(activate_second_client(&f)).await;
        assert_ne!(other_owner, f.owner);
        assert_eq!(other_active.platform_config_id, active.platform_config_id);
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::platform_setup::COLLECTION_NAME)
                .count_documents(doc! {"platform_config_id": &f.profile.id, "state": "active"})
                .await
                .unwrap(),
            2
        );
        assert!(
            setups::get_owned(&f.state.db, &f.owner, &other_active.id)
                .await
                .is_err()
        );
        assert!(
            setups::get_owned(&f.state.db, &other_owner, &active.id)
                .await
                .is_err()
        );
        let private_facts = crate::models::platform_setup::SetupFacts {
            name: "D PRIVATE UNPUBLISHED".into(),
            timezone: "UTC".into(),
            ..Default::default()
        };
        setups::create_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &other_owner,
            &uuid::Uuid::new_v4().to_string(),
            crate::models::platform_setup::SetupTemplate::RestaurantStarter,
            &private_facts,
        )
        .await
        .unwrap();
        f.state
            .db
            .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id": other_active.resources.agent_id.as_ref().unwrap()},
                doc! {"$set": {"persona": "D PRIVATE PERSONA NEVER PUBLIC"}},
            )
            .await
            .unwrap();
        // Expiring bounded onboarding authority does not expire child ownership.
        f.state
            .db
            .collection::<Document>(crate::models::platform_binding::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.binding.id},
                doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        r::capture(&f.state, bot, 800, 9006, false).await.unwrap();
        // Actual native delivery carries fixed restaurant copy even after onboarding expiry.
        let update = serde_json::json!({"update_id": 9007, "message": {"chat": {"id": 800, "type": "private"}, "from": {"id": 800, "is_bot": false}, "text": "Can I reserve?"}});
        assert_eq!(
            super::super::telegram_new_service::private_sender(&update["message"]),
            Some(800)
        );
        use sha2::Digest;
        let secret = "local-customer-webhook-test";
        f.state.db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(doc! {"_id": bot}, doc! {"$set": {"webhook_secret_hash": hex::encode(sha2::Sha256::digest(secret.as_bytes()))}}).await.unwrap();
        let body = bytes::Bytes::from(serde_json::to_vec(&update).unwrap());
        let uri: axum::http::Uri = "/".parse().unwrap();
        let denied = Box::pin(crate::handlers::channel_webhooks::channel_webhook_with_api(
            axum::extract::State(f.state.clone()),
            axum::extract::Path(("telegram-new".into(), bot.to_owned())),
            uri.clone(),
            axum::http::HeaderMap::new(),
            body.clone(),
            &base,
        ))
        .await;
        assert_eq!(denied.status(), axum::http::StatusCode::UNAUTHORIZED);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-telegram-bot-api-secret-token", secret.parse().unwrap());
        f.state.db.run_command(doc! {"collMod": super::super::platform_native_receipt_service::COLLECTION,
            "validator": {"fault_injection_required": {"$exists": true}}, "validationLevel": "strict"}).await.unwrap();
        let unavailable = Box::pin(crate::handlers::channel_webhooks::channel_webhook_with_api(
            axum::extract::State(f.state.clone()),
            axum::extract::Path(("telegram-new".into(), bot.to_owned())),
            uri.clone(),
            headers.clone(),
            body.clone(),
            &base,
        ))
        .await;
        assert_eq!(
            unavailable.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        f.state
            .db
            .run_command(
                doc! {"collMod": super::super::platform_native_receipt_service::COLLECTION,
                "validator": {}, "validationLevel": "strict"},
            )
            .await
            .unwrap();
        for _ in 0..2 {
            let admitted = Box::pin(crate::handlers::channel_webhooks::channel_webhook_with_api(
                axum::extract::State(f.state.clone()),
                axum::extract::Path(("telegram-new".into(), bot.to_owned())),
                uri.clone(),
                headers.clone(),
                body.clone(),
                &base,
            ))
            .await;
            assert_eq!(admitted.status(), axum::http::StatusCode::OK);
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let delivered = f
                    .server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|request| {
                        request.url.path().ends_with("/sendMessage")
                                && serde_json::from_slice::<serde_json::Value>(&request.body)
                                    .unwrap()["text"]
                                    .as_str()
                                    .is_some_and(|text| {
                                        text.ends_with(
                                            "Reservation requests need staff confirmation.",
                                        )
                                    })
                    });
                if delivered {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        drain_settlements(&f.state.db).await;
        assert_funded_owner(&f.state.db, &f.owner, 3).await;
        let other_bot = other_active.resources.channel_bot_id.as_deref().unwrap();
        // Telegram update IDs are per bot: the same customer/update must deliver on both children.
        for _ in 0..2 {
            let response = Box::pin(crate::handlers::channel_webhooks::channel_webhook_with_api(
                axum::extract::State(f.state.clone()),
                axum::extract::Path(("telegram-new".into(), other_bot.to_owned())),
                uri.clone(),
                other_headers.clone(),
                body.clone(),
                &base,
            ))
            .await;
            assert_eq!(response.status(), axum::http::StatusCode::OK);
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let count = f
                    .server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .filter(|r| r.url.path() == "/bot901:other-child-test-secret/sendMessage")
                    .count();
                if count == 2 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_funded_owner(&f.state.db, &other_owner, 2).await;
        let mut substituted = public.clone();
        if let PublicPurpose::Customer { setup, .. } = &mut substituted.purpose {
            *setup = other_active.clone();
        }
        assert!(admit_effect(&f.state, &substituted).await.is_err());
        let recovery_authority = r::capture(&f.state, bot, 800, 9008, false).await.unwrap();
        let receipt =
            super::super::platform_native_receipt_service::admit(&f.state, &recovery_authority)
                .await
                .unwrap()
                .unwrap();
        super::super::platform_native_receipt_service::finish(&f.state.db, &receipt, false)
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>(super::super::platform_native_receipt_service::COLLECTION)
            .update_one(
                doc! {"_id": &receipt.id},
                vec![doc! {"$set": {"retry_at": {"$subtract": ["$$NOW", 1_i64]}}}],
            )
            .await
            .unwrap();
        Box::pin(
            super::super::platform_native_receipt_service::sweep_with_service(&f.state, &service),
        )
        .await
        .unwrap();
        assert!(f.server.received_requests().await.unwrap().iter().any(|request| {
            request.url.path().ends_with("/sendMessage") && serde_json::from_slice::<serde_json::Value>(&request.body).unwrap()["text"] == "Your message did not finish delivery. Please send it again to continue."
        }));
        assert_funded_owner(&f.state.db, &f.owner, 3).await;

        assert!(
            super::super::platform_setup_service::runtime_notice(&f.state.db, &active)
                .await
                .unwrap()
                .is_none()
        );
        f.state
            .db
            .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
            .update_one(
                doc! {"provider": "telegram-new"},
                doc! {"$set": {"fields.webhook_ready": "false"}},
            )
            .await
            .unwrap();
        assert!(
            super::super::platform_setup_service::runtime_notice(&f.state.db, &active)
                .await
                .unwrap()
                .is_some()
        );
        assert!(r::capture(&f.state, bot, 800, 9010, false).await.is_err());
        f.state
            .db
            .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
            .update_one(
                doc! {"provider": "telegram-new"},
                doc! {"$set": {"fields.webhook_ready": "true"}},
            )
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(doc! {"_id": bot}, doc! {"$set": {"is_active": false}})
            .await
            .unwrap();
        assert!(
            super::super::platform_setup_service::runtime_notice(&f.state.db, &active)
                .await
                .unwrap()
                .is_some()
        );
        assert!(r::capture(&f.state, bot, 800, 9011, false).await.is_err());
        f.state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(doc! {"_id": bot}, doc! {"$set": {"is_active": true}})
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>(crate::models::telegram_bot_request::MANAGED_BOTS)
            .update_one(
                doc! {"telegram_bot_id": 900_i64},
                doc! {"$inc": {"revision": 1_i64}},
            )
            .await
            .unwrap();
        assert!(
            super::super::platform_setup_service::runtime_notice(&f.state.db, &active)
                .await
                .unwrap()
                .is_some()
        );
        assert!(r::capture(&f.state, bot, 800, 9012, false).await.is_err());
        f.state
            .db
            .collection::<Document>(crate::models::telegram_bot_request::MANAGED_BOTS)
            .update_one(
                doc! {"telegram_bot_id": 900_i64},
                doc! {"$inc": {"revision": -1_i64}},
            )
            .await
            .unwrap();
        // Destroyed client specialist fails closed before any additional model charge.
        f.state
            .db
            .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id": active.resources.agent_id.as_ref().unwrap()},
                doc! {"$set": {"destroyed_at": bson::DateTime::now()}},
            )
            .await
            .unwrap();
        assert!(r::capture(&f.state, bot, 800, 9007, false).await.is_err());
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(doc! {"billing_owner_id": &f.owner, "forwarded": true})
                .await
                .unwrap(),
            3
        );
        assert_eq!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.url.path() == "/chat/completions")
                .count(),
            5
        );
        assert!(admit_effect(&f.state, &public).await.is_err());
        r::capture(&f.state, other_bot, 800, 9019, false)
            .await
            .unwrap();
        let requests = f.server.received_requests().await.unwrap();
        let prompts: Vec<serde_json::Value> = requests
            .iter()
            .filter(|r| r.url.path() == "/chat/completions")
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        let d_prompts: Vec<&serde_json::Value> = prompts
            .iter()
            .filter(|body| {
                body["messages"][0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("D restaurant publication")
            })
            .collect();
        assert_eq!(d_prompts.len(), 2);
        for body in &prompts {
            assert!(body.get("tools").is_none());
            assert!(!body.to_string().contains("D PRIVATE"));
        }
        for body in d_prompts {
            assert!(
                !body["messages"][0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("Client cafe")
            );
        }
        assert_funded_owner(&f.state.db, &f.owner, 3).await;
        assert_funded_owner(&f.state.db, &other_owner, 2).await;
    }

    #[tokio::test]
    async fn platform_onboarding_models_discovery_prices_operation_and_uses_sponsor_and_protocol_headers()
     {
        use crate::models::{
            downstream_service::*, service_billing::*, service_endpoint::ServiceEndpoint,
        };
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{header, method, path},
        };
        let f = Box::pin(funded_fixture()).await;
        let id = &f.profile.guide.as_ref().unwrap().model_service_id;
        let service = selected_service(&f.state, &f.profile).await.unwrap();
        let code =
            super::super::billing::pricing::operation_metric_code(&service.slug, "pk", "models")
                .unwrap();
        f.state
            .db
            .collection::<ServiceEndpoint>(crate::models::service_endpoint::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_service_endpoint(
                id, "models", "GET", "/models",
            ))
            .await
            .unwrap();
        f.state.db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id": id}, doc! {"$set": {
            "inference.wire_protocol": "anthropic_messages", "billing.platform_key_pricing.operations": bson::to_bson(&vec![OperationPrice {
                operation: "models".into(), credits_per_unit: "0.002".into(), lago_metric_code: code.clone(),
                sync_status: PricingSyncStatus::Synced, sync_error: None
            }]).unwrap()}}).await.unwrap();
        let mut rate = crate::billing_integration_tests::rate(&code, chrono::Utc::now());
        rate.credits_per_unit_micros = 2000;
        f.state
            .db
            .collection::<crate::models::billing_rate_cache::BillingRateCache>(
                crate::models::billing_rate_cache::COLLECTION_NAME,
            )
            .insert_one(rate)
            .await
            .unwrap();
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": [{"id": "test-model"}]})),
            )
            .mount(&f.server)
            .await;
        assert_eq!(
            discover_models(&f.state, &f.profile).await.unwrap(),
            vec!["test-model"]
        );
        drain_settlements(&f.state.db).await;
        let row = f
            .state
            .db
            .collection::<crate::models::usage_meter::UsageMeterRow>(
                crate::models::usage_meter::COLLECTION_NAME,
            )
            .find_one(doc! {"billing_owner_id": &f.profile.funding_owner_user_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.operation.as_deref(), Some("models"));
        assert_eq!(row.lago_metric_code, code);
        assert_eq!(
            row.funding.unwrap().wallet_charge_credits,
            Some(Credits::from_micros(2000))
        );
        for owner in [&f.operator, &f.owner] {
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                    .count_documents(doc! {"owner_id": owner})
                    .await
                    .unwrap(),
                0
            );
        }
    }

    #[tokio::test]
    async fn platform_onboarding_definite_model_refusal_settles_zero_but_keeps_dispatch_budget() {
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let f = Box::pin(funded_fixture()).await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"error": {"code": "model_not_found"}})),
            )
            .mount(&f.server)
            .await;
        let authority = GuideAuthority {
            profile: f.profile.clone(),
            binding: Some(f.binding.clone()),
            sender: "700".into(),
            update_id: 12000,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        assert!(
            boxed_guide_text(&f.state, &authority, "Closed guide", "Public question")
                .await
                .is_err()
        );
        drain_settlements(&f.state.db).await;
        let row = f
            .state
            .db
            .collection::<crate::models::usage_meter::UsageMeterRow>(
                crate::models::usage_meter::COLLECTION_NAME,
            )
            .find_one(doc! {"billing_owner_id": &f.profile.funding_owner_user_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.quantity, Some(0));
        assert_eq!(
            row.funding.unwrap().wallet_charge_credits,
            Some(Credits::ZERO)
        );
        let turn = f
            .state
            .db
            .collection::<Document>(TURNS)
            .find_one(doc! {"update_id": 12000_i64})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(turn.get_str("state").unwrap(), "completed");
        assert!(turn.get_bool("forwarded").unwrap());
        sweep_abandoned(&f.state.db).await.unwrap();
        let quota = f
            .state
            .db
            .collection::<Document>(QUOTAS)
            .find_one(doc! {"scope": scope(&authority)})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(exact_money(&quota, "spent"), Credits::from_micros(1000));
    }

    #[tokio::test]
    async fn platform_onboarding_admin_revoked_during_funded_readiness_cannot_enable() {
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let mut f = Box::pin(funded_fixture()).await;
        f.profile.enabled = false;
        f.state
            .db
            .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.profile.id},
                doc! {"$set": {"enabled": false}},
            )
            .await
            .unwrap();
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": [{"id": "test-model"}]})),
            )
            .mount(&f.server)
            .await;
        Mock::given(method("POST")).and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(500)).set_body_json(serde_json::json!({
                "model": "test-model", "choices": [{"message": {"content": "Ready"}}], "usage": {"prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3}
            }))).mount(&f.server).await;
        let configure = super::super::platform_bot_config_service::configure_guide(
            &f.state,
            &f.operator,
            &f.profile.id,
            f.profile.generation,
            f.profile.guide.clone().unwrap(),
            true,
        );
        let revoke = async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if f.server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|request| request.url.path() == "/chat/completions")
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            f.state
                .db
                .collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &f.operator},
                    doc! {"$set": {"role_ids": [], "is_admin": true}},
                )
                .await
                .unwrap();
        };
        let (result, ()) = tokio::join!(configure, revoke);
        assert!(result.is_err());
        let profile = super::super::platform_bot_config_service::get(&f.state.db, &f.profile.id)
            .await
            .unwrap();
        assert!(!profile.enabled);
        assert_eq!(profile.generation, f.profile.generation);
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                .count_documents(doc! {"owner_id": &f.operator})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn platform_onboarding_stalled_telegram_recovery_notice_does_not_block_budget_refund() {
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let f = Box::pin(funded_fixture()).await;
        let a = GuideAuthority {
            profile: f.profile.clone(),
            binding: Some(f.binding.clone()),
            sender: "700".into(),
            update_id: 14000,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        let turn = admit(&f.state, &a).await.unwrap().unwrap();
        reserve_budget(&f.state, &a, &turn, Credits::from_micros(1000))
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>(TURNS)
            .update_one(
                doc! {"_id": &turn},
                doc! {"$set": {"lease_expires_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let receipt = super::super::platform_native_receipt_service::admit(&f.state, &a)
            .await
            .unwrap()
            .unwrap();
        super::super::platform_native_receipt_service::finish(&f.state.db, &receipt, false)
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>(super::super::platform_native_receipt_service::COLLECTION)
            .update_one(
                doc! {"_id": &receipt.id},
                doc! {"$set": {"retry_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/bot100:manager-test-secret/sendMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_secs(1))
                    .set_body_json(serde_json::json!({"ok": true, "result": {"message_id": 444}})),
            )
            .with_priority(1)
            .mount(&f.server)
            .await;
        let base = f.server.uri();
        let service = super::super::telegram_new_tests::service(&f.state, &base);
        let completed = std::sync::atomic::AtomicBool::new(false);
        let notice = async {
            super::super::platform_native_receipt_service::sweep_with_service(&f.state, &service)
                .await
                .unwrap();
            completed.store(true, std::sync::atomic::Ordering::SeqCst);
        };
        let budget = async {
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    if f.server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|request| {
                            request
                                .body_json::<serde_json::Value>()
                                .ok()
                                .is_some_and(|body| {
                                    body["text"].as_str().is_some_and(|text| {
                                        text.contains("did not finish delivery")
                                    })
                                })
                        })
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            sweep_abandoned(&f.state.db).await.unwrap();
            assert!(!completed.load(std::sync::atomic::Ordering::SeqCst));
            let row = f
                .state
                .db
                .collection::<Document>(QUOTAS)
                .find_one(doc! {"scope": scope(&a)})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(exact_money(&row, "spent"), Credits::ZERO);
        };
        tokio::join!(notice, budget);
    }

    #[tokio::test]
    async fn platform_onboarding_legacy_catalog_defaults_do_not_break_paused_configuration() {
        let f = Box::pin(funded_fixture()).await;
        let id = &f.profile.guide.as_ref().unwrap().model_service_id;
        f.state.db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME).update_one(doc! {"_id": id}, doc! {"$unset": {
            "topics": "", "offering_kind": "", "streaming_supported": "", "catalog_manager_created": ""
        }}).await.unwrap();
        let configured = super::super::platform_bot_config_service::configure_guide(
            &f.state,
            &f.operator,
            &f.profile.id,
            f.profile.generation,
            f.profile.guide.clone().unwrap(),
            false,
        )
        .await
        .unwrap();
        assert!(!configured.enabled);
        assert!(configured.generation > f.profile.generation);
        let projection =
            readiness_projection(&selected_service(&f.state, &configured).await.unwrap());
        assert!(projection.get("credential_encrypted").is_none());
    }

    #[tokio::test]
    async fn platform_onboarding_service_changed_during_readiness_cannot_enable() {
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let mut f = Box::pin(funded_fixture()).await;
        f.profile.enabled = false;
        f.state
            .db
            .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.profile.id},
                doc! {"$set": {"enabled": false}},
            )
            .await
            .unwrap();
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": [{"id": "test-model"}]})),
            )
            .mount(&f.server)
            .await;
        Mock::given(method("POST")).and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(500)).set_body_json(serde_json::json!({"model": "test-model", "choices": [{"message": {"content": "Ready"}}]}))).mount(&f.server).await;
        let configure = super::super::platform_bot_config_service::configure_guide(
            &f.state,
            &f.operator,
            &f.profile.id,
            f.profile.generation,
            f.profile.guide.clone().unwrap(),
            true,
        );
        let change = async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if f.server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|request| request.url.path() == "/chat/completions")
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            f.state.db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME).update_one(doc! {"_id": &f.profile.guide.as_ref().unwrap().model_service_id},
                doc! {"$set": {"base_url": format!("{}/unreviewed-target", f.server.uri()), "updated_at": bson::DateTime::now()}}).await.unwrap();
        };
        let (result, ()) = tokio::join!(configure, change);
        assert!(result.is_err());
        let profile = super::super::platform_bot_config_service::get(&f.state.db, &f.profile.id)
            .await
            .unwrap();
        assert!(!profile.enabled);
        assert_eq!(profile.generation, f.profile.generation);
        assert!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| !request.url.path().contains("unreviewed-target"))
        );
    }

    #[tokio::test]
    async fn platform_onboarding_abandoned_budget_recovery_refunds_once_and_preserves_uncertain_dispatch()
     {
        let fixture = super::super::platform_onboarding_test_support::fixture().await;
        let db = &fixture.state.db;
        let mut authority = GuideAuthority {
            profile: fixture.profile.clone(),
            binding: Some(fixture.binding.clone()),
            sender: "700".into(),
            update_id: 6000,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        let cost = Credits::from_micros(1000);
        for (forward, rollover) in [(false, false), (true, false), (false, true)] {
            authority.update_id += 1;
            let turn = admit(&fixture.state, &authority).await.unwrap().unwrap();
            reserve_budget(&fixture.state, &authority, &turn, cost)
                .await
                .unwrap();
            if forward {
                admit_forward(&fixture.state, &authority, &turn)
                    .await
                    .unwrap();
            }
            if rollover {
                // A new day's counter must never be debited by an old hold.
                db.collection::<Document>(TURNS)
                    .update_one(
                        doc! {"_id": &turn},
                        doc! {"$set": {"reserved_day": bson::DateTime::from_millis(0)}},
                    )
                    .await
                    .unwrap();
            }
            // Simulate process loss after the production admission and reservation,
            // with no Drop cleanup; only advance the durable lease deadline.
            db.collection::<Document>(TURNS)
                .update_one(
                    doc! {"_id": &turn},
                    doc! {"$set": {"lease_expires_at": bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
            sweep_abandoned(db).await.unwrap();
            sweep_abandoned(db).await.unwrap();
            assert!(
                admit_forward(&fixture.state, &authority, &turn)
                    .await
                    .is_err()
            );
            let row = db
                .collection::<Document>(TURNS)
                .find_one(doc! {"_id": &turn})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.get_str("state").unwrap(), "completed");
            assert_eq!(row.get_bool("forwarded").unwrap(), forward);
            let quota = db
                .collection::<Document>(QUOTAS)
                .find_one(doc! {"scope": scope(&authority)})
                .await
                .unwrap()
                .unwrap();
            let expected = if forward {
                cost
            } else if rollover {
                cost.checked_mul(2).unwrap()
            } else {
                Credits::ZERO
            };
            assert_eq!(exact_money(&quota, "spent"), expected);
            assert!(quota.get_array("leases").unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn explicit_turn_release_allows_immediate_same_scope_admission_and_is_idempotent() {
        let fixture = super::super::platform_onboarding_test_support::fixture().await;
        let mut profile = fixture.profile.clone();
        let config = profile.guide.as_mut().unwrap();
        config.limits.concurrent_turns = 1;
        config.platform_concurrent_turns = 1;
        let mut authority = GuideAuthority {
            profile,
            binding: Some(fixture.binding.clone()),
            sender: "700".into(),
            update_id: 5000,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        let first = admit(&fixture.state, &authority).await.unwrap().unwrap();
        authority.update_id += 1;
        assert!(matches!(
            admit(&fixture.state, &authority).await,
            Err(AppError::RateLimited)
        ));
        let mut guard = TurnGuard {
            db: fixture.state.db.clone(),
            id: first.clone(),
            forwarded: false,
            armed: true,
        };
        guard.finish().await.unwrap();
        assert!(!guard.armed);
        // No timer, scheduler yield or spawned Drop cleanup separates these admissions.
        let second = admit(&fixture.state, &authority).await.unwrap().unwrap();
        release(&fixture.state.db, &first, false).await.unwrap();
        let row = fixture
            .state
            .db
            .collection::<Document>(QUOTAS)
            .find_one(doc! {"scope": scope(&authority)})
            .await
            .unwrap()
            .unwrap();
        let leases = row.get_array("leases").unwrap();
        assert_eq!(leases.len(), 1);
        assert_eq!(
            leases[0].as_document().unwrap().get_str("turn_id").unwrap(),
            second
        );
        release(&fixture.state.db, &second, false).await.unwrap();
    }
}

pub(crate) async fn eligible_catalog_models(
    state: &AppState,
    profile: &PlatformBotConfig,
    after: Option<&str>,
) -> AppResult<(Vec<(String, String)>, Option<String>)> {
    use futures::TryStreamExt;
    let owner = sponsor_owner(state, profile).await?;
    let grants = super::platform_key_service::OwnerGrants::load(&state.db, &owner).await?;
    let providers = super::platform_key_service::load_providers(&state.db).await?;
    let mut filter = doc! {"is_active": true, "service_type": "http", "inference.model_list": true, "slug": {"$ne": "llm-nyx"}};
    if let Some(after) = after {
        filter.insert("_id", doc! {"$gt": after});
    }
    let rows: Vec<DownstreamService> = state
        .db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id": 1})
        .limit(201)
        .await?
        .try_collect()
        .await?;
    let next = if rows.len() > 200 {
        Some(rows[199].id.clone())
    } else {
        None
    };
    let result = rows
        .into_iter()
        .take(200)
        .filter(|service| {
            super::platform_key_service::available_with_grants(
                service,
                service
                    .provider_config_id
                    .as_ref()
                    .and_then(|id| providers.get(id)),
                &owner,
                &grants,
            )
        })
        .map(|service| (service.id, service.name))
        .collect();
    Ok((result, next))
}
