//! Durable, exact-identity gateway retirement. Never retries channel creation.
use super::{api_key_mutation_service as transactions, nyxbot_gateway_service as gateway};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        nyxbot_channel::{COLLECTION_NAME as CHANNELS, NyxbotChannel},
        nyxbot_gateway_release::{
            COLLECTION_NAME as RELEASES, NyxbotGatewayRelease as Release, RECONCILE_ID,
            ReleaseReason, ReleaseReconcileCursor, ReleaseState,
        },
    },
};
use chrono::{Duration, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use serde_json::{Value, json};
use uuid::Uuid;

const MIGRATIONS: &str = "schema_migrations";
/// Shared by the provisional deadline and stale transport-move selection.
pub(crate) const SETUP_MINUTES: i64 = 30;
const UNCERTAIN_HOURS: i64 = 24;

fn conflict() -> AppError {
    AppError::Conflict("Gateway lifecycle changed; reload and retry".into())
}
fn canonical(platform: &str) -> &str {
    if platform == "telegram-new" {
        "telegram"
    } else {
        platform
    }
}
fn bot_owner(row: &NyxbotChannel) -> &str {
    row.bot_owner_id.as_deref().unwrap_or(&row.user_id)
}
fn valid_channel_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn encoded(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

pub(crate) async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    for keys in [
        doc! {"state":1,"next_attempt_at":1,"lease_until":1,"_id":1},
        doc! {"state":1,"provisional_expires_at":1,"_id":1},
        doc! {"channel_id":1,"state":1},
        doc! {"agent_api_key_id":1,"binding_id":1},
    ] {
        db.collection::<Release>(RELEASES)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    for field in ["record_id", "gateway_channel_id"] {
        db.collection::<Release>(RELEASES)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"gateway_service_id":1,"owner_user_id":1,field:1})
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(doc! {field:{"$type":"string"}})
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    db.collection::<Document>(CHANNELS).create_index(IndexModel::builder().keys(doc! {"_id":1,"status":1})
        .options(IndexOptions::builder().name("gateway_release_reconcile".to_owned()).partial_filter_expression(
            doc! {"$or":[{"gateway_channel_id":{"$type":"string"}},{"gateway_record_id":{"$type":"string"}}]}).build()).build()).await?;
    Ok(())
}

pub(crate) async fn audit(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    event: &str,
    data: Value,
) -> AppResult<()> {
    if owner == "system" {
        let key = super::audit_service::audit_chain_hmac_key()
            .ok_or_else(|| AppError::Internal("Audit authority unavailable".into()))?;
        return super::audit_service::log_system_event_in_session(db, session, key, event, data)
            .await;
    }
    super::audit_service::log_actor_event_in_transaction(
        db,
        session,
        &super::audit_service::AuditActor {
            user_id: owner.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        event,
        data,
    )
    .await
}

async fn new_release(
    state: &AppState,
    row: &NyxbotChannel,
    record_id: Option<String>,
) -> AppResult<Release> {
    let service = super::assistant_service::resolve_admin_service_by_slug(&state.db, "cmaeg")
        .await
        .map_err(|_| AppError::OAuthAuthorityUnavailable)?;
    let base = service.base_url.trim_end_matches('/').to_owned();
    let parsed = url::Url::parse(&base).map_err(|_| AppError::OAuthAuthorityUnavailable)?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(parsed.scheme() == "https" || (local && parsed.scheme() == "http"))
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AppError::OAuthAuthorityUnavailable);
    }
    let now = Utc::now();
    Ok(Release {
        id: record_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        schema_version: 1,
        owner_user_id: row.user_id.clone(),
        channel_id: row.id.clone(),
        channel_bot_id: row.channel_bot_id.clone(),
        bot_owner_id: bot_owner(row).into(),
        platform: canonical(&row.platform).into(),
        gateway_service_id: service.id,
        gateway_base_url: base,
        creator_issuer: state.config.jwt_issuer.clone(),
        record_id,
        gateway_channel_id: None,
        binding_id: String::new(),
        agent_api_key_id: row.agent_api_key_id.clone().unwrap_or_default(),
        route_api_key_id: row.route_api_key_id.clone(),
        expected_version: None,
        state: ReleaseState::Provisional,
        reason: ReleaseReason::Setup,
        dispatch_started_at: None,
        provisional_expires_at: Some(now + Duration::minutes(SETUP_MINUTES)),
        due_at: None,
        next_attempt_at: None,
        attempt_count: 0,
        claim_id: None,
        lease_until: None,
        revision: 0,
        last_error: None,
        observed_version: None,
        resolution: None,
        reviewed_by: None,
        settled_at: None,
        created_at: now,
        updated_at: now,
    })
}

/// Reserve the deterministic provider identity before any dispatch. The fence
/// includes the dedicated keys so an older setup cannot attach to a new attempt.
pub(crate) async fn reserve(
    state: &AppState,
    row: &NyxbotChannel,
    fence: Document,
) -> AppResult<Release> {
    let record = Uuid::new_v4();
    let mut release = new_release(state, row, Some(record.to_string())).await?;
    release.binding_id = format!("bnd_{}", record.simple());
    let db = state.db.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            let mut fence=fence.clone(); fence.insert("gateway_setup_release_id",bson::Bson::Null);
            if db.collection::<Document>(CHANNELS).update_one(fence,doc! {"$set":{"gateway_setup_release_id":&release.id,"binding_id":&release.binding_id}})
                .session(&mut *session).await?.matched_count!=1 {return Err(conflict());}
            db.collection::<Release>(RELEASES).insert_one(&release).session(&mut *session).await?;
            Ok(release.clone())
        }).await; transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

fn setup_fence(release: &Release) -> Document {
    doc! {"_id":&release.id,"state":"provisional","$expr":{"$gt":["$provisional_expires_at","$$NOW"]}}
}

pub(crate) async fn dispatch(db: &Database, release: &Release) -> AppResult<()> {
    let mut filter = setup_fence(release);
    filter.insert("dispatch_started_at", bson::Bson::Null);
    if db
        .collection::<Document>(RELEASES)
        .update_one(
            filter,
            vec![
                doc! {"$set":{"dispatch_started_at":"$$NOW","updated_at":"$$NOW"}},
                doc! {"$unset":"claim_id"},
            ],
        )
        .await?
        .matched_count
        != 1
    {
        return Err(conflict());
    }
    Ok(())
}

/// Acknowledged identities and versions are persisted before another effect.
pub(crate) async fn record_ack(
    db: &Database,
    release: &Release,
    channel: &str,
    version: i64,
) -> AppResult<()> {
    if version <= 0 || channel.is_empty() {
        return Err(conflict());
    }
    let mut filter = setup_fence(release);
    filter.insert(
        "$or",
        bson::to_bson(&[
            doc! {"gateway_channel_id":bson::Bson::Null},
            doc! {"gateway_channel_id":channel},
        ])
        .map_err(|_| conflict())?,
    );
    if db.collection::<Document>(RELEASES).update_one(filter,doc! {"$set":{"gateway_channel_id":channel,"expected_version":version,"updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64}})
        .await?.matched_count!=1 {return Err(conflict());}
    Ok(())
}

pub(crate) fn validate_create(
    release: &Release,
    value: &Value,
) -> AppResult<(String, String, i64)> {
    let channel = value["channel_id"]
        .as_str()
        .filter(|v| valid_channel_id(v))
        .ok_or_else(conflict)?;
    let callback = value["endpoints"]["nyxid_callback_url"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(conflict)?;
    let version = value["version"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(conflict)?;
    if value["record_id"].as_str() != release.record_id.as_deref()
        || value["provider"]["slug"] != "nyxbot"
        || value["provider"]["binding_id"] != release.binding_id
    {
        return Err(conflict());
    }
    // The callback is a credential destination too; require the captured origin.
    let base = url::Url::parse(&release.gateway_base_url).map_err(|_| conflict())?;
    let target = url::Url::parse(callback).map_err(|_| conflict())?;
    if base.origin() != target.origin()
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
    {
        return Err(conflict());
    }
    Ok((channel.into(), callback.into(), version))
}

pub(crate) async fn adopt_in_session(
    db: &Database,
    row: &NyxbotChannel,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(id) = row.gateway_setup_release_id.as_deref() else {
        if row.transport == "gateway" {
            return Err(conflict());
        }
        return Ok(());
    };
    let filter = doc! {"_id":id,"channel_id":&row.id,"state":"provisional","gateway_channel_id":&row.gateway_channel_id,
    "expected_version":row.gateway_version,"agent_api_key_id":&row.agent_api_key_id,"route_api_key_id":&row.route_api_key_id,
    "$expr":{"$gt":["$provisional_expires_at","$$NOW"]}};
    if row.gateway_channel_id.is_none()
        || row.gateway_version.is_none_or(|v| v <= 0)
        || db
            .collection::<Release>(RELEASES)
            .delete_one(filter)
            .session(&mut *session)
            .await?
            .deleted_count
            != 1
    {
        return Err(conflict());
    }
    Ok(())
}

async fn queue(
    db: &Database,
    release: &Release,
    reason: ReleaseReason,
    session: &mut ClientSession,
) -> AppResult<()> {
    if release.state != ReleaseState::Provisional {
        return Ok(());
    }
    let state = if release.dispatch_started_at.is_none() {
        "settled"
    } else {
        "due"
    };
    let mut set = doc! {"state":state,"reason":bson::to_bson(&reason).map_err(|_| conflict())?,"due_at":bson::DateTime::now(),
    "next_attempt_at":bson::DateTime::now(),"claim_id":bson::Bson::Null,"lease_until":bson::Bson::Null,"updated_at":bson::DateTime::now()};
    if state == "settled" {
        set.insert("resolution", "cancelled");
        set.insert("settled_at", bson::DateTime::now());
    }
    if db
        .collection::<Document>(RELEASES)
        .update_one(
            doc! {"_id":&release.id,"state":"provisional","revision":release.revision},
            doc! {"$set":set,"$inc":{"revision":1_i64}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(conflict());
    }
    audit(
        db,
        session,
        &release.owner_user_id,
        if state == "settled" {
            "nyxbot_gateway_release_settled"
        } else {
            "nyxbot_gateway_release_queued"
        },
        json!({"obligation_id":release.id,"channel_id":release.channel_id,"reason":reason}),
    )
    .await
}

/// Capture current identity before its local references are removed. Pending
/// attempts already have their own immutable reservation.
pub(crate) async fn retire_in_session(
    state: &AppState,
    row: &NyxbotChannel,
    reason: ReleaseReason,
    session: &mut ClientSession,
) -> AppResult<Vec<String>> {
    let db = &state.db;
    let mut ids = Vec::new();
    if let Some(id) = row.gateway_setup_release_id.as_deref() {
        let pending = db
            .collection::<Release>(RELEASES)
            .find_one(doc! {"_id":id,"channel_id":&row.id})
            .session(&mut *session)
            .await?
            .ok_or_else(conflict)?;
        queue(db, &pending, reason, session).await?;
        ids.push(id.into());
    }
    if row.gateway_channel_id.is_none() && row.gateway_record_id.is_none() {
        return Ok(ids);
    }
    if conversion_owns(db, row, session).await? {
        return Ok(ids);
    }
    let mut identity = identity_filter(row);
    identity.insert("channel_id", &row.id);
    if let Some(existing) = db
        .collection::<Release>(RELEASES)
        .find_one(identity)
        .session(&mut *session)
        .await?
    {
        queue(db, &existing, reason, session).await?;
        if !ids.contains(&existing.id) {
            ids.push(existing.id);
        }
        return Ok(ids);
    }
    let record = row
        .gateway_record_id
        .clone()
        .filter(|v| Uuid::parse_str(v).is_ok());
    let mut release = new_release(state, row, record).await?;
    // A second retained row may name the same remote identity. Use the unique
    // destination indexes, or the reserved record's primary key after drift.
    let mut identity = identity_filter(row);
    identity.insert("gateway_service_id", &release.gateway_service_id);
    if let Some(existing) = db
        .collection::<Release>(RELEASES)
        .find_one(identity)
        .session(&mut *session)
        .await?
    {
        queue(db, &existing, reason, session).await?;
        ids.push(existing.id);
        return Ok(ids);
    }
    if release.record_id.is_some()
        && let Some(existing) = db
            .collection::<Release>(RELEASES)
            .find_one(doc! {"_id":&release.id,"owner_user_id":&row.user_id,"record_id":&release.record_id})
            .session(&mut *session)
            .await?
    {
        queue(db, &existing, reason, session).await?;
        ids.push(existing.id);
        return Ok(ids);
    }
    release.gateway_channel_id = row.gateway_channel_id.clone();
    release.binding_id = row.binding_id.clone().unwrap_or_default();
    release.expected_version = row.gateway_version.filter(|v| *v > 0);
    release.reason = reason;
    release.dispatch_started_at = Some(row.created_at);
    release.provisional_expires_at = None;
    release.due_at = Some(Utc::now());
    release.next_attempt_at = Some(Utc::now());
    let record_recovery = release.gateway_channel_id.is_none()
        && release
            .record_id
            .as_ref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_some_and(|id| release.binding_id == format!("bnd_{}", id.simple()));
    let proof = !release.binding_id.is_empty()
        && !release.agent_api_key_id.is_empty()
        && (release.expected_version.is_some() || record_recovery);
    release.state = if proof {
        ReleaseState::Due
    } else {
        ReleaseState::ReviewRequired
    };
    if !proof {
        release.last_error = Some("retirement_proof_missing".into());
    }
    db.collection::<Release>(RELEASES)
        .insert_one(&release)
        .session(&mut *session)
        .await?;
    audit(
        db,
        session,
        &row.user_id,
        if proof {
            "nyxbot_gateway_release_queued"
        } else {
            "nyxbot_gateway_release_review_required"
        },
        json!({"obligation_id":release.id,"channel_id":row.id,"reason":reason}),
    )
    .await?;
    if !proof {
        tracing::error!(obligation_id=%release.id, channel_id=%release.channel_id,
            reason="retirement_proof_missing", "Gateway release requires review");
    }
    ids.push(release.id);
    Ok(ids)
}

fn identity_filter(row: &NyxbotChannel) -> Document {
    let mut alternatives = Vec::new();
    if let Some(id) = &row.gateway_channel_id {
        alternatives.push(doc! {"gateway_channel_id":id});
    }
    if let Some(id) = &row.gateway_record_id {
        alternatives.push(doc! {"record_id":id});
    }
    doc! {"owner_user_id":&row.user_id,"$or":alternatives}
}

async fn conversion_owns(
    db: &Database,
    row: &NyxbotChannel,
    session: &mut ClientSession,
) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
        .find_one(doc! {"kind":"gateway","resource_id":&row.id,"owner_user_id":&row.user_id})
        .session(session)
        .await?
        .is_some())
}

pub(crate) async fn revoke_in_session(
    db: &Database,
    owner: &str,
    key: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    match super::key_service::delete_api_key_in_session(db, owner, key, None, Some(session)).await {
        Ok(_) | Err(AppError::NotFound(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Retire only a still-unadopted attempt. A failed read/unknown commit never
/// authorizes rollback of a subsequently adopted connection.
pub(crate) async fn retire_attempt(
    state: &AppState,
    id: &str,
    reason: ReleaseReason,
) -> AppResult<()> {
    let state = state.clone();
    let id = id.to_owned();
    let db = state.db.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            let mut filter=doc! {"_id":&id,"$or":[{"state":"provisional"},{"state":"settled","resolution":"not_created"}]};
            if reason==ReleaseReason::StaleSetup {filter.insert("$expr",doc! {"$lte":["$provisional_expires_at","$$NOW"]});}
            let Some(release)=db.collection::<Release>(RELEASES).find_one(filter).session(&mut *session).await? else {return Ok(());};
            let row=db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {"_id":&release.channel_id}).session(&mut *session).await?;
            if let Some(row)=row {
                if row.status=="active" && row.transport=="gateway" && row.agent_api_key_id.as_deref()==Some(&release.agent_api_key_id)
                    && row.gateway_channel_id==release.gateway_channel_id
                    && row.gateway_record_id==release.record_id && row.binding_id.as_deref()==Some(&release.binding_id)
                    && row.route_api_key_id==release.route_api_key_id && row.gateway_setup_release_id.is_none() {
                    db.collection::<Release>(RELEASES).delete_one(doc! {"_id":&id,"state":"provisional"}).session(&mut *session).await?;
                    return Ok(());
                }
                if row.gateway_setup_release_id.as_deref()==Some(&id) {
                    let mut set=doc! {"gateway_no_progress":false,"updated_at":bson::DateTime::now()};
                    let mut unset=doc! {"gateway_setup_release_id":"","pending_agent_api_key_id":"","pending_route_api_key_id":"",
                        "binding_id":""};
                    if row.status=="pending" {
                        set.insert("status","failed");set.insert("last_error","setup_interrupted");
                        unset.insert("agent_key_ciphertext","");
                        unset.insert("link_code_hash","");unset.insert("link_code_expires_at","");
                    }
                    db.collection::<Document>(CHANNELS).update_one(doc! {"_id":&row.id,"gateway_setup_release_id":&id},doc! {
                        "$set":set,"$unset":unset}).session(&mut *session).await?;
                }
            }
            queue(&db,&release,reason,session).await?;
            revoke_in_session(&db,&release.owner_user_id,&release.agent_api_key_id,session).await?;
            revoke_in_session(&db,&release.bot_owner_id,&release.route_api_key_id,session).await?;
            super::channel_routing_service::retire_key_routes_in_session(&db,&release.bot_owner_id,&release.channel_bot_id,&release.route_api_key_id,session).await?;
            Ok(())
        }).await;transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

pub(crate) async fn retire_connection(
    state: &AppState,
    row: &NyxbotChannel,
    reason: ReleaseReason,
    status: &str,
    error: Option<&str>,
) -> AppResult<Vec<String>> {
    let state = state.clone();
    let db = state.db.clone();
    let row = row.clone();
    let status = status.to_owned();
    let error = error.map(str::to_owned);
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            let current=db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {"_id":&row.id,"user_id":&row.user_id}).session(&mut *session).await?.ok_or_else(conflict)?;
            if (reason==ReleaseReason::SetupFailed && current.status!="pending") || (reason==ReleaseReason::OrgAccessLost && !matches!(current.status.as_str(),"pending"|"active")) {return Ok(Vec::new());}
            let ids=retire_in_session(&state,&current,reason,session).await?;
            db.collection::<Document>(CHANNELS).update_one(doc! {"_id":&current.id},doc! {"$set":{"status":&status,"last_error":&error,"gateway_no_progress":false,"updated_at":bson::DateTime::now()},
                "$unset":{"gateway_setup_release_id":"","agent_key_ciphertext":"","pending_agent_api_key_id":"","pending_route_api_key_id":"","link_code_hash":"","link_code_expires_at":""}}).session(&mut *session).await?;
            super::channel_thread_follow_service::suspend_in_session(&db,&current.user_id,&current.id,error.as_deref().unwrap_or("disconnected"),session).await?;
            for key in [Some(current.route_api_key_id.as_str()),current.pending_route_api_key_id.as_deref()].into_iter().flatten() {
                super::channel_routing_service::retire_key_routes_in_session(&db,bot_owner(&current),&current.channel_bot_id,key,session).await?;
                revoke_in_session(&db,bot_owner(&current),key,session).await?;
            }
            for key in [current.agent_api_key_id.as_deref(),current.pending_agent_api_key_id.as_deref()].into_iter().flatten() {
                revoke_in_session(&db,&current.user_id,key,session).await?;
            }
            if reason == ReleaseReason::OrgAccessLost {
                audit(&db, session, &current.user_id, "nyxbot_channel_org_access_lost",
                    json!({"channel_agent_id":current.id,"platform":current.platform})).await?;
            }
            Ok(ids)
        }).await;transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

fn live_filter(release: &Release) -> Document {
    let mut identity = vec![
        doc! {"agent_api_key_id":&release.agent_api_key_id},
        doc! {"pending_agent_api_key_id":&release.agent_api_key_id},
        doc! {"route_api_key_id":&release.route_api_key_id},
        doc! {"pending_route_api_key_id":&release.route_api_key_id},
        doc! {"binding_id":&release.binding_id},
    ];
    if let Some(id) = &release.gateway_channel_id {
        identity.push(doc! {"gateway_channel_id":id});
    }
    if let Some(id) = &release.record_id {
        identity.push(doc! {"gateway_record_id":id});
    }
    // The retired pending attempt's own working direct route is unrelated.
    doc! {"status":{"$in":["pending","active"]},"$or":identity}
}

async fn live_reference(db: &Database, release: &Release) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(CHANNELS)
        .find_one(live_filter(release))
        .await?
        .is_some())
}

fn claim_fence(release: &Release) -> Document {
    doc! {"_id":&release.id,"claim_id":&release.claim_id,"revision":release.revision,"state":"due","$expr":{"$gt":["$lease_until","$$NOW"]}}
}

pub(crate) async fn claim(db: &Database, id: Option<&str>) -> AppResult<Option<Release>> {
    let mut filter = doc! {"state":"due","$expr":{"$and":[{"$lte":["$next_attempt_at","$$NOW"]},
    {"$lte":[{"$ifNull":["$lease_until",bson::DateTime::from_millis(0)]},"$$NOW"]}]}};
    if let Some(id) = id {
        filter.insert("_id", id);
    }
    Ok(db.collection::<Release>(RELEASES).find_one_and_update(filter,vec![doc! {"$set":{"claim_id":Uuid::new_v4().to_string(),
        "lease_until":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":120_i64}},"updated_at":"$$NOW",
        "revision":{"$add":["$revision",1_i64]},"attempt_count":{"$add":["$attempt_count",1_i64]}}}])
        .sort(doc! {"next_attempt_at":1,"_id":1}).return_document(ReturnDocument::After).await?)
}

async fn admit(db: &Database, release: &Release) -> AppResult<()> {
    if db
        .collection::<Document>(RELEASES)
        .find_one(claim_fence(release))
        .await?
        .is_none()
    {
        return Err(conflict());
    }
    if live_reference(db, release).await? {
        return Err(conflict());
    }
    Ok(())
}

async fn transition(
    db: &Database,
    release: &Release,
    state: ReleaseState,
    code: &str,
    observed: Option<i64>,
) -> AppResult<bool> {
    let db = db.clone();
    let release = release.clone();
    let code = code.to_owned();
    let mut session = db.client().start_session().await?;
    let log_id = release.id.clone();
    let log_channel = release.channel_id.clone();
    let log_code = code.clone();
    let changed=session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            let now=Utc::now();
            let mut set=doc! {"state":bson::to_bson(&state).map_err(|_| conflict())?,"last_error":&code,"observed_version":observed,
                "claim_id":bson::Bson::Null,"lease_until":bson::Bson::Null,"updated_at":bson::DateTime::from_chrono(now)};
            if state==ReleaseState::Due {set.insert("next_attempt_at",bson::DateTime::from_chrono(now+Duration::seconds(backoff(release.attempt_count))));}
            if state==ReleaseState::Settled {set.insert("settled_at",bson::DateTime::from_chrono(now));set.insert("resolution",&code);}
            if db.collection::<Document>(RELEASES).update_one(claim_fence(&release),doc! {"$set":set,"$inc":{"revision":1_i64}}).session(&mut *session).await?.matched_count!=1 {return Ok(false);}
            if state!=ReleaseState::Due {
                audit(&db,session,&release.owner_user_id,if state==ReleaseState::Settled {"nyxbot_gateway_release_settled"} else {"nyxbot_gateway_release_review_required"},
                    json!({"obligation_id":release.id,"channel_id":release.channel_id,"reason":code})).await?;
            } Ok(true)
        }).await;transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)?;
    if changed && state == ReleaseState::ReviewRequired {
        tracing::error!(obligation_id=%log_id,channel_id=%log_channel,reason=log_code,"Gateway release requires review");
    }
    Ok(changed)
}

fn backoff(attempt: i64) -> i64 {
    if attempt >= 8 {
        3600
    } else {
        30_i64 << (attempt.max(1) - 1)
    }
}

async fn destination(state: &AppState, release: &Release) -> AppResult<bool> {
    let row =
        match super::assistant_service::resolve_admin_service_by_slug(&state.db, "cmaeg").await {
            Ok(row) => row,
            Err(AppError::Internal(_)) => return Ok(false),
            Err(_) => return Err(AppError::OAuthAuthorityUnavailable),
        };
    Ok(row.id == release.gateway_service_id
        && row.base_url.trim_end_matches('/') == release.gateway_base_url
        && state.config.jwt_issuer == release.creator_issuer)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Observation {
    Missing,
    Matching { channel_id: String, version: i64 },
    Changed { version: Option<i64> },
    Malformed,
    DestinationChanged,
    LiveReference,
    OwnerUnavailable,
}

impl Observation {
    fn status(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Matching { .. } => "matching",
            Self::Changed { .. } => "changed",
            Self::Malformed => "malformed",
            Self::DestinationChanged => "destination_changed",
            Self::LiveReference => "live_reference",
            Self::OwnerUnavailable => "owner_unavailable",
        }
    }
}

fn remote_identity(release: &Release, body: &Value) -> Observation {
    // Only record-id recovery of an unknown channel may capture a first version.
    // A known legacy channel without acknowledged version proof stays in review.
    if release.gateway_channel_id.is_some() && release.expected_version.is_none_or(|v| v <= 0) {
        return Observation::Malformed;
    }
    let version = body["version"].as_i64().filter(|v| *v > 0);
    let channel = body["channel_id"].as_str().filter(|v| valid_channel_id(v));
    if release.gateway_channel_id.is_none()
        && !release
            .record_id
            .as_ref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_some_and(|id| release.binding_id == format!("bnd_{}", id.simple()))
    {
        return Observation::Malformed;
    }
    let definition = &body["definition"];
    let sources = definition["sources"].as_array();
    let source_matches = sources.is_some_and(|sources| {
        sources.iter().any(|source| {
            source["type"] == "nyxid_relay"
                && source["issuer"] == release.creator_issuer
                && source["key_id"] == release.route_api_key_id
                && source["platform"] == release.platform
        })
    });
    let owner = &definition["owner"]["subject"];
    if release
        .record_id
        .as_deref()
        .is_some_and(|id| body["record_id"].as_str() != Some(id))
        || body["provider"]["slug"] != "nyxbot"
        || body["provider"]["binding_id"] != release.binding_id
        || definition["owner"]["kind"] != "human"
        || owner != &json!(release.owner_user_id)
        || !source_matches
    {
        return Observation::Malformed;
    }
    let (Some(channel), Some(version)) = (channel, version) else {
        return Observation::Malformed;
    };
    if release
        .gateway_channel_id
        .as_deref()
        .is_some_and(|id| id != channel)
        || release.expected_version.is_some_and(|v| v != version)
    {
        return Observation::Changed {
            version: Some(version),
        };
    }
    Observation::Matching {
        channel_id: channel.into(),
        version,
    }
}

pub(crate) async fn observe(state: &AppState, release: &Release) -> AppResult<Observation> {
    if !destination(state, release).await? {
        return Ok(Observation::DestinationChanged);
    }
    if live_reference(&state.db, release).await? {
        return Ok(Observation::LiveReference);
    }
    if state
        .db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":&release.owner_user_id,"is_active":true})
        .await
        .map_err(|_| AppError::OAuthAuthorityUnavailable)?
        .is_none()
    {
        return Ok(Observation::OwnerUnavailable);
    }
    let bearer = gateway::creator_bearer(state, &release.owner_user_id)?;
    let path = if let Some(record) = &release.record_id {
        format!("/channel-records/{}", encoded(record))
    } else if let Some(channel) = &release.gateway_channel_id {
        format!("/channels/{}", encoded(channel))
    } else {
        return Ok(Observation::Malformed);
    };
    let response = match gateway::gateway_call_at_base(
        &release.gateway_base_url,
        reqwest::Method::GET,
        &path,
        &bearer,
        None,
        None,
    )
    .await
    {
        Err(AppError::ValidationError(code)) if code == "gateway_invalid_response" => {
            return Ok(Observation::Malformed);
        }
        result => result?,
    };
    match response.status {
        404 => Ok(Observation::Missing),
        200 => Ok(remote_identity(release, &response.body)),
        401 | 403 => Err(AppError::Forbidden("gateway_authority_refused".into())),
        _ => Err(AppError::OAuthAuthorityUnavailable),
    }
}

pub(crate) async fn execute(state: &AppState, mut release: Release) -> AppResult<bool> {
    let observation = match observe(state, &release).await {
        Ok(observation) => observation,
        Err(error) => {
            let auth = matches!(error, AppError::Forbidden(_));
            let old = release
                .due_at
                .is_some_and(|t| Utc::now() - t >= Duration::hours(UNCERTAIN_HOURS));
            transition(
                &state.db,
                &release,
                if auth && old {
                    ReleaseState::ReviewRequired
                } else {
                    ReleaseState::Due
                },
                if auth {
                    "gateway_authority_refused"
                } else {
                    "gateway_unavailable"
                },
                None,
            )
            .await?;
            return Ok(false);
        }
    };
    match observation {
        Observation::Missing => {
            // A known remote identity can be conclusively absent. An uncertain
            // dispatch's 404 is only an observation and never proves no create.
            if release.gateway_channel_id.is_some() {
                return transition(&state.db, &release, ReleaseState::Settled, "missing", None)
                    .await;
            }
            let expired = release
                .dispatch_started_at
                .is_some_and(|t| Utc::now() - t >= Duration::hours(UNCERTAIN_HOURS));
            transition(
                &state.db,
                &release,
                if expired {
                    ReleaseState::ReviewRequired
                } else {
                    ReleaseState::Due
                },
                if expired {
                    "create_outcome_unconfirmed"
                } else {
                    "create_not_observed"
                },
                None,
            )
            .await?;
            Ok(false)
        }
        Observation::Matching {
            channel_id,
            version,
        } => {
            if release.gateway_channel_id.is_none() {
                let changed=state.db.collection::<Release>(RELEASES).find_one_and_update(claim_fence(&release),doc! {
                    "$set":{"gateway_channel_id":&channel_id,"expected_version":version},"$inc":{"revision":1_i64}})
                    .return_document(ReturnDocument::After).await?;
                let Some(updated) = changed else {
                    return Ok(false);
                };
                release = updated;
            }
            admit(&state.db, &release).await?;
            if !destination(state, &release).await? {
                transition(
                    &state.db,
                    &release,
                    ReleaseState::ReviewRequired,
                    "gateway_destination_changed",
                    None,
                )
                .await?;
                return Ok(false);
            }
            let bearer = gateway::creator_bearer(state, &release.owner_user_id)?;
            let response = gateway::gateway_call_at_base(
                &release.gateway_base_url,
                reqwest::Method::DELETE,
                &format!(
                    "/channels/{}?expected_version={}",
                    encoded(&channel_id),
                    release.expected_version.ok_or_else(conflict)?
                ),
                &bearer,
                None,
                None,
            )
            .await;
            match response {
                Ok(r) if matches!(r.status, 200 | 204 | 404) => {
                    transition(
                        &state.db,
                        &release,
                        ReleaseState::Settled,
                        "released",
                        Some(version),
                    )
                    .await
                }
                Ok(r) if r.status == 409 => {
                    transition(
                        &state.db,
                        &release,
                        ReleaseState::ReviewRequired,
                        "remote_version_changed",
                        None,
                    )
                    .await?;
                    Ok(false)
                }
                result => {
                    let refused = result.as_ref().is_ok_and(|r| matches!(r.status, 401 | 403));
                    let expired = release
                        .due_at
                        .is_some_and(|at| Utc::now() - at >= Duration::hours(UNCERTAIN_HOURS));
                    transition(
                        &state.db,
                        &release,
                        if refused && expired {
                            ReleaseState::ReviewRequired
                        } else {
                            ReleaseState::Due
                        },
                        if refused {
                            "gateway_authority_refused"
                        } else {
                            "release_unconfirmed"
                        },
                        None,
                    )
                    .await?;
                    Ok(false)
                }
            }
        }
        other => {
            let (code, version) = match other {
                Observation::Changed { version } => ("remote_version_changed", version),
                Observation::LiveReference => ("live_reference", None),
                Observation::DestinationChanged => ("gateway_destination_changed", None),
                Observation::OwnerUnavailable => ("owner_authority_unavailable", None),
                _ => ("remote_identity_invalid", None),
            };
            transition(
                &state.db,
                &release,
                ReleaseState::ReviewRequired,
                code,
                version,
            )
            .await?;
            Ok(false)
        }
    }
}

pub(crate) async fn try_inline(state: &AppState, ids: &[String]) -> bool {
    let mut released = true;
    for id in ids {
        match claim(&state.db, Some(id)).await {
            Ok(Some(release)) => released &= execute(state, release).await.unwrap_or(false),
            Ok(None) => {
                // Preservation settles responsibility without confirming release.
                released &= state
                    .db
                    .collection::<Release>(RELEASES)
                    .find_one(doc! {"_id":id,"state":"settled","resolution":{"$in":[
                    "released","missing","cancelled","not_created","confirmed_missing"]}})
                    .await
                    .ok()
                    .flatten()
                    .is_some()
            }
            Err(_) => released = false,
        }
    }
    released
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetiredAcknowledgement {
    NotApplicable,
    Acknowledged,
    ConflictingLiveReference,
    StorageUnavailable,
}

pub(crate) async fn acknowledge_retired(
    state: &AppState,
    raw: &str,
    binding: &str,
) -> RetiredAcknowledgement {
    match retired_proof(state, raw, binding).await {
        Ok(outcome) => outcome,
        Err(_) => RetiredAcknowledgement::StorageUnavailable,
    }
}

async fn retired_proof(
    state: &AppState,
    raw: &str,
    binding: &str,
) -> AppResult<RetiredAcknowledgement> {
    let db = &state.db;
    let hash = crate::crypto::token::hash_token(raw);
    let Some(key) = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"key_hash":hash})
        .await?
    else {
        return Ok(RetiredAcknowledgement::NotApplicable);
    };
    if db
        .collection::<Document>(KEYS)
        .find_one(doc! {"rotation_predecessor_id":&key.id})
        .await?
        .is_some()
    {
        return Ok(RetiredAcknowledgement::NotApplicable);
    }
    if let Some(release) = db
        .collection::<Release>(RELEASES)
        .find_one(
            doc! {"agent_api_key_id":&key.id,"owner_user_id":&key.user_id,
            "binding_id":binding,"state":{"$in":["due","review_required","settled"]}},
        )
        .await?
    {
        return Ok(if live_reference(db, &release).await? {
            RetiredAcknowledgement::ConflictingLiveReference
        } else {
            RetiredAcknowledgement::Acknowledged
        });
    }
    if let Some(outcome) =
        super::platform_entry_gateway_service::acknowledge_retired_binding(db, &key, binding)
            .await?
    {
        return Ok(outcome);
    }
    let cleanup = db.collection::<crate::models::platform_bot_config::PlatformBotCleanup>(
        crate::models::platform_bot_config::CLEANUP_COLLECTION,
    );
    let mut task=cleanup.find_one(doc! {"kind":"gateway","owner_user_id":&key.user_id,"agent_api_key_id":&key.id,"binding_id":binding}).await?;
    if task.is_none() && let Some(retired)=db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {
        "agent_api_key_id":&key.id,"user_id":&key.user_id,"binding_id":binding,"status":{"$nin":["pending","active"]},"last_error":"platform_conversion"}).await? {
        task=cleanup.find_one(doc! {"kind":"gateway","owner_user_id":&key.user_id,"resource_id":&retired.id,"agent_api_key_id":bson::Bson::Null}).await?;
    }
    if let Some(task) = task {
        let proof = super::platform_bot_config_service::retired_gateway_proof(db, &task).await?;
        if proof
            .as_ref()
            .is_some_and(|(agent, bound)| agent == &key.id && bound == binding)
        {
            let live=db.collection::<Document>(CHANNELS).find_one(doc! {"status":{"$in":["pending","active"]},"$or":[
                {"agent_api_key_id":&key.id},{"pending_agent_api_key_id":&key.id},{"binding_id":binding}]}).await?.is_some();
            return Ok(if live {
                RetiredAcknowledgement::ConflictingLiveReference
            } else {
                RetiredAcknowledgement::Acknowledged
            });
        }
    }
    Ok(RetiredAcknowledgement::NotApplicable)
}

pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    let stale: Vec<Release> = state
        .db
        .collection(RELEASES)
        .find(doc! {"state":"provisional","$expr":{"$lte":["$provisional_expires_at","$$NOW"]}})
        .sort(doc! {"provisional_expires_at":1,"_id":1})
        .limit(10)
        .await?
        .try_collect()
        .await?;
    let remaining = 10_usize.saturating_sub(stale.len());
    for row in stale {
        retire_attempt(state, &row.id, ReleaseReason::StaleSetup).await?;
    }
    let results: Vec<_> = futures::stream::iter(0..remaining)
        .map(|_| async {
            match claim(&state.db, None).await? {
                Some(row) => execute(state, row).await.map(|_| ()),
                None => Ok(()),
            }
        })
        .buffer_unordered(2)
        .collect()
        .await;
    for result in results {
        result?;
    }
    reconcile_step(state).await
}

pub(crate) fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if sweep(&state).await.is_err() {
                tracing::debug!("Gateway release maintenance deferred");
            }
        }
    });
}

pub(crate) async fn reconcile_step(state: &AppState) -> AppResult<()> {
    let db = &state.db;
    db.collection::<Document>(MIGRATIONS).update_one(doc! {"_id":RECONCILE_ID},doc! {"$setOnInsert":{"last_id":bson::Bson::Null,"claim_id":bson::Bson::Null,
        "lease_until":bson::Bson::Null,"pass_started_at":bson::Bson::Null,"pass_completed_at":bson::Bson::Null}}).upsert(true).await?;
    let claim_id = Uuid::new_v4().to_string();
    let cursor=db.collection::<ReleaseReconcileCursor>(MIGRATIONS).find_one_and_update(doc! {"_id":RECONCILE_ID,"$expr":{"$and":[
        {"$lte":[{"$ifNull":["$lease_until",bson::DateTime::from_millis(0)]},"$$NOW"]},
        {"$or":[{"$ne":["$last_id",bson::Bson::Null]},{"$lte":[{"$ifNull":["$pass_completed_at",bson::DateTime::from_millis(0)]},
            {"$dateSubtract":{"startDate":"$$NOW","unit":"hour","amount":1_i64}}]}]}]}},
        vec![doc! {"$set":{"claim_id":&claim_id,"lease_until":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":120_i64}},
            "pass_started_at":{"$cond":[{"$eq":["$last_id",bson::Bson::Null]},"$$NOW","$pass_started_at"]}}}])
        .return_document(ReturnDocument::After).await?;
    let Some(cursor) = cursor else {
        return Ok(());
    };
    let filter = doc! {"_id":{"$gt":cursor.last_id.as_deref().unwrap_or("")},"status":{"$nin":["pending","active"]},
    "$or":[{"gateway_channel_id":{"$type":"string"}},{"gateway_record_id":{"$type":"string"}}]};
    let rows: Vec<NyxbotChannel> = db
        .collection(CHANNELS)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for candidate in &rows {
        let state = state.clone();
        let candidate = candidate.clone();
        let db = state.db.clone();
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run2(async move |session| {
                let work: AppResult<_> = async {
                    if let Some(row) = db
                        .collection::<NyxbotChannel>(CHANNELS)
                        .find_one(doc! {"_id":&candidate.id,"status":{"$nin":["pending","active"]}})
                        .session(&mut *session)
                        .await?
                    {
                        if row.gateway_channel_id.is_none() && row.gateway_record_id.is_none() {
                            return Ok(());
                        }
                        let mut check =
                            new_release(&state, &row, row.gateway_record_id.clone()).await?;
                        check.gateway_channel_id = row.gateway_channel_id.clone();
                        check.binding_id = row.binding_id.clone().unwrap_or_default();
                        if db
                            .collection::<Document>(CHANNELS)
                            .find_one(live_filter(&check))
                            .session(&mut *session)
                            .await?
                            .is_none()
                        {
                            retire_in_session(&state, &row, ReleaseReason::Reconcile, session)
                                .await?;
                        }
                    }
                    Ok(())
                }
                .await;
                transactions::transaction_result(work)
            })
            .await
            .map_err(transactions::map_transaction_error)?;
    }
    let state = state.clone();
    let db = state.db.clone();
    let last = rows.last().map(|r| r.id.clone());
    let complete = rows.len() < 100;
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            let mut set=doc! {"last_id":if complete {None} else {last.clone()},"claim_id":bson::Bson::Null,"lease_until":bson::Bson::Null};
            if complete {set.insert("pass_completed_at",bson::DateTime::now());}
            if db.collection::<Document>(MIGRATIONS).update_one(doc! {"_id":RECONCILE_ID,"claim_id":&claim_id,"$expr":{"$gt":["$lease_until","$$NOW"]}},
                doc! {"$set":set}).session(&mut *session).await?.matched_count!=1 {return Err(conflict());}
            if complete {audit(&db,session,"system","nyxbot_gateway_release_reconcile_pass_completed",json!({"pass_started_at":cursor.pass_started_at})).await?;}
            Ok(())
        }).await;transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

pub(crate) async fn not_created(db: &Database, release: &Release) -> AppResult<()> {
    let db = db.clone();
    let release = release.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            if db.collection::<Release>(RELEASES).update_one(setup_fence(&release),doc! {"$set":{"state":"settled","resolution":"not_created","settled_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64}})
                .session(&mut *session).await?.matched_count!=1 {return Err(conflict());}
            audit(&db,session,&release.owner_user_id,"nyxbot_gateway_release_settled",json!({"obligation_id":release.id,"channel_id":release.channel_id,"resolution":"not_created"})).await
        }).await;transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

pub(crate) async fn fallback(
    state: &AppState,
    row: &NyxbotChannel,
    next_key: &str,
) -> AppResult<Vec<String>> {
    let state = state.clone();
    let db = state.db.clone();
    let row = row.clone();
    let next_key = next_key.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            let current=db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {"_id":&row.id,"status":"pending","transport":"gateway","route_api_key_id":&row.route_api_key_id})
                .session(&mut *session).await?.ok_or_else(conflict)?;
            let ids=retire_in_session(&state,&current,ReleaseReason::Fallback,session).await?;
            for key in [Some(current.route_api_key_id.as_str()),current.pending_route_api_key_id.as_deref()].into_iter().flatten() {
                super::channel_routing_service::retire_key_routes_in_session(&db,bot_owner(&current),&current.channel_bot_id,key,session).await?;
                revoke_in_session(&db,bot_owner(&current),key,session).await?;
            }
            if let Some(key)=current.agent_api_key_id.as_deref() {revoke_in_session(&db,&current.user_id,key,session).await?;}
            super::channel_routing_service::fence_route_key(&db,&next_key,session).await?;
            db.collection::<Document>(CHANNELS).update_one(doc! {"_id":&current.id,"status":"pending"},doc! {
                "$set":{"transport":"direct","route_api_key_id":&next_key,"gateway_no_progress":false,"gateway_fallback_at":bson::DateTime::now()},
                "$unset":{"gateway_setup_release_id":"","agent_api_key_id":"","agent_key_ciphertext":"","gateway_channel_id":"","gateway_record_id":"","gateway_version":"","binding_id":"","gateway_bot_id":""}})
                .session(&mut *session).await?;
            Ok(ids)
        }).await;transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

pub(crate) async fn retire_pending(state: &AppState, row: &NyxbotChannel) -> AppResult<()> {
    if let Some(id) = &row.gateway_setup_release_id {
        return retire_attempt(state, id, ReleaseReason::StaleSetup).await;
    }
    let db = state.db.clone();
    let row = row.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            if db.collection::<Document>(CHANNELS).update_one(doc! {"_id":&row.id,"transport":&row.transport,"status":"active",
                "pending_agent_api_key_id":&row.pending_agent_api_key_id,"pending_route_api_key_id":&row.pending_route_api_key_id,"gateway_setup_release_id":bson::Bson::Null},
                doc! {"$unset":{"pending_agent_api_key_id":"","pending_route_api_key_id":""}}).session(&mut *session).await?.matched_count!=1 {return Ok(());}
            if let Some(key)=row.pending_agent_api_key_id.as_deref() {revoke_in_session(&db,&row.user_id,key,session).await?;}
            if let Some(key)=row.pending_route_api_key_id.as_deref() {revoke_in_session(&db,bot_owner(&row),key,session).await?;}
            Ok(())
        }).await;transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

pub(crate) async fn list(
    db: &Database,
    after: Option<&str>,
    states: Vec<ReleaseState>,
    limit: i64,
) -> AppResult<Vec<Release>> {
    Ok(db.collection(RELEASES).find(doc! {"_id":{"$gt":after.unwrap_or("")},"state":{"$in":bson::to_bson(&states).map_err(|_| conflict())?}})
        .sort(doc! {"_id":1}).limit(limit.clamp(1,100)).await?.try_collect().await?)
}

pub(crate) async fn get(db: &Database, id: &str) -> AppResult<Release> {
    db.collection(RELEASES)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(|| AppError::NotFound("Gateway release not found".into()))
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Resolution {
    RetryOriginal,
    PreserveChanged,
    ConfirmMissing,
}

pub(crate) async fn resolve(
    state: &AppState,
    actor: &str,
    id: &str,
    revision: i64,
    inspected: Observation,
    action: Resolution,
    dispatch_drained: bool,
) -> AppResult<()> {
    let release = get(&state.db, id).await?;
    if release.revision != revision || release.state != ReleaseState::ReviewRequired {
        return Err(conflict());
    }
    let fresh = observe(state, &release).await?;
    if fresh != inspected {
        return Err(conflict());
    }
    let (next, resolution) = match action {
        Resolution::RetryOriginal
            if matches!(fresh, Observation::Matching { .. })
                || (release.gateway_channel_id.is_none()
                    && matches!(fresh, Observation::Missing)) =>
        {
            (ReleaseState::Due, "retry_original")
        }
        Resolution::PreserveChanged
            if !matches!(fresh, Observation::Matching { .. } | Observation::Missing) =>
        {
            (ReleaseState::Settled, "preserved")
        }
        Resolution::ConfirmMissing if fresh == Observation::Missing => {
            if release.gateway_channel_id.is_none()
                && !(dispatch_drained
                    && release
                        .dispatch_started_at
                        .is_some_and(|t| Utc::now() - t >= Duration::hours(UNCERTAIN_HOURS)))
            {
                return Err(AppError::ValidationError(
                    "Uncertain creation requires 24 hours and drained-dispatch attestation".into(),
                ));
            }
            (ReleaseState::Settled, "confirmed_missing")
        }
        _ => {
            return Err(AppError::ValidationError(
                "Resolution does not match the inspected identity".into(),
            ));
        }
    };
    let observation_status = fresh.status();
    let db = state.db.clone();
    let actor = actor.to_owned();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db, actor, id, release), |session, (db, actor, id, release)| Box::pin(async move {
        let db = &*db;
        let actor = actor.as_str();
        let id = id.as_str();
        let work:AppResult<_>=Box::pin(async {
            super::platform_bot_config_service::fence_admin(db,actor,session).await?;
            let mut set=doc! {"state":bson::to_bson(&next).map_err(|_| conflict())?,"resolution":resolution,"reviewed_by":actor,
                "claim_id":bson::Bson::Null,"lease_until":bson::Bson::Null,"next_attempt_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()};
            if next==ReleaseState::Settled {set.insert("settled_at",bson::DateTime::now());}
            if db.collection::<Release>(RELEASES).update_one(doc! {"_id":id,"state":"review_required","revision":revision},doc! {"$set":set,"$inc":{"revision":1_i64}})
                .session(&mut *session).await?.matched_count!=1 {return Err(conflict());}
            audit(db,session,actor,"nyxbot_gateway_release_reviewed",json!({"obligation_id":id,"channel_id":release.channel_id,"resolution":resolution,"observation_status":observation_status})).await?;
            if next==ReleaseState::Settled {audit(db,session,actor,"nyxbot_gateway_release_settled",json!({"obligation_id":id,"resolution":resolution,"observation_status":observation_status})).await?;}
            Ok(())
        }).await;transactions::transaction_result(work)
    })).await.map_err(transactions::map_transaction_error)
}

pub(crate) async fn continue_setup(state: &AppState, release: &Release) -> AppResult<()> {
    if state
        .db
        .collection::<Release>(RELEASES)
        .find_one(setup_fence(release))
        .await?
        .is_none()
        || !destination(state, release).await?
    {
        return Err(conflict());
    }
    Ok(())
}
