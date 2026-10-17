//! Durable consent and write fences. Session-taking operations never own commit.
use super::platform_entry_scope as scope;
use super::platform_manager_contract::SenderIdentity;
use crate::{
    errors::{AppError, AppResult},
    models::{
        platform_bot_runtime_controls::{
            ExactMachineJob, ExactMachineScope, PlatformBotRuntimeControls,
            PlatformPersonalStopEffect,
        },
        platform_personal_binding::*,
    },
};
use async_trait::async_trait;
use chrono::Utc;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::IndexOptions,
};

pub enum ResolvedPersonalSender {
    Unlinked,
    Personal {
        binding: Box<PlatformPersonalBinding>,
        controls: Box<PlatformBotRuntimeControls>,
    },
    Unavailable(super::platform_manager_contract::UnavailableReason),
}

/// No default: callers must bind the real Part 2 and Part 3 implementations.
#[async_trait]
pub trait PersonalLifecycleEffects: Send + Sync {
    async fn capture_stop(
        &self,
        db: &Database,
        session: &mut ClientSession,
        active: &PersonalActiveTurn,
    ) -> AppResult<Option<CapturedPersonalStop>>;
    async fn revoke_public_sender(
        &self,
        db: &Database,
        session: &mut ClientSession,
        binding: &PlatformPersonalBinding,
    ) -> AppResult<()>;
}

pub struct CapturedPersonalStop {
    pub active_turn: PersonalActiveTurn,
    pub stop_requested_at: chrono::DateTime<Utc>,
    pub machine_jobs: Vec<ExactMachineJob>,
    pub machine_scopes: Vec<ExactMachineScope>,
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    super::platform_entry_migration::ensure_indexes(db).await?;
    let rows = db.collection::<PlatformPersonalBinding>(COLLECTION_NAME);
    for index in [
        IndexModel::builder()
            .keys(doc! {"user_id":1,"_id":-1})
            .build(),
        IndexModel::builder()
            .keys(doc! {"platform_config_id":1,"state":1,"_id":1})
            .build(),
        IndexModel::builder()
            .keys(doc! {"conversation_id":1})
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .partial_filter_expression(doc! {"conversation_id":{"$type":"string"}})
                    .name("personal_thread_owner".to_owned())
                    .build(),
            )
            .build(),
    ] {
        rows.create_index(index).await?;
    }
    db.collection::<Document>(crate::models::platform_manager_management::SELECTIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"purge_after":1})
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

pub(crate) fn unavailable() -> AppError {
    AppError::NotFound("Personal linking request unavailable".into())
}
pub(crate) fn conflict() -> AppError {
    AppError::Conflict("Personal linking review changed; review again".into())
}

pub(crate) async fn fence_person(
    db: &Database,
    session: &mut ClientSession,
    user_id: &str,
) -> AppResult<bool> {
    check_person(db, session, user_id, true).await
}

async fn check_person(
    db: &Database,
    session: &mut ClientSession,
    user_id: &str,
    write_fence: bool,
) -> AppResult<bool> {
    let users = db.collection::<Document>(crate::models::user::COLLECTION_NAME);
    let filter = doc! {"_id":user_id,"is_active":true,"$or":[{"user_type":"person"},{"user_type":{"$exists":false}}]};
    Ok(if write_fence {
        users
            .update_one(
                filter,
                doc! {"$inc":{"platform_personal_effect_revision":1_i64}},
            )
            .session(session)
            .await?
            .matched_count
            == 1
    } else {
        users.find_one(filter).session(session).await?.is_some()
    })
}

pub async fn latest_for_sender(
    db: &Database,
    session: &mut ClientSession,
    profile_id: &str,
    epoch: i64,
    sender: &SenderIdentity,
) -> AppResult<Option<PlatformPersonalBinding>> {
    let mut filter = doc! {"platform_config_id":profile_id,"manager_authority_epoch":epoch};
    filter.extend(scope::sender_key_fields(sender, false));
    Ok(db
        .collection::<PlatformPersonalBinding>(COLLECTION_NAME)
        .find_one(filter)
        .sort(doc! {"generation":-1})
        .session(session)
        .await?)
}

pub async fn resolve_sender_in_session(
    db: &Database,
    session: &mut ClientSession,
    update: &super::platform_manager_contract::VerifiedManagerUpdate,
) -> AppResult<ResolvedPersonalSender> {
    use super::platform_manager_contract::{ManagerChatKind, UnavailableReason as Reason};
    if update.chat_kind != ManagerChatKind::Private
        || scope::validate(&update.entry, &update.manager_bot_id, Some(&update.sender)).is_err()
    {
        return Ok(ResolvedPersonalSender::Unavailable(
            Reason::ManagerUnavailable,
        ));
    }
    let Some(controls) = super::platform_bot_runtime_control_service::fence_manager_in_session(
        db,
        session,
        &update.profile_id,
        &update.manager_bot_id,
        &update.entry,
        Some(&update.observation_id),
    )
    .await?
    else {
        return Ok(ResolvedPersonalSender::Unavailable(
            Reason::ManagerUnavailable,
        ));
    };
    if controls.retired {
        return Ok(ResolvedPersonalSender::Unavailable(
            Reason::ManagerUnavailable,
        ));
    }
    if controls.emergency_stopped {
        return Ok(ResolvedPersonalSender::Unavailable(
            Reason::EmergencyStopped,
        ));
    }
    let Some(binding) = latest_for_sender(
        db,
        session,
        &update.profile_id,
        controls.manager_authority_epoch,
        &update.sender,
    )
    .await?
    else {
        return Ok(ResolvedPersonalSender::Unlinked);
    };
    if binding.entry != update.entry
        || binding.sender != update.sender
        || binding.manager_bot_id != update.manager_bot_id
    {
        return Err(AppError::Internal(
            "Personal binding identity is invalid".into(),
        ));
    }
    if binding.state == PersonalBindingState::Revoked {
        if binding.logged_out_at.is_some() {
            return Ok(ResolvedPersonalSender::Unlinked);
        }
        return Ok(match binding.revocation_reason {
            Some(PersonalRevocationReason::Unlinked) => ResolvedPersonalSender::Unlinked,
            Some(
                PersonalRevocationReason::OwnerDisabled | PersonalRevocationReason::OwnerDeleted,
            ) => ResolvedPersonalSender::Unavailable(Reason::OwnerUnavailable),
            _ => ResolvedPersonalSender::Unavailable(Reason::OriginRevoked),
        });
    }
    if !fence_person(db, session, &binding.user_id).await? {
        return Ok(ResolvedPersonalSender::Unavailable(
            Reason::OwnerUnavailable,
        ));
    }
    if !controls.personal_enabled {
        return Ok(ResolvedPersonalSender::Unavailable(
            Reason::PersonalDisabled,
        ));
    }
    if db
        .collection::<PlatformPersonalBinding>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&binding.id,"generation":binding.generation,"state":"active"},
            doc! {"$inc":{"effect_revision":1_i64}},
        )
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(conflict());
    }
    Ok(ResolvedPersonalSender::Personal {
        binding: Box::new(binding),
        controls: Box::new(controls),
    })
}

/// Checks a captured consent snapshot without consulting guide/public policy.
pub async fn fence_binding_in_session(
    db: &Database,
    session: &mut ClientSession,
    binding: &PlatformPersonalBinding,
    personal_revision: i64,
) -> AppResult<bool> {
    check_binding_in_session(db, session, binding, personal_revision, true).await
}

async fn check_binding_in_session(
    db: &Database,
    session: &mut ClientSession,
    binding: &PlatformPersonalBinding,
    personal_revision: i64,
    write_fence: bool,
) -> AppResult<bool> {
    let controls = if write_fence {
        super::platform_bot_runtime_control_service::fence_manager_in_session(
            db,
            session,
            &binding.platform_config_id,
            &binding.manager_bot_id,
            &binding.entry,
            None,
        )
        .await?
    } else {
        super::platform_bot_runtime_control_service::validate_manager_in_session(
            db,
            session,
            &binding.platform_config_id,
            &binding.manager_bot_id,
            &binding.entry,
            None,
        )
        .await?
    };
    let Some(controls) = controls else {
        return Ok(false);
    };
    if controls.retired
        || controls.emergency_stopped
        || !controls.personal_enabled
        || controls.manager_authority_epoch != binding.manager_authority_epoch
        || controls.personal_control_revision != personal_revision
    {
        return Ok(false);
    }
    if !check_person(db, session, &binding.user_id, write_fence).await? {
        return Ok(false);
    }
    let rows = db.collection::<PlatformPersonalBinding>(COLLECTION_NAME);
    let mut filter = doc! {"_id":&binding.id,"generation":binding.generation,"user_id":&binding.user_id,"state":"active","platform_config_id":&binding.platform_config_id,"manager_authority_epoch":binding.manager_authority_epoch};
    filter.extend(scope::entry_fields(&binding.entry));
    filter.extend(scope::sender_fields(&binding.sender, false));
    scope::exclude_other_version(&mut filter, &binding.entry);
    Ok(if write_fence {
        rows.update_one(filter, doc! {"$inc":{"effect_revision":1_i64}})
            .session(session)
            .await?
            .matched_count
            == 1
    } else {
        rows.find_one(filter).session(session).await?.is_some()
    })
}

/// Validate before private/key work; callers must still take the same write
/// fence after preparation and before committing any admitted effect.
pub(crate) async fn validate_personal_origin_in_session(
    db: &Database,
    session: &mut ClientSession,
    expected: &super::platform_manager_contract::PersonalBindingAuthority,
) -> AppResult<bool> {
    let Some(row) = binding_for_authority(db, session, expected).await? else {
        return Ok(false);
    };
    check_binding_in_session(db, session, &row, expected.personal_control_revision, false).await
}

/// Read validation and final admission use the same live policy predicates.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_thread_in_session(
    db: &Database,
    session: &mut ClientSession,
    user: &str,
    binding: &str,
    generation: i64,
    conversation: &str,
    write_fence: bool,
) -> AppResult<super::platform_manager_contract::ThreadAuthorityResolution> {
    use super::platform_manager_contract::ThreadAuthorityResolution;
    let Some(row) = db.collection::<PlatformPersonalBinding>(COLLECTION_NAME)
        .find_one(doc! {"_id":binding,"user_id":user,"generation":generation,"conversation_id":conversation,"state":"active"})
        .session(&mut *session).await? else {
            return Ok(ThreadAuthorityResolution::WebOnly);
        };
    let Some(controls) = super::platform_bot_runtime_control_service::get_in_session(
        db,
        session,
        &row.platform_config_id,
    )
    .await?
    else {
        return Ok(ThreadAuthorityResolution::WebOnly);
    };
    Ok(
        if check_binding_in_session(
            db,
            session,
            &row,
            controls.personal_control_revision,
            write_fence,
        )
        .await?
        {
            ThreadAuthorityResolution::Personal(Box::new(authority(
                &row,
                controls.personal_control_revision,
            )))
        } else {
            ThreadAuthorityResolution::WebOnly
        },
    )
}

pub async fn claim_thread_in_session(
    db: &Database,
    session: &mut ClientSession,
    binding: &PlatformPersonalBinding,
    personal_revision: i64,
    agent_id: &str,
    conversation_id: &str,
) -> AppResult<(String, String)> {
    if !check_binding_in_session(db, session, binding, personal_revision, false).await? {
        return Err(conflict());
    }
    // The final claim_binding_turn fences shared manager rows after Part 2
    // finishes key preparation. This pointer/agent write is owner-specific.
    // Part 2 also checks its agent authority. This local write protects the pointer
    // from being assigned an unrelated owner's or specialist's agent.
    if db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME).find_one_and_update(doc! {"_id":agent_id,"user_id":&binding.user_id,"kind":"nyxbot","destroyed_at":bson::Bson::Null},doc! {"$inc":{"platform_thread_revision":1_i64}}).session(&mut *session).await?.is_none() { return Err(conflict()); }
    let rows = db.collection::<PlatformPersonalBinding>(COLLECTION_NAME);
    let current = rows
        .find_one(doc! {"_id":&binding.id,"generation":binding.generation})
        .session(&mut *session)
        .await?
        .ok_or_else(conflict)?;
    if let Some(conversation) = current.conversation_id {
        if current.nyxbot_agent_id.as_deref() != Some(agent_id) {
            return Err(conflict());
        }
        return Ok((agent_id.to_owned(), conversation));
    }
    if current
        .nyxbot_agent_id
        .as_deref()
        .is_some_and(|id| id != agent_id)
    {
        return Err(conflict());
    }
    if rows.update_one(doc! {"_id":&binding.id,"generation":binding.generation,"conversation_id":bson::Bson::Null},doc! {"$set":{"nyxbot_agent_id":agent_id,"conversation_id":conversation_id}}).session(session).await?.matched_count != 1 { return Err(conflict()); }
    Ok((agent_id.to_owned(), conversation_id.to_owned()))
}

pub async fn claim_turn_in_session(
    db: &Database,
    session: &mut ClientSession,
    binding: &PlatformPersonalBinding,
    active: &PersonalActiveTurn,
) -> AppResult<()> {
    let origin = &active.origin;
    if origin.binding_id != binding.id
        || origin.binding_generation != binding.generation
        || origin.user_id != binding.user_id
        || origin.platform_config_id != binding.platform_config_id
        || origin.manager_bot_id != binding.manager_bot_id
        || origin.manager_authority_epoch != binding.manager_authority_epoch
        || origin.entry != binding.entry
        || origin.sender != binding.sender
        || !fence_binding_in_session(db, session, binding, origin.personal_control_revision).await?
    {
        return Err(conflict());
    }
    let saved = db.collection::<PlatformPersonalBinding>(COLLECTION_NAME).update_one(doc! {"_id":&binding.id,"generation":binding.generation,"conversation_id":&active.conversation_id,"nyxbot_agent_id":&origin.nyxbot_agent_id,"active_turn":bson::Bson::Null},doc! {"$set":{"active_turn":bson::to_bson(active).map_err(|_|conflict())?}}).session(session).await?;
    if saved.matched_count != 1 {
        return Err(conflict());
    }
    Ok(())
}

/// Clear after revocation/Stop is valid. No live-control check belongs here.
pub async fn clear_turn_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    binding_id: &str,
    generation: i64,
    conversation: &str,
    turn: &str,
) -> AppResult<bool> {
    Ok(db.collection::<PlatformPersonalBinding>(COLLECTION_NAME).update_one(doc! {"_id":binding_id,"user_id":owner,"generation":generation,"active_turn.conversation_id":conversation,"active_turn.turn_id":turn},doc! {"$set":{"active_turn":bson::Bson::Null}}).session(session).await?.matched_count == 1)
}
pub async fn clear_thread_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    binding_id: &str,
    generation: i64,
    conversation: &str,
) -> AppResult<bool> {
    Ok(db.collection::<PlatformPersonalBinding>(COLLECTION_NAME).update_one(doc! {"_id":binding_id,"user_id":owner,"generation":generation,"conversation_id":conversation},doc! {"$set":{"conversation_id":bson::Bson::Null,"nyxbot_agent_id":bson::Bson::Null,"active_turn":bson::Bson::Null}}).session(session).await?.matched_count == 1)
}

pub async fn enqueue_stop_in_session(
    db: &Database,
    session: &mut ClientSession,
    effects: &CapturedPersonalStop,
) -> AppResult<String> {
    let turn = &effects.active_turn;
    let rows = db.collection::<PlatformPersonalStopEffect>(
        crate::models::platform_bot_runtime_controls::STOP_EFFECTS,
    );
    let filter = doc! {"active_turn.conversation_id":&turn.conversation_id,"active_turn.turn_id":&turn.turn_id,"active_turn.origin.binding_id":&turn.origin.binding_id,"active_turn.origin.binding_generation":turn.origin.binding_generation};
    if let Some(existing) = rows.find_one(filter).session(&mut *session).await? {
        return Ok(existing.id);
    }
    let row = PlatformPersonalStopEffect {
        id: uuid::Uuid::new_v4().to_string(),
        active_turn: turn.clone(),
        stop_requested_at: effects.stop_requested_at,
        machine_jobs: effects.machine_jobs.clone(),
        machine_scopes: effects.machine_scopes.clone(),
        progress: None,
        completed: false,
        claim_id: None,
        lease_expires_at: None,
        retry_at: Utc::now(),
        created_at: Utc::now(),
    };
    rows.insert_one(&row).session(session).await?;
    Ok(row.id)
}

pub async fn revoke_binding_in_session(
    db: &Database,
    session: &mut ClientSession,
    binding: &PlatformPersonalBinding,
    reason: PersonalRevocationReason,
    effects: &dyn PersonalLifecycleEffects,
    except_intent: Option<&str>,
) -> AppResult<bool> {
    db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":&binding.platform_config_id},
            doc! {"$inc":{"effect_revision":1_i64}},
        )
        .session(&mut *session)
        .await?;
    let Some(current) = db
        .collection::<PlatformPersonalBinding>(COLLECTION_NAME)
        .find_one(
            doc! {"_id":&binding.id,"generation":binding.generation,"user_id":&binding.user_id},
        )
        .session(&mut *session)
        .await?
    else {
        return Err(unavailable());
    };
    if current.state == PersonalBindingState::Revoked {
        if reason == PersonalRevocationReason::Unlinked {
            // Preserve the retirement audit reason. Logout changes only the
            // sender's routing state and cancels every pending proof.
            db.collection::<Document>(COLLECTION_NAME).update_one(
                doc! {"_id":&current.id,"generation":current.generation,"state":"revoked"},
                doc! {"$set":{"logged_out_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()},"$inc":{"effect_revision":1_i64}},
            ).session(&mut *session).await?;
            cancel_sender_proofs_in_session(db, session, &current, except_intent).await?;
        }
        return Ok(false);
    }
    if let Some(active) = &current.active_turn
        && let Some(captured) = effects.capture_stop(db, session, active).await?
    {
        if captured.active_turn != *active {
            return Err(AppError::Internal("Exact Stop capture mismatch".into()));
        }
        enqueue_stop_in_session(db, session, &captured).await?;
    }
    db.collection::<PlatformPersonalBinding>(COLLECTION_NAME).update_one(doc! {"_id":&current.id,"generation":current.generation,"state":"active"},doc! {"$set":{"state":"revoked","revocation_reason":bson::to_bson(&reason).map_err(|_|conflict())?,"revoked_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()},"$inc":{"effect_revision":1_i64}}).session(&mut *session).await?;
    cancel_sender_proofs_in_session(db, session, &current, except_intent).await?;
    Ok(true)
}

pub(crate) async fn cancel_sender_proofs_in_session(
    db: &Database,
    session: &mut ClientSession,
    binding: &PlatformPersonalBinding,
    except_intent: Option<&str>,
) -> AppResult<()> {
    let mut filter = doc! {"platform_config_id":&binding.platform_config_id,"manager_authority_epoch":binding.manager_authority_epoch,scope::actor_key(&binding.sender,false):scope::actor_value(&binding.sender),"state":"pending"};
    if let Some(id) = except_intent {
        filter.insert("_id", doc! {"$ne":id});
    }
    db.collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
        .update_many(filter, doc! {"$set":{"state":"cancelled"}})
        .session(&mut *session)
        .await?;
    let mut filter = doc! {"platform_config_id":&binding.platform_config_id,"manager_authority_epoch":binding.manager_authority_epoch,scope::actor_key(&binding.sender,false):scope::actor_value(&binding.sender),"state":{"$in":["review","awaiting_chat_confirmation"]}};
    if let Some(id) = except_intent {
        filter.insert("intent_id", doc! {"$ne":id});
    }
    db.collection::<Document>(crate::models::platform_link_intent::CHALLENGES)
        .update_many(filter, doc! {"$set":{"state":"cancelled"}})
        .session(session)
        .await?;
    Ok(())
}

pub async fn list_owned(
    db: &Database,
    owner: &str,
    before_id: Option<&str>,
) -> AppResult<Vec<PlatformPersonalBinding>> {
    use futures::TryStreamExt;
    let mut filter = doc! {"user_id":owner};
    if let Some(before) = before_id {
        if uuid::Uuid::parse_str(before).is_err() {
            return Err(AppError::ValidationError("Invalid binding cursor".into()));
        }
        filter.insert("_id", doc! {"$lt":before});
    }
    Ok(db
        .collection::<PlatformPersonalBinding>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id":-1})
        .limit(51)
        .await?
        .try_collect()
        .await?)
}

pub async fn revoke_owned(
    db: &Database,
    actor: &super::audit_service::AuditActor,
    audit_key: &[u8],
    binding_id: &str,
    generation: i64,
    effects: &dyn PersonalLifecycleEffects,
) -> AppResult<()> {
    let db = db.clone();
    let actor = actor.clone();
    let key = zeroize::Zeroizing::new(audit_key.to_vec());
    let id = binding_id.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (db, actor, key, id, generation, effects),
            |session, (db, actor, key, id, generation, effects)| {
                Box::pin(async move {
                    let result: AppResult<_> =
                        async {
                            if !fence_person(db, session, &actor.user_id).await? {
                                return Err(unavailable());
                            }
                            let binding = db
                    .collection::<PlatformPersonalBinding>(COLLECTION_NAME)
                    .find_one(doc! {"_id":&*id,"user_id":&actor.user_id,"generation":*generation})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(unavailable)?;
                            if revoke_binding_in_session(
                                db,
                                session,
                                &binding,
                                PersonalRevocationReason::Unlinked,
                                *effects,
                                None,
                            )
                            .await?
                            {
                                super::audit_service::log_actor_event_in_session(
                                    db,
                                    session,
                                    key,
                                    actor,
                                    "platform_personal_binding_revoked",
                                    serde_json::json!({"binding_id":id,"generation":*generation}),
                                )
                                .await?;
                            }
                            Ok(())
                        }
                        .await;
                    super::api_key_mutation_service::transaction_result(result)
                })
            },
        )
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

/// Call while the account remains inactive, including before reactivation. An
/// empty batch means old consent/proofs have been fully retired.
pub async fn retire_owner_batch_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    deleted: bool,
    effects: &dyn PersonalLifecycleEffects,
) -> AppResult<usize> {
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":owner,"is_active":true})
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Disable account before retiring personal consent".into(),
        ));
    }
    // Even when missing, account reads stay in the caller's snapshot. The
    // lifecycle entry point keeps the person inactive until all batches finish.
    let mut cursor = db
        .collection::<PlatformPersonalBinding>(COLLECTION_NAME)
        .find(doc! {"user_id":owner,"state":"active"})
        .sort(doc! {"_id":1})
        .limit(50)
        .session(&mut *session)
        .await?;
    let mut batch = Vec::new();
    while let Some(row) = cursor.next(&mut *session).await.transpose()? {
        batch.push(row);
    }
    for row in &batch {
        revoke_binding_in_session(
            db,
            session,
            row,
            if deleted {
                PersonalRevocationReason::OwnerDeleted
            } else {
                PersonalRevocationReason::OwnerDisabled
            },
            effects,
            None,
        )
        .await?;
    }
    db.collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
        .update_many(
            doc! {"claimant_user_id":owner,"state":"pending"},
            doc! {"$set":{"state":"cancelled"}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<Document>(crate::models::platform_link_intent::CHALLENGES)
        .update_many(
            doc! {"reviewed_user_id":owner,"state":{"$in":["review","awaiting_chat_confirmation"]}},
            doc! {"$set":{"state":"cancelled"}},
        )
        .session(session)
        .await?;
    Ok(batch.len())
}

pub async fn stop_manager_batch_in_session(
    db: &Database,
    session: &mut ClientSession,
    profile_id: &str,
    after_binding_id: Option<&str>,
    retire: bool,
    effects: &dyn PersonalLifecycleEffects,
) -> AppResult<Option<String>> {
    let mut filter = doc! {"platform_config_id":profile_id,"state":"active"};
    if !retire {
        let controls =
            super::platform_bot_runtime_control_service::get_in_session(db, session, profile_id)
                .await?
                .ok_or_else(conflict)?;
        filter.insert("active_turn", doc! {"$ne":bson::Bson::Null});
        if controls.personal_enabled && !controls.emergency_stopped {
            filter.insert("$or",vec![doc! {"active_turn.origin.personal_control_revision":{"$ne":controls.personal_control_revision}},doc! {"active_turn.origin.manager_authority_epoch":{"$ne":controls.manager_authority_epoch}}]);
        }
    }
    if let Some(id) = after_binding_id {
        filter.insert("_id", doc! {"$gt":id});
    }
    let mut cursor = db
        .collection::<PlatformPersonalBinding>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(50)
        .session(&mut *session)
        .await?;
    let mut batch = Vec::new();
    while let Some(row) = cursor.next(&mut *session).await.transpose()? {
        batch.push(row);
    }
    for row in &batch {
        if retire {
            revoke_binding_in_session(
                db,
                session,
                row,
                PersonalRevocationReason::ManagerRetired,
                effects,
                None,
            )
            .await?;
        } else if let Some(active) = &row.active_turn {
            if let Some(captured) = effects.capture_stop(db, session, active).await? {
                if captured.active_turn != *active {
                    return Err(conflict());
                }
                enqueue_stop_in_session(db, session, &captured).await?;
            }
            clear_turn_in_session(
                db,
                session,
                &row.user_id,
                &row.id,
                row.generation,
                &active.conversation_id,
                &active.turn_id,
            )
            .await?;
        }
    }
    if retire {
        db.collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
            .update_many(
                doc! {"platform_config_id":profile_id,"state":"pending"},
                doc! {"$set":{"state":"cancelled"}},
            )
            .session(&mut *session)
            .await?;
        db.collection::<Document>(crate::models::platform_link_intent::CHALLENGES).update_many(doc! {"platform_config_id":profile_id,"state":{"$in":["review","awaiting_chat_confirmation"]}},doc! {"$set":{"state":"cancelled"}}).session(&mut *session).await?;
    }
    Ok(batch.last().map(|b| b.id.clone()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::{
        platform_bot_config::{PlatformBotConfig, PlatformExecutionProfile},
        platform_bot_runtime_controls::PlatformBotRuntimeControls,
    };
    use crate::services::platform_manager_contract::{
        EntryIdentity, EventIdentity, ManagerChatKind, VerifiedManagerUpdate,
    };

    pub(crate) async fn fixture(name: &str) -> (Database, VerifiedManagerUpdate, String) {
        let db = crate::test_utils::connect_transaction_test_database(name).await;
        ensure_indexes(&db).await.unwrap();
        super::super::platform_link_intent_service::ensure_indexes(&db)
            .await
            .unwrap();
        super::super::platform_bot_runtime_control_service::ensure_indexes(&db)
            .await
            .unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        let profile_id = uuid::Uuid::new_v4().to_string();
        let bot_id = uuid::Uuid::new_v4().to_string();
        let observation = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let profile = PlatformBotConfig {
            identity_version: None,
            native_entry: None,
            id: profile_id.clone(),
            channel_bot_id: bot_id.clone(),
            manager_bot_id: "991100".into(),
            observation_id: observation.clone(),
            execution_profile: PlatformExecutionProfile::PlatformOnboarding,
            generation: 40,
            enabled: false,
            guide: None,
            funding_owner_user_id: uuid::Uuid::new_v4().to_string(),
            configured_by: owner.clone(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
            .insert_one(profile)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"provider":"telegram-new","fields":{"manager_bot_id":"991100","observation_id":&observation,"webhook_ready":"true"}}).await.unwrap();
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).insert_one(doc! {"_id":bot_id,"platform_profile_id":&profile_id,"platform_bot_id":"991100","is_active":true}).await.unwrap();
        db.collection::<PlatformBotRuntimeControls>(
            crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
        )
        .insert_one(PlatformBotRuntimeControls {
            id: uuid::Uuid::new_v4().to_string(),
            contract_version: 1,
            platform_config_id: profile_id.clone(),
            manager_bot_id: "991100".into(),
            entry: EntryIdentity::TelegramV1 {
                telegram_bot_id: 991100,
            },
            manager_authority_epoch: 1,
            personal_control_revision: 1,
            public_policy_revision: 1,
            personal_enabled: true,
            public_enabled: false,
            emergency_stopped: false,
            retired: false,
            public_policy: None,
            effect_revision: 0,
            cleanup_pending: false,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
        (
            db,
            VerifiedManagerUpdate {
                profile_id,
                manager_bot_id: "991100".into(),
                entry: EntryIdentity::TelegramV1 { telegram_bot_id:991100 },
                observation_id: observation,
                sender: SenderIdentity::TelegramV1 { telegram_sender_id:123,telegram_chat_id:123 },
                chat_kind: ManagerChatKind::Private,
                event: EventIdentity::TelegramV1 { update_id:1,message_id:1 },
            command_eligibility: super::super::platform_manager_contract::ManagerCommandEligibility::OriginalPrivateMessage,
            },
            owner,
        )
    }

    pub(crate) fn binding(update: &VerifiedManagerUpdate, owner: &str) -> PlatformPersonalBinding {
        let now = Utc::now();
        PlatformPersonalBinding {
            id: uuid::Uuid::new_v4().to_string(),
            platform_config_id: update.profile_id.clone(),
            manager_bot_id: update.manager_bot_id.clone(),
            entry: update.entry.clone(),
            manager_authority_epoch: 1,
            sender: update.sender.clone(),
            user_id: owner.into(),
            generation: 1,
            state: PersonalBindingState::Active,
            revocation_reason: None,
            consent_version: 1,
            review_digest: "test-review".into(),
            reviewed_access_mode: "nyxmode".into(),
            nyxbot_agent_id: None,
            conversation_id: None,
            active_turn: None,
            effect_revision: 0,
            created_at: now,
            consented_at: now,
            updated_at: now,
            revoked_at: None,
            logged_out_at: None,
        }
    }

    #[tokio::test]
    async fn disabled_owner_never_resolves_unlinked_and_guide_pause_is_independent() {
        let (db, update, owner) = fixture("personal_resolve_closed").await;
        let binding = binding(&update, &owner);
        db.collection::<PlatformPersonalBinding>(COLLECTION_NAME)
            .insert_one(&binding)
            .await
            .unwrap();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(matches!(
            resolve_sender_in_session(&db, &mut session, &update)
                .await
                .unwrap(),
            ResolvedPersonalSender::Personal { .. }
        ));
        session.commit_transaction().await.unwrap();
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id":&owner}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        session.start_transaction().await.unwrap();
        assert!(matches!(
            resolve_sender_in_session(&db, &mut session, &update)
                .await
                .unwrap(),
            ResolvedPersonalSender::Unavailable(
                super::super::platform_manager_contract::UnavailableReason::OwnerUnavailable
            )
        ));
        session.commit_transaction().await.unwrap();
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&binding.id},
                doc! {"$set":{"state":"revoked","revocation_reason":"owner_disabled"}},
            )
            .await
            .unwrap();
        session.start_transaction().await.unwrap();
        assert!(matches!(
            resolve_sender_in_session(&db, &mut session, &update)
                .await
                .unwrap(),
            ResolvedPersonalSender::Unavailable(_)
        ));
        session.commit_transaction().await.unwrap();
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&binding.id},
                doc! {"$set":{"revocation_reason":"unlinked"}},
            )
            .await
            .unwrap();
        session.start_transaction().await.unwrap();
        assert!(matches!(
            resolve_sender_in_session(&db, &mut session, &update)
                .await
                .unwrap(),
            ResolvedPersonalSender::Unlinked
        ));
        session.commit_transaction().await.unwrap();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn revoked_thread_clear_is_exact_and_does_not_need_live_controls() {
        let (db, update, owner) = fixture("personal_clear_revoked").await;
        let mut binding = binding(&update, &owner);
        binding.state = PersonalBindingState::Revoked;
        binding.revocation_reason = Some(PersonalRevocationReason::Unlinked);
        binding.conversation_id = Some("new-thread".into());
        db.collection::<PlatformPersonalBinding>(COLLECTION_NAME)
            .insert_one(&binding)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .delete_many(doc! {})
            .await
            .unwrap();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            !clear_thread_in_session(
                &db,
                &mut session,
                &owner,
                &binding.id,
                binding.generation,
                "old-thread"
            )
            .await
            .unwrap()
        );
        assert!(
            !clear_thread_in_session(
                &db,
                &mut session,
                &owner,
                &binding.id,
                binding.generation + 1,
                "new-thread"
            )
            .await
            .unwrap()
        );
        assert!(
            clear_thread_in_session(
                &db,
                &mut session,
                &owner,
                &binding.id,
                binding.generation,
                "new-thread"
            )
            .await
            .unwrap()
        );
        session.commit_transaction().await.unwrap();
        db.drop().await.unwrap();
    }
}

/// Stateless implementation of the shared identity boundary.
pub struct IdentityProducer;

pub(crate) fn authority(
    row: &PlatformPersonalBinding,
    revision: i64,
) -> super::platform_manager_contract::PersonalBindingAuthority {
    use super::platform_manager_contract::{ManagerAuthority, PersonalBindingAuthority};
    PersonalBindingAuthority {
        manager: ManagerAuthority {
            profile_id: row.platform_config_id.clone(),
            manager_bot_id: row.manager_bot_id.clone(),
            entry: row.entry.clone(),
            manager_authority_epoch: row.manager_authority_epoch,
        },
        sender: row.sender.clone(),
        binding_id: row.id.clone(),
        binding_generation: row.generation,
        personal_control_revision: revision,
        user_id: row.user_id.clone(),
    }
}

pub(crate) fn stored_origin(
    origin: &super::platform_manager_contract::PlatformTurnOrigin,
) -> PersonalTurnProof {
    use super::platform_manager_contract::PlatformOriginSource;
    let a = &origin.authority;
    PersonalTurnProof {
        platform_config_id: a.manager.profile_id.clone(),
        manager_bot_id: a.manager.manager_bot_id.clone(),
        entry: a.manager.entry.clone(),
        manager_authority_epoch: a.manager.manager_authority_epoch,
        sender: a.sender.clone(),
        binding_id: a.binding_id.clone(),
        binding_generation: a.binding_generation,
        personal_control_revision: a.personal_control_revision,
        user_id: a.user_id.clone(),
        nyxbot_agent_id: origin.nyxbot_agent_id.clone(),
        source: match &origin.source {
            PlatformOriginSource::Ingress(claim) => PersonalOriginSource::Ingress {
                receipt_id: claim.receipt_id.clone(),
                claim_id: claim.claim_id.clone(),
            },
            PlatformOriginSource::ThreadEvent { admission_id } => {
                PersonalOriginSource::ThreadEvent {
                    admission_id: admission_id.clone(),
                }
            }
        },
    }
}

pub(crate) fn service_origin(
    proof: &PersonalTurnProof,
) -> super::platform_manager_contract::PlatformTurnOrigin {
    use super::platform_manager_contract::*;
    PlatformTurnOrigin {
        authority: PersonalBindingAuthority {
            manager: ManagerAuthority {
                profile_id: proof.platform_config_id.clone(),
                manager_bot_id: proof.manager_bot_id.clone(),
                entry: proof.entry.clone(),
                manager_authority_epoch: proof.manager_authority_epoch,
            },
            sender: proof.sender.clone(),
            binding_id: proof.binding_id.clone(),
            binding_generation: proof.binding_generation,
            personal_control_revision: proof.personal_control_revision,
            user_id: proof.user_id.clone(),
        },
        nyxbot_agent_id: proof.nyxbot_agent_id.clone(),
        source: match &proof.source {
            PersonalOriginSource::Ingress {
                receipt_id,
                claim_id,
            } => PlatformOriginSource::Ingress(IngressClaim {
                receipt_id: receipt_id.clone(),
                claim_id: claim_id.clone(),
            }),
            PersonalOriginSource::ThreadEvent { admission_id } => {
                PlatformOriginSource::ThreadEvent {
                    admission_id: admission_id.clone(),
                }
            }
        },
    }
}

async fn binding_for_authority(
    db: &Database,
    session: &mut ClientSession,
    expected: &super::platform_manager_contract::PersonalBindingAuthority,
) -> AppResult<Option<PlatformPersonalBinding>> {
    let row=db.collection::<PlatformPersonalBinding>(COLLECTION_NAME).find_one(doc! {"_id":&expected.binding_id,"generation":expected.binding_generation,"user_id":&expected.user_id}).session(session).await?;
    Ok(row.filter(|row| authority(row, expected.personal_control_revision) == *expected))
}

#[async_trait]
impl super::platform_manager_contract::ManagerIdentityProducer for IdentityProducer {
    async fn runtime_policy(
        &self,
        db: &Database,
        profile_id: &str,
    ) -> AppResult<Option<super::platform_manager_contract::ManagerRuntimePolicy>> {
        Ok(
            super::platform_bot_runtime_control_service::get(db, profile_id)
                .await?
                .filter(|r| {
                    matches!(r.contract_version, 1 | 2)
                        && r.contract_version == scope::version(&r.entry)
                })
                .as_ref()
                .map(super::platform_bot_runtime_control_service::policy),
        )
    }
    async fn resolve_sender(
        &self,
        db: &Database,
        session: &mut ClientSession,
        update: &super::platform_manager_contract::VerifiedManagerUpdate,
    ) -> AppResult<super::platform_manager_contract::SenderResolution> {
        use super::platform_manager_contract::SenderResolution;
        Ok(
            match resolve_sender_in_session(db, session, update).await? {
                ResolvedPersonalSender::Unlinked => SenderResolution::Unlinked,
                ResolvedPersonalSender::Unavailable(reason) => {
                    SenderResolution::Unavailable(reason)
                }
                ResolvedPersonalSender::Personal { binding, controls } => {
                    SenderResolution::Personal {
                        authority: Box::new(authority(
                            &binding,
                            controls.personal_control_revision,
                        )),
                        nyxbot_agent_id: binding.nyxbot_agent_id,
                        conversation_id: binding.conversation_id,
                    }
                }
            },
        )
    }
    async fn fence_personal_origin(
        &self,
        db: &Database,
        session: &mut ClientSession,
        expected: &super::platform_manager_contract::PersonalBindingAuthority,
    ) -> AppResult<super::platform_manager_contract::FenceOutcome> {
        use super::platform_manager_contract::{FenceOutcome, UnavailableReason};
        let Some(row) = binding_for_authority(db, session, expected).await? else {
            return Ok(FenceOutcome::Unavailable(UnavailableReason::OriginRevoked));
        };
        Ok(
            if fence_binding_in_session(db, session, &row, expected.personal_control_revision)
                .await?
            {
                FenceOutcome::Authorized
            } else {
                FenceOutcome::Unavailable(UnavailableReason::OriginRevoked)
            },
        )
    }
    async fn claim_binding_thread(
        &self,
        db: &Database,
        session: &mut ClientSession,
        expected: &super::platform_manager_contract::PersonalBindingAuthority,
        agent: &str,
        candidate: &str,
    ) -> AppResult<super::platform_manager_contract::PersonalThreadClaim> {
        let row = binding_for_authority(db, session, expected)
            .await?
            .ok_or_else(conflict)?;
        let (nyxbot_agent_id, conversation_id) = claim_thread_in_session(
            db,
            session,
            &row,
            expected.personal_control_revision,
            agent,
            candidate,
        )
        .await?;
        Ok(super::platform_manager_contract::PersonalThreadClaim {
            nyxbot_agent_id,
            conversation_id,
        })
    }
    async fn claim_binding_turn(
        &self,
        db: &Database,
        session: &mut ClientSession,
        origin: &super::platform_manager_contract::PlatformTurnOrigin,
        turn: &super::platform_manager_contract::TurnRef,
    ) -> AppResult<()> {
        let row = binding_for_authority(db, session, &origin.authority)
            .await?
            .ok_or_else(conflict)?;
        claim_turn_in_session(
            db,
            session,
            &row,
            &PersonalActiveTurn {
                conversation_id: turn.conversation_id.clone(),
                turn_id: turn.turn_id.clone(),
                origin: stored_origin(origin),
            },
        )
        .await
    }
    async fn clear_binding_turn(
        &self,
        db: &Database,
        session: &mut ClientSession,
        user: &str,
        binding: &str,
        generation: i64,
        turn: &super::platform_manager_contract::TurnRef,
    ) -> AppResult<bool> {
        clear_turn_in_session(
            db,
            session,
            user,
            binding,
            generation,
            &turn.conversation_id,
            &turn.turn_id,
        )
        .await
    }
    async fn clear_binding_thread_reference(
        &self,
        db: &Database,
        session: &mut ClientSession,
        user: &str,
        binding: &str,
        generation: i64,
        conversation: &str,
    ) -> AppResult<bool> {
        clear_thread_in_session(db, session, user, binding, generation, conversation).await
    }
    async fn resolve_thread_authority(
        &self,
        db: &Database,
        session: &mut ClientSession,
        user: &str,
        binding: &str,
        generation: i64,
        conversation: &str,
    ) -> AppResult<super::platform_manager_contract::ThreadAuthorityResolution> {
        resolve_thread_in_session(db, session, user, binding, generation, conversation, true).await
    }
}

/// Construct only with the real personal and public producers. No successful
/// no-op implementation exists for public retirement or exact Stop capture.
pub struct LifecycleProducer {
    pub personal: std::sync::Arc<dyn super::platform_manager_contract::PersonalTurnProducer>,
    pub public: std::sync::Arc<dyn super::platform_manager_contract::PublicTurnProducer>,
}

#[async_trait]
impl PersonalLifecycleEffects for LifecycleProducer {
    async fn capture_stop(
        &self,
        db: &Database,
        session: &mut ClientSession,
        active: &PersonalActiveTurn,
    ) -> AppResult<Option<CapturedPersonalStop>> {
        use super::platform_manager_contract::{ExactStopOutcome, TurnRef};
        let origin = service_origin(&active.origin);
        let turn = TurnRef {
            conversation_id: active.conversation_id.clone(),
            turn_id: active.turn_id.clone(),
        };
        match self
            .personal
            .stop_exact_platform_turn(db, session, &origin, &turn)
            .await?
        {
            ExactStopOutcome::NoMatchingActiveTurn => Ok(None),
            ExactStopOutcome::Captured(capture) => {
                if capture.origin != origin || capture.turn != turn {
                    return Err(conflict());
                }
                Ok(Some(CapturedPersonalStop {
                    active_turn: active.clone(),
                    stop_requested_at: capture.stop_requested_at,
                    machine_scopes: capture
                        .machine_scopes
                        .into_iter()
                        .map(|scope| ExactMachineScope {
                            node_id: scope.node_id,
                            runtime_id: scope.runtime_id,
                        })
                        .collect(),
                    machine_jobs: capture
                        .machine_jobs
                        .into_iter()
                        .map(|j| ExactMachineJob {
                            node_id: j.node_id,
                            runtime_id: j.runtime_id,
                            job_id: j.job_id,
                        })
                        .collect(),
                }))
            }
        }
    }
    async fn revoke_public_sender(
        &self,
        db: &Database,
        session: &mut ClientSession,
        binding: &PlatformPersonalBinding,
    ) -> AppResult<()> {
        self.public
            .revoke_public_sender(db, session, &authority(binding, 0).manager, &binding.sender)
            .await
    }
}

#[async_trait]
impl super::platform_manager_contract::ManagerLifecycleProducer for LifecycleProducer {
    async fn unlink_sender_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
        command: &super::platform_manager_contract::OriginalManagerCommand,
    ) -> AppResult<super::platform_manager_contract::PersonalRevocationResult> {
        use super::platform_manager_contract::{
            ManagerChatKind, ManagerCommandEligibility, PublicRevocationReason,
            PublicRevocationScope,
        };
        let update = command.update();
        if update.chat_kind != ManagerChatKind::Private
            || scope::validate(&update.entry, &update.manager_bot_id, Some(&update.sender)).is_err()
            || update.command_eligibility != ManagerCommandEligibility::OriginalPrivateMessage
        {
            return Err(unavailable());
        }
        if command
            .text()
            .split_whitespace()
            .next()
            .is_none_or(|word| !matches!(word.split('@').next(), Some("/unlink" | "/logout")))
        {
            return Err(unavailable());
        }
        // Unlink remains available while either lane is paused or Stop is set.
        let controls = super::platform_bot_runtime_control_service::fence_manager_in_session(
            db,
            session,
            &update.profile_id,
            &update.manager_bot_id,
            &update.entry,
            Some(&update.observation_id),
        )
        .await?
        .ok_or_else(unavailable)?;
        let mut revoked = false;
        if let Some(row) = latest_for_sender(
            db,
            session,
            &update.profile_id,
            controls.manager_authority_epoch,
            &update.sender,
        )
        .await?
        {
            if row.entry != update.entry || row.sender != update.sender {
                return Err(unavailable());
            }
            revoked = revoke_binding_in_session(
                db,
                session,
                &row,
                PersonalRevocationReason::Unlinked,
                self,
                None,
            )
            .await?;
        }
        db.collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME).update_many(doc! {"platform_config_id":&update.profile_id,"manager_authority_epoch":controls.manager_authority_epoch,scope::actor_key(&update.sender,false):scope::actor_value(&update.sender),scope::destination_key(&update.sender,false):scope::destination_value(&update.sender),"state":"pending"},doc! {"$set":{"state":"cancelled"}}).session(&mut *session).await?;
        db.collection::<Document>(crate::models::platform_link_intent::CHALLENGES).update_many(doc! {"platform_config_id":&update.profile_id,"manager_authority_epoch":controls.manager_authority_epoch,scope::actor_key(&update.sender,false):scope::actor_value(&update.sender),scope::destination_key(&update.sender,false):scope::destination_value(&update.sender),"state":{"$in":["review","awaiting_chat_confirmation"]}},doc! {"$set":{"state":"cancelled"}}).session(&mut *session).await?;
        // Native logout never touches the separate legacy Telegram onboarding namespace.
        if let SenderIdentity::TelegramV1 {
            telegram_sender_id,
            telegram_chat_id,
        } = &update.sender
        {
            super::platform_binding_service::unlink_sender_in_session(
                db,
                session,
                &update.profile_id,
                &update.manager_bot_id,
                &telegram_sender_id.to_string(),
                &telegram_chat_id.to_string(),
            )
            .await?;
        }
        self.public
            .revoke_public_sessions_in_session(
                db,
                session,
                &PublicRevocationScope::Sender {
                    manager: super::platform_bot_runtime_control_service::policy(&controls).manager,
                    sender: update.sender.clone(),
                },
                PublicRevocationReason::SenderUnlinked,
            )
            .await?;
        let mut stop_filter =
            doc! {"active_turn.origin.platform_config_id":&update.profile_id,"completed":false};
        stop_filter.extend(scope::prefix(
            scope::sender_key_fields(&update.sender, false),
            "active_turn.origin.",
        ));
        let mut cursor = db
            .collection::<PlatformPersonalStopEffect>(
                crate::models::platform_bot_runtime_controls::STOP_EFFECTS,
            )
            .find(stop_filter)
            .limit(50)
            .session(&mut *session)
            .await?;
        let mut ids = Vec::new();
        while let Some(row) = cursor.next(&mut *session).await.transpose()? {
            ids.push(row.id);
        }
        Ok(super::platform_manager_contract::PersonalRevocationResult {
            revoked,
            stop_effect_record_ids: ids,
        })
    }
    async fn recover_personal_lifecycle(
        &self,
        db: &Database,
        personal: &dyn super::platform_manager_contract::PersonalTurnProducer,
        limit: u32,
    ) -> AppResult<super::platform_manager_contract::RecoveryBatch> {
        use super::platform_manager_contract::*;
        use crate::models::platform_bot_runtime_controls::{
            COLLECTION_NAME as CONTROLS, STOP_EFFECTS,
        };
        let limit = limit.clamp(1, 100);
        // Cleanup is local and bounded. A conditional controls write serializes
        // against resume; scans stop only origins from earlier personal revisions.
        for _ in 0..limit.min(8) {
            let Some(pending) = db
                .collection::<PlatformBotRuntimeControls>(CONTROLS)
                .find_one(doc! {"cleanup_pending":true})
                .sort(doc! {"_id":1})
                .await?
            else {
                break;
            };
            let db = db.clone();
            let mut session = db.client().start_session().await?;
            session
                .start_transaction()
                .and_run((db, pending, self), |session, (db, pending, lifecycle)| {
                    Box::pin(async move {
                        let result: AppResult<_> = async {
                            let Some(current) = db
                                .collection::<PlatformBotRuntimeControls>(CONTROLS)
                                .find_one_and_update(
                                    doc! {"_id":&pending.id,"cleanup_pending":true},
                                    doc! {"$inc":{"effect_revision":1_i64}},
                                )
                                .session(&mut *session)
                                .await?
                            else {
                                return Ok(());
                            };
                            let next = stop_manager_batch_in_session(
                                db,
                                session,
                                &current.platform_config_id,
                                None,
                                current.retired,
                                *lifecycle,
                            )
                            .await?;
                            if !current.public_enabled
                                || current.retired
                                || current.emergency_stopped
                            {
                                let mut manager =
                                    super::platform_bot_runtime_control_service::policy(&current)
                                        .manager;
                                if current.retired {
                                    manager.manager_authority_epoch = manager
                                        .manager_authority_epoch
                                        .checked_sub(1)
                                        .ok_or_else(conflict)?;
                                }
                                lifecycle
                                    .public
                                    .revoke_public_sessions_in_session(
                                        db,
                                        session,
                                        &PublicRevocationScope::Manager { manager },
                                        PublicRevocationReason::ManagerRetired,
                                    )
                                    .await?;
                            }
                            if next.is_none() {
                                db.collection::<PlatformBotRuntimeControls>(CONTROLS)
                                    .update_one(
                                        doc! {"_id":&current.id},
                                        doc! {"$set":{"cleanup_pending":false}},
                                    )
                                    .session(&mut *session)
                                    .await?;
                            }
                            Ok(())
                        }
                        .await;
                        super::api_key_mutation_service::transaction_result(result)
                    })
                })
                .await
                .map_err(super::api_key_mutation_service::map_transaction_error)?;
        }
        let mut processed = 0;
        for _ in 0..limit {
            let Some(row) =
                super::platform_bot_runtime_control_service::claim_stop_effect(db).await?
            else {
                break;
            };
            let capture = CapturedPlatformStop {
                origin: service_origin(&row.active_turn.origin),
                turn: TurnRef {
                    conversation_id: row.active_turn.conversation_id.clone(),
                    turn_id: row.active_turn.turn_id.clone(),
                },
                stop_requested_at: row.stop_requested_at,
                machine_scopes: row
                    .machine_scopes
                    .iter()
                    .map(
                        |scope| super::platform_manager_contract::ExactMachineScope {
                            node_id: scope.node_id.clone(),
                            runtime_id: scope.runtime_id.clone(),
                        },
                    )
                    .collect(),
                machine_jobs: row
                    .machine_jobs
                    .iter()
                    .map(|j| super::platform_manager_contract::ExactMachineJob {
                        node_id: j.node_id.clone(),
                        runtime_id: j.runtime_id.clone(),
                        job_id: j.job_id.clone(),
                    })
                    .collect(),
            };
            let outcome = personal.apply_exact_platform_stop(&capture).await;
            let complete = outcome
                .as_ref()
                .is_ok_and(|progress| stop_effect_complete(&capture, progress));
            if let Ok(progress) = &outcome {
                super::platform_bot_runtime_control_service::record_stop_progress(
                    db,
                    &row.id,
                    row.claim_id.as_deref().ok_or_else(conflict)?,
                    progress,
                )
                .await?;
            }
            super::platform_bot_runtime_control_service::finish_stop_effect(
                db,
                &row.id,
                row.claim_id.as_deref().ok_or_else(conflict)?,
                complete,
            )
            .await?;
            processed += 1;
            // Provider failure is left durably retryable, and stops this bounded
            // pass so a broken node/provider does not cause a tight retry loop.
            outcome?;
        }
        let has_more = db
            .collection::<Document>(STOP_EFFECTS)
            .find_one(doc! {"completed":false})
            .await?
            .is_some()
            || db
                .collection::<Document>(CONTROLS)
                .find_one(doc! {"cleanup_pending":true})
                .await?
                .is_some();
        Ok(RecoveryBatch {
            processed,
            has_more,
        })
    }
}

fn stop_effect_complete(
    capture: &super::platform_manager_contract::CapturedPlatformStop,
    progress: &super::platform_manager_contract::ExactStopProgress,
) -> bool {
    use super::platform_manager_contract::{ExactMachineCancelOutcome, ExactStopSettlement};
    let terminal = |outcome| {
        matches!(
            outcome,
            ExactMachineCancelOutcome::Cancelled
                | ExactMachineCancelOutcome::AlreadyFinished
                | ExactMachineCancelOutcome::RuntimeChanged
        )
    };
    progress.settlement != ExactStopSettlement::Pending
        && capture.machine_jobs.iter().all(|j| {
            progress.machine_jobs.iter().any(|done| {
                done.job.node_id == j.node_id
                    && done.job.runtime_id == j.runtime_id
                    && done.job.job_id == j.job_id
                    && terminal(done.outcome)
            })
        })
        && capture.machine_scopes.iter().all(|scope| {
            progress.machine_scopes.iter().any(|done| {
                done.scope.node_id == scope.node_id
                    && done.scope.runtime_id == scope.runtime_id
                    && terminal(done.outcome)
            })
        })
}

pub fn producer(
    _state: crate::AppState,
) -> std::sync::Arc<dyn super::platform_manager_contract::ManagerIdentityProducer> {
    std::sync::Arc::new(IdentityProducer)
}

#[cfg(test)]
mod stop_completion_tests {
    use super::*;
    use crate::services::platform_manager_contract::*;
    #[test]
    fn settlement_alone_cannot_complete_non_job_cancellation() {
        let at = Utc::now();
        let capture = CapturedPlatformStop {
            origin: PlatformTurnOrigin {
                authority: PersonalBindingAuthority {
                    manager: ManagerAuthority {
                        profile_id: "profile".into(),
                        manager_bot_id: "991100".into(),
                        entry: EntryIdentity::TelegramV1 {
                            telegram_bot_id: 991100,
                        },
                        manager_authority_epoch: 1,
                    },
                    sender: SenderIdentity::TelegramV1 {
                        telegram_sender_id: 123,
                        telegram_chat_id: 123,
                    },
                    binding_id: "binding".into(),
                    binding_generation: 1,
                    personal_control_revision: 1,
                    user_id: "owner".into(),
                },
                nyxbot_agent_id: "agent".into(),
                source: PlatformOriginSource::ThreadEvent {
                    admission_id: "admission".into(),
                },
            },
            turn: TurnRef {
                conversation_id: "conversation".into(),
                turn_id: "turn".into(),
            },
            stop_requested_at: at,
            machine_jobs: vec![],
            machine_scopes: vec![
                crate::services::platform_manager_contract::ExactMachineScope {
                    node_id: "node".into(),
                    runtime_id: "captured-runtime".into(),
                },
            ],
        };
        let mut progress = ExactStopProgress {
            settlement: ExactStopSettlement::Settled,
            machine_jobs: vec![],
            machine_scopes: vec![],
        };
        assert!(!stop_effect_complete(&capture, &progress));
        progress.machine_scopes.push(ExactMachineScopeProgress {
            scope: crate::services::platform_manager_contract::ExactMachineScope {
                node_id: "node".into(),
                runtime_id: "new-runtime".into(),
            },
            outcome: ExactMachineCancelOutcome::Cancelled,
        });
        assert!(!stop_effect_complete(&capture, &progress));
        progress.machine_scopes[0].scope.runtime_id = "captured-runtime".into();
        progress.machine_scopes[0].outcome = ExactMachineCancelOutcome::NodeUnavailable;
        assert!(!stop_effect_complete(&capture, &progress));
        progress.machine_scopes[0].outcome = ExactMachineCancelOutcome::Cancelled;
        assert!(stop_effect_complete(&capture, &progress));
        progress.settlement = ExactStopSettlement::Pending;
        assert!(!stop_effect_complete(&capture, &progress));
    }
}

pub struct ManagerDisplay {
    pub username: String,
}

pub(crate) async fn native_manager_display(
    db: &Database,
    profile_id: &str,
    manager_bot_id: &str,
    entry: &crate::models::platform_entry_identity::NativeEntryIdentity,
) -> AppResult<Option<(Option<String>, String)>> {
    let identity = crate::models::platform_entry_projection::StoredEntryIdentity::NativeV2 {
        platform_entry: entry.clone(),
    };
    super::platform_entry_scope::validate(&identity, manager_bot_id, None)?;
    if db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
        .find_one(doc! {"_id":profile_id,"identity_version":2,"channel_bot_id":&entry.channel_bot_id,
            "manager_bot_id":manager_bot_id,"native_entry.identity":bson::to_bson(entry).map_err(|_|unavailable())?,"deleted_at":bson::Bson::Null})
        .projection(doc! {"_id":1}).await?.is_none() {return Ok(None);}
    let Some(bot)=db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&entry.channel_bot_id,"platform_profile_id":profile_id,"platform_entry_version":2,
            "platform_bot_id":manager_bot_id,"platform":super::platform_entry_native::platform_name(entry.platform),"is_active":true,"status":"active"})
        .projection(doc! {"_id":0,"platform_bot_username":1,"label":1}).await? else {return Ok(None);};
    let bounded = |value: &str| {
        !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    };
    let username = bot
        .get_str("platform_bot_username")
        .ok()
        .filter(|value| bounded(value))
        .map(str::to_owned);
    let display_name = bot
        .get_str("label")
        .ok()
        .filter(|value| bounded(value))
        .map(str::to_owned)
        .or_else(|| username.clone())
        .unwrap_or_else(|| {
            format!(
                "{} bot {}",
                super::platform_chat_presentation::ChatPresentation::for_entry(&identity)
                    .expect("checked identity")
                    .name(),
                manager_bot_id
            )
        });
    Ok(Some((username, display_name)))
}

/// Safe current manager metadata only. The projections never load credentials
/// or bot secrets, and both metadata sources must match the profile identity and
/// the current verified credential observation.
pub async fn manager_display(
    db: &Database,
    profile_id: &str,
    manager_bot_id: &str,
) -> AppResult<Option<ManagerDisplay>> {
    let Some(profile) = db
        .collection::<crate::models::platform_bot_config::PlatformBotConfig>(
            crate::models::platform_bot_config::COLLECTION_NAME,
        )
        .find_one(
            doc! {"_id":profile_id,"manager_bot_id":manager_bot_id,"native_entry":bson::Bson::Null,"deleted_at":bson::Bson::Null},
        )
        .await?
    else {
        return Ok(None);
    };
    let Some(credential)=db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).find_one(doc! {"provider":"telegram-new","fields.manager_bot_id":manager_bot_id,"fields.observation_id":&profile.observation_id,"fields.webhook_ready":"true"}).projection(doc! {"_id":0,"fields.manager_username":1}).await? else {return Ok(None);};
    let Some(bot)=db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).find_one(doc! {"_id":&profile.channel_bot_id,"platform_profile_id":profile_id,"platform_bot_id":manager_bot_id,"is_active":true}).projection(doc! {"_id":0,"platform_bot_username":1}).await? else {return Ok(None);};
    let valid = |value: &str| {
        (5..=32).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    };
    let username = credential
        .get_document("fields")
        .ok()
        .and_then(|fields| fields.get_str("manager_username").ok())
        .filter(|value| valid(value))
        .or_else(|| {
            bot.get_str("platform_bot_username")
                .ok()
                .filter(|value| valid(value))
        });
    Ok(username.map(|username| ManagerDisplay {
        username: username.to_owned(),
    }))
}
