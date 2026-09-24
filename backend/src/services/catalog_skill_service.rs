use std::collections::HashSet;

use chrono::Utc;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{api_key_mutation_service as transactions, curation_grant_service as grants};
use crate::{
    errors::{AppError, AppResult},
    models::{
        catalog_skill_revision::{
            COLLECTION_NAME as HISTORY, CatalogSkillOperation, CatalogSkillRevision, OPERATIONS,
            SkillReference, SkillState,
        },
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        service_account::{COLLECTION_NAME as ACCOUNTS, ServiceAccount},
        service_account_token::{COLLECTION_NAME as TOKENS, ServiceAccountToken},
    },
};

#[cfg(test)]
tokio::task_local! {
    pub(crate) static COLLISION: transactions::TransactionCollisionHook;
    pub(crate) static AUTHORITY_PAUSE: (std::sync::Arc<tokio::sync::Barrier>, std::sync::Arc<tokio::sync::Barrier>, std::sync::Arc<std::sync::atomic::AtomicBool>);
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillUpdate {
    pub recommended_skills: Option<Vec<String>>,
    pub recommended_skill_refs: Option<Vec<SkillReference>>,
    #[serde(default)]
    pub clear_refs: bool,
}

impl SkillUpdate {
    pub fn is_present(&self) -> bool {
        self.recommended_skills.is_some()
            || self.recommended_skill_refs.is_some()
            || self.clear_refs
    }
}

#[derive(Clone, Debug)]
pub enum SkillActor {
    Human {
        id: String,
    },
    Curation {
        id: String,
        scope: String,
        token_jti: String,
    },
}

impl SkillActor {
    fn identity(&self) -> (&str, &str) {
        match self {
            Self::Human { id } => ("human", id),
            Self::Curation { id, .. } => ("service_account", id),
        }
    }
}

pub struct SkillCommit {
    pub service: DownstreamService,
    pub state: SkillState,
    pub revision: i64,
    pub replayed: bool,
    pub changed: bool,
    pub mutated: bool,
}

pub fn state(service: &DownstreamService) -> SkillState {
    SkillState {
        recommended_skills: service.recommended_skills.clone(),
        recommended_skill_refs: service.recommended_skill_refs.clone(),
    }
}

pub fn manifest_digest(state: &SkillState) -> String {
    let bytes =
        serde_json::to_vec(&("nyxid.skills-manifest.v1", state)).expect("skill state serializes");
    format!("v1:{}", hex::encode(Sha256::digest(bytes)))
}

fn bounded_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn validate_pin(source: &str, id: &str, name: &str, version: &str, sha256: &str) -> AppResult<()> {
    let exact_version = regex::Regex::new(r"^[0-9]+(?:\.[0-9]+){1,2}(?:-[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?(?:\+[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?$").expect("static regex");
    if !bounded_text(source, 256)
        || !bounded_text(id, 256)
        || !bounded_text(name, 256)
        || version.len() > 128
        || !exact_version.is_match(version)
        || sha256.len() != 64
        || !sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(AppError::ValidationError("Skill refs require bounded source/id/name, an exact numeric version, and lowercase SHA-256".into()));
    }
    Ok(())
}

pub fn resolve_update(current: &SkillState, input: &SkillUpdate) -> AppResult<SkillState> {
    if serde_json::to_vec(input)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .len()
        > 65_536
    {
        return Err(AppError::ValidationError(
            "Skill recommendations exceed 64 KiB".into(),
        ));
    }
    if let Some(names) = &input.recommended_skills {
        let unique: HashSet<_> = names.iter().collect();
        if names.len() > 50
            || unique.len() != names.len()
            || names.iter().any(|n| !bounded_text(n, 256))
        {
            return Err(AppError::ValidationError(
                "Supply at most 50 distinct skill names of 1-256 characters".into(),
            ));
        }
    }
    if input.clear_refs && input.recommended_skill_refs.is_some() {
        return Err(AppError::ValidationError(
            "Supply replacement refs or clear_refs, not both".into(),
        ));
    }
    if let Some(refs) = &input.recommended_skill_refs {
        if refs.len() > 50 {
            return Err(AppError::ValidationError(
                "At most 50 skill refs are allowed".into(),
            ));
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for reference in refs {
            validate_pin(
                &reference.source,
                &reference.skill_id,
                &reference.name,
                &reference.version,
                &reference.sha256,
            )?;
            if !ids.insert((&reference.source, &reference.skill_id))
                || !names.insert(&reference.name)
                || reference.dependencies.len() > 50
            {
                return Err(AppError::ValidationError(
                    "Duplicate skill refs or excessive dependency pins".into(),
                ));
            }
            let mut dependencies = HashSet::new();
            for pin in &reference.dependencies {
                validate_pin(
                    &pin.source,
                    &pin.skill_id,
                    &pin.name,
                    &pin.version,
                    &pin.sha256,
                )?;
                if !dependencies.insert((&pin.source, &pin.skill_id)) {
                    return Err(AppError::ValidationError("Duplicate dependency pin".into()));
                }
            }
        }
        let names: Vec<_> = refs.iter().map(|r| r.name.clone()).collect();
        if input
            .recommended_skills
            .as_ref()
            .is_some_and(|supplied| supplied != &names)
        {
            return Err(AppError::ValidationError(
                "Skill names must match reference names in order".into(),
            ));
        }
        return Ok(SkillState {
            recommended_skills: Some(names),
            recommended_skill_refs: Some(refs.clone()),
        });
    }
    if current.recommended_skill_refs.is_some()
        && !input.clear_refs
        && input
            .recommended_skills
            .as_ref()
            .is_some_and(|names| Some(names) != current.recommended_skills.as_ref())
    {
        return Err(AppError::ValidationError(
            "Changing advisory names requires replacement refs or explicit clear_refs".into(),
        ));
    }
    Ok(SkillState {
        recommended_skills: input
            .recommended_skills
            .clone()
            .or_else(|| current.recommended_skills.clone()),
        recommended_skill_refs: if input.clear_refs {
            None
        } else {
            current.recommended_skill_refs.clone()
        },
    })
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    for (collection, keys) in [
        (HISTORY, doc! {"service_id": 1, "revision": 1}),
        (
            OPERATIONS,
            doc! {"actor_kind": 1, "actor_id": 1, "request_id": 1},
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
    }
    Ok(())
}

async fn machine_authority(
    db: &Database,
    session: &mut ClientSession,
    actor: &SkillActor,
    service_id: &str,
) -> AppResult<Option<ServiceAccount>> {
    let SkillActor::Curation {
        id,
        scope,
        token_jti,
    } = actor
    else {
        return Ok(None);
    };
    let sa = db
        .collection::<ServiceAccount>(ACCOUNTS)
        .find_one(doc! {"_id": id})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Service account not found".into()))?;
    if sa.purpose == crate::models::service_account::ServiceAccountPurpose::CatalogEditor {
        super::catalog_editor_service::authorize_in_session(
            db,
            session,
            &sa,
            scope,
            grants::WRITE_SCOPE,
        )
        .await?;
    } else {
        grants::require_service(&sa, scope, grants::WRITE_SCOPE, service_id)?;
    }
    let token = db
        .collection::<ServiceAccountToken>(TOKENS)
        .find_one(doc! {"jti": token_jti, "service_account_id": id})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Service account token not found".into()))?;
    if token.revoked
        || token.expires_at <= Utc::now()
        || token.scope != *scope
        || token.credential_generation != sa.credential_generation
    {
        return Err(AppError::Unauthorized(
            "Invalid service account token".into(),
        ));
    }
    Ok(Some(sa))
}

async fn reserve_budget(
    db: &Database,
    session: &mut ClientSession,
    sa: &ServiceAccount,
) -> AppResult<String> {
    if sa.purpose == crate::models::service_account::ServiceAccountPurpose::CatalogEditor {
        super::catalog_editor_service::fence_write_in_session(db, session, sa).await?;
        let now = Utc::now();
        let current = db
            .collection::<Document>(ACCOUNTS)
            .find_one(doc! {"_id": &sa.id})
            .projection(doc! {"catalog_editor_write_window": 1, "catalog_editor_writes_used": 1})
            .session(&mut *session)
            .await?
            .ok_or_else(|| AppError::Unauthorized("Service account not found".into()))?;
        let started = current
            .get_datetime("catalog_editor_write_window")
            .ok()
            .copied();
        let reset = started.is_none_or(|started| {
            now.signed_duration_since(started.to_chrono())
                .num_milliseconds()
                >= 1000
        });
        let used = if reset {
            0
        } else {
            current.get_i64("catalog_editor_writes_used").unwrap_or(0)
        };
        let limit = sa.rate_limit_override.unwrap_or(60).min(i64::MAX as u64) as i64;
        if used >= limit {
            return Err(AppError::RateLimited);
        }
        let window = if reset {
            bson::DateTime::from_chrono(now)
        } else {
            started.unwrap()
        };
        let result = db.collection::<Document>(ACCOUNTS).update_one(
            doc! {"_id": &sa.id, "purpose": "catalog_editor", "platform_protected": true, "is_active": true, "$expr": {"$eq": [{"$ifNull": ["$credential_generation", 0_i64]}, sa.credential_generation]}},
            doc! {"$inc": {"catalog_editor_write_sequence": 1_i64}, "$set": {"catalog_editor_write_window": window, "catalog_editor_writes_used": used + 1}},
        ).session(session).await?;
        if result.matched_count != 1 {
            return Err(AppError::Conflict(
                "Catalog editor authority changed".into(),
            ));
        }
        return Ok(format!("catalog-editor:{}", sa.id));
    }
    let grant = grants::live_grant(sa)?;
    let now = Utc::now();
    let reset = now
        .signed_duration_since(grant.window_started_at)
        .num_seconds()
        >= grant.window_seconds;
    if !reset && grant.writes_used >= grant.max_writes {
        return Err(AppError::RateLimited);
    }
    let used = if reset { 1 } else { grant.writes_used + 1 };
    let started = if reset { now } else { grant.window_started_at };
    let result = db.collection::<ServiceAccount>(ACCOUNTS).update_one(
        doc! {"_id": &sa.id, "curation_grant.id": &grant.id, "curation_grant.writes_used": grant.writes_used},
        doc! {"$set": {"curation_grant.writes_used": used, "curation_grant.window_started_at": bson::DateTime::from_chrono(started)}})
        .session(session).await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict("Curation grant changed".into()));
    }
    Ok(grant.id.clone())
}

fn duplicate_key(error: &AppError) -> bool {
    match error {
        AppError::DatabaseError(error) => {
            matches!(error.kind.as_ref(),
            mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(e)) if e.code == 11000)
                || matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(e) if e.code == 11000)
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn commit(
    db: &Database,
    service_id: &str,
    actor: &SkillActor,
    input: &SkillUpdate,
    base_revision: i64,
    request_id: &str,
    metadata: &Document,
    identity_filter: &Document,
    restore_revision: Option<i64>,
    fingerprint_input: Option<&serde_json::Value>,
) -> AppResult<SkillCommit> {
    // Duplicate receipt keys do not have MongoDB's transient-transaction label.
    // A fresh transaction rechecks current authority and resolves the winner.
    for attempt in 0..2 {
        let result = commit_once(
            db,
            service_id,
            actor,
            input,
            base_revision,
            request_id,
            metadata,
            identity_filter,
            restore_revision,
            fingerprint_input,
        )
        .await;
        if result.as_ref().err().is_some_and(duplicate_key) {
            if attempt == 0 {
                continue;
            }
            return Err(AppError::Conflict(
                "Concurrent request identity conflict; retry with the same request_id".into(),
            ));
        }
        return result;
    }
    unreachable!()
}

/// Metadata fields and identity filters are supplied only by the validated human handler.
/// Restore resolves a historical state inside the same transaction as the current CAS.
#[allow(clippy::too_many_arguments)]
async fn commit_once(
    db: &Database,
    service_id: &str,
    actor: &SkillActor,
    input: &SkillUpdate,
    base_revision: i64,
    request_id: &str,
    metadata: &Document,
    identity_filter: &Document,
    restore_revision: Option<i64>,
    fingerprint_input: Option<&serde_json::Value>,
) -> AppResult<SkillCommit> {
    if base_revision < 0
        || Uuid::parse_str(request_id).is_err()
        || restore_revision.is_some_and(|r| r < 0)
    {
        return Err(AppError::ValidationError(
            "A nonnegative revision and UUID request_id are required".into(),
        ));
    }
    if matches!(actor, SkillActor::Curation { .. })
        && (!metadata.is_empty() || !identity_filter.is_empty())
    {
        return Err(AppError::Forbidden(
            "Curation cannot update service metadata".into(),
        ));
    }
    // Timestamps and the reconciliation marker are server-generated for each
    // attempt; fingerprint their validated inputs, not the attempt clock.
    let mut fingerprint_metadata = metadata.clone();
    fingerprint_metadata.remove("updated_at");
    fingerprint_metadata.remove(crate::models::catalog_identity_reconciliation::FIELD_NAME);
    let fingerprint_metadata_value = serde_json::to_value(&fingerprint_metadata)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            service_id,
            base_revision,
            input,
            fingerprint_input.unwrap_or(&fingerprint_metadata_value),
            restore_revision,
        ))
        .map_err(|e| AppError::Internal(e.to_string()))?,
    ));
    let (kind, actor_id) = actor.identity();
    let operation_filter =
        doc! {"actor_kind": kind, "actor_id": actor_id, "request_id": request_id};
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let service_id = service_id.to_owned();
    let actor = actor.clone();
    let input = input.clone();
    let metadata = metadata.clone();
    let identity_filter = identity_filter.clone();
    let request_id = request_id.to_owned();
    let kind = kind.to_owned();
    let actor_id = actor_id.to_owned();
    session.start_transaction().and_run2(async move |session| {
        let db = &db;
        let service_id = service_id.as_str();
        let actor = &actor;
        let input = &input;
        let request_id = request_id.as_str();
        let kind = kind.as_str();
        let actor_id = actor_id.as_str();
        #[cfg(test)]
        if let Ok(hook) = COLLISION.try_with(Clone::clone) { hook.begin_attempt(); }

        let result: AppResult<SkillCommit> = async {
            let sa = machine_authority(db, session, actor, service_id).await?;
            let current = db.collection::<DownstreamService>(SERVICES).find_one(doc! {"_id": service_id}).session(&mut *session).await?
                .ok_or_else(|| AppError::NotFound("Service not found".into()))?;
            if let Some(receipt) = db.collection::<CatalogSkillOperation>(OPERATIONS).find_one(operation_filter.clone()).session(&mut *session).await? {
                if receipt.fingerprint != fingerprint || receipt.service_id != service_id {
                    return Err(AppError::Conflict("request_id was already committed with different inputs".into()));
                }
                return Ok(SkillCommit { service: current, state: receipt.state, revision: receipt.revision, replayed: true, changed: false, mutated: false });
            }
            #[cfg(test)]
            if let Ok(hook) = COLLISION.try_with(Clone::clone) { hook.after_reads().await; }
            #[cfg(test)]
            if let Ok((reached, resume, paused)) = AUTHORITY_PAUSE.try_with(Clone::clone)
                && !paused.swap(true, std::sync::atomic::Ordering::SeqCst) {
                reached.wait().await;
                resume.wait().await;
            }
            if current.skills_revision != base_revision { return Err(AppError::Conflict(format!("Skills revision changed; expected {base_revision}, current {}", current.skills_revision))); }
            let previous = state(&current);
            let desired = if let Some(revision) = restore_revision {
                let row = db.collection::<CatalogSkillRevision>(HISTORY).find_one(doc! {"service_id": service_id, "$or": [{"revision": revision}, {"previous_revision": revision}]}).session(&mut *session).await?
                    .ok_or_else(|| AppError::NotFound("Skill revision not found".into()))?;
                if row.revision == revision { row.current } else { row.previous }
            } else { resolve_update(&previous, input)? };
            let changed = desired != previous;
            let current_doc = bson::to_document(&current).map_err(|e| AppError::Internal(e.to_string()))?;
            let metadata_changed = fingerprint_metadata.iter().any(|(k,v)| match (current_doc.get(k), v) {
                (None | Some(bson::Bson::Null), bson::Bson::Null) => false,
                (current, desired) => current != Some(desired),
            });
            if !changed && !metadata_changed {
                return Ok(SkillCommit { revision: current.skills_revision, service: current, state: desired, replayed: false, changed: false, mutated: false });
            }
            if changed && let SkillActor::Curation { id, token_jti, .. } = actor {
                let fenced = db.collection::<ServiceAccountToken>(TOKENS).update_one(
                    doc! {"service_account_id": id, "jti": token_jti, "revoked": false},
                    doc! {"$inc": {"curation_write_fence": 1_i64}}).session(&mut *session).await?;
                if fenced.matched_count != 1 { return Err(AppError::Unauthorized("Service account token revoked".into())); }
            }
            let grant_id = if changed { if let Some(sa) = &sa { Some(reserve_budget(db, session, sa).await?) } else { None } } else { None };
            let revision = current.skills_revision.checked_add(i64::from(changed)).ok_or_else(|| AppError::Conflict("Skill revision exhausted".into()))?;
            let now = Utc::now();
            let mut set = metadata.clone();
            if changed {
                set.insert("recommended_skills", bson::to_bson(&desired.recommended_skills).map_err(|e| AppError::Internal(e.to_string()))?);
                set.insert("recommended_skill_refs", bson::to_bson(&desired.recommended_skill_refs).map_err(|e| AppError::Internal(e.to_string()))?);
                set.insert("skills_revision", revision);
            }
            if !set.contains_key("updated_at") { set.insert("updated_at", bson::DateTime::from_chrono(now)); }
            let mut filter = identity_filter.clone();
            filter.insert("_id", service_id);
            filter.insert("$or", vec![doc! {"skills_revision": base_revision}, if base_revision == 0 { doc! {"skills_revision": {"$exists": false}} } else {doc! {"skills_revision": base_revision}}]);
            let service = db.collection::<DownstreamService>(SERVICES).find_one_and_update(filter,
                doc! {"$set": set, "$unset": {"api_spec_url": ""}}).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("Service changed or has unresolved identity reconciliation; reload before retrying".into()))?;
            if changed {
                db.collection::<CatalogSkillRevision>(HISTORY).insert_one(CatalogSkillRevision {
                    id: Uuid::new_v4().to_string(), service_id: service_id.into(), previous_revision: current.skills_revision,
                    revision, previous, current: desired.clone(), actor_kind: kind.into(), actor_id: actor_id.into(),
                    grant_id, request_id: request_id.into(), fingerprint: fingerprint.clone(), created_at: now,
                }).session(&mut *session).await?;
            }
            db.collection::<CatalogSkillOperation>(OPERATIONS).insert_one(CatalogSkillOperation {
                id: Uuid::new_v4().to_string(), actor_kind: kind.into(), actor_id: actor_id.into(), request_id: request_id.into(),
                service_id: service_id.into(), fingerprint: fingerprint.clone(), revision, state: desired.clone(), created_at: now,
            }).session(&mut *session).await?;
            Ok(SkillCommit { service, state: desired, revision, replayed: false, changed, mutated: true })
        }.await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)
}

pub fn create_fingerprint(body: &serde_json::Value) -> AppResult<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&("catalog-service-create.v1", body))
            .map_err(|e| AppError::Internal(e.to_string()))?,
    )))
}

pub async fn replay_create(
    db: &Database,
    actor_id: &str,
    request_id: &str,
    fingerprint: &str,
) -> AppResult<Option<DownstreamService>> {
    if Uuid::parse_str(request_id).is_err() {
        return Err(AppError::ValidationError(
            "skills_request_id must be a UUID".into(),
        ));
    }
    let receipt = db
        .collection::<CatalogSkillOperation>(OPERATIONS)
        .find_one(doc! {"actor_kind": "human", "actor_id": actor_id, "request_id": request_id})
        .await?;
    let Some(receipt) = receipt else {
        return Ok(None);
    };
    if receipt.fingerprint != fingerprint {
        return Err(AppError::Conflict(
            "request_id was already committed with different inputs".into(),
        ));
    }
    db.collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": receipt.service_id})
        .await?
        .map(Some)
        .ok_or_else(|| AppError::NotFound("Created service no longer exists".into()))
}

pub async fn create(
    db: &Database,
    service: &DownstreamService,
    actor_id: &str,
    input: &SkillUpdate,
    request_id: &str,
    fingerprint: &str,
) -> AppResult<DownstreamService> {
    let desired = resolve_update(&SkillState::default(), input)?;
    if let Some(replay) = replay_create(db, actor_id, request_id, fingerprint).await? {
        return Ok(replay);
    }
    let changed = desired != SkillState::default();
    let mut service = service.clone();
    service.recommended_skills = desired.recommended_skills.clone();
    service.recommended_skill_refs = desired.recommended_skill_refs.clone();
    service.skills_revision = i64::from(changed);
    let mut session = db.client().start_session().await?;
    let created = service.clone();
    let transaction_db = db.clone();
    let transaction_actor = actor_id.to_owned();
    let transaction_request = request_id.to_owned();
    let transaction_fingerprint = fingerprint.to_owned();
    let committed = session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &transaction_db;
            let actor_id = transaction_actor.as_str();
            let request_id = transaction_request.as_str();
            let fingerprint = transaction_fingerprint.as_str();
            #[cfg(test)]
            if let Ok(hook) = COLLISION.try_with(Clone::clone) {
                hook.begin_attempt();
                hook.after_reads().await;
            }

            let result: AppResult<()> = async {
                if let Some(provider_id) = service.provider_config_id.as_deref() {
                    super::provider_link_service::link_in_session(
                        db,
                        provider_id,
                        &service.id,
                        Some(&service),
                        session,
                    )
                    .await?;
                } else {
                    db.collection::<DownstreamService>(SERVICES)
                        .insert_one(&service)
                        .session(&mut *session)
                        .await?;
                }
                if changed {
                    db.collection::<CatalogSkillRevision>(HISTORY)
                        .insert_one(CatalogSkillRevision {
                            id: Uuid::new_v4().to_string(),
                            service_id: service.id.clone(),
                            previous_revision: 0,
                            revision: service.skills_revision,
                            previous: SkillState::default(),
                            current: desired.clone(),
                            actor_kind: "human".into(),
                            actor_id: actor_id.into(),
                            grant_id: None,
                            request_id: request_id.into(),
                            fingerprint: fingerprint.into(),
                            created_at: service.created_at,
                        })
                        .session(&mut *session)
                        .await?;
                }
                db.collection::<CatalogSkillOperation>(OPERATIONS)
                    .insert_one(CatalogSkillOperation {
                        id: Uuid::new_v4().to_string(),
                        actor_kind: "human".into(),
                        actor_id: actor_id.into(),
                        request_id: request_id.into(),
                        service_id: service.id.clone(),
                        fingerprint: fingerprint.into(),
                        revision: service.skills_revision,
                        state: desired.clone(),
                        created_at: service.created_at,
                    })
                    .session(&mut *session)
                    .await?;
                Ok(())
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error);
    if let Err(error) = committed {
        if duplicate_key(&error) {
            if let Some(replay) = replay_create(db, actor_id, request_id, fingerprint).await? {
                return Ok(replay);
            }
            return Err(AppError::Conflict("Catalog service already exists".into()));
        }
        return Err(error);
    }
    db.collection::<DownstreamService>(SERVICES)
        .find_one(doc! { "_id": &created.id })
        .await?
        .ok_or_else(|| AppError::NotFound("Created service no longer exists".into()))
}

pub async fn has_human_operation(
    db: &Database,
    actor_id: &str,
    request_id: &str,
) -> AppResult<bool> {
    Ok(db
        .collection::<CatalogSkillOperation>(OPERATIONS)
        .find_one(doc! {"actor_kind": "human", "actor_id": actor_id, "request_id": request_id})
        .await?
        .is_some())
}

#[cfg(test)]
#[path = "catalog_skill_service_tests.rs"]
mod tests;
