//! Runtime consent authority is independent of the legacy guide generation.
use super::{
    api_key_mutation_service::{map_transaction_error, transaction_result},
    audit_service::AuditActor,
    platform_manager_contract::{
        ManagerAuthority, ManagerRuntimePolicy, PublicPolicySelection, PublicReadinessEvidence,
    },
};
use crate::{
    errors::{AppError, AppResult},
    models::{platform_bot_config::PlatformBotConfig, platform_bot_runtime_controls::*},
};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};

#[async_trait]
pub trait PublicControlReadiness: Send + Sync {
    async fn probe(
        &self,
        db: &Database,
        candidate: &ManagerRuntimePolicy,
    ) -> AppResult<PublicReadinessEvidence>;
    async fn revoke_public_manager(
        &self,
        db: &Database,
        session: &mut ClientSession,
        previous: &ManagerRuntimePolicy,
    ) -> AppResult<()>;
    async fn fence(
        &self,
        db: &Database,
        session: &mut ClientSession,
        candidate: &ManagerRuntimePolicy,
        evidence: &PublicReadinessEvidence,
    ) -> AppResult<()>;
}

#[derive(Clone)]
pub struct RuntimeChange {
    pub public_deployment_digest: Option<String>,
    pub initialize: bool,
    pub expected_profile_generation: Option<i64>,
    pub expected_manager_authority_epoch: i64,
    pub expected_personal_control_revision: i64,
    pub expected_public_policy_revision: i64,
    pub personal_enabled: Option<bool>,
    pub public_enabled: Option<bool>,
    pub emergency_stopped: Option<bool>,
    /// Outer None preserves; Some(None) clears only with explicit public disable.
    pub public_policy: Option<Option<PublicRuntimePolicy>>,
}

fn changed() -> AppError {
    AppError::Conflict("Runtime controls changed; refresh before deciding".into())
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform_config_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"cleanup_pending":1,"_id":1})
                .build(),
        )
        .await?;
    let queue = db.collection::<PlatformPersonalStopEffect>(STOP_EFFECTS);
    queue.create_index(IndexModel::builder().keys(doc! {"active_turn.conversation_id":1,"active_turn.turn_id":1,"active_turn.origin.binding_id":1,"active_turn.origin.binding_generation":1}).options(IndexOptions::builder().unique(true).build()).build()).await?;
    queue
        .create_index(
            IndexModel::builder()
                .keys(doc! {"completed":1,"retry_at":1,"lease_expires_at":1})
                .build(),
        )
        .await?;
    Ok(())
}

pub async fn get(db: &Database, profile_id: &str) -> AppResult<Option<PlatformBotRuntimeControls>> {
    Ok(db
        .collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
        .find_one(doc! {"platform_config_id":profile_id})
        .await?)
}

pub(crate) async fn get_in_session(
    db: &Database,
    session: &mut ClientSession,
    profile_id: &str,
) -> AppResult<Option<PlatformBotRuntimeControls>> {
    Ok(db
        .collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
        .find_one(doc! {"platform_config_id":profile_id})
        .session(session)
        .await?)
}

pub fn policy(row: &PlatformBotRuntimeControls) -> ManagerRuntimePolicy {
    ManagerRuntimePolicy {
        manager: ManagerAuthority {
            profile_id: row.platform_config_id.clone(),
            manager_bot_id: row.manager_bot_id.clone(),
            telegram_bot_id: row.telegram_bot_id,
            manager_authority_epoch: row.manager_authority_epoch,
        },
        personal_enabled: row.personal_enabled,
        public_enabled: row.public_enabled,
        emergency_stopped: row.emergency_stopped,
        personal_control_revision: row.personal_control_revision,
        public_policy_revision: row.public_policy_revision,
        public_policy: row.public_policy.as_ref().map(|p| PublicPolicySelection {
            sponsor_owner_user_id: p.sponsor_owner_user_id.clone(),
            nyxagent_profile: p.nyxagent_profile.clone(),
            total_daily_credits: p.total_daily_credits,
            sender_daily_credits: p.sender_daily_credits,
        }),
    }
}

/// Real conditional writes fence identity, current transport and controls. The
/// guide's enabled/generation, sponsor and public budget are deliberately absent.
pub(crate) async fn fence_manager_in_session(
    db: &Database,
    session: &mut ClientSession,
    profile_id: &str,
    manager_id: &str,
    telegram_id: i64,
    observation: Option<&str>,
) -> AppResult<Option<PlatformBotRuntimeControls>> {
    check_manager_in_session(
        db,
        session,
        profile_id,
        manager_id,
        telegram_id,
        observation,
        true,
    )
    .await
}

/// Same identity/transport predicates as the commit fence, without acquiring
/// shared writes while the caller prepares encrypted conversation material.
pub(crate) async fn validate_manager_in_session(
    db: &Database,
    session: &mut ClientSession,
    profile_id: &str,
    manager_id: &str,
    telegram_id: i64,
    observation: Option<&str>,
) -> AppResult<Option<PlatformBotRuntimeControls>> {
    check_manager_in_session(
        db,
        session,
        profile_id,
        manager_id,
        telegram_id,
        observation,
        false,
    )
    .await
}

async fn check_manager_in_session(
    db: &Database,
    session: &mut ClientSession,
    profile_id: &str,
    manager_id: &str,
    telegram_id: i64,
    observation: Option<&str>,
    write_fence: bool,
) -> AppResult<Option<PlatformBotRuntimeControls>> {
    let profiles =
        db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME);
    let profile_filter =
        doc! {"_id":profile_id,"manager_bot_id":manager_id,"deleted_at":bson::Bson::Null};
    let profile = if write_fence {
        profiles
            .find_one_and_update(
                profile_filter,
                doc! {"$inc":{"personal_identity_revision":1_i64}},
            )
            .return_document(ReturnDocument::After)
            .session(&mut *session)
            .await?
    } else {
        profiles
            .find_one(profile_filter)
            .session(&mut *session)
            .await?
    };
    let Some(profile) = profile else {
        return Ok(None);
    };
    if manager_id.parse::<i64>().ok() != Some(telegram_id) || telegram_id <= 0 {
        return Ok(None);
    }
    let mut credential_filter = doc! {"provider":"telegram-new","fields.manager_bot_id":manager_id,"fields.webhook_ready":"true"};
    if let Some(value) = observation {
        credential_filter.insert("fields.observation_id", value);
    }
    let credentials =
        db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME);
    let credential_exists = if write_fence {
        credentials
            .update_one(
                credential_filter,
                doc! {"$inc":{"personal_effect_revision":1_i64}},
            )
            .session(&mut *session)
            .await?
            .matched_count
            == 1
    } else {
        credentials
            .find_one(credential_filter)
            .session(&mut *session)
            .await?
            .is_some()
    };
    if !credential_exists {
        return Ok(None);
    }
    let bots = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME);
    let bot_filter = doc! {"_id":&profile.channel_bot_id,"platform_profile_id":profile_id,"platform_bot_id":manager_id,"is_active":true};
    let bot_exists = if write_fence {
        bots.update_one(bot_filter, doc! {"$inc":{"personal_effect_revision":1_i64}})
            .session(&mut *session)
            .await?
            .matched_count
            == 1
    } else {
        bots.find_one(bot_filter)
            .session(&mut *session)
            .await?
            .is_some()
    };
    if !bot_exists {
        return Ok(None);
    }
    let controls = db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME);
    let controls_filter = doc! {"platform_config_id":profile_id,"manager_bot_id":manager_id,"telegram_bot_id":telegram_id,"contract_version":1_i32};
    Ok(if write_fence {
        controls.find_one_and_update(controls_filter,vec![doc! {"$set":{"effect_revision":{"$add":[{"$ifNull":["$effect_revision",0_i64]},1_i64]},"updated_at":"$$NOW"}}])
            .return_document(ReturnDocument::After).session(session).await?
    } else {
        controls.find_one(controls_filter).session(session).await?
    })
}

fn candidate(
    profile: &PlatformBotConfig,
    current: Option<&PlatformBotRuntimeControls>,
    change: &RuntimeChange,
) -> AppResult<PlatformBotRuntimeControls> {
    candidate_for_review(profile, current, change, false)
}
fn candidate_for_review(
    profile: &PlatformBotConfig,
    current: Option<&PlatformBotRuntimeControls>,
    change: &RuntimeChange,
    allow_unselected: bool,
) -> AppResult<PlatformBotRuntimeControls> {
    let initializing = current.is_none();
    let mut row = if let Some(row) = current {
        if change.initialize
            || row.manager_bot_id != profile.manager_bot_id
            || row.platform_config_id != profile.id
            || row.contract_version != 1
            || row.retired
            || row.manager_authority_epoch != change.expected_manager_authority_epoch
            || row.personal_control_revision != change.expected_personal_control_revision
            || row.public_policy_revision != change.expected_public_policy_revision
        {
            return Err(changed());
        }
        row.clone()
    } else {
        if !change.initialize
            || change.expected_profile_generation != Some(profile.generation)
            || change.expected_manager_authority_epoch != 0
            || change.expected_personal_control_revision != 0
            || change.expected_public_policy_revision != 0
        {
            return Err(changed());
        }
        PlatformBotRuntimeControls {
            id: uuid::Uuid::new_v4().to_string(),
            contract_version: 1,
            platform_config_id: profile.id.clone(),
            manager_bot_id: profile.manager_bot_id.clone(),
            telegram_bot_id: profile.manager_bot_id.parse().map_err(|_| changed())?,
            manager_authority_epoch: 1,
            personal_control_revision: 1,
            public_policy_revision: 1,
            personal_enabled: false,
            public_enabled: false,
            emergency_stopped: false,
            retired: false,
            public_policy: None,
            effect_revision: 0,
            cleanup_pending: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    };
    let old_personal = row.personal_enabled;
    let old_public = row.public_enabled;
    let old_policy = row.public_policy.clone();
    // The UI explicitly requests both lanes for Enable bot. Missing lane fields
    // remain off on creation; personal-only opt-in needs no public readiness.
    if let Some(enabled) = change.personal_enabled {
        row.personal_enabled = enabled;
    }
    if let Some(enabled) = change.public_enabled {
        row.public_enabled = enabled;
    }
    if let Some(value) = &change.public_policy {
        if value.is_none() && change.public_enabled != Some(false) {
            return Err(AppError::ValidationError(
                "Disable public routing before clearing public policy".into(),
            ));
        }
        row.public_policy = value.clone();
    }
    if change.emergency_stopped == Some(false) {
        return Err(AppError::ValidationError(
            "Use explicit emergency-stop clearing".into(),
        ));
    }
    if change.emergency_stopped == Some(true) {
        row.emergency_stopped = true;
        row.personal_enabled = false;
        row.public_enabled = false;
        row.cleanup_pending = true;
    }
    if row.public_enabled && row.public_policy.is_none() {
        return Err(AppError::ValidationError(
            "Public runtime configuration is required".into(),
        ));
    }
    if row.emergency_stopped && (row.personal_enabled || row.public_enabled) {
        return Err(AppError::Conflict(
            "Clear emergency stop explicitly before enabling routing".into(),
        ));
    }
    if let Some(public) = &row.public_policy {
        if uuid::Uuid::parse_str(&public.sponsor_owner_user_id).is_err() {
            return Err(AppError::ValidationError(
                "A funding organization is required".into(),
            ));
        }
        if (!allow_unselected && public.nyxagent_profile.trim().is_empty())
            || public.nyxagent_profile.len() > 128
            || public.total_daily_credits <= crate::models::credits::Credits::ZERO
            || public.sender_daily_credits <= crate::models::credits::Credits::ZERO
            || public.sender_daily_credits > public.total_daily_credits
        {
            return Err(AppError::ValidationError(
                "Invalid public runtime configuration".into(),
            ));
        }
    }
    if !initializing {
        if row.personal_enabled != old_personal {
            row.personal_control_revision = row
                .personal_control_revision
                .checked_add(1)
                .ok_or_else(changed)?;
        }
        if row.public_enabled != old_public || row.public_policy != old_policy {
            row.public_policy_revision = row
                .public_policy_revision
                .checked_add(1)
                .ok_or_else(changed)?;
        }
    }
    if old_personal && !row.personal_enabled {
        row.cleanup_pending = true;
    }
    Ok(row)
}

pub async fn update(
    db: &Database,
    actor: &AuditActor,
    audit_key: &[u8],
    profile_id: &str,
    change: &RuntimeChange,
    readiness: &dyn PublicControlReadiness,
) -> AppResult<PlatformBotRuntimeControls> {
    if change
        .public_deployment_digest
        .as_ref()
        .is_some_and(|digest| {
            change.public_enabled != Some(true)
                || digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
    {
        return Err(AppError::ValidationError(
            "Deployment identity requires explicit public enable and a SHA-256 digest".into(),
        ));
    }
    let profile = super::platform_bot_config_service::get(db, profile_id).await?;
    let previous = get(db, profile_id).await?;
    let proposed = candidate(&profile, previous.as_ref(), change)?;
    let public_changed = previous.as_ref().is_none_or(|r| {
        r.public_enabled != proposed.public_enabled || r.public_policy != proposed.public_policy
    });
    let needs_probe = proposed.public_enabled
        && (change.public_enabled == Some(true)
            || previous
                .as_ref()
                .is_none_or(|r| !r.public_enabled || r.public_policy != proposed.public_policy));
    let evidence = if needs_probe {
        authorize_public_probe(db, &actor.user_id, &policy(&proposed)).await?;
        Some(readiness.probe(db, &policy(&proposed)).await?)
    } else {
        None
    };
    if change
        .public_deployment_digest
        .as_ref()
        .is_some_and(|digest| {
            evidence
                .as_ref()
                .is_none_or(|proof| &proof.policy_digest != digest)
        })
    {
        return Err(AppError::Conflict(
            "Validated deployment changed; validate again before applying".into(),
        ));
    }
    let db = db.clone();
    let actor = actor.clone();
    let key = zeroize::Zeroizing::new(audit_key.to_vec());
    let change = change.clone();
    let profile_id = profile_id.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (db, actor, key, change, profile_id, profile, proposed, evidence, readiness, public_changed, needs_probe),
            |session, (db, actor, key, change, profile_id, profile, proposed, evidence, readiness, public_changed, needs_probe)| {
                Box::pin(async move {
                    let result: AppResult<_> = async {
                        use crate::models::platform_public_contract::{
                            ACTIVATIONS, PublicDeploymentActivation,
                        };
                        super::platform_bot_config_service::fence_admin(db, &actor.user_id, session).await?;
                        let mut filter = doc! {
                            "_id": &*profile_id,
                            "manager_bot_id": &profile.manager_bot_id,
                            "deleted_at": bson::Bson::Null,
                        };
                        if change.initialize {
                            filter.insert("generation", change.expected_profile_generation.ok_or_else(changed)?);
                        }
                        let live = db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
                            .find_one_and_update(filter, doc! {"$inc": {"runtime_configuration_revision": 1_i64}})
                            .return_document(ReturnDocument::After).session(&mut *session)
                            .await?.ok_or_else(changed)?;
                        let current = db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
                            .find_one(doc! {"platform_config_id": &*profile_id})
                            .session(&mut *session).await?;
                        let rebuilt = candidate(&live, current.as_ref(), change)?;
                        if policy_digest_fields(&rebuilt) != policy_digest_fields(proposed) {
                            return Err(changed());
                        }
                        proposed.effect_revision = rebuilt.effect_revision;
                        proposed.updated_at = Utc::now();
                        if proposed.personal_enabled || proposed.public_enabled {
                            for (collection, filter) in [
                                (crate::models::platform_credential::COLLECTION_NAME,
                                    doc! {"provider": "telegram-new", "fields.manager_bot_id": &live.manager_bot_id, "fields.webhook_ready": "true"}),
                                (crate::models::channel_bot::COLLECTION_NAME,
                                    doc! {"_id": &live.channel_bot_id, "platform_profile_id": &*profile_id, "platform_bot_id": &live.manager_bot_id, "is_active": true}),
                            ] {
                                let fenced = db.collection::<Document>(collection)
                                    .update_one(filter, doc! {"$inc": {"runtime_configuration_revision": 1_i64}})
                                    .session(&mut *session).await?;
                                if fenced.matched_count != 1 {
                                    return Err(changed());
                                }
                            }
                        }
                        let old_activation = db.collection::<PublicDeploymentActivation>(ACTIVATIONS)
                            .find_one(doc! {"_id": &*profile_id}).session(&mut *session).await?;
                        if proposed.public_enabled && (*public_changed || *needs_probe) {
                            let sponsor = proposed.public_policy.as_ref().ok_or_else(changed)?;
                            for (collection, filter) in [
                                (crate::models::user::COLLECTION_NAME,
                                    doc! {"_id": &sponsor.sponsor_owner_user_id, "is_active": true, "user_type": "org"}),
                                (crate::models::org_membership::COLLECTION_NAME,
                                    doc! {"org_user_id": &sponsor.sponsor_owner_user_id, "member_user_id": &actor.user_id, "role": "admin", "revoked_at": bson::Bson::Null}),
                            ] {
                                let fenced = db.collection::<Document>(collection)
                                    .update_one(filter, doc! {"$inc": {"runtime_funding_revision": 1_i64}})
                                    .session(&mut *session).await?;
                                if fenced.matched_count != 1 {
                                    return Err(changed());
                                }
                            }
                            let proof = evidence.as_ref().ok_or_else(changed)?;
                            readiness.fence(db, session, &policy(proposed), proof).await?;
                        }
                        let deployment_changed = evidence.as_ref().is_some_and(|proof| {
                            old_activation.as_ref().is_none_or(|old| old.policy_digest != proof.policy_digest)
                        });
                        if let Some(old) = &current
                            && old.public_enabled
                            && (!proposed.public_enabled || old.public_policy != proposed.public_policy || deployment_changed)
                        {
                            // Activation and revocation share the controls/manager fence.
                            // Existing senders can then create a fresh session safely.
                            readiness.revoke_public_manager(db, session, &policy(old)).await?;
                        }
                        if let Some(proof) = evidence.as_ref() {
                            let activation = PublicDeploymentActivation {
                                profile_id: profile_id.clone(),
                                binding: super::platform_public_policy::binding(&policy(proposed))?,
                                policy_digest: proof.policy_digest.clone(),
                                activated_by: actor.user_id.clone(),
                                activated_at: Utc::now(),
                                effect_revision: 0,
                            };
                            db.collection::<PublicDeploymentActivation>(ACTIVATIONS)
                                .replace_one(doc! {"_id": &*profile_id}, activation).upsert(true)
                                .session(&mut *session).await?;
                        }
                        if current.is_some() {
                            db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
                                .replace_one(doc! {"platform_config_id": &*profile_id}, &*proposed)
                                .session(&mut *session).await?;
                        } else {
                            db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
                                .insert_one(&*proposed).session(&mut *session).await?;
                        }
                        super::audit_service::log_actor_event_in_session(
                            db, session, key, actor, "platform_manager_runtime_changed",
                            serde_json::json!({"profile_id": profile_id, "epoch": proposed.manager_authority_epoch,
                                "personal_revision": proposed.personal_control_revision, "public_revision": proposed.public_policy_revision,
                                "personal_enabled": proposed.personal_enabled, "public_enabled": proposed.public_enabled,
                                "activated_policy_digest": evidence.as_ref().map(|proof| &proof.policy_digest)}),
                        ).await?;
                        Ok(proposed.clone())
                    }.await;
                    transaction_result(result)
                })
            },
        ).await.map_err(map_transaction_error)
}

fn policy_digest_fields(
    row: &PlatformBotRuntimeControls,
) -> (i64, i64, i64, bool, bool, Option<PublicRuntimePolicy>) {
    (
        row.manager_authority_epoch,
        row.personal_control_revision,
        row.public_policy_revision,
        row.personal_enabled,
        row.public_enabled,
        row.public_policy.clone(),
    )
}

pub async fn emergency_stop_in_session(
    db: &Database,
    session: &mut ClientSession,
    profile_id: &str,
    retire: bool,
) -> AppResult<()> {
    let mut increments = doc! {"personal_control_revision":1_i64,"public_policy_revision":1_i64};
    if retire {
        increments.insert("manager_authority_epoch", 1_i64);
    }
    let mut set = doc! {"personal_enabled":false,"public_enabled":false,"emergency_stopped":true,"cleanup_pending":true,"updated_at":bson::DateTime::now()};
    if retire {
        set.insert("retired", true);
    }
    db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":profile_id,"retired":false},
            doc! {"$set":set,"$inc":increments},
        )
        .session(session)
        .await?;
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Frozen transaction seam binds actor, audit and all expected runtime revisions"
)]
pub async fn clear_emergency_stop_in_session(
    db: &Database,
    session: &mut ClientSession,
    actor: &AuditActor,
    audit_key: &[u8],
    profile_id: &str,
    epoch: i64,
    personal_revision: i64,
    public_revision: i64,
) -> AppResult<()> {
    super::platform_bot_config_service::fence_admin(db, &actor.user_id, session).await?;
    if db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME).update_one(doc! {"platform_config_id":profile_id,"manager_authority_epoch":epoch,"personal_control_revision":personal_revision,"public_policy_revision":public_revision,"retired":false,"emergency_stopped":true},doc! {"$set":{"emergency_stopped":false,"personal_enabled":false,"public_enabled":false},"$inc":{"personal_control_revision":1_i64,"public_policy_revision":1_i64}}).session(&mut *session).await?.matched_count!=1 {return Err(changed());}
    super::audit_service::log_actor_event_in_session(
        db,
        session,
        audit_key,
        actor,
        "platform_manager_emergency_stop_cleared",
        serde_json::json!({"profile_id":profile_id,"epoch":epoch}),
    )
    .await
}

pub async fn claim_stop_effect(db: &Database) -> AppResult<Option<PlatformPersonalStopEffect>> {
    Ok(db.collection::<PlatformPersonalStopEffect>(STOP_EFFECTS).find_one_and_update(doc! {"completed":false,"$expr":{"$and":[{"$lte":["$retry_at","$$NOW"]},{"$or":[{"$eq":["$claim_id",bson::Bson::Null]},{"$lte":["$lease_expires_at","$$NOW"]}]}]}},vec![doc! {"$set":{"claim_id":uuid::Uuid::new_v4().to_string(),"lease_expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":30_i64}}}}]).sort(doc! {"retry_at":1,"_id":1}).return_document(ReturnDocument::After).await?)
}

pub async fn finish_stop_effect(
    db: &Database,
    id: &str,
    claim_id: &str,
    completed: bool,
) -> AppResult<bool> {
    Ok(db.collection::<PlatformPersonalStopEffect>(STOP_EFFECTS).update_one(doc! {"_id":id,"claim_id":claim_id,"completed":false},doc! {"$set":{"completed":completed,"claim_id":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,"retry_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(10))}}).await?.matched_count==1)
}

/// Resolve omitted public fields from saved configuration, preserving the
/// sponsor and exact budgets from the existing setup on first explicit opt-in.
pub fn change_from_patch(
    profile: &PlatformBotConfig,
    current: Option<&PlatformBotRuntimeControls>,
    patch: &super::platform_manager_contract::RuntimePatch,
) -> AppResult<RuntimeChange> {
    use super::platform_manager_contract::PolicyPatch;
    let public_policy = match &patch.public_policy {
        PolicyPatch::Unchanged => None,
        PolicyPatch::Clear => Some(None),
        PolicyPatch::Update(p) => {
            let saved = current.and_then(|r| r.public_policy.as_ref());
            let profile_name = p
                .nyxagent_profile
                .clone()
                .or_else(|| saved.map(|s| s.nyxagent_profile.clone()))
                .unwrap_or_default();
            Some(Some(PublicRuntimePolicy {
                sponsor_owner_user_id: p
                    .sponsor_owner_user_id
                    .clone()
                    .or_else(|| saved.map(|s| s.sponsor_owner_user_id.clone()))
                    .unwrap_or_else(|| profile.funding_owner_user_id.clone()),
                nyxagent_profile: profile_name,
                total_daily_credits: p
                    .total_daily_credits
                    .or_else(|| saved.map(|s| s.total_daily_credits))
                    .or_else(|| profile.guide.as_ref().map(|g| g.limits.daily_credits))
                    .unwrap_or("10".parse().map_err(|_| changed())?),
                sender_daily_credits: p
                    .sender_daily_credits
                    .or_else(|| saved.map(|s| s.sender_daily_credits))
                    .or_else(|| profile.guide.as_ref().map(|g| g.anonymous_daily_credits))
                    .unwrap_or("0.5".parse().map_err(|_| changed())?),
            }))
        }
    };
    if !patch.initialize && patch.expected_profile_generation.is_some() {
        return Err(AppError::ValidationError(
            "Profile generation is only used for initialization".into(),
        ));
    }
    Ok(RuntimeChange {
        public_deployment_digest: patch.public_deployment_digest.clone(),
        initialize: patch.initialize,
        expected_profile_generation: patch.expected_profile_generation,
        expected_manager_authority_epoch: patch.expected.manager_authority_epoch,
        expected_personal_control_revision: patch.expected.personal_control_revision,
        expected_public_policy_revision: patch.expected.public_policy_revision,
        personal_enabled: patch.personal_enabled,
        public_enabled: patch.public_enabled,
        emergency_stopped: patch.emergency_stopped,
        public_policy,
    })
}

pub async fn reviewed_policy(
    db: &Database,
    actor: &str,
    profile_id: &str,
    patch: &super::platform_manager_contract::RuntimePatch,
) -> AppResult<ManagerRuntimePolicy> {
    let profile = super::platform_bot_config_service::get(db, profile_id).await?;
    if profile.deleted_at.is_some() {
        return Err(changed());
    }
    let current = get(db, profile_id).await?;
    let mut change = change_from_patch(&profile, current.as_ref(), patch)?;
    change.public_enabled = Some(true);
    let row = candidate_for_review(&profile, current.as_ref(), &change, true)?;
    if row.public_policy.is_none() {
        return Err(AppError::ValidationError(
            "Public runtime configuration is required".into(),
        ));
    }
    let reviewed = policy(&row);
    authorize_public_probe(db, actor, &reviewed).await?;
    Ok(reviewed)
}

/// Reserve no money here. Establish the human/org authority before Part 3 may
/// perform a sponsor-funded discovery or readiness request, then recheck it at
/// configuration commit. The remote request never runs in this transaction.
async fn authorize_public_probe(
    db: &Database,
    actor: &str,
    reviewed: &ManagerRuntimePolicy,
) -> AppResult<()> {
    let sponsor = reviewed
        .public_policy
        .as_ref()
        .ok_or_else(changed)?
        .sponsor_owner_user_id
        .clone();
    let db = db.clone();
    let actor = actor.to_owned();
    let reviewed = reviewed.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                super::platform_bot_config_service::fence_admin(&db, &actor, session).await?;
                for (collection, filter) in [
                    (
                        crate::models::user::COLLECTION_NAME,
                        doc! {"_id": &sponsor, "is_active": true, "user_type": "org"},
                    ),
                    (
                        crate::models::org_membership::COLLECTION_NAME,
                        doc! {"org_user_id": &sponsor, "member_user_id": &actor,
                        "role": "admin", "revoked_at": bson::Bson::Null},
                    ),
                ] {
                    let fenced = db
                        .collection::<Document>(collection)
                        .update_one(filter, doc! {"$inc": {"runtime_probe_revision": 1_i64}})
                        .session(&mut *session)
                        .await?;
                    if fenced.matched_count != 1 {
                        return Err(AppError::Forbidden("Funding consent changed".into()));
                    }
                }
                super::platform_public_contract_service::record_probe_consent(
                    &db, session, &actor, &reviewed,
                )
                .await?;
                Ok(())
            }
            .await;
            transaction_result(result)
        })
        .await
        .map_err(map_transaction_error)
}

pub struct ReadinessAdapter<'a>(pub &'a dyn super::platform_manager_contract::PublicTurnProducer);
#[async_trait]
impl PublicControlReadiness for ReadinessAdapter<'_> {
    async fn probe(
        &self,
        db: &Database,
        candidate: &ManagerRuntimePolicy,
    ) -> AppResult<PublicReadinessEvidence> {
        if let super::platform_manager_contract::PublicReadiness::Ready(proof) =
            super::platform_public_policy::assess(db, candidate).await?
            && proof.expires_at
                > Utc::now() + Duration::seconds(i64::from(proof.limits.max_duration_secs) + 5)
        {
            return Ok(proof);
        }
        match self.0.probe_public_readiness(db, candidate).await? {
            super::platform_manager_contract::PublicReadiness::Ready(evidence) => Ok(evidence),
            super::platform_manager_contract::PublicReadiness::Unavailable(_) => Err(
                AppError::Conflict("Public runtime unavailable; keep public routing paused".into()),
            ),
        }
    }
    async fn revoke_public_manager(
        &self,
        db: &Database,
        session: &mut ClientSession,
        previous: &ManagerRuntimePolicy,
    ) -> AppResult<()> {
        use super::platform_manager_contract::{PublicRevocationReason, PublicRevocationScope};
        self.0
            .revoke_public_sessions_in_session(
                db,
                session,
                &PublicRevocationScope::Manager {
                    manager: previous.manager.clone(),
                },
                PublicRevocationReason::ManagerRetired,
            )
            .await?;
        Ok(())
    }
    async fn fence(
        &self,
        db: &Database,
        session: &mut ClientSession,
        candidate: &ManagerRuntimePolicy,
        evidence: &PublicReadinessEvidence,
    ) -> AppResult<()> {
        match self
            .0
            .fence_public_readiness(db, session, candidate, evidence)
            .await?
        {
            super::platform_manager_contract::FenceOutcome::Authorized => Ok(()),
            super::platform_manager_contract::FenceOutcome::Unavailable(_) => Err(changed()),
        }
    }
}

pub async fn update_patch(
    db: &Database,
    actor: &AuditActor,
    audit_key: &[u8],
    profile_id: &str,
    patch: &super::platform_manager_contract::RuntimePatch,
    public: &dyn super::platform_manager_contract::PublicTurnProducer,
) -> AppResult<PlatformBotRuntimeControls> {
    let profile = super::platform_bot_config_service::get(db, profile_id).await?;
    let previous = get(db, profile_id).await?;
    let change = change_from_patch(&profile, previous.as_ref(), patch)?;
    update(
        db,
        actor,
        audit_key,
        profile_id,
        &change,
        &ReadinessAdapter(public),
    )
    .await
}

pub async fn clear_emergency_stop(
    db: &Database,
    actor: &AuditActor,
    audit_key: &[u8],
    profile_id: &str,
    expected: &super::platform_manager_contract::ExpectedRuntimeRevisions,
) -> AppResult<PlatformBotRuntimeControls> {
    let db = db.clone();
    let actor = actor.clone();
    let key = zeroize::Zeroizing::new(audit_key.to_vec());
    let profile_id = profile_id.to_owned();
    let expected = expected.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<_> = async {
                clear_emergency_stop_in_session(
                    &db,
                    session,
                    &actor,
                    &key,
                    &profile_id,
                    expected.manager_authority_epoch,
                    expected.personal_control_revision,
                    expected.public_policy_revision,
                )
                .await?;
                get_in_session(&db, session, &profile_id)
                    .await?
                    .ok_or_else(changed)
            }
            .await;
            transaction_result(result)
        })
        .await
        .map_err(map_transaction_error)
}

pub fn producer(
    _state: crate::AppState,
    personal: std::sync::Arc<dyn super::platform_manager_contract::PersonalTurnProducer>,
    public: std::sync::Arc<dyn super::platform_manager_contract::PublicTurnProducer>,
) -> std::sync::Arc<dyn super::platform_manager_contract::ManagerLifecycleProducer> {
    std::sync::Arc::new(
        super::platform_personal_binding_service::LifecycleProducer { personal, public },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::platform_personal_binding_service::tests::fixture;

    struct NoPublic;
    #[async_trait]
    impl PublicControlReadiness for NoPublic {
        async fn probe(
            &self,
            _: &Database,
            _: &ManagerRuntimePolicy,
        ) -> AppResult<PublicReadinessEvidence> {
            Err(AppError::Conflict("public proof unavailable".into()))
        }
        async fn fence(
            &self,
            _: &Database,
            _: &mut ClientSession,
            _: &ManagerRuntimePolicy,
            _: &PublicReadinessEvidence,
        ) -> AppResult<()> {
            panic!("no evidence may reach the transaction")
        }
        async fn revoke_public_manager(
            &self,
            _: &Database,
            _: &mut ClientSession,
            _: &ManagerRuntimePolicy,
        ) -> AppResult<()> {
            panic!("no public session exists in this fixture")
        }
    }
    async fn admin_fixture(name: &str) -> (Database, String, AuditActor) {
        let (db, update, owner) = fixture(name).await;
        super::super::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let roles = super::super::role_service::get_platform_role_ids(&db)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&owner},
                doc! {"$set":{"role_ids":[roles.admin]}},
            )
            .await
            .unwrap();
        db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
            .delete_many(doc! {})
            .await
            .unwrap();
        (
            db,
            update.profile_id,
            AuditActor {
                user_id: owner,
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
            },
        )
    }
    fn initialize() -> RuntimeChange {
        RuntimeChange {
            public_deployment_digest: None,
            initialize: true,
            expected_profile_generation: Some(40),
            expected_manager_authority_epoch: 0,
            expected_personal_control_revision: 0,
            expected_public_policy_revision: 0,
            personal_enabled: Some(true),
            public_enabled: Some(false),
            emergency_stopped: None,
            public_policy: None,
        }
    }
    #[tokio::test]
    async fn failed_combined_enable_leaves_no_controls_and_personal_only_needs_no_public_proof() {
        let (db, id, actor) = admin_fixture("personal_runtime_initialize").await;
        let sponsor = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &sponsor,
                crate::models::user::UserType::Org,
            ))
            .await
            .unwrap();
        db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME).insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"org_user_id":&sponsor,"member_user_id":&actor.user_id,"role":"admin","revoked_at":bson::Bson::Null}).await.unwrap();
        let mut combined = initialize();
        combined.public_enabled = Some(true);
        combined.public_policy = Some(Some(PublicRuntimePolicy {
            sponsor_owner_user_id: sponsor,
            nyxagent_profile: "nyxagent/chat".into(),
            total_daily_credits: "2".parse().unwrap(),
            sender_daily_credits: "0.2".parse().unwrap(),
        }));
        assert!(
            update(&db, &actor, b"test-audit-key", &id, &combined, &NoPublic)
                .await
                .is_err()
        );
        assert!(get(&db, &id).await.unwrap().is_none());
        let mut stale = initialize();
        stale.expected_profile_generation = Some(39);
        assert!(
            update(&db, &actor, b"test-audit-key", &id, &stale, &NoPublic)
                .await
                .is_err()
        );
        let row = update(
            &db,
            &actor,
            b"test-audit-key",
            &id,
            &initialize(),
            &NoPublic,
        )
        .await
        .unwrap();
        assert!(row.personal_enabled);
        assert!(!row.public_enabled);
        assert_eq!(row.manager_authority_epoch, 1);
        assert!(row.public_policy.is_none());
        assert!(
            update(
                &db,
                &actor,
                b"test-audit-key",
                &id,
                &initialize(),
                &NoPublic
            )
            .await
            .is_err()
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn concurrent_initialization_has_one_winner() {
        let (db, id, actor) = admin_fixture("personal_runtime_create_race").await;
        let change = initialize();
        let (a, b) = tokio::join!(
            update(&db, &actor, b"test-audit-key", &id, &change, &NoPublic),
            update(&db, &actor, b"test-audit-key", &id, &change, &NoPublic)
        );
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(
            db.collection::<PlatformBotRuntimeControls>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn personal_pause_resume_versions_authority_and_emergency_clear_keeps_lanes_off() {
        let (db, id, actor) = admin_fixture("personal_runtime_revision").await;
        let row = update(
            &db,
            &actor,
            b"test-audit-key",
            &id,
            &initialize(),
            &NoPublic,
        )
        .await
        .unwrap();
        let mut patch = RuntimeChange {
            public_deployment_digest: None,
            initialize: false,
            expected_profile_generation: None,
            expected_manager_authority_epoch: row.manager_authority_epoch,
            expected_personal_control_revision: row.personal_control_revision,
            expected_public_policy_revision: row.public_policy_revision,
            personal_enabled: Some(false),
            public_enabled: None,
            emergency_stopped: None,
            public_policy: None,
        };
        let paused = update(&db, &actor, b"test-audit-key", &id, &patch, &NoPublic)
            .await
            .unwrap();
        assert_eq!(paused.personal_control_revision, 2);
        assert_eq!(paused.public_policy_revision, 1);
        patch.expected_personal_control_revision = 2;
        patch.personal_enabled = Some(true);
        let resumed = update(&db, &actor, b"test-audit-key", &id, &patch, &NoPublic)
            .await
            .unwrap();
        assert_eq!(resumed.personal_control_revision, 3);
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        emergency_stop_in_session(&db, &mut session, &id, false)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        let stopped = get(&db, &id).await.unwrap().unwrap();
        patch.expected_personal_control_revision = stopped.personal_control_revision;
        patch.expected_public_policy_revision = stopped.public_policy_revision;
        assert!(
            update(&db, &actor, b"test-audit-key", &id, &patch, &NoPublic)
                .await
                .is_err()
        );
        let cleared = clear_emergency_stop(
            &db,
            &actor,
            b"test-audit-key",
            &id,
            &super::super::platform_manager_contract::ExpectedRuntimeRevisions {
                manager_authority_epoch: stopped.manager_authority_epoch,
                personal_control_revision: stopped.personal_control_revision,
                public_policy_revision: stopped.public_policy_revision,
            },
        )
        .await
        .unwrap();
        assert!(!cleared.emergency_stopped && !cleared.personal_enabled && !cleared.public_enabled);
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod stop_queue_tests {
    use super::*;
    use crate::services::{
        platform_manager_contract::*, platform_personal_binding_service as bindings,
    };

    #[tokio::test]
    async fn stop_capture_is_atomic_and_reclaimed_effect_rejects_the_old_lease() {
        let (db, update, owner) = bindings::tests::fixture("personal_stop_lease").await;
        let binding = bindings::tests::binding(&update, &owner);
        let origin = PlatformTurnOrigin {
            authority: bindings::authority(&binding, 1),
            nyxbot_agent_id: uuid::Uuid::new_v4().to_string(),
            source: PlatformOriginSource::ThreadEvent {
                admission_id: uuid::Uuid::new_v4().to_string(),
            },
        };
        let at = chrono::DateTime::from_timestamp_millis(1_000_000).unwrap();
        let capture = bindings::CapturedPersonalStop {
            active_turn: crate::models::platform_personal_binding::PersonalActiveTurn {
                conversation_id: "exact-conversation".into(),
                turn_id: "exact-turn".into(),
                origin: bindings::stored_origin(&origin),
            },
            stop_requested_at: at,
            machine_jobs: vec![
                crate::models::platform_bot_runtime_controls::ExactMachineJob {
                    node_id: "node".into(),
                    runtime_id: "runtime-before-reconnect".into(),
                    job_id: "job".into(),
                },
            ],
            machine_scopes: vec![
                crate::models::platform_bot_runtime_controls::ExactMachineScope {
                    node_id: "node".into(),
                    runtime_id: "runtime-before-reconnect".into(),
                },
            ],
        };
        let rows = db.collection::<PlatformPersonalStopEffect>(STOP_EFFECTS);
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        bindings::enqueue_stop_in_session(&db, &mut session, &capture)
            .await
            .unwrap();
        session.abort_transaction().await.unwrap();
        assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 0);
        session.start_transaction().await.unwrap();
        let id = bindings::enqueue_stop_in_session(&db, &mut session, &capture)
            .await
            .unwrap();
        assert_eq!(
            bindings::enqueue_stop_in_session(&db, &mut session, &capture)
                .await
                .unwrap(),
            id
        );
        session.commit_transaction().await.unwrap();
        let first = claim_stop_effect(&db).await.unwrap().unwrap();
        rows.update_one(
            doc! {"_id":&id},
            doc! {"$set":{"lease_expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
        let reclaimed = claim_stop_effect(&db).await.unwrap().unwrap();
        assert_ne!(first.claim_id, reclaimed.claim_id);
        assert!(
            !finish_stop_effect(&db, &id, first.claim_id.as_deref().unwrap(), true)
                .await
                .unwrap()
        );
        assert_eq!(reclaimed.stop_requested_at, at);
        assert_eq!(reclaimed.machine_jobs, capture.machine_jobs);
        assert_eq!(reclaimed.machine_scopes, capture.machine_scopes);
        assert!(
            !finish_stop_effect(&db, &id, "unrelated-claim", true)
                .await
                .unwrap()
        );
        assert!(
            finish_stop_effect(&db, &id, reclaimed.claim_id.as_deref().unwrap(), true)
                .await
                .unwrap()
        );
        assert!(claim_stop_effect(&db).await.unwrap().is_none());
        db.drop().await.unwrap();
    }
}

pub(crate) async fn record_stop_progress(
    db: &Database,
    id: &str,
    claim_id: &str,
    progress: &super::platform_manager_contract::ExactStopProgress,
) -> AppResult<bool> {
    use super::platform_manager_contract::{ExactMachineCancelOutcome, ExactStopSettlement};
    let outcome = |value| match value {
        ExactMachineCancelOutcome::Cancelled => MachineCancelState::Cancelled,
        ExactMachineCancelOutcome::AlreadyFinished => MachineCancelState::AlreadyFinished,
        ExactMachineCancelOutcome::NodeUnavailable => MachineCancelState::NodeUnavailable,
        ExactMachineCancelOutcome::Unsupported => MachineCancelState::Unsupported,
        ExactMachineCancelOutcome::RuntimeChanged => MachineCancelState::RuntimeChanged,
    };
    let stored = PersonalStopProgress {
        settlement: match progress.settlement {
            ExactStopSettlement::Pending => StopSettlementState::Pending,
            ExactStopSettlement::Settled => StopSettlementState::Settled,
            ExactStopSettlement::AlreadySettled => StopSettlementState::AlreadySettled,
            ExactStopSettlement::ConversationDeleted => StopSettlementState::ConversationDeleted,
        },
        machine_jobs: progress
            .machine_jobs
            .iter()
            .map(|p| MachineJobProgress {
                job: ExactMachineJob {
                    node_id: p.job.node_id.clone(),
                    runtime_id: p.job.runtime_id.clone(),
                    job_id: p.job.job_id.clone(),
                },
                outcome: outcome(p.outcome),
            })
            .collect(),
        machine_scopes: progress
            .machine_scopes
            .iter()
            .map(|p| MachineScopeProgress {
                scope: ExactMachineScope {
                    node_id: p.scope.node_id.clone(),
                    runtime_id: p.scope.runtime_id.clone(),
                },
                outcome: outcome(p.outcome),
            })
            .collect(),
    };
    Ok(db
        .collection::<PlatformPersonalStopEffect>(STOP_EFFECTS)
        .update_one(
            doc! {"_id":id,"claim_id":claim_id,"completed":false},
            doc! {"$set":{"progress":bson::to_bson(&stored).map_err(|_|changed())?}},
        )
        .await?
        .matched_count
        == 1)
}
