//! Public keys and disposable encrypted transcripts, separate from owner state.
use super::{
    api_key_mutation_service as tx, assistant_agent_credential_service as crypto,
    platform_manager_contract::*, platform_public_budget_service as budget,
    platform_public_policy as policy,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as KEYS},
        platform_manager_ingress::{COLLECTION_NAME as RECEIPTS, PlatformManagerIngress},
        platform_public_session::{
            COLLECTION_NAME as SESSIONS, PlatformPublicSession, PublicTurnBinding,
        },
        platform_public_turn::{COLLECTION_NAME as TURNS, PlatformPublicTurn},
    },
};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use rand::{RngCore, rngs::OsRng};
use std::sync::Arc;
use zeroize::Zeroizing;

pub fn producer(state: AppState) -> Arc<dyn PublicNyxAgentRuntime> {
    Arc::new(PublicRuntime { state })
}
struct PublicRuntime {
    state: AppState,
}
fn denied() -> AppError {
    AppError::Conflict("Public session authority changed".into())
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<PlatformPublicTurn>(TURNS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"proof_record_id":1})
                .build(),
        )
        .await?;
    db.collection::<PlatformPublicTurn>(TURNS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<PlatformPublicTurn>(TURNS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"public_session_id":1,"reply_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

fn new_key(
    owner: &str,
    expires_at: chrono::DateTime<Utc>,
    predecessor: Option<String>,
) -> (ApiKey, RuntimeCredential) {
    let mut entropy = Zeroizing::new([0_u8; 32]);
    OsRng.fill_bytes(entropy.as_mut());
    let raw = Zeroizing::new(format!(
        "{}{}",
        crate::crypto::token::AGENT_KEY_PREFIX,
        hex::encode(entropy.as_ref())
    ));
    let now = Utc::now();
    let id = uuid::Uuid::new_v4().to_string();
    let key = ApiKey {
        id: id.clone(),
        user_id: owner.into(),
        name: "Public NyxBot session".into(),
        key_prefix: crate::crypto::token::agent_key_display_prefix(&raw)
            .expect("generated prefix")
            .into(),
        key_hash: crate::crypto::token::hash_token(&raw),
        scopes: "proxy".into(),
        last_used_at: None,
        expires_at: Some(expires_at),
        is_active: true,
        created_at: now,
        rotation_predecessor_id: predecessor,
        machine_external_grants_present: false,
        state_version: 1,
        updated_at: Some(now),
        description: None,
        allowed_service_ids: vec![],
        allowed_platform_service_ids: vec![],
        assistant_agent_owner_id: None,
        assistant_group_id: None,
        assistant_operation_scopes: Default::default(),
        allowed_node_ids: vec![],
        allow_all_services: false,
        allow_auto_connected_services: false,
        allow_all_nodes: false,
        rate_limit_per_second: Some(2),
        rate_limit_burst: Some(4),
        platform: Some("nyxbot-public".into()),
        callback_url: None,
        purpose: ApiKeyPurpose::PublicNyxbot,
        scheduled_write_enabled: false,
    };
    (
        key,
        RuntimeCredential {
            api_key_id: id,
            raw_key: raw,
        },
    )
}

/// Issuance and claim commit together. KMS work precedes shared write fences.
pub(crate) async fn issue_context(
    state: &AppState,
    store: Arc<dyn ManagerIngressTurnStore>,
    admitted: &PublicIngress,
) -> AppResult<PublicExecutionContext> {
    let (_, evidence) = policy::verified_readiness(
        &state.db,
        &admitted.manager,
        admitted.public_policy_revision,
        None,
        None,
    )
    .await?;
    if evidence.manager != admitted.manager
        || evidence.public_policy_revision != admitted.public_policy_revision
        || admitted.sender_id <= 0
        || admitted.sender_id != admitted.chat_id
    {
        return Err(denied());
    }
    let hello = state.db.run_command(doc! {"hello":1}).await?;
    let now = hello
        .get_datetime("localTime")
        .map_err(|_| denied())?
        .to_chrono();
    let manager = &admitted.manager;
    let existing = state
        .db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find_one(doc! {
            "profile_id":&manager.profile_id,"manager_bot_id":&manager.manager_bot_id,
            "telegram_bot_id":manager.telegram_bot_id,"sender_id":admitted.sender_id,
            "chat_id":admitted.chat_id,"revoked_at":bson::Bson::Null,
        })
        .await?;
    let (row, key) = if let Some(row) = existing {
        // Expired, busy and changed-policy sessions must be retired by the
        // durable lifecycle. Admission never silently creates a new identity.
        if row.active_turn.is_some()
            || row.expires_at <= now
            || row.manager_authority_epoch != manager.manager_authority_epoch
            || row.public_policy_revision != admitted.public_policy_revision
            || row.sponsor_owner_user_id != evidence.sponsor_owner_user_id
            || row.policy_digest != evidence.policy_digest
            || row.nyxagent_profile != evidence.nyxagent_profile
        {
            return Err(denied());
        }
        (row, None)
    } else {
        let absolute = now + Duration::hours(24);
        let (key, credential) = new_key(&evidence.sponsor_owner_user_id, absolute, None);
        let cipher =
            crypto::encrypt_runtime_key(&state.encryption_keys, &credential.raw_key).await?;
        (
            PlatformPublicSession {
                id: uuid::Uuid::new_v4().to_string(),
                profile_id: manager.profile_id.clone(),
                manager_bot_id: manager.manager_bot_id.clone(),
                telegram_bot_id: manager.telegram_bot_id,
                manager_authority_epoch: manager.manager_authority_epoch,
                sender_id: admitted.sender_id,
                chat_id: admitted.chat_id,
                sponsor_owner_user_id: evidence.sponsor_owner_user_id.clone(),
                public_policy_revision: admitted.public_policy_revision,
                policy_digest: evidence.policy_digest.clone(),
                nyxagent_profile: evidence.nyxagent_profile.clone(),
                credential_api_key_id: key.id.clone(),
                credential_ciphertext: cipher,
                upstream_binding_ciphertext: vec![],
                upstream_may_exist: false,
                active_turn: None,
                effect_revision: 0,
                cleanup_record_id: None,
                cleanup_complete: false,
                revocation_reason: None,
                revoked_at: None,
                created_at: now,
                expires_at: absolute,
            },
            Some(key),
        )
    };
    let context = PublicExecutionContext {
        proof_record_id: evidence.proof_record_id.clone(),
        ingress: admitted.clone(),
        public_session_id: row.id.clone(),
        turn: TurnRef {
            conversation_id: row.id.clone(),
            turn_id: uuid::Uuid::new_v4().to_string(),
        },
        sponsor_owner_user_id: evidence.sponsor_owner_user_id,
        policy_digest: evidence.policy_digest,
        nyxagent_profile: evidence.nyxagent_profile,
        model_operations: evidence.model_operations,
        expires_at: (now + Duration::seconds(i64::from(evidence.limits.max_duration_secs)))
            .min(row.expires_at)
            .min(evidence.expires_at),
        limits: evidence.limits,
    };
    policy::verified_context(&state.db, &context).await?;
    let db = state.db.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (db, store, context, row, key),
            |session, (db, store, context, row, key)| {
                Box::pin(async move {
                    let result: AppResult<_> = async {
                        let manager = &context.ingress.manager;
                        let receipt = db
                            .collection::<PlatformManagerIngress>(RECEIPTS)
                            .find_one(doc! {
                                "_id": &context.ingress.ingress.receipt_id,
                                "claim_id": &context.ingress.ingress.claim_id, "phase": "admitted",
                            })
                            .session(&mut *session)
                            .await?
                            .ok_or_else(denied)?;
                        let controls =
                            super::platform_bot_runtime_control_service::fence_manager_in_session(
                                db,
                                session,
                                &manager.profile_id,
                                &manager.manager_bot_id,
                                manager.telegram_bot_id,
                                Some(&receipt.observation_id),
                            )
                            .await?
                            .ok_or_else(denied)?;
                        let selected = controls.public_policy.as_ref().ok_or_else(denied)?;
                        if controls.manager_authority_epoch != manager.manager_authority_epoch
                            || controls.public_policy_revision
                                != context.ingress.public_policy_revision
                            || !controls.public_enabled
                            || controls.retired
                            || controls.emergency_stopped
                            || selected.sponsor_owner_user_id != context.sponsor_owner_user_id
                            || selected.nyxagent_profile != context.nyxagent_profile
                        {
                            return Err(denied());
                        }
                        let personal_links = db
                            .collection::<Document>(
                                crate::models::platform_personal_binding::COLLECTION_NAME,
                            )
                            .count_documents(doc! {
                                "platform_config_id": &manager.profile_id,
                                "manager_bot_id": &manager.manager_bot_id,
                                "telegram_sender_id": context.ingress.sender_id, "state": "active",
                            })
                            .session(&mut *session)
                            .await?;
                        if personal_links > 0 {
                            return Err(denied());
                        }
                        let admitted =
                            super::coordination_service::RateWindowStore::admit_in_session(
                                db,
                                "public_turn_starts",
                                &format!("{}:{}", manager.profile_id, context.ingress.sender_id),
                                10,
                                std::time::Duration::from_secs(3600),
                                session,
                            )
                            .await?;
                        if !admitted {
                            return Err(AppError::RateLimited);
                        }
                        policy::verified_context_in_session(db, session, context).await?;
                        install_turn_in_session(db, session, context, row, key.as_ref()).await?;
                        store.claim_public_execution(db, session, context).await?;
                        budget::fence_context(db, session, context, &row.credential_api_key_id)
                            .await?;
                        Ok(context.clone())
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)
}

/// Caller owns claim/controls/scope fences and transaction commit. Reuse never
/// changes the credential, creation time or absolute lifetime.
async fn install_turn_in_session(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
    row: &PlatformPublicSession,
    new_key: Option<&ApiKey>,
) -> AppResult<()> {
    let manager = &context.ingress.manager;
    if row.id != context.public_session_id
        || row.id != context.turn.conversation_id
        || row.profile_id != manager.profile_id
        || row.manager_bot_id != manager.manager_bot_id
        || row.telegram_bot_id != manager.telegram_bot_id
        || row.manager_authority_epoch != manager.manager_authority_epoch
        || row.sender_id != context.ingress.sender_id
        || row.chat_id != context.ingress.chat_id
        || row.sponsor_owner_user_id != context.sponsor_owner_user_id
        || row.policy_digest != context.policy_digest
        || row.public_policy_revision != context.ingress.public_policy_revision
        || row.nyxagent_profile != context.nyxagent_profile
        || row.revoked_at.is_some()
        || row.active_turn.is_some()
        || row.expires_at <= Utc::now()
        || row.expires_at > row.created_at + Duration::hours(24)
        || context.expires_at > row.expires_at
        || context.expires_at <= Utc::now()
    {
        return Err(denied());
    }
    let binding = PublicTurnBinding {
        conversation_id: context.turn.conversation_id.clone(),
        turn_id: context.turn.turn_id.clone(),
        receipt_id: context.ingress.ingress.receipt_id.clone(),
        claim_id: context.ingress.ingress.claim_id.clone(),
        stop_requested: false,
        expires_at: context.expires_at,
    };
    if let Some(key) = new_key {
        if key.id != row.credential_api_key_id
            || key.user_id != row.sponsor_owner_user_id
            || key.purpose != ApiKeyPurpose::PublicNyxbot
            || key.expires_at != Some(row.expires_at)
            || !restricted_key(key)
        {
            return Err(denied());
        }
        tx::insert_one(db, key, Some(&mut *session)).await?;
        let mut saved = row.clone();
        saved.active_turn = Some(binding);
        db.collection::<PlatformPublicSession>(SESSIONS)
            .insert_one(saved)
            .session(session)
            .await?;
    } else {
        let exact = doc! {"_id":&row.id,"credential_api_key_id":&row.credential_api_key_id,
        "manager_authority_epoch":row.manager_authority_epoch,"public_policy_revision":row.public_policy_revision,
        "sponsor_owner_user_id":&row.sponsor_owner_user_id,"policy_digest":&row.policy_digest,
        "nyxagent_profile":&row.nyxagent_profile,"revoked_at":bson::Bson::Null,"active_turn":bson::Bson::Null,
        "expires_at":bson::DateTime::from_chrono(row.expires_at),"created_at":bson::DateTime::from_chrono(row.created_at)};
        if db.collection::<PlatformPublicSession>(SESSIONS).update_one(exact,
            doc! {"$set":{"active_turn":bson::to_bson(&binding).map_err(|_|denied())?},"$inc":{"effect_revision":1_i64}},
        ).session(session).await?.matched_count!=1 {return Err(denied());}
    }
    Ok(())
}

fn restricted_key(key: &ApiKey) -> bool {
    key.is_active
        && key.scopes == "proxy"
        && !key.allow_all_services
        && !key.allow_auto_connected_services
        && !key.allow_all_nodes
        && key.allowed_service_ids.is_empty()
        && key.allowed_platform_service_ids.is_empty()
        && key.allowed_node_ids.is_empty()
        && key.assistant_agent_owner_id.is_none()
        && key.assistant_group_id.is_none()
        && key.callback_url.is_none()
        && key.assistant_operation_scopes.is_empty()
        && !key.scheduled_write_enabled
}

async fn load_credential(
    state: &AppState,
    context: &PublicExecutionContext,
) -> AppResult<RuntimeCredential> {
    let row = state
        .db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find_one(doc! {"_id":&context.public_session_id,"revoked_at":bson::Bson::Null})
        .await?
        .ok_or_else(denied)?;
    let raw =
        crypto::decrypt_runtime_key(&state.encryption_keys, &row.credential_ciphertext).await?;
    let (owner, key, child) = super::key_service::validate_api_key(&state.db, &raw).await?;
    if child.is_some()
        || owner != context.sponsor_owner_user_id
        || key.id != row.credential_api_key_id
        || key.purpose != ApiKeyPurpose::PublicNyxbot
        || key.expires_at != Some(row.expires_at)
        || !restricted_key(&key)
    {
        return Err(denied());
    }
    Ok(RuntimeCredential {
        api_key_id: key.id,
        raw_key: raw,
    })
}

impl PublicRuntime {
    async fn prepare_inner(
        &self,
        db: &Database,
        context: &PublicExecutionContext,
        input: TurnInput,
    ) -> AppResult<PreparedPublicTurn> {
        let proof = policy::verified_context(db, context).await?;
        if input.text.is_empty()
            || input.text.len() > context.limits.max_input_bytes as usize
            || input.text.len() > context.limits.max_context_bytes as usize
        {
            return Err(denied());
        }
        let credential = load_credential(&self.state, context).await?;
        let ciphertext = crypto::encrypt_runtime_key(
            &self.state.encryption_keys,
            &Zeroizing::new(input.text.clone()),
        )
        .await?;
        if context.model_operations.is_empty() || context.limits.max_model_calls == 0 {
            return Err(denied());
        }
        let mut maximum = crate::models::credits::Credits::ZERO;
        for operation in &context.model_operations {
            let quote = budget::quote_operation(
                &self.state,
                &proof,
                context,
                &credential.api_key_id,
                operation,
                &uuid::Uuid::new_v4().to_string(),
            )
            .await?;
            maximum = maximum.max(quote.maximum());
        }
        maximum = maximum.checked_mul(i64::from(context.limits.max_model_calls))?;
        budget::seed_buckets(db, context).await?;
        let retained = db.collection::<PlatformPublicSession>(SESSIONS).find_one(doc! {"_id":&context.public_session_id,"credential_api_key_id":&credential.api_key_id}).await?.ok_or_else(denied)?;
        let upstream_session_id = retained_session_handle(&self.state, &retained).await?;
        let outer = super::platform_public_proxy_service::load_outer_catalog(db).await?;
        let outer_digest = super::platform_public_proxy_service::catalog_digest(&outer)?;
        let row = PlatformPublicTurn {
            proof_record_id: context.proof_record_id.clone(),
            id: context.turn.turn_id.clone(),
            public_session_id: context.public_session_id.clone(),
            conversation_id: context.turn.conversation_id.clone(),
            receipt_id: context.ingress.ingress.receipt_id.clone(),
            claim_id: context.ingress.ingress.claim_id.clone(),
            sponsor_owner_user_id: context.sponsor_owner_user_id.clone(),
            policy_digest: context.policy_digest.clone(),
            credential_api_key_id: credential.api_key_id.clone(),
            input_ciphertext: ciphertext,
            reply_ciphertext: vec![],
            reply_digest: String::new(),
            reply_id: None,
            outcome: None,
            max_output_bytes: context.limits.max_output_bytes,
            nyxagent_service_id: outer.id,
            nyxagent_catalog_digest: outer_digest,
            outer_dispatch_admitted: false,
            expires_at: retained.expires_at,
        };
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run(
                (
                    db.clone(),
                    context.clone(),
                    credential.api_key_id.clone(),
                    row,
                ),
                |session, (db, context, key, row)| {
                    Box::pin(async move {
                        let result: AppResult<_> = async {
                            let proof =
                                policy::verified_context_in_session(db, session, context).await?;
                            budget::reserve_loop_in_session(
                                db,
                                session,
                                &proof,
                                context,
                                key,
                                maximum,
                                context.limits.max_model_calls,
                            )
                            .await?;
                            db.collection::<PlatformPublicTurn>(TURNS)
                                .insert_one(row)
                                .session(&mut *session)
                                .await?;
                            Ok(())
                        }
                        .await;
                        tx::transaction_result(result)
                    })
                },
            )
            .await
            .map_err(tx::map_transaction_error)?;
        Ok(PreparedPublicTurn {
            context: context.clone(),
            credential,
            input,
            instructions: super::platform_public_tools::INSTRUCTIONS.into(),
            upstream_session_id,
        })
    }
}

#[async_trait]
impl PublicNyxAgentRuntime for PublicRuntime {
    async fn prepare(
        &self,
        db: &Database,
        context: &PublicExecutionContext,
        input: TurnInput,
    ) -> AppResult<PreparedPublicTurn> {
        match self.prepare_inner(db, context, input).await {
            Ok(prepared) => Ok(prepared),
            Err(error) => {
                // Claim may already be durable even when KMS/quote/cap admission
                // fails. Release that exact turn; never strand its sender lease.
                interrupt_turn(db, context, false).await?;
                Err(error)
            }
        }
    }

    async fn replace_credential(
        &self,
        db: &Database,
        context: &PublicExecutionContext,
        expected_key_id: &str,
    ) -> AppResult<RuntimeCredential> {
        policy::verified_context(db, context).await?;
        let row = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(
                doc! {"_id":&context.public_session_id,"credential_api_key_id":expected_key_id},
            )
            .await?
            .ok_or_else(denied)?;
        if row.upstream_may_exist || !row.upstream_binding_ciphertext.is_empty() {
            return Err(policy::unavailable());
        }
        let (key, credential) = new_key(
            &context.sponsor_owner_user_id,
            row.expires_at,
            Some(expected_key_id.into()),
        );
        let cipher =
            crypto::encrypt_runtime_key(&self.state.encryption_keys, &credential.raw_key).await?;
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run(
                (db.clone(), context.clone(), expected_key_id.to_owned(), key, cipher),
                |session, (db, context, old, key, cipher)| {
                    Box::pin(async move {
                        let result: AppResult<_> = async {
                            policy::verified_context_in_session(db, session, context).await?;
                            budget::fence_context(db, session, context, old).await?;
                            tx::insert_one(db, key, Some(&mut *session)).await?;
                            let revoked = tx::update_one(
                                db,
                                doc! {"_id": &*old, "purpose": "public_nyxbot", "is_active": true},
                                doc! {"$set": {"is_active": false}},
                                Some(&mut *session),
                            ).await?;
                            if revoked.matched_count != 1 {
                                return Err(denied());
                            }
                            let replaced = db.collection::<PlatformPublicSession>(SESSIONS)
                                .update_one(
                                    doc! {"_id": &context.public_session_id,
                                        "credential_api_key_id": &*old, "upstream_may_exist": false},
                                    doc! {"$set": {
                                        "credential_api_key_id": &key.id,
                                        "credential_ciphertext": bson::Binary {
                                            subtype: bson::spec::BinarySubtype::Generic, bytes: cipher.clone(),
                                        },
                                        "upstream_binding_ciphertext": bson::Binary {
                                            subtype: bson::spec::BinarySubtype::Generic, bytes: vec![],
                                        },
                                    }},
                                ).session(&mut *session).await?;
                            if replaced.matched_count != 1 {
                                return Err(denied());
                            }
                            let loops = db.collection::<Document>(crate::models::platform_public_budget::LOOPS);
                            let used = loops.count_documents(doc! {
                                "public_session_id": &context.public_session_id,
                                "turn_id": &context.turn.turn_id, "calls": {"$gt": 0},
                            }).session(&mut *session).await?;
                            if used > 0 {
                                return Err(denied());
                            }
                            loops.update_one(
                                doc! {"public_session_id": &context.public_session_id,
                                    "turn_id": &context.turn.turn_id, "credential_api_key_id": &*old},
                                doc! {"$set": {"credential_api_key_id": &key.id}},
                            ).session(&mut *session).await?;
                            db.collection::<PlatformPublicTurn>(TURNS).update_one(
                                doc! {"_id": &context.turn.turn_id,
                                    "credential_api_key_id": &*old, "reply_id": bson::Bson::Null},
                                doc! {"$set": {"credential_api_key_id": &key.id}},
                            ).session(&mut *session).await?;
                            Ok(())
                        }.await;
                        tx::transaction_result(result)
                    })
                },
            )
            .await
            .map_err(tx::map_transaction_error)?;
        Ok(credential)
    }

    async fn record_upstream_binding(
        &self,
        db: &Database,
        context: &PublicExecutionContext,
        key_id: &str,
        binding: &ObservedUpstreamBinding,
    ) -> AppResult<()> {
        policy::verified_context(db, context).await?;
        if binding.session_id.is_none() && binding.response_id.is_none() {
            return Err(denied());
        }
        for handle in [&binding.session_id, &binding.response_id]
            .into_iter()
            .flatten()
        {
            if handle.is_empty()
                || handle.len() > 256
                || !handle
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            {
                return Err(denied());
            }
        }
        let encoded=Zeroizing::new(serde_json::to_string(&serde_json::json!({"session_id":binding.session_id,"response_id":binding.response_id})).map_err(|_|denied())?);
        let cipher = crypto::encrypt_runtime_key(&self.state.encryption_keys, &encoded).await?;
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run(
                (db.clone(), context.clone(), key_id.to_owned(), cipher),
                |session, (db, context, key, cipher)| {
                    Box::pin(async move {
                        let result: AppResult<_> = async {
                            policy::verified_context_in_session(db, session, context).await?;
                            budget::fence_context(db, session, context, key).await?;
                            db.collection::<PlatformPublicSession>(SESSIONS)
                                .update_one(
                                    doc! {"_id": &context.public_session_id, "credential_api_key_id": &*key},
                                    doc! {"$set": {
                                        "upstream_may_exist": true,
                                        "upstream_binding_ciphertext": bson::Binary {
                                            subtype: bson::spec::BinarySubtype::Generic, bytes: cipher.clone(),
                                        },
                                    }},
                                ).session(session).await?;
                            Ok(())
                        }.await;
                        tx::transaction_result(result)
                    })
                },
            )
            .await
            .map_err(tx::map_transaction_error)
    }

    async fn settle(
        &self,
        db: &Database,
        context: &PublicExecutionContext,
        message_id: &str,
        result: &super::assistant_nyxagent::TurnResult,
    ) -> AppResult<SettledExecution> {
        if uuid::Uuid::parse_str(message_id).is_err() {
            return Err(denied());
        }
        let text = if result.error.is_some() {
            "The public guide could not finish this turn. Send /link to use your own NyxBot."
                .to_owned()
        } else {
            redact_intent_bearers(&result.text)
        };
        let text_bytes = text.len();
        let digest = reply_digest(&self.state, &text)?;
        let cipher =
            crypto::encrypt_runtime_key(&self.state.encryption_keys, &Zeroizing::new(text)).await?;
        let reply = ReplyRef {
            turn: context.turn.clone(),
            reply_id: message_id.into(),
        };
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run(
                (
                    db.clone(),
                    context.clone(),
                    reply.clone(),
                    cipher,
                    text_bytes,
                    result.error.is_some(),
                    digest,
                ),
                |session, (db, context, reply, cipher, text_bytes, failed, digest)| {
                    Box::pin(async move {
                        let result: AppResult<_> = async {
                            let cleared = settle_reply_in_session(
                                db,
                                session,
                                context,
                                reply,
                                cipher,
                                *text_bytes,
                                digest,
                            )
                            .await?;
                            if *failed && cleared {
                                db.collection::<PlatformPublicTurn>(TURNS)
                                    .update_one(
                                        doc! {"_id": &context.turn.turn_id, "reply_id": &reply.reply_id},
                                        doc! {"$set": {"outcome": "failed"}},
                                    )
                                    .session(&mut *session)
                                    .await?;
                                revoke_ambiguous_session_in_session(db, session, context).await?;
                            }
                            Ok(())
                        }
                        .await;
                        tx::transaction_result(result)
                    })
                },
            )
            .await
            .map_err(tx::map_transaction_error)?;
        Ok(SettledExecution::Saved(reply))
    }
}

/// Settlement calls this after compare-clearing its own active turn. A stale
/// duplicate cannot revoke a successor turn or a replaced credential.
async fn revoke_ambiguous_session_in_session(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
) -> AppResult<()> {
    let retained = db
        .collection::<PlatformPublicTurn>(TURNS)
        .find_one(doc! {
            "_id": &context.turn.turn_id, "public_session_id": &context.public_session_id,
            "receipt_id": &context.ingress.ingress.receipt_id, "claim_id": &context.ingress.ingress.claim_id,
            "sponsor_owner_user_id": &context.sponsor_owner_user_id, "policy_digest": &context.policy_digest,
        })
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    // Revocation ends access. Keep the original absolute deadline so cleanup
    // tombstones also block an already dispatched request that arrives later.
    let fenced = db
        .collection::<PlatformPublicSession>(SESSIONS)
        .update_one(
            doc! {
                "_id": &context.public_session_id, "credential_api_key_id": &retained.credential_api_key_id,
                "sponsor_owner_user_id": &context.sponsor_owner_user_id, "policy_digest": &context.policy_digest,
                "upstream_may_exist": true, "active_turn": bson::Bson::Null, "revoked_at": bson::Bson::Null,
            },
            doc! {"$inc": {"effect_revision": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    if fenced.matched_count == 1 {
        super::platform_public_agent_service::revoke_in_session(
            db,
            session,
            &PublicRevocationScope::Session {
                public_session_id: context.public_session_id.clone(),
            },
            PublicRevocationReason::SessionExpired,
        )
        .await?;
    }
    Ok(())
}

fn redact_intent_bearers(text: &str) -> String {
    let mut output = text.to_owned();
    while let Some(start) = output.find("nyx_pli_") {
        let end = output[start..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .map_or(output.len(), |n| start + n);
        output.replace_range(start..end, "[private linking token removed]");
    }
    output
}

fn reply_digest(state: &AppState, body: &str) -> AppResult<String> {
    use hmac::Mac;
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(state.auth_device_hmac_key.as_slice())
        .map_err(|_| denied())?;
    mac.update(b"nyxid-public-reply.v1\0");
    mac.update(body.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}
fn reply_filter(destination: &ReplyDestination, reply: &ReplyRef) -> AppResult<Document> {
    let ReplyAuthority::Public {
        ingress,
        public_session_id,
        turn,
    } = &destination.authority
    else {
        return Err(denied());
    };
    if *turn != reply.turn
        || destination.manager != ingress.manager
        || destination.sender_id != ingress.sender_id
        || destination.chat_id != ingress.chat_id
        || destination.expires_at <= Utc::now()
    {
        return Err(denied());
    }
    Ok(
        doc! {"_id":&turn.turn_id,"conversation_id":&turn.conversation_id,"public_session_id":public_session_id,"receipt_id":&ingress.ingress.receipt_id,
        "claim_id":&ingress.ingress.claim_id,"reply_id":&reply.reply_id,"expires_at":{"$gt":bson::DateTime::now()}},
    )
}
pub(crate) async fn load_public_reply(
    state: &AppState,
    destination: &ReplyDestination,
    reply: &ReplyRef,
) -> AppResult<Zeroizing<String>> {
    let filter = reply_filter(destination, reply)?;
    let ReplyAuthority::Public {
        ingress,
        public_session_id,
        ..
    } = &destination.authority
    else {
        return Err(denied());
    };
    let session = state
        .db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find_one(doc! {"_id":public_session_id})
        .await?
        .ok_or_else(denied)?;
    let row = state
        .db
        .collection::<PlatformPublicTurn>(TURNS)
        .find_one(filter)
        .await?
        .ok_or_else(denied)?;
    policy::verified_readiness(
        &state.db,
        &ingress.manager,
        ingress.public_policy_revision,
        Some(&session.policy_digest),
        Some(&row.proof_record_id),
    )
    .await?;
    let text = crypto::decrypt_runtime_key(&state.encryption_keys, &row.reply_ciphertext).await?;
    if row.reply_digest != reply_digest(state, &text)? {
        return Err(denied());
    }
    Ok(text)
}

/// Select only a saved failure marker. The notice body is fixed by NyxID and
/// separately fenced at delivery; this never decrypts a revoked transcript.
pub(crate) async fn public_failure_notice_expiry(
    db: &Database,
    destination: &ReplyDestination,
    reply: &ReplyRef,
) -> AppResult<Option<chrono::DateTime<Utc>>> {
    let mut filter = reply_filter(destination, reply)?;
    filter.insert("outcome", "failed");
    if db
        .collection::<PlatformPublicTurn>(TURNS)
        .count_documents(filter)
        .limit(1)
        .await?
        != 1
    {
        return Ok(None);
    }
    let ReplyAuthority::Public {
        ingress,
        public_session_id,
        ..
    } = &destination.authority
    else {
        return Err(denied());
    };
    let receipt = db
        .collection::<PlatformManagerIngress>(RECEIPTS)
        .find_one(doc! {
            "_id": &ingress.ingress.receipt_id, "claim_id": &ingress.ingress.claim_id,
            "phase": "settled", "reply_id": &reply.reply_id,
            "execution.public_session_id": public_session_id,
            "execution.conversation_id": &reply.turn.conversation_id,
            "execution.turn_id": &reply.turn.turn_id,
        })
        .await?
        .ok_or_else(denied)?;
    Ok(Some(destination.expires_at.min(
        receipt.settled_at.ok_or_else(denied)? + Duration::minutes(2),
    )))
}
/// The body was decrypted before delivery admission. Compare its HMAC against
/// the immutable settled row without nested transactions or remote KMS calls.
pub(crate) async fn fence_public_reply_body_in_session(
    state: &AppState,
    session: &mut ClientSession,
    destination: &ReplyDestination,
    reply: &ReplyRef,
    body: &str,
) -> AppResult<()> {
    let mut filter = reply_filter(destination, reply)?;
    filter.insert("reply_digest", reply_digest(state, body)?);
    if state
        .db
        .collection::<PlatformPublicTurn>(TURNS)
        .update_one(filter, doc! {"$inc":{"reply_body_fence":1_i64}})
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(denied());
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedBinding {
    session_id: Option<String>,
    response_id: Option<String>,
}
fn valid_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= 256
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
async fn retained_session_handle(
    state: &AppState,
    row: &PlatformPublicSession,
) -> AppResult<Option<String>> {
    if row.upstream_binding_ciphertext.is_empty() {
        return if row.upstream_may_exist {
            Err(policy::unavailable())
        } else {
            Ok(None)
        };
    }
    let encoded =
        crypto::decrypt_runtime_key(&state.encryption_keys, &row.upstream_binding_ciphertext)
            .await?;
    let binding: RetainedBinding = serde_json::from_str(&encoded).map_err(|_| denied())?;
    if binding
        .session_id
        .as_deref()
        .is_none_or(|s| !valid_handle(s))
        || binding
            .response_id
            .as_deref()
            .is_some_and(|s| !valid_handle(s))
    {
        return Err(denied());
    }
    Ok(binding.session_id)
}

async fn settle_reply_in_session(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
    reply: &ReplyRef,
    ciphertext: &[u8],
    text_bytes: usize,
    digest: &str,
) -> AppResult<bool> {
    let row=db.collection::<PlatformPublicTurn>(TURNS).find_one(doc! {
        "_id":&context.turn.turn_id,"public_session_id":&context.public_session_id,
        "conversation_id":&context.turn.conversation_id,"receipt_id":&context.ingress.ingress.receipt_id,
        "claim_id":&context.ingress.ingress.claim_id,"sponsor_owner_user_id":&context.sponsor_owner_user_id,
        "policy_digest":&context.policy_digest,
    }).session(&mut *session).await?.ok_or_else(denied)?;
    if reply.turn != context.turn {
        return Err(denied());
    }
    if let Some(existing) = &row.reply_id {
        if *existing != reply.reply_id {
            return Err(denied());
        }
        // The original reply and ingress settlement committed together. A late
        // result for that reply is a read of the saved reference, and must not
        // clear a successor or replace the original body, digest or outcome.
        return Ok(false);
    }
    if text_bytes > row.max_output_bytes as usize {
        return Err(denied());
    }
    if db.collection::<PlatformPublicTurn>(TURNS).update_one(
        doc! {"_id":&row.id,"reply_id":bson::Bson::Null},
        doc! {"$set":{"reply_id":&reply.reply_id,"reply_ciphertext":bson::Binary {
            subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext.to_vec()},"outcome":"settled","reply_digest":digest}},
    ).session(&mut *session).await?.matched_count!=1 {return Err(denied());}
    ManagerIngressTurnStore::settle_public_execution(
        &super::platform_manager_ingress_service::IngressTurnStore,
        db,
        session,
        &context.ingress.ingress,
        reply,
    )
    .await?;
    let cleared = db.collection::<PlatformPublicSession>(SESSIONS).update_one(doc! {
        "_id":&context.public_session_id,"active_turn.turn_id":&context.turn.turn_id,
        "credential_api_key_id":&row.credential_api_key_id,
        "sponsor_owner_user_id":&context.sponsor_owner_user_id,"policy_digest":&context.policy_digest,
        "active_turn.conversation_id":&context.turn.conversation_id,
        "active_turn.receipt_id":&context.ingress.ingress.receipt_id,"active_turn.claim_id":&context.ingress.ingress.claim_id,
    },doc! {"$set":{"active_turn":bson::Bson::Null}}).session(&mut *session).await?.matched_count == 1;
    if let Some(loop_row) = db
        .collection::<crate::models::platform_public_budget::PublicBudgetLoop>(
            crate::models::platform_public_budget::LOOPS,
        )
        .find_one(
            doc! {"public_session_id":&context.public_session_id,"turn_id":&context.turn.turn_id},
        )
        .session(&mut *session)
        .await?
    {
        budget::close_loop_in_session(db, session, &loop_row.id).await?;
    }
    Ok(cleared)
}

/// Transport retains a delivery deadline separate from the shorter execution
/// lease. It can only use the exact stored session's immutable absolute expiry.
pub(crate) async fn public_reply_expiry(
    db: &Database,
    context: &PublicExecutionContext,
) -> AppResult<chrono::DateTime<Utc>> {
    let row = db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find_one(doc! {
            "_id":&context.public_session_id,"sponsor_owner_user_id":&context.sponsor_owner_user_id,
            "policy_digest":&context.policy_digest,"active_turn.turn_id":&context.turn.turn_id,
            "active_turn.receipt_id":&context.ingress.ingress.receipt_id,
            "active_turn.claim_id":&context.ingress.ingress.claim_id,"revoked_at":bson::Bson::Null,
            "$expr":{"$gt":["$expires_at","$$NOW"]},
        })
        .await?
        .ok_or_else(denied)?;
    Ok(row.expires_at)
}

/// A settled reply fence is independent of the execution deadline/active turn.
/// Caller owns the delivery transaction; no bytes or bearer are persisted here.
pub(crate) async fn fence_reply_in_session(
    db: &Database,
    session: &mut ClientSession,
    destination: &ReplyDestination,
) -> AppResult<()> {
    let ReplyAuthority::Public {
        ingress,
        public_session_id,
        turn,
    } = &destination.authority
    else {
        return Err(denied());
    };
    if destination.manager != ingress.manager
        || destination.sender_id != ingress.sender_id
        || destination.chat_id != ingress.chat_id
        || destination.expires_at <= Utc::now()
        || ingress.sender_id <= 0
        || ingress.sender_id != ingress.chat_id
    {
        return Err(denied());
    }
    let manager = &ingress.manager;
    let retained=db.collection::<PlatformPublicTurn>(TURNS).find_one(doc! {
        "_id":&turn.turn_id,"conversation_id":&turn.conversation_id,"public_session_id":public_session_id,
        "receipt_id":&ingress.ingress.receipt_id,"claim_id":&ingress.ingress.claim_id,
        "reply_id":{"$type":"string"},"outcome":"settled","$expr":{"$gt":["$expires_at","$$NOW"]},
    }).session(&mut *session).await?.ok_or_else(denied)?;
    let row=db.collection::<PlatformPublicSession>(SESSIONS).find_one_and_update(doc! {
        "_id":public_session_id,"profile_id":&manager.profile_id,"manager_bot_id":&manager.manager_bot_id,
        "telegram_bot_id":manager.telegram_bot_id,"manager_authority_epoch":manager.manager_authority_epoch,
        "sender_id":ingress.sender_id,"chat_id":ingress.chat_id,"public_policy_revision":ingress.public_policy_revision,
        "sponsor_owner_user_id":&retained.sponsor_owner_user_id,"policy_digest":&retained.policy_digest,
        "revoked_at":bson::Bson::Null,"$expr":{"$gt":["$expires_at","$$NOW"]},
    },doc! {"$inc":{"effect_revision":1_i64}}).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(denied)?;
    if !super::platform_public_agent_service::fence_local_session_in_session(db, session, &row)
        .await?
    {
        return Err(denied());
    }
    db.collection::<PlatformManagerIngress>(RECEIPTS).find_one_and_update(doc! {
        "_id":&ingress.ingress.receipt_id,"claim_id":&ingress.ingress.claim_id,"phase":"settled",
        "profile_id":&manager.profile_id,"manager_bot_id":&manager.manager_bot_id,
        "telegram_bot_id":manager.telegram_bot_id,"manager_authority_epoch":manager.manager_authority_epoch,
        "sender_id":ingress.sender_id,"chat_id":ingress.chat_id,"audience.kind":"public",
        "audience.public_policy_revision":ingress.public_policy_revision,
        "execution.public_session_id":public_session_id,"execution.conversation_id":&turn.conversation_id,
        "execution.turn_id":&turn.turn_id,"execution.payer_user_id":&row.sponsor_owner_user_id,
        "execution.public_policy_digest":&row.policy_digest,"reply_id":retained.reply_id.as_deref().ok_or_else(denied)?,
    },doc! {"$inc":{"effect_revision":1_i64}}).session(&mut *session).await?.ok_or_else(denied)?;
    // Token rotation for the same numeric bot does not change authority epoch.
    // Bind the current profile observation, not the historical receipt token.
    let profile = db
        .collection::<crate::models::platform_bot_config::PlatformBotConfig>(
            crate::models::platform_bot_config::COLLECTION_NAME,
        )
        .find_one(
            doc! {"_id":&manager.profile_id,"manager_bot_id":&manager.manager_bot_id,
            "deleted_at":bson::Bson::Null},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    let controls = super::platform_bot_runtime_control_service::fence_manager_in_session(
        db,
        session,
        &manager.profile_id,
        &manager.manager_bot_id,
        manager.telegram_bot_id,
        Some(&profile.observation_id),
    )
    .await?
    .ok_or_else(denied)?;
    let selected = controls.public_policy.ok_or_else(denied)?;
    if !controls.public_enabled
        || controls.emergency_stopped
        || controls.retired
        || controls.manager_authority_epoch != row.manager_authority_epoch
        || controls.public_policy_revision != row.public_policy_revision
        || selected.sponsor_owner_user_id != row.sponsor_owner_user_id
        || selected.nyxagent_profile != row.nyxagent_profile
    {
        return Err(denied());
    }
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.sponsor_owner_user_id,"user_type":"org","is_active":true},
            doc! {"$inc":{"public_reply_fence":1_i64}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(denied());
    }
    if db.collection::<Document>(KEYS).update_one(
        doc! {"_id":&row.credential_api_key_id,"user_id":&row.sponsor_owner_user_id,"purpose":"public_nyxbot",
            "is_active":true,"$expr":{"$gt":["$expires_at","$$NOW"]}},doc! {"$inc":{"public_reply_fence":1_i64}},
    ).session(session).await?.matched_count!=1 {return Err(denied());}
    Ok(())
}

async fn interrupt_turn(
    db: &Database,
    context: &PublicExecutionContext,
    expired: bool,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (db.clone(), context.clone(), expired),
            |session, (db, context, expired)| {
                Box::pin(async move {
                    tx::transaction_result(
                        interrupt_turn_in_session(db, session, context, *expired).await,
                    )
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)
}

/// Recovery uses the exact old turn tuple, including when preparation never
/// created a disposable transcript. It does not declare any provider usage zero.
pub(crate) async fn interrupt_turn_in_session(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
    expired: bool,
) -> AppResult<()> {
    interrupt_identity_in_session(
        db,
        session,
        &InterruptionIdentity::from_context(context),
        expired,
    )
    .await
}

/// Settlement/recovery identity has no execution proof or inference authority.
#[derive(Clone)]
struct InterruptionIdentity {
    public_session_id: String,
    sponsor_owner_user_id: String,
    policy_digest: String,
    turn: TurnRef,
    ingress: IngressClaim,
}

impl InterruptionIdentity {
    fn from_context(context: &PublicExecutionContext) -> Self {
        Self {
            public_session_id: context.public_session_id.clone(),
            sponsor_owner_user_id: context.sponsor_owner_user_id.clone(),
            policy_digest: context.policy_digest.clone(),
            turn: context.turn.clone(),
            ingress: context.ingress.ingress.clone(),
        }
    }
}

async fn interrupt_identity_in_session(
    db: &Database,
    session: &mut ClientSession,
    identity: &InterruptionIdentity,
    expired: bool,
) -> AppResult<()> {
    let mut filter = doc! {"_id":&identity.public_session_id,"sponsor_owner_user_id":&identity.sponsor_owner_user_id,
    "policy_digest":&identity.policy_digest,"active_turn.conversation_id":&identity.turn.conversation_id,
    "active_turn.turn_id":&identity.turn.turn_id,"active_turn.receipt_id":&identity.ingress.receipt_id,
    "active_turn.claim_id":&identity.ingress.claim_id};
    if expired {
        filter.insert("$expr", doc! {"$lte":["$active_turn.expires_at","$$NOW"]});
    }
    let Some(row) = db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find_one_and_update(
            filter,
            doc! {"$set":{"active_turn":bson::Bson::Null},"$inc":{"effect_revision":1_i64}},
        )
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    let reason = if expired {
        "public_turn_expired"
    } else {
        "public_prepare_failed"
    };
    let changed=db.collection::<Document>(RECEIPTS).update_one(doc! {
        "_id":&identity.ingress.receipt_id,"claim_id":&identity.ingress.claim_id,
        "phase":"executing","audience.kind":"public","execution.public_session_id":&identity.public_session_id,
        "execution.conversation_id":&identity.turn.conversation_id,"execution.turn_id":&identity.turn.turn_id,
        "execution.payer_user_id":&identity.sponsor_owner_user_id,"execution.public_policy_digest":&identity.policy_digest,
        "reply_id":bson::Bson::Null,
    },vec![doc! {"$set":{"phase":"interrupted","interrupted_reason":reason,"settled_at":"$$NOW"}}])
        .session(&mut *session).await?;
    if changed.matched_count != 1 {
        return Err(denied());
    }
    db.collection::<Document>(TURNS).update_one(doc! {"_id":&identity.turn.turn_id,
        "public_session_id":&identity.public_session_id,"receipt_id":&identity.ingress.receipt_id,
        "claim_id":&identity.ingress.claim_id,"reply_id":bson::Bson::Null},doc! {"$set":{"outcome":reason}})
        .session(&mut *session).await?;
    if let Some(loop_row) = db
        .collection::<crate::models::platform_public_budget::PublicBudgetLoop>(
            crate::models::platform_public_budget::LOOPS,
        )
        .find_one(doc! {"public_session_id":&row.id,"turn_id":&identity.turn.turn_id})
        .session(&mut *session)
        .await?
    {
        budget::close_loop_in_session(db, session, &loop_row.id).await?;
    }
    if expired && row.upstream_may_exist {
        // A timed-out upstream may still be running. Revoke its exact key and
        // queue cleanup; the proof gap remains recorded, never reported erased.
        super::platform_public_agent_service::revoke_in_session(
            db,
            session,
            &PublicRevocationScope::Session {
                public_session_id: row.id,
            },
            PublicRevocationReason::SessionExpired,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn recover_expired_turn(
    db: &Database,
    row: &PlatformPublicSession,
) -> AppResult<()> {
    let Some(turn) = &row.active_turn else {
        return Ok(());
    };
    let identity = InterruptionIdentity {
        public_session_id: row.id.clone(),
        sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
        policy_digest: row.policy_digest.clone(),
        turn: TurnRef {
            conversation_id: turn.conversation_id.clone(),
            turn_id: turn.turn_id.clone(),
        },
        ingress: IngressClaim {
            receipt_id: turn.receipt_id.clone(),
            claim_id: turn.claim_id.clone(),
        },
    };
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run((db.clone(), identity), |session, (db, identity)| {
            Box::pin(async move {
                tx::transaction_result(
                    interrupt_identity_in_session(db, session, identity, true).await,
                )
            })
        })
        .await
        .map_err(tx::map_transaction_error)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::platform_manager_ingress::{
        IngressAudience, IngressExecution, IngressPhase,
    };
    pub(crate) fn fixture() -> (PlatformPublicSession, ApiKey, PublicExecutionContext) {
        let now = Utc::now();
        let expires = now + Duration::hours(24);
        let (key, _) = new_key("sponsor", expires, None);
        let row = PlatformPublicSession {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: "profile".into(),
            manager_bot_id: "99".into(),
            telegram_bot_id: 99,
            manager_authority_epoch: 1,
            sender_id: 5,
            chat_id: 5,
            sponsor_owner_user_id: "sponsor".into(),
            public_policy_revision: 1,
            policy_digest: "policy".into(),
            nyxagent_profile: "public".into(),
            credential_api_key_id: key.id.clone(),
            credential_ciphertext: vec![1, 2, 3],
            upstream_binding_ciphertext: vec![],
            upstream_may_exist: false,
            active_turn: None,
            effect_revision: 0,
            cleanup_record_id: None,
            cleanup_complete: false,
            revocation_reason: None,
            revoked_at: None,
            created_at: now,
            expires_at: expires,
        };
        let context = PublicExecutionContext {
            proof_record_id: "test-proof".into(),
            ingress: PublicIngress {
                ingress: IngressClaim {
                    receipt_id: uuid::Uuid::new_v4().to_string(),
                    claim_id: uuid::Uuid::new_v4().to_string(),
                },
                manager: ManagerAuthority {
                    profile_id: row.profile_id.clone(),
                    manager_bot_id: row.manager_bot_id.clone(),
                    telegram_bot_id: 99,
                    manager_authority_epoch: 1,
                },
                sender_id: 5,
                chat_id: 5,
                public_policy_revision: 1,
            },
            public_session_id: row.id.clone(),
            turn: TurnRef {
                conversation_id: row.id.clone(),
                turn_id: uuid::Uuid::new_v4().to_string(),
            },
            sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
            policy_digest: row.policy_digest.clone(),
            nyxagent_profile: row.nyxagent_profile.clone(),
            model_operations: vec![],
            limits: PublicExecutionLimits {
                max_duration_secs: 60,
                max_model_calls: 1,
                max_tool_calls: 3,
                max_continuations: 0,
                max_input_tokens: 100,
                max_output_tokens: 100,
                max_input_bytes: 100,
                max_context_bytes: 100,
                max_output_bytes: 100,
            },
            expires_at: now + Duration::minutes(1),
        };
        (row, key, context)
    }
    #[test]
    fn public_keys_are_restricted_at_birth_and_successors_keep_absolute_expiry() {
        let (row, key, _) = fixture();
        assert!(restricted_key(&key));
        assert_eq!(key.purpose, ApiKeyPurpose::PublicNyxbot);
        let (next, _) = new_key(&key.user_id, row.expires_at, Some(key.id.clone()));
        assert_eq!(next.purpose, key.purpose);
        assert_eq!(next.expires_at, key.expires_at);
        assert_eq!(
            next.rotation_predecessor_id.as_deref(),
            Some(key.id.as_str())
        );
        assert!(restricted_key(&next));
        assert_eq!(
            redact_intent_bearers("See nyx_pli_secret123&other=nyx_pli_secret456."),
            "See [private linking token removed]&other=[private linking token removed]."
        );
    }
    #[tokio::test]
    async fn issuance_rollback_and_idle_reuse_keep_the_exact_credential_and_expiry() {
        let db = crate::test_utils::connect_transaction_test_database("public_issue_reuse").await;
        super::super::platform_public_agent_service::ensure_indexes(&db)
            .await
            .unwrap();
        let (row, key, mut context) = fixture();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        install_turn_in_session(&db, &mut session, &context, &row, Some(&key))
            .await
            .unwrap();
        session.abort_transaction().await.unwrap();
        assert_eq!(
            db.collection::<Document>(KEYS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(SESSIONS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        session.start_transaction().await.unwrap();
        install_turn_in_session(&db, &mut session, &context, &row, Some(&key))
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        assert!(install_turn_attempt(&db, &context, &row).await.is_err());
        db.collection::<Document>(SESSIONS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"active_turn":bson::Bson::Null}},
            )
            .await
            .unwrap();
        context.turn.turn_id = uuid::Uuid::new_v4().to_string();
        install_turn_attempt(&db, &context, &row).await.unwrap();
        let saved = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.credential_api_key_id, key.id);
        assert_eq!(saved.credential_ciphertext, row.credential_ciphertext);
        assert_eq!(
            saved.created_at.timestamp_millis(),
            row.created_at.timestamp_millis()
        );
        assert_eq!(
            saved.expires_at.timestamp_millis(),
            row.expires_at.timestamp_millis()
        );
        assert_eq!(
            db.collection::<Document>(KEYS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        let mut changed = context.clone();
        changed.sponsor_owner_user_id = "another".into();
        assert!(install_turn_attempt(&db, &changed, &row).await.is_err());
        db.drop().await.unwrap();
    }
    async fn install_turn_attempt(
        db: &Database,
        context: &PublicExecutionContext,
        row: &PlatformPublicSession,
    ) -> AppResult<()> {
        let mut session = db.client().start_session().await?;
        session.start_transaction().await?;
        let result = install_turn_in_session(db, &mut session, context, row, None).await;
        if result.is_ok() {
            session.commit_transaction().await?;
        } else {
            session.abort_transaction().await?;
        }
        result
    }
    #[tokio::test]
    async fn competing_turn_reuse_conflicts_in_the_callers_transaction() {
        let db = crate::test_utils::connect_transaction_test_database("public_reuse_race").await;
        let (row, key, context) = fixture();
        db.collection::<PlatformPublicSession>(SESSIONS)
            .insert_one(&row)
            .await
            .unwrap();
        tx::insert_one(&db, &key, None).await.unwrap();
        let mut a = db.client().start_session().await.unwrap();
        let mut b = db.client().start_session().await.unwrap();
        a.start_transaction().await.unwrap();
        b.start_transaction().await.unwrap();
        db.collection::<Document>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .session(&mut b)
            .await
            .unwrap();
        install_turn_in_session(&db, &mut a, &context, &row, None)
            .await
            .unwrap();
        a.commit_transaction().await.unwrap();
        let mut other = context.clone();
        other.turn.turn_id = uuid::Uuid::new_v4().to_string();
        assert!(
            install_turn_in_session(&db, &mut b, &other, &row, None)
                .await
                .is_err()
        );
        b.abort_transaction().await.unwrap();
        let saved = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.active_turn.unwrap().turn_id, context.turn.turn_id);
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn public_reply_body_fence_rejects_altered_content_and_original_tuple_changes() {
        let db = crate::test_utils::connect_transaction_test_database("public_reply_body").await;
        let (row, context) = prepared_recovery_fixture(&db).await;
        let state = crate::test_utils::test_app_state(db.clone());
        let body = "Public FAQ reply";
        let reply = ReplyRef {
            turn: context.turn.clone(),
            reply_id: uuid::Uuid::new_v4().to_string(),
        };
        let settled = db.collection::<Document>(TURNS).update_one(doc! {"_id":&context.turn.turn_id},
            doc! {"$set":{"reply_id":&reply.reply_id,"reply_digest":reply_digest(&state,body).unwrap(),"outcome":"settled"}}).await.unwrap();
        assert_eq!(settled.matched_count, 1, "prepared turn must be settled");
        let destination = ReplyDestination {
            manager: context.ingress.manager.clone(),
            sender_id: context.ingress.sender_id,
            chat_id: context.ingress.chat_id,
            authority: ReplyAuthority::Public {
                ingress: context.ingress.clone(),
                public_session_id: row.id,
                turn: context.turn.clone(),
            },
            expires_at: row.expires_at,
        };
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        fence_public_reply_body_in_session(&state, &mut session, &destination, &reply, body)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            fence_public_reply_body_in_session(
                &state,
                &mut session,
                &destination,
                &reply,
                "Altered reply"
            )
            .await
            .is_err()
        );
        session.abort_transaction().await.unwrap();
        let mut changed = destination.clone();
        if let ReplyAuthority::Public { ingress, .. } = &mut changed.authority {
            ingress.ingress.claim_id = uuid::Uuid::new_v4().to_string();
        }
        session.start_transaction().await.unwrap();
        assert!(
            fence_public_reply_body_in_session(&state, &mut session, &changed, &reply, body)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn public_reply_and_ingress_settle_together_after_revoke_with_persisted_bounds() {
        let db = crate::test_utils::connect_transaction_test_database("public_settle_exact").await;
        let (mut row, _, mut context) = fixture();
        row.revoked_at = Some(Utc::now());
        row.active_turn = Some(PublicTurnBinding {
            conversation_id: context.turn.conversation_id.clone(),
            turn_id: context.turn.turn_id.clone(),
            receipt_id: context.ingress.ingress.receipt_id.clone(),
            claim_id: context.ingress.ingress.claim_id.clone(),
            stop_requested: true,
            expires_at: context.expires_at,
        });
        let retained = PlatformPublicTurn {
            proof_record_id: "test-proof".into(),
            id: context.turn.turn_id.clone(),
            public_session_id: row.id.clone(),
            conversation_id: context.turn.conversation_id.clone(),
            receipt_id: context.ingress.ingress.receipt_id.clone(),
            claim_id: context.ingress.ingress.claim_id.clone(),
            sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
            policy_digest: row.policy_digest.clone(),
            credential_api_key_id: row.credential_api_key_id.clone(),
            input_ciphertext: vec![1],
            reply_ciphertext: vec![],
            reply_digest: String::new(),
            reply_id: None,
            outcome: None,
            max_output_bytes: 5,
            nyxagent_service_id: "outer".into(),
            nyxagent_catalog_digest: "outer-digest".into(),
            outer_dispatch_admitted: false,
            expires_at: row.expires_at,
        };
        let receipt = PlatformManagerIngress {
            id: retained.receipt_id.clone(),
            claim_id: retained.claim_id.clone(),
            profile_id: row.profile_id.clone(),
            manager_bot_id: row.manager_bot_id.clone(),
            telegram_bot_id: 99,
            manager_authority_epoch: 1,
            observation_id: "observation".into(),
            sender_id: 5,
            chat_id: 5,
            update_id: 1,
            message_id: 1,
            original_private_command: false,
            audience: IngressAudience::Public {
                public_policy_revision: 1,
            },
            phase: IngressPhase::Executing,
            execution: Some(IngressExecution {
                conversation_id: retained.conversation_id.clone(),
                turn_id: retained.id.clone(),
                public_session_id: Some(row.id.clone()),
                payer_user_id: row.sponsor_owner_user_id.clone(),
                public_policy_digest: Some(row.policy_digest.clone()),
            }),
            decision: None,
            reply_id: None,
            delivery_recovery_queued: false,
            interrupted_reason: None,
            effect_revision: 0,
            admitted_at: Utc::now(),
            start_before: Utc::now(),
            settled_at: None,
        };
        db.collection::<PlatformPublicSession>(SESSIONS)
            .insert_one(&row)
            .await
            .unwrap();
        db.collection::<PlatformPublicTurn>(TURNS)
            .insert_one(&retained)
            .await
            .unwrap();
        db.collection::<PlatformManagerIngress>(RECEIPTS)
            .insert_one(receipt)
            .await
            .unwrap();
        context.expires_at = Utc::now() - Duration::seconds(1);
        context.limits.max_output_bytes = u32::MAX;
        let reply = ReplyRef {
            turn: context.turn.clone(),
            reply_id: uuid::Uuid::new_v4().to_string(),
        };
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            settle_reply_in_session(
                &db,
                &mut session,
                &context,
                &reply,
                &[2],
                6,
                "local-test-digest"
            )
            .await
            .is_err()
        );
        session.abort_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        settle_reply_in_session(
            &db,
            &mut session,
            &context,
            &reply,
            &[2],
            5,
            "local-test-digest",
        )
        .await
        .unwrap();
        session.abort_transaction().await.unwrap();
        assert!(
            db.collection::<PlatformPublicTurn>(TURNS)
                .find_one(doc! {"_id":&retained.id})
                .await
                .unwrap()
                .unwrap()
                .reply_id
                .is_none()
        );
        session.start_transaction().await.unwrap();
        settle_reply_in_session(
            &db,
            &mut session,
            &context,
            &reply,
            &[2],
            5,
            "local-test-digest",
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        settle_reply_in_session(
            &db,
            &mut session,
            &context,
            &reply,
            &[3],
            5,
            "local-test-digest",
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        assert_eq!(
            db.collection::<PlatformPublicTurn>(TURNS)
                .find_one(doc! {"_id":&retained.id})
                .await
                .unwrap()
                .unwrap()
                .reply_ciphertext,
            vec![2]
        );
        assert_eq!(
            db.collection::<PlatformManagerIngress>(RECEIPTS)
                .find_one(doc! {"_id":&retained.receipt_id})
                .await
                .unwrap()
                .unwrap()
                .reply_id
                .as_deref(),
            Some(reply.reply_id.as_str())
        );
        assert!(
            db.collection::<PlatformPublicSession>(SESSIONS)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap()
                .active_turn
                .is_none()
        );
        db.drop().await.unwrap();
    }
    async fn recovery_fixture(db: &Database) -> (PlatformPublicSession, PublicExecutionContext) {
        let (row, key, context) = fixture();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        install_turn_in_session(db, &mut session, &context, &row, Some(&key))
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        db.collection::<Document>(RECEIPTS).insert_one(doc! {"_id":&context.ingress.ingress.receipt_id,
            "claim_id":&context.ingress.ingress.claim_id,"phase":"executing","audience":{"kind":"public"},"reply_id":bson::Bson::Null,
            "execution":{"public_session_id":&context.public_session_id,"conversation_id":&context.turn.conversation_id,
                "turn_id":&context.turn.turn_id,"payer_user_id":&context.sponsor_owner_user_id,"public_policy_digest":&context.policy_digest},
        }).await.unwrap();
        let saved = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        (saved, context)
    }
    async fn prepared_recovery_fixture(
        db: &Database,
    ) -> (PlatformPublicSession, PublicExecutionContext) {
        let (row, context) = recovery_fixture(db).await;
        db.collection::<PlatformPublicTurn>(TURNS)
            .insert_one(PlatformPublicTurn {
                proof_record_id: "test-proof".into(),
                id: context.turn.turn_id.clone(),
                public_session_id: row.id.clone(),
                conversation_id: context.turn.conversation_id.clone(),
                receipt_id: context.ingress.ingress.receipt_id.clone(),
                claim_id: context.ingress.ingress.claim_id.clone(),
                sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
                policy_digest: row.policy_digest.clone(),
                credential_api_key_id: row.credential_api_key_id.clone(),
                input_ciphertext: vec![1],
                reply_ciphertext: vec![],
                reply_digest: String::new(),
                reply_id: None,
                outcome: None,
                max_output_bytes: context.limits.max_output_bytes,
                nyxagent_service_id: "outer".into(),
                nyxagent_catalog_digest: "pin".into(),
                outer_dispatch_admitted: false,
                expires_at: row.expires_at,
            })
            .await
            .unwrap();
        (row, context)
    }
    #[tokio::test]
    async fn ambiguous_outer_failure_settles_revokes_and_allows_fresh_sender_without_refund() {
        use crate::models::{
            credits::Credits,
            platform_public_budget::{
                CALLS, LOOPS, PublicBudgetLoop, PublicCallReservation, PublicCallState,
            },
            platform_public_session::{CLEANUP_RECORDS, PublicCleanupRecord},
        };
        let db =
            crate::test_utils::connect_transaction_test_database("public_outer_failure_settle")
                .await;
        super::super::platform_public_agent_service::ensure_indexes(&db)
            .await
            .unwrap();
        let (row, context) = prepared_recovery_fixture(&db).await;
        let held = Credits::from_pico(10).unwrap();
        let allocated = Credits::from_pico(6).unwrap();
        let loop_row = PublicBudgetLoop {
            id: uuid::Uuid::new_v4().to_string(),
            public_session_id: row.id.clone(),
            conversation_id: context.turn.conversation_id.clone(),
            turn_id: context.turn.turn_id.clone(),
            receipt_id: context.ingress.ingress.receipt_id.clone(),
            claim_id: context.ingress.ingress.claim_id.clone(),
            profile_id: row.profile_id.clone(),
            sender_id: row.sender_id,
            sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
            credential_api_key_id: row.credential_api_key_id.clone(),
            policy_digest: row.policy_digest.clone(),
            bucket_ids: vec![],
            day: row.created_at,
            expires_at: context.expires_at,
            maximum: held,
            held,
            allocated,
            spent: Credits::ZERO,
            calls: 1,
            max_calls: 1,
            input_tokens: 10,
            output_tokens: 10,
            max_input_tokens: 100,
            max_output_tokens: 100,
            tool_calls: 0,
            max_tool_calls: 3,
            closed: false,
        };
        let call = PublicCallReservation {
            id: uuid::Uuid::new_v4().to_string(),
            loop_id: loop_row.id.clone(),
            sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
            credential_api_key_id: row.credential_api_key_id.clone(),
            billing_request_id: uuid::Uuid::new_v4().to_string(),
            route_digest: "pinned-model-route".into(),
            model_id: "pinned-model".into(),
            state: PublicCallState::DispatchAdmitted,
            maximum: allocated,
            cap_charge: Credits::ZERO,
            meter_row_ids: vec![],
            next_reconcile_at: context.expires_at,
            expires_at: context.expires_at,
        };
        db.collection::<PublicBudgetLoop>(LOOPS)
            .insert_one(&loop_row)
            .await
            .unwrap();
        db.collection::<PublicCallReservation>(CALLS)
            .insert_one(&call)
            .await
            .unwrap();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        super::super::platform_public_proxy_service::mark_outer_attempt_in_session(
            &db,
            &mut session,
            &context,
            &row.credential_api_key_id,
            "outer",
            "pin",
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        let reply = ReplyRef {
            turn: context.turn.clone(),
            reply_id: uuid::Uuid::new_v4().to_string(),
        };
        session.start_transaction().await.unwrap();
        assert!(
            settle_reply_in_session(
                &db,
                &mut session,
                &context,
                &reply,
                &[2],
                1,
                "local-test-digest"
            )
            .await
            .unwrap()
        );
        revoke_ambiguous_session_in_session(&db, &mut session, &context)
            .await
            .unwrap();
        session.abort_transaction().await.unwrap();
        let unchanged = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(unchanged.active_turn.is_some() && unchanged.revoked_at.is_none());
        assert!(
            db.collection::<PlatformPublicTurn>(TURNS)
                .find_one(doc! {"_id":&context.turn.turn_id})
                .await
                .unwrap()
                .unwrap()
                .reply_id
                .is_none()
        );
        assert_eq!(
            db.collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        // No response.created was received. Use the actual settlement producer,
        // which must preserve a safe error reply while revoking ambiguous auth.
        let runtime = PublicRuntime {
            state: crate::test_utils::test_app_state(db.clone()),
        };
        let result = super::super::assistant_nyxagent::TurnResult {
            text: "upstream text must not be saved".into(),
            session_id: None,
            response_id: None,
            error: Some(super::super::assistant_nyxagent::TurnError::internal(
                "assistant_unavailable",
                "public_execution",
            )),
        };
        assert!(
            matches!(runtime.settle(&db, &context, &reply.reply_id, &result).await.unwrap(), SettledExecution::Saved(saved) if saved.turn == reply.turn && saved.reply_id == reply.reply_id)
        );
        let saved = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(saved.active_turn.is_none() && saved.revoked_at.is_some());
        assert_eq!(saved.expires_at, row.expires_at);
        assert!(saved.upstream_may_exist && saved.upstream_binding_ciphertext.is_empty());
        assert!(saved.credential_ciphertext.is_empty());
        assert!(
            !db.collection::<ApiKey>(KEYS)
                .find_one(doc! {"_id":&row.credential_api_key_id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        let cleanup = db
            .collection::<PublicCleanupRecord>(CLEANUP_RECORDS)
            .find_one(doc! {"_id":saved.cleanup_record_id.unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert!(!cleanup.completed);
        assert_eq!(cleanup.session_ids, vec![row.id.clone()]);
        let retained = db
            .collection::<PlatformPublicTurn>(TURNS)
            .find_one(doc! {"_id":&context.turn.turn_id})
            .await
            .unwrap()
            .unwrap();
        let text =
            crypto::decrypt_runtime_key(&runtime.state.encryption_keys, &retained.reply_ciphertext)
                .await
                .unwrap();
        assert_eq!(
            text.as_str(),
            "The public guide could not finish this turn. Send /link to use your own NyxBot."
        );
        let receipt = db
            .collection::<Document>(RECEIPTS)
            .find_one(doc! {"_id":&context.ingress.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.get_str("phase").unwrap(), "settled");
        assert_eq!(receipt.get_str("reply_id").unwrap(), reply.reply_id);
        // An admitted model call keeps its conservative cap/dispatch evidence;
        // settlement does not release it or manufacture a billing disposition.
        assert_eq!(
            db.collection::<Document>(LOOPS)
                .find_one(doc! {"_id":&loop_row.id})
                .await
                .unwrap()
                .unwrap(),
            bson::to_document(&loop_row).unwrap()
        );
        assert_eq!(
            db.collection::<Document>(CALLS)
                .find_one(doc! {"_id":&call.id})
                .await
                .unwrap()
                .unwrap(),
            bson::to_document(&call).unwrap()
        );
        let (next_row, next_key, next_context) = fixture();
        assert_eq!(next_row.sender_id, row.sender_id);
        session.start_transaction().await.unwrap();
        install_turn_in_session(&db, &mut session, &next_context, &next_row, Some(&next_key))
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        assert_eq!(
            db.collection::<PlatformPublicSession>(SESSIONS)
                .count_documents(doc! {"sender_id":row.sender_id,"revoked_at":bson::Bson::Null})
                .await
                .unwrap(),
            1
        );
        assert_ne!(next_row.id, row.id);
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn failed_duplicate_settlement_cannot_revoke_an_idle_or_active_successor_or_replaced_key()
    {
        let db =
            crate::test_utils::connect_transaction_test_database("public_stale_settlement").await;
        let (row, context) = prepared_recovery_fixture(&db).await;
        db.collection::<PlatformPublicSession>(SESSIONS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"upstream_may_exist":true}},
            )
            .await
            .unwrap();
        let runtime = PublicRuntime {
            state: crate::test_utils::test_app_state(db.clone()),
        };
        let reply_id = uuid::Uuid::new_v4().to_string();
        let mut result = super::super::assistant_nyxagent::TurnResult {
            text: "safe reply".into(),
            session_id: Some("upstream".into()),
            response_id: Some("response".into()),
            error: None,
        };
        let SettledExecution::Saved(original_reply) = runtime
            .settle(&db, &context, &reply_id, &result)
            .await
            .unwrap();
        let original_turn = db
            .collection::<Document>(TURNS)
            .find_one(doc! {"_id": &context.turn.turn_id})
            .await
            .unwrap()
            .unwrap();
        let original_receipt = db
            .collection::<Document>(RECEIPTS)
            .find_one(doc! {"_id": &context.ingress.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        result.error = Some(super::super::assistant_nyxagent::TurnError::internal(
            "assistant_unavailable",
            "public_execution",
        ));
        let SettledExecution::Saved(duplicate) = runtime
            .settle(&db, &context, &reply_id, &result)
            .await
            .unwrap();
        assert_eq!(duplicate, original_reply);
        let idle = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(idle.revoked_at.is_none());
        let mut next = context.clone();
        next.turn.turn_id = uuid::Uuid::new_v4().to_string();
        next.ingress.ingress.receipt_id = uuid::Uuid::new_v4().to_string();
        next.ingress.ingress.claim_id = uuid::Uuid::new_v4().to_string();
        install_turn_attempt(&db, &next, &idle).await.unwrap();
        let successor_before = db
            .collection::<Document>(SESSIONS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        let SettledExecution::Saved(duplicate) = runtime
            .settle(&db, &context, &reply_id, &result)
            .await
            .unwrap();
        assert_eq!(duplicate, original_reply);
        assert_eq!(
            db.collection::<Document>(SESSIONS)
                .find_one(doc! {"_id": &row.id})
                .await
                .unwrap()
                .unwrap(),
            successor_before
        );
        let active = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(active.revoked_at.is_none());
        assert_eq!(active.active_turn.unwrap().turn_id, next.turn.turn_id);
        // Exact old turn + a replacement key is also insufficient authority to
        // clear/revoke. The old receipt may still settle idempotently.
        let (replacement, _) = new_key(
            &row.sponsor_owner_user_id,
            row.expires_at,
            Some(row.credential_api_key_id.clone()),
        );
        db.collection::<ApiKey>(KEYS)
            .insert_one(&replacement)
            .await
            .unwrap();
        db.collection::<PlatformPublicSession>(SESSIONS).update_one(doc! {"_id":&row.id},doc! {"$set":{"credential_api_key_id":&replacement.id,"active_turn":bson::to_bson(&row.active_turn).unwrap()}}).await.unwrap();
        let replaced_before = db
            .collection::<Document>(SESSIONS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        let SettledExecution::Saved(duplicate) = runtime
            .settle(&db, &context, &reply_id, &result)
            .await
            .unwrap();
        assert_eq!(duplicate, original_reply);
        assert_eq!(
            db.collection::<Document>(SESSIONS)
                .find_one(doc! {"_id": &row.id})
                .await
                .unwrap()
                .unwrap(),
            replaced_before
        );
        let replaced = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(replaced.revoked_at.is_none());
        assert_eq!(replaced.active_turn.unwrap().turn_id, context.turn.turn_id);
        assert!(
            db.collection::<ApiKey>(KEYS)
                .find_one(doc! {"_id":&replacement.id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        assert_eq!(
            db.collection::<Document>(TURNS)
                .find_one(doc! {"_id": &context.turn.turn_id})
                .await
                .unwrap()
                .unwrap(),
            original_turn,
            "late failures must preserve the original ciphertext, HMAC and outcome"
        );
        assert_eq!(
            db.collection::<Document>(RECEIPTS)
                .find_one(doc! {"_id": &context.ingress.ingress.receipt_id})
                .await
                .unwrap()
                .unwrap(),
            original_receipt
        );
        let retained = db
            .collection::<PlatformPublicTurn>(TURNS)
            .find_one(doc! {"_id": &context.turn.turn_id})
            .await
            .unwrap()
            .unwrap();
        let text =
            crypto::decrypt_runtime_key(&runtime.state.encryption_keys, &retained.reply_ciphertext)
                .await
                .unwrap();
        assert_eq!(text.as_str(), "safe reply");
        assert_eq!(
            retained.reply_digest,
            reply_digest(&runtime.state, &text).unwrap()
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn failed_preparation_without_turn_row_releases_only_the_exact_claim_and_allows_next_turn()
     {
        let db =
            crate::test_utils::connect_transaction_test_database("public_prepare_recovery").await;
        let (row, context) = recovery_fixture(&db).await;
        assert_eq!(
            db.collection::<Document>(TURNS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        interrupt_turn_in_session(&db, &mut session, &context, false)
            .await
            .unwrap();
        session.abort_transaction().await.unwrap();
        assert!(
            db.collection::<PlatformPublicSession>(SESSIONS)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap()
                .active_turn
                .is_some()
        );
        // Same compensation is used for KMS, quote and reservation errors, even
        // before a disposable turn/budget row exists.
        interrupt_turn(&db, &context, false).await.unwrap();
        let mut idle = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(idle.active_turn.is_none());
        assert!(idle.revoked_at.is_none());
        let receipt = db
            .collection::<Document>(RECEIPTS)
            .find_one(doc! {"_id":&context.ingress.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.get_str("phase").unwrap(), "interrupted");
        assert_eq!(
            receipt.get_str("interrupted_reason").unwrap(),
            "public_prepare_failed"
        );
        let mut next = context.clone();
        next.turn.turn_id = uuid::Uuid::new_v4().to_string();
        next.ingress.ingress.receipt_id = uuid::Uuid::new_v4().to_string();
        install_turn_attempt(&db, &next, &idle).await.unwrap();
        interrupt_turn(&db, &context, false).await.unwrap();
        idle = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(idle.active_turn.unwrap().turn_id, next.turn.turn_id);
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn expired_pre_prepare_turn_recovers_without_execution_proof() {
        assert_expired_turn_recovery(false).await;
    }

    #[tokio::test]
    async fn expired_prepared_turn_recovers_without_waiting_for_session_expiry() {
        assert_expired_turn_recovery(true).await;
    }

    async fn assert_expired_turn_recovery(prepared: bool) {
        let db = crate::test_utils::connect_transaction_test_database("public_turn_lease_recovery")
            .await;
        let (row, context) = if prepared {
            prepared_recovery_fixture(&db).await
        } else {
            recovery_fixture(&db).await
        };
        assert_eq!(
            db.collection::<Document>(TURNS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            u64::from(prepared)
        );
        recover_expired_turn(&db, &row).await.unwrap();
        assert!(
            db.collection::<PlatformPublicSession>(SESSIONS)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap()
                .active_turn
                .is_some()
        );
        db.collection::<Document>(SESSIONS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"active_turn.expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let expired = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        recover_expired_turn(&db, &expired).await.unwrap();
        let saved = db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(saved.active_turn.is_none());
        assert!(saved.revoked_at.is_none());
        assert!(saved.expires_at > Utc::now() + Duration::hours(23));
        let receipt = db
            .collection::<Document>(RECEIPTS)
            .find_one(doc! {"_id":&context.ingress.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            receipt.get_str("interrupted_reason").unwrap(),
            "public_turn_expired"
        );
        let retained = db
            .collection::<PlatformPublicTurn>(TURNS)
            .find_one(doc! {"_id": &context.turn.turn_id})
            .await
            .unwrap();
        if prepared {
            assert_eq!(
                retained.unwrap().outcome.as_deref(),
                Some("public_turn_expired")
            );
        } else {
            assert!(
                retained.is_none(),
                "recovery must not fabricate an execution record"
            );
        }
        // Repeated recovery remains safe after the exact tuple has been cleared.
        recover_expired_turn(&db, &expired).await.unwrap();
        db.drop().await.unwrap();
    }
}
