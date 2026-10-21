//! Stable, encrypted assistant keys. No public response contains this material.
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    crypto::{aes::EncryptionKeys, token::hash_token},
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as KEYS},
        assistant_agent::{AgentGrants, AgentKind, AssistantAgent, COLLECTION_NAME as AGENTS},
        assistant_agent_credential::{AssistantAgentCredential, COLLECTION_NAME as CREDENTIALS},
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
    },
    mw::auth::ASSISTANT_ACCOUNT_SCOPE,
    services::{api_key_mutation_service as mutations, key_service},
};

/// The authority a thread key carries. NyxBot threads always run with Full
/// access; specialist threads carry exactly their agent's grants.
#[derive(Clone, Debug)]
pub enum KeyAuthority {
    Orchestrator,
    Subagent(
        AgentGrants,
        crate::models::agent_operation_scope::OperationScopes,
        String,
    ),
}

impl KeyAuthority {
    pub fn for_agent(agent: &AssistantAgent) -> Self {
        match agent.kind {
            AgentKind::Nyxbot => Self::Orchestrator,
            AgentKind::Specialist => Self::Subagent(
                agent.grants.clone(),
                agent.operation_scopes.clone(),
                agent.user_id.clone(),
            ),
        }
    }

    /// The key fields this authority implies. Applied at every turn start so a
    /// rotated or hand-edited key converges back to its conversation's authority.
    pub fn key_fields(&self) -> bson::Document {
        match self {
            Self::Orchestrator => doc! {
                "assistant_agent_owner_id": bson::Bson::Null,
                "assistant_operation_scopes": bson::Document::new(),
                "allow_all_services": true,
                "allow_all_nodes": true,
                "allow_auto_connected_services": true,
                "scopes": format!("{ASSISTANT_SCOPES} {ASSISTANT_ACCOUNT_SCOPE}"),
            },
            Self::Subagent(grants, scopes, owner) => doc! {
                "assistant_agent_owner_id": owner,
                "assistant_operation_scopes": bson::to_bson(scopes).expect("operation scope serialization"),
                "allow_all_services": false,
                "allow_all_nodes": true,
                "allow_auto_connected_services": false,
                "allowed_service_ids": &grants.service_ids,
                "allowed_platform_service_ids": &grants.platform_service_ids,
                "scopes": if grants.account_read {
                    format!("{ASSISTANT_SCOPES} {ASSISTANT_ACCOUNT_SCOPE}")
                } else {
                    ASSISTANT_SCOPES.to_owned()
                },
            },
        }
    }
}

/// Converge a live conversation key to its authority inside the caller's transaction.
pub async fn apply_authority(
    db: &Database,
    user: &str,
    key: &str,
    authority: &KeyAuthority,
    session: &mut ClientSession,
) -> AppResult<()> {
    require_ordinary_key(db, user, key, Some(&mut *session)).await?;
    let result = mutations::update_one(
        db,
        doc! {"_id": key, "user_id": user, "is_active": true,
        "$or": [{"purpose": "general"}, {"purpose": {"$exists": false}}]},
        doc! {"$set": authority.key_fields()},
        Some(session),
    )
    .await?;
    if result.matched_count != 1 {
        return Err(AppError::NotFound("Conversation key not found".into()));
    }
    Ok(())
}

/// The authority of a thread, read from its agent in the caller's
/// transaction. Rows without an agent are legacy NyxBot threads. A destroyed
/// agent's threads cannot act.
pub async fn authority_in_session(
    db: &Database,
    conversation: &AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<KeyAuthority> {
    Box::pin(authority_with_access(db, conversation, session, None)).await
}

pub(crate) async fn authority_with_access(
    db: &Database,
    conversation: &AssistantConversation,
    session: &mut ClientSession,
    snapshot: Option<&std::sync::Arc<super::org_agent_service::RequestAccess>>,
) -> AppResult<KeyAuthority> {
    if !conversation.credential_api_key_id.is_empty() {
        require_ordinary_key(
            db,
            &conversation.user_id,
            &conversation.credential_api_key_id,
            Some(&mut *session),
        )
        .await?;
    }
    let Some(agent_id) = conversation.agent_id.as_deref() else {
        return Ok(KeyAuthority::Orchestrator);
    };
    let agents = db.collection::<AssistantAgent>(AGENTS);
    let filter = doc! {"_id": agent_id};
    let agent = agents
        .find_one(filter.clone())
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;
    if agent.user_id != conversation.user_id && conversation.group_id.is_some() {
        let access = match snapshot {
            Some(access) if access.matches(&conversation.user_id, &agent.user_id) => access.clone(),
            Some(_) => return Err(super::org_group_service::missing()),
            None => super::org_agent_service::resolve_key_access(
                db,
                &conversation.user_id,
                Some(&agent.user_id),
            )
            .await?
            .ok_or_else(super::org_group_service::missing)?,
        };
        super::org_group_service::check_thread_participation(db, conversation, &access).await?;
        let group = super::org_group_service::get(
            db,
            &conversation.user_id,
            conversation.group_id.as_deref().unwrap_or_default(),
            Some(&access),
        )
        .await?;
        if group.org.is_some() {
            super::org_group_service::fence(db, &group, session).await?;
            if let Some(request) = &conversation.group_request_id {
                super::org_group_service::request(db, &group, request).await?;
            }
        }
    } else if let Some(access) = snapshot {
        if !access.matches(&conversation.user_id, &agent.user_id) {
            return Err(super::org_group_service::missing());
        }
    } else {
        super::org_agent_service::require_use(db, &conversation.user_id, &agent).await?;
    }
    if agent.user_id != conversation.user_id
        && (conversation.guest_turn || conversation.channel.is_some())
    {
        return Err(AppError::Forbidden(
            "Organization agents require a private member thread".into(),
        ));
    }
    let destroyed =
        || AppError::Conflict("This agent was destroyed; its threads are read-only".into());
    if agent.destroyed_at.is_some() {
        return Err(destroyed());
    }
    if agent.kind == AgentKind::Nyxbot {
        return Ok(KeyAuthority::Orchestrator);
    }
    // Fence the specialist row: a concurrent destroy or grant change writes
    // the same document, so one of the two transactions retries and a new or
    // rotated key never carries authority the agent no longer has.
    let mut live = filter;
    live.insert("destroyed_at", bson::Bson::Null);
    let agent = agents
        .find_one_and_update(live, doc! {"$inc": {"thread_fence": 1}})
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *session)
        .await?
        .ok_or_else(destroyed)?;
    Ok(KeyAuthority::for_agent(&agent))
}

// MCP x-api-key initialization/tools/call require REST proxy scope. `proxy`
// also authorizes the LLM proxy (mw::auth::scope_allows_llm_proxy); llm:proxy
// is not a valid ordinary API-key scope in key_service's registry.
pub const ASSISTANT_SCOPES: &str = "proxy";
pub const ASSISTANT_PLATFORM: &str = "nyxid-assistant";

pub struct AssistantCredential {
    pub api_key_id: String,
    pub raw_key: Zeroizing<String>,
    pub newly_provisioned: bool,
    revoked_children: Vec<crate::models::api_key_credential::ApiKeyCredential>,
}
impl std::fmt::Debug for AssistantCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssistantCredential { [REDACTED] }")
    }
}

fn unavailable() -> AppError {
    AppError::Internal("Assistant credential storage unavailable".into())
}

fn require_ordinary_purpose(row: &bson::Document) -> AppResult<()> {
    match row.get("purpose") {
        None => Ok(()),
        Some(bson::Bson::String(purpose)) if purpose == "general" => Ok(()),
        _ => Err(AppError::Forbidden(
            "Key is not an ordinary assistant credential".into(),
        )),
    }
}

/// Check the unfiltered key before decryption, convergence or invalid-key
/// recovery. A purpose mismatch must never become a newly minted owner key.
async fn require_ordinary_key(
    db: &Database,
    user: &str,
    key: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let collection = db.collection::<bson::Document>(KEYS);
    let read = collection
        .find_one(doc! {"_id": key})
        .projection(doc! {"purpose": 1, "user_id": 1});
    let row = match session {
        Some(session) => read.session(session).await?,
        None => read.await?,
    };
    if let Some(row) = row {
        if row.get_str("user_id").ok() != Some(user) {
            return Err(AppError::Forbidden(
                "Assistant credential owner changed".into(),
            ));
        }
        require_ordinary_purpose(&row)?;
    }
    Ok(())
}

/// Whether `key_id` is a live assistant conversation key of `user_id` (NyxID
/// stores one per conversation; people cannot create these rows).
pub async fn is_conversation_key(db: &Database, user_id: &str, key_id: &str) -> AppResult<bool> {
    Ok(db
        .collection::<AssistantAgentCredential>(CREDENTIALS)
        .count_documents(doc! {"user_id": user_id, "api_key_id": key_id})
        .limit(1)
        .await
        .map_err(|_| unavailable())?
        > 0)
}

/// Used by key revocation in the same transaction when one exists.
/// Exact key matching prevents a late invalidation from deleting its successor.
pub async fn invalidate_for_key(
    db: &Database,
    user_id: &str,
    key_id: &str,
    mut session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let credential_collection = db.collection::<AssistantAgentCredential>(CREDENTIALS);
    let delete = credential_collection.delete_one(doc! {"user_id": user_id, "api_key_id": key_id});
    match session.as_deref_mut() {
        Some(s) => delete.session(s).await.map_err(|_| unavailable())?,
        None => delete.await.map_err(|_| unavailable())?,
    };
    clear_key_bindings(db, user_id, key_id, session).await
}

async fn clear_key_bindings(
    db: &Database,
    user_id: &str,
    key_id: &str,
    mut session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let acknowledgements =
        db.collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME);
    let expire = acknowledgements.update_many(
        doc! {"user_id": user_id, "api_key_id": key_id, "status": {"$in": ["pending", "allowed"]}},
        doc! {"$set": {"status": "expired"}},
    );
    match session.as_deref_mut() {
        Some(s) => expire.session(s).await?,
        None => expire.await?,
    };
    let conversation_collection = db.collection::<AssistantConversation>(CONVERSATIONS);
    let update = conversation_collection.update_many(
        doc! {"user_id": user_id, "credential_api_key_id": key_id},
        doc! {"$set": {
            "nyxagent_session_id": bson::Bson::Null,
            "nyxagent_instruction_binding": bson::Bson::Null,
            "nyxagent_last_response_id": bson::Bson::Null,
            "context_reset_at": bson::DateTime::from_chrono(Utc::now()),
            "context_reset_reason": "credential_replaced",
        }},
    );
    match session {
        Some(s) => update.session(s).await?,
        None => update.await?,
    };
    Ok(())
}

/// Adopt a rotation successor without minting another assistant key. The caller
/// owns the rotation transaction, so ciphertext and key lineage commit together.
pub async fn adopt_rotated_key(
    db: &Database,
    keys: &EncryptionKeys,
    user_id: &str,
    predecessor_id: &str,
    successor_id: &str,
    successor_key: &str,
    session: &mut ClientSession,
) -> AppResult<bool> {
    let collection = db.collection::<bson::Document>(CREDENTIALS);
    let filter = doc! {"user_id": user_id, "api_key_id": predecessor_id};
    let stored = collection
        .find_one(filter.clone())
        .session(&mut *session)
        .await
        .map_err(|_| unavailable())?;
    let exists = stored.is_some();
    if let Some(stored) = stored {
        let mut row: AssistantAgentCredential =
            bson::from_document(stored.clone()).map_err(|_| unavailable())?;
        row.api_key_id = successor_id.to_owned();
        require_ordinary_key(db, user_id, predecessor_id, Some(&mut *session)).await?;
        require_ordinary_key(db, user_id, successor_id, Some(&mut *session)).await?;
        let ciphertext = super::assistant_record_service::encrypt_credential(
            keys,
            &row,
            successor_key.as_bytes(),
        )
        .await?;
        super::credential_record_service::mark_version(
            &mut row.secret_context_versions,
            keys,
            "key_ciphertext",
        );
        let filter = super::record_storage::exact_document_predicate(&stored);
        let updated = collection
            .update_one(
                filter,
                doc! {"$set": {
                    "api_key_id": successor_id,
                    "secret_context_versions":bson::to_bson(&row.secret_context_versions).map_err(|_|unavailable())?,
                    "key_ciphertext": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: ciphertext,
                    },
                    "last_used_at": bson::DateTime::from_chrono(Utc::now()),
                }},
            )
            .session(&mut *session)
            .await
            .map_err(|_| unavailable())?;
        if updated.matched_count != 1 {
            return Err(AppError::Conflict(
                "Assistant credential changed during rotation".into(),
            ));
        }
    }
    clear_key_bindings(db, user_id, predecessor_id, Some(session)).await?;
    Ok(exists)
}

/// User disable revokes every API key; remove its private assistant copy too.
pub async fn invalidate_for_owner(db: &Database, user_id: &str) -> AppResult<()> {
    let rows: Vec<AssistantAgentCredential> = db
        .collection(CREDENTIALS)
        .find(doc! {"user_id": user_id})
        .await
        .map_err(|_| unavailable())?
        .try_collect()
        .await
        .map_err(|_| unavailable())?;
    for row in rows {
        invalidate_for_key(db, user_id, &row.api_key_id, None).await?;
    }
    Ok(())
}

pub async fn exists(db: &Database, user_id: &str) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(CREDENTIALS)
        .find_one(doc! {"user_id": user_id})
        .projection(doc! {"_id": 1})
        .await
        .map_err(|_| unavailable())?
        .is_some())
}

/// A generation boundary must not invalidate another family's pending work.
/// Called only for invocation-controlled transitions, within admission's txn.
pub(crate) async fn require_invocation_boundary_clear(
    db: &Database,
    owner: &str,
    conversation: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let checks = [
        (
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"user_id":owner,"conversation_id":conversation,"$or":[
                {"status":"pending","expires_at":{"$gt":bson::DateTime::now()}},
                {"status":"allowed","kind":{"$in":["action","skills"]},"expires_at":{"$gt":bson::DateTime::now()}},
                {"skill_notification_pending":true}
            ]},
        ),
        (
            crate::models::async_service_operation::COLLECTION_NAME,
            doc! {"user_id":owner,"conversation_id":conversation,"state":{"$in":["submitting","waiting","cancelling","ready","queued"]}},
        ),
        (
            crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
            doc! {"user_id":owner,"conversation_id":conversation,"status":{"$in":["pending","claimed"]}},
        ),
        (
            crate::models::machine_job::COLLECTION_NAME,
            doc! {"user_id":owner,"conversation_id":conversation,"state":"running"},
        ),
    ];
    for (collection, filter) in checks {
        if db
            .collection::<bson::Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .session(&mut *session)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict(
                "Conversation has pending work from an earlier credential generation; finish or cancel it, or choose a new conversation".into(),
            ));
        }
    }
    // Authored drafts can reuse expired cards. Their durable claims and cards
    // retain source credentials until publication is finished or discarded.
    use crate::models::assistant_agent_learning as learning;
    for (collection, filter, proposal_field) in [
        (
            learning::AUTHORED_CLAIMS_COLLECTION_NAME,
            doc! {"claim.conversation_id":conversation},
            "claim.proposal_id",
        ),
        (
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"user_id":owner,"conversation_id":conversation,"authored_skill.proposal_id":{"$exists":true}},
            "authored_skill.proposal_id",
        ),
    ] {
        let mut cursor = db.collection::<bson::Document>(collection).aggregate([
            doc! {"$match":filter},
            doc! {"$lookup":{"from":learning::PROPOSALS_COLLECTION_NAME,"localField":proposal_field,"foreignField":"_id","as":"proposal"}},
            doc! {"$match":{"proposal.owner_id":owner,"proposal.status":{"$in":["pending","publishing","published_unpinned","publication_failed"]}}},
            doc! {"$limit":1},
            doc! {"$project":{"_id":1}},
        ]).session(&mut *session).await?;
        if cursor.next(&mut *session).await.transpose()?.is_some() {
            return Err(AppError::Conflict(
                "Conversation has an unfinished skill draft from an earlier credential generation; finish or discard it, or choose a new conversation".into(),
            ));
        }
    }
    Ok(())
}

/// Called inside the turn transaction: the conversation, key and ciphertext
/// become durable together. Touching the key fences concurrent revocation.
pub async fn load_or_provision_in_session(
    db: &Database,
    keys: &EncryptionKeys,
    user_id: &str,
    conversation_id: &str,
    authority: &KeyAuthority,
    session: &mut ClientSession,
) -> AppResult<AssistantCredential> {
    super::erasure_admission::fence_in_session(db, user_id, session).await?;
    let old = db
        .collection::<AssistantAgentCredential>(CREDENTIALS)
        .find_one(doc! {"user_id": user_id, "conversation_id": conversation_id})
        .session(&mut *session)
        .await
        .map_err(|_| unavailable())?;
    let mut revoked_children = Vec::new();
    if let Some(old) = old {
        require_ordinary_key(db, user_id, &old.api_key_id, Some(&mut *session)).await?;
        let raw = decrypt(keys, &old).await?;
        let valid = db
            .collection::<ApiKey>(KEYS)
            .find_one(valid_key_filter(user_id, &old.api_key_id, &raw))
            .session(&mut *session)
            .await?
            .is_some();
        if valid {
            let mut fields = authority.key_fields();
            fields.insert("last_used_at", bson::DateTime::now());
            mutations::update_one(
                db,
                doc! {"_id": &old.api_key_id, "user_id": user_id},
                // Upgrades legacy Ask-mode keys and re-applies subagent grants.
                doc! {"$set": fields},
                Some(&mut *session),
            )
            .await?;
            db.collection::<AssistantAgentCredential>(CREDENTIALS)
                .update_one(
                    doc! {"_id": &old.id},
                    doc! {"$set": {"last_used_at": bson::DateTime::now()}},
                )
                .session(&mut *session)
                .await
                .map_err(|_| unavailable())?;
            return Ok(AssistantCredential {
                api_key_id: old.api_key_id,
                raw_key: raw,
                newly_provisioned: false,
                revoked_children: Vec::new(),
            });
        }
        match key_service::delete_api_key_in_session(
            db,
            user_id,
            &old.api_key_id,
            None,
            Some(&mut *session),
        )
        .await
        {
            Ok(children) => revoked_children = children,
            Err(AppError::NotFound(_)) => {
                invalidate_for_key(db, user_id, &old.api_key_id, Some(&mut *session)).await?;
            }
            Err(error) => return Err(error),
        }
    }
    let suffix = conversation_id
        .strip_prefix("nyxa-")
        .unwrap_or(conversation_id);
    let name = format!(
        "NyxID Assistant chat {}",
        suffix.chars().take(8).collect::<String>()
    );
    let created = key_service::create_api_key_with_security_class_and_id(
        db,
        user_id,
        // Empty bootstrap grants need no visibility enumeration. The checked
        // agent authority is applied below in the same transaction.
        None,
        None,
        &name,
        ASSISTANT_SCOPES,
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(false),
        Some(true),
        Some(false),
        None,
        None,
        Some(ASSISTANT_PLATFORM),
        None,
        None,
        ApiKeyPurpose::General,
        false,
        Some(&mut *session),
    )
    .await?;
    apply_authority(db, user_id, &created.id, authority, session).await?;
    let raw = Zeroizing::new(created.full_key);
    let now = Utc::now();
    let mut row = AssistantAgentCredential {
        secret_context_versions: Default::default(),
        id: Uuid::new_v4().to_string(),
        user_id: user_id.into(),
        conversation_id: conversation_id.into(),
        api_key_id: created.id.clone(),
        key_ciphertext: Vec::new(),
        created_at: now,
        last_used_at: now,
    };
    row.key_ciphertext =
        super::assistant_record_service::encrypt_credential(keys, &row, raw.as_bytes()).await?;
    super::credential_record_service::mark_version(
        &mut row.secret_context_versions,
        keys,
        "key_ciphertext",
    );
    db.collection::<AssistantAgentCredential>(CREDENTIALS)
        .insert_one(row)
        .session(&mut *session)
        .await
        .map_err(|_| unavailable())?;
    Ok(AssistantCredential {
        api_key_id: created.id,
        raw_key: raw,
        newly_provisioned: true,
        revoked_children,
    })
}

/// Recovery test helper; production recovery occurs through turn admission or replacement.
#[cfg(test)]
pub async fn load_or_provision(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    user_id: &str,
    conversation_id: &str,
) -> AppResult<AssistantCredential> {
    recover_or_replace(db, keys, user_id, conversation_id, None, None).await
}

async fn recover_or_replace(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    user_id: &str,
    conversation_id: &str,
    replace_key_id: Option<&str>,
    platform: Option<(
        super::platform_manager_contract::TurnRef,
        super::platform_manager_contract::PlatformTurnOrigin,
    )>,
) -> AppResult<AssistantCredential> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let keys = keys.clone();
    let user_id = user_id.to_owned();
    let conversation_id = conversation_id.to_owned();
    let replace_key_id = replace_key_id.map(str::to_owned);
    let audit_db = db.clone();
    let audit_user = user_id.clone();
    let audit_conversation = conversation_id.clone();
    let credential = session
        .start_transaction()
        .and_run(
            (db, keys, user_id, conversation_id, replace_key_id, platform),
            |session, (db, keys, user_id, conversation_id, replace_key_id, platform)| {
                Box::pin(async move {
                    let operation = async {
                        // Write the conversation as well to fence concurrent deletion.
                        let result = db
                            .collection::<AssistantConversation>(CONVERSATIONS)
                            .update_one(
                                doc! {"_id": conversation_id.as_str(), "user_id": user_id.as_str()},
                                doc! {"$inc": {"credential_fence": 1}},
                            )
                            .session(&mut *session)
                            .await?;
                        if result.matched_count != 1 {
                            return Err(AppError::NotFound("Conversation not found".into()));
                        }
                        let conversation = db
                            .collection::<AssistantConversation>(CONVERSATIONS)
                            .find_one(doc! {"_id": conversation_id.as_str(), "user_id": user_id.as_str()})
                            .session(&mut *session)
                            .await?
                            .ok_or_else(|| AppError::NotFound("Conversation not found".into()))?;
                        if let Some((turn, expected)) = platform.as_ref() {
                            let expected_model = super::platform_personal_turn_service::model_origin(expected);
                            if expected.authority.user_id != user_id.as_str()
                                || turn.conversation_id != conversation_id.as_str()
                                || conversation.active_turn.as_ref().is_none_or(|active| {
                                    active.turn_id != turn.turn_id
                                        || active.stop_requested
                                        || active.platform_origin.as_ref() != Some(&expected_model)
                                })
                                || super::assistant_nyxagent::live_turn(&conversation, Utc::now()).is_none()
                                || replace_key_id.as_deref().is_some_and(|key| key != conversation.credential_api_key_id)
                                || !super::platform_personal_binding_service::validate_personal_origin_in_session(
                                    db, session, &expected.authority,
                                ).await?
                            {
                                return Err(AppError::AssistantTurnRequired);
                            }
                        } else if conversation.active_turn.as_ref().is_some_and(|active| active.platform_origin.is_some()) {
                            // Metadata/ordinary recovery has no admitted platform proof.
                            return Err(AppError::AssistantTurnRequired);
                        }
                        let authority = authority_in_session(db, &conversation, session).await?;
                        let mut revoked_children = Vec::new();
                        if let Some(key_id) = replace_key_id.as_deref() {
                            require_ordinary_key(db, user_id.as_str(), key_id, Some(&mut *session)).await?;
                            match key_service::delete_api_key_in_session(
                                db, user_id.as_str(), key_id, None, Some(&mut *session),
                            ).await {
                                Ok(children) => revoked_children = children,
                                Err(AppError::NotFound(_)) => {
                                    invalidate_for_key(db, user_id.as_str(), key_id, Some(&mut *session)).await?;
                                }
                                Err(error) => return Err(error),
                            }
                        }
                        let mut credential = load_or_provision_in_session(
                            db, keys, user_id.as_str(), conversation_id.as_str(), &authority, session,
                        ).await?;
                        if let Some((turn, expected)) = platform.as_ref() {
                            let update = db
                                .collection::<AssistantConversation>(CONVERSATIONS)
                                .update_one(
                                    doc! {
                                        "_id": conversation_id.as_str(),
                                        "user_id": user_id.as_str(),
                                        "active_turn.turn_id": &turn.turn_id,
                                        "credential_api_key_id": &conversation.credential_api_key_id,
                                        "active_turn.stop_requested": false,
                                        "active_turn.platform_origin": bson::to_bson(
                                            &super::platform_personal_turn_service::model_origin(expected),
                                        ).map_err(|_| AppError::AssistantTurnRequired)?,
                                    },
                                    doc! {"$set": {
                                        "credential_api_key_id": &credential.api_key_id,
                                        "nyxagent_session_id": bson::Bson::Null,
                                        "nyxagent_instruction_binding": bson::Bson::Null,
                                        "nyxagent_last_response_id": bson::Bson::Null,
                                        "context_reset_at": bson::DateTime::now(),
                                        "context_reset_reason": "credential_replaced",
                                    }},
                                )
                                .session(&mut *session)
                                .await?;
                            if update.matched_count != 1 {
                                return Err(AppError::AssistantTurnRequired);
                            }
                            // KMS unwrap/wrap is finished. Fence the exact adopted key,
                            // turn and live origin before commit; denial rolls back both
                            // deletion of the predecessor and creation of the successor.
                            if super::platform_personal_turn_service::fence_execution_in_session(
                                db, session, &super::platform_personal_binding_service::IdentityProducer,
                                user_id.as_str(), &credential.api_key_id, turn, expected,
                            ).await? != super::platform_manager_contract::FenceOutcome::Authorized {
                                return Err(AppError::AssistantTurnRequired);
                            }
                        }
                        credential.revoked_children.extend(revoked_children);
                        Ok(credential)
                    }.await;
                    mutations::transaction_result(operation)
                })
            },
        )
        .await
        .map_err(mutations::map_transaction_error)?;
    audit_provision(
        &audit_db,
        &audit_user,
        &audit_conversation,
        &credential,
        true,
    )
    .await;
    Ok(credential)
}

pub async fn audit_provision(
    db: &Database,
    user_id: &str,
    conversation_id: &str,
    credential: &AssistantCredential,
    replaced: bool,
) {
    super::api_key_credential_service::audit_revocations(
        db,
        &credential.revoked_children,
        crate::models::api_key_credential::CredentialRevokedReason::ParentRevoked,
    );
    if credential.newly_provisioned {
        let _ = super::audit_service::log_actor_event(
            db.clone(),
            &super::audit_service::AuditActor {
                user_id: user_id.into(),
                api_key_id: Some(credential.api_key_id.clone()),
                api_key_name: None,
                ip_address: None,
                user_agent: None,
            },
            if replaced {
                "assistant_agent_credential_replaced"
            } else {
                "assistant_agent_credential_provisioned"
            },
            Some(serde_json::json!({
                "conversation_id": conversation_id,
                "api_key_id": credential.api_key_id,
            })),
        )
        .await;
    }
}

pub async fn replace(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    user_id: &str,
    conversation_id: &str,
    old_key_id: &str,
) -> AppResult<AssistantCredential> {
    recover_or_replace(db, keys, user_id, conversation_id, Some(old_key_id), None).await
}

/// Personal recovery adopts its successor before the next typed request. Its
/// old turn/origin and live consent are revalidated before revoking any key.
pub(crate) async fn replace_for_turn(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    row: &AssistantConversation,
    old_key_id: &str,
) -> AppResult<AssistantCredential> {
    let platform = row
        .active_turn
        .as_ref()
        .and_then(|turn| turn.platform_origin.as_ref().map(|origin| (turn, origin)))
        .map(|(turn, origin)| {
            Ok::<_, AppError>((
                super::platform_manager_contract::TurnRef {
                    conversation_id: row.id.clone(),
                    turn_id: turn.turn_id.clone(),
                },
                super::platform_personal_turn_service::contract_origin(origin)?,
            ))
        })
        .transpose()?;
    if platform.is_none() {
        return replace(db, keys, &row.user_id, &row.id, old_key_id).await;
    }
    recover_or_replace(db, keys, &row.user_id, &row.id, Some(old_key_id), platform).await
}

fn valid_key_filter(user_id: &str, key_id: &str, raw: &str) -> bson::Document {
    doc! {
        "_id": key_id, "user_id": user_id, "key_hash": hash_token(raw), "is_active": true,
        "$and": [
            {"$or": [{"expires_at": bson::Bson::Null}, {"expires_at": {"$gt": bson::DateTime::now()}}]},
            {"$or": [{"purpose": "general"}, {"purpose": {"$exists": false}}]},
        ],
    }
}

async fn decrypt(
    keys: &EncryptionKeys,
    row: &AssistantAgentCredential,
) -> AppResult<Zeroizing<String>> {
    super::assistant_record_service::decrypt_credential(keys, row).await
}

/// Metadata and deletion never provision a key as a side effect. Models select
/// the most recently used valid conversation credential, skipping revoked rows.
pub async fn load_existing(
    db: &Database,
    keys: &EncryptionKeys,
    user_id: &str,
) -> AppResult<Option<AssistantCredential>> {
    load_existing_filtered(db, keys, user_id, doc! {"user_id": user_id}).await
}

pub async fn load_for_conversation(
    db: &Database,
    keys: &EncryptionKeys,
    user_id: &str,
    conversation_id: &str,
) -> AppResult<Option<AssistantCredential>> {
    load_existing_filtered(
        db,
        keys,
        user_id,
        doc! {"user_id": user_id, "conversation_id": conversation_id},
    )
    .await
}

async fn load_existing_filtered(
    db: &Database,
    keys: &EncryptionKeys,
    user_id: &str,
    filter: bson::Document,
) -> AppResult<Option<AssistantCredential>> {
    let mut cursor = db
        .collection::<AssistantAgentCredential>(CREDENTIALS)
        .find(filter)
        .sort(doc! {"last_used_at": -1})
        .await
        .map_err(|_| unavailable())?;
    while let Some(row) = cursor.try_next().await.map_err(|_| unavailable())? {
        require_ordinary_key(db, user_id, &row.api_key_id, None).await?;
        let raw = decrypt(keys, &row).await?;
        if db
            .collection::<ApiKey>(KEYS)
            .find_one(valid_key_filter(user_id, &row.api_key_id, &raw))
            .await?
            .is_some()
        {
            return Ok(Some(AssistantCredential {
                api_key_id: row.api_key_id,
                raw_key: raw,
                newly_provisioned: false,
                revoked_children: Vec::new(),
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::user::{COLLECTION_NAME as USERS, UserType},
        services::assistant_nyxagent as engine,
        test_utils::{connect_transaction_test_database, test_app_state, test_user},
    };

    async fn fixture(name: &str) -> (crate::AppState, String, AssistantConversation) {
        let db = connect_transaction_test_database(name).await;
        engine::ensure_indexes(&db).await.unwrap();
        let owner = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let row = new_chat(&state, &owner).await;
        (state, owner, row)
    }

    async fn new_chat(state: &crate::AppState, owner: &str) -> AssistantConversation {
        engine::begin_turn(
            &state.db,
            owner,
            &engine::TurnRequest {
                acknowledgement_id: None,
                retry_of: None,
                attachment_ids: Vec::new(),
                agent_id: None,
                conversation_id: None,
                text: "hello".into(),
                model: None,
                access_mode: None,
            },
            &state.encryption_keys,
        )
        .await
        .unwrap()
    }

    #[test]
    fn ordinary_purpose_accepts_only_general_and_legacy_absence() {
        assert!(require_ordinary_purpose(&doc! {}).is_ok());
        assert!(require_ordinary_purpose(&doc! {"purpose": "general"}).is_ok());
        for purpose in [
            bson::Bson::Null,
            bson::Bson::Int32(1),
            "public_nyxbot".into(),
            "scheduled_invocation".into(),
            "permission_bound".into(),
            "unknown".into(),
        ] {
            assert!(require_ordinary_purpose(&doc! {"purpose": purpose}).is_err());
        }
    }

    #[tokio::test]
    async fn nonordinary_key_cannot_be_loaded_converged_replaced_or_adopted() {
        let (state, owner, row) = fixture("nyxa_nonordinary_credentials").await;
        let db = &state.db;
        let keys = db.collection::<bson::Document>(KEYS);
        let credentials = db.collection::<AssistantAgentCredential>(CREDENTIALS);
        let before = credentials
            .find_one(doc! {"conversation_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        for purpose in [
            "public_nyxbot",
            "scheduled_invocation",
            "permission_bound",
            "unknown",
        ] {
            keys.update_one(
                doc! {"_id": &row.credential_api_key_id},
                doc! {"$set": {"purpose": purpose}},
            )
            .await
            .unwrap();
            assert!(
                load_for_conversation(db, &state.encryption_keys, &owner, &row.id)
                    .await
                    .is_err()
            );
            assert!(
                load_or_provision(db, &state.encryption_keys, &owner, &row.id)
                    .await
                    .is_err()
            );
            assert!(
                replace(
                    db,
                    &state.encryption_keys,
                    &owner,
                    &row.id,
                    &row.credential_api_key_id
                )
                .await
                .is_err()
            );
            let mut session = db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            assert!(
                apply_authority(
                    db,
                    &owner,
                    &row.credential_api_key_id,
                    &KeyAuthority::Orchestrator,
                    &mut session
                )
                .await
                .is_err()
            );
            assert!(
                adopt_rotated_key(
                    db,
                    &state.encryption_keys,
                    &owner,
                    &row.credential_api_key_id,
                    &row.credential_api_key_id,
                    "unusable-successor",
                    &mut session
                )
                .await
                .is_err()
            );
            session.abort_transaction().await.unwrap();
            let key = keys
                .find_one(doc! {"_id": &row.credential_api_key_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(key.get_str("purpose").unwrap(), purpose);
            assert!(key.get_bool("is_active").unwrap());
            assert_eq!(
                keys.count_documents(doc! {"user_id": &owner})
                    .await
                    .unwrap(),
                1
            );
            let after = credentials
                .find_one(doc! {"conversation_id": &row.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(after.api_key_id, before.api_key_id);
            assert_eq!(after.key_ciphertext, before.key_ciphertext);
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn foreign_restricted_pointer_cannot_be_recovered_as_an_owner_key() {
        let (state, owner, row) = fixture("nyxa_foreign_public_credential").await;
        let db = &state.db;
        let keys = db.collection::<bson::Document>(KEYS);
        let mut foreign = keys
            .find_one(doc! {"_id": &row.credential_api_key_id})
            .await
            .unwrap()
            .unwrap();
        foreign.insert("_id", "foreign-restricted-key");
        foreign.insert("user_id", "sponsor-owner");
        foreign.insert("purpose", "public_nyxbot");
        keys.insert_one(foreign).await.unwrap();
        db.collection::<bson::Document>(CREDENTIALS)
            .update_one(
                doc! {"conversation_id": &row.id},
                doc! {"$set": {"api_key_id": "foreign-restricted-key"}},
            )
            .await
            .unwrap();
        db.collection::<bson::Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"credential_api_key_id": "foreign-restricted-key"}},
            )
            .await
            .unwrap();
        assert!(
            load_for_conversation(db, &state.encryption_keys, &owner, &row.id)
                .await
                .is_err()
        );
        assert!(
            load_or_provision(db, &state.encryption_keys, &owner, &row.id)
                .await
                .is_err()
        );
        assert!(
            replace(
                db,
                &state.encryption_keys,
                &owner,
                &row.id,
                "foreign-restricted-key"
            )
            .await
            .is_err()
        );
        assert_eq!(keys.count_documents(doc! {}).await.unwrap(), 2);
        let foreign = keys
            .find_one(doc! {"_id": "foreign-restricted-key"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(foreign.get_str("purpose").unwrap(), "public_nyxbot");
        assert!(foreign.get_bool("is_active").unwrap());
        let stored = db
            .collection::<AssistantAgentCredential>(CREDENTIALS)
            .find_one(doc! {"conversation_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.api_key_id, "foreign-restricted-key");
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn replacement_authority_failure_rolls_back_key_revocation() {
        let (state, owner, row) = fixture("nyxa_replace_authority_rollback").await;
        let db = &state.db;
        db.collection::<bson::Document>(AGENTS)
            .update_one(
                doc! {"_id": &row.agent_id},
                doc! {"$set": {"destroyed_at": bson::DateTime::now()}},
            )
            .await
            .unwrap();
        assert!(
            replace(
                db,
                &state.encryption_keys,
                &owner,
                &row.id,
                &row.credential_api_key_id
            )
            .await
            .is_err()
        );
        let key = db
            .collection::<ApiKey>(KEYS)
            .find_one(doc! {"_id": &row.credential_api_key_id})
            .await
            .unwrap()
            .unwrap();
        assert!(key.is_active);
        let stored = db
            .collection::<AssistantAgentCredential>(CREDENTIALS)
            .find_one(doc! {"conversation_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.api_key_id, key.id);
        assert!(
            load_for_conversation(db, &state.encryption_keys, &owner, &row.id)
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_provisioning_is_stable_encrypted_and_scoped_for_mcp_and_llm() {
        let (state, owner, row) = fixture("nyxa_credentials").await;
        let db = &state.db;
        let (a, b) = tokio::join!(
            load_or_provision(db, &state.encryption_keys, &owner, &row.id),
            load_or_provision(db, &state.encryption_keys, &owner, &row.id),
        );
        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(a.api_key_id, b.api_key_id);
        assert_eq!(a.raw_key.as_str(), b.raw_key.as_str());
        assert!(a.raw_key.starts_with("nyxid_ag_"));
        let stored = db
            .collection::<AssistantAgentCredential>(CREDENTIALS)
            .find_one(doc! {"conversation_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.user_id, owner);
        assert_ne!(stored.key_ciphertext, a.raw_key.as_bytes());
        assert_eq!(
            super::super::assistant_record_service::decrypt_credential(
                &state.encryption_keys,
                &stored
            )
            .await
            .unwrap()
            .as_bytes(),
            a.raw_key.as_bytes()
        );
        assert!(!format!("{stored:?} {a:?}").contains(a.raw_key.as_str()));
        assert!(!format!("{stored:?} {a:?}").contains(&a.api_key_id));
        let bson = bson::to_document(&stored).unwrap();
        assert!(matches!(
            bson.get("created_at"),
            Some(bson::Bson::DateTime(_))
        ));
        assert!(matches!(
            bson.get("last_used_at"),
            Some(bson::Bson::DateTime(_))
        ));
        assert!(matches!(
            bson.get("key_ciphertext"),
            Some(bson::Bson::Binary(_))
        ));
        let key = key_service::get_api_key(db, &owner, &a.api_key_id)
            .await
            .unwrap();
        assert_eq!(key.key_hash, hash_token(&a.raw_key));
        assert_eq!(key.platform.as_deref(), Some(ASSISTANT_PLATFORM));
        assert_eq!(key.name, format!("NyxID Assistant chat {}", &row.id[5..13]));
        // New chats are NyxBot orchestrators: Full access.
        assert!(key.allow_all_services && key.allow_all_nodes && key.allow_auto_connected_services);
        assert!(key.allowed_service_ids.is_empty() && key.allowed_node_ids.is_empty());
        assert!(key.expires_at.is_none());
        assert_eq!(key.scopes, format!("proxy {ASSISTANT_ACCOUNT_SCOPE}"));
        assert!(crate::mw::auth::scope_allows_rest_proxy(&key.scopes));
        assert!(crate::mw::auth::scope_allows_llm_proxy(&key.scopes));
        let second = new_chat(&state, &owner).await;
        assert_ne!(second.credential_api_key_id, a.api_key_id);
        assert_eq!(
            db.collection::<ApiKey>(KEYS)
                .count_documents(doc! {"user_id": &owner})
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            db.collection::<AssistantAgentCredential>(CREDENTIALS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
        assert!(
            load_for_conversation(db, &state.encryption_keys, "other", &row.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn existing_conversation_keys_converge_to_their_role_authority() {
        let (state, owner, row) = fixture("nyxa_restore_nodes").await;
        let db = &state.db;
        db.collection::<ApiKey>(KEYS)
            .update_one(
                doc! {"_id": &row.credential_api_key_id},
                doc! {"$set": {"allow_all_nodes": false, "allow_all_services": false,
                "scopes": "proxy"}},
            )
            .await
            .unwrap();
        let credential = load_or_provision(db, &state.encryption_keys, &owner, &row.id)
            .await
            .unwrap();
        assert_eq!(credential.api_key_id, row.credential_api_key_id);
        assert!(!credential.newly_provisioned);
        let key = key_service::get_api_key(db, &owner, &credential.api_key_id)
            .await
            .unwrap();
        assert!(key.allow_all_nodes && key.allow_all_services);
        assert_eq!(key.scopes, format!("proxy {ASSISTANT_ACCOUNT_SCOPE}"));
        // A subagent's key never widens beyond its recorded grants.
        let (_, subagent) = crate::services::assistant_team_service::create_specialist(
            db,
            &state.encryption_keys,
            &owner,
            crate::services::assistant_team_service::CreateRequest {
                machines: None,
                logins: None,
                name: "reader".into(),
                description: "Read things".into(),
                display_name: None,
                persona: None,
                targets: Default::default(),
                account_read: false,
                specialty: None,
                created_by: "user",
            },
        )
        .await
        .unwrap()
        .unwrap();
        mutations::update_one(
            db,
            doc! {"_id": &subagent.credential_api_key_id},
            doc! {"$set": {"allow_all_services": true, "scopes": "proxy assistant:account"}},
            None,
        )
        .await
        .unwrap();
        let credential = load_or_provision(db, &state.encryption_keys, &owner, &subagent.id)
            .await
            .unwrap();
        let key = key_service::get_api_key(db, &owner, &credential.api_key_id)
            .await
            .unwrap();
        assert!(!key.allow_all_services && !key.allow_auto_connected_services);
        assert!(key.allow_all_nodes && key.allowed_service_ids.is_empty());
        assert_eq!(key.scopes, "proxy");
    }

    #[tokio::test]
    async fn revoke_expire_and_hash_change_remove_ciphertext_and_rebind() {
        let (state, owner, row) = fixture("nyxa_key_lifecycle").await;
        let db = &state.db;
        let mut credential = load_for_conversation(db, &state.encryption_keys, &owner, &row.id)
            .await
            .unwrap()
            .unwrap();
        let other = new_chat(&state, &owner).await;
        for action in ["revoke", "expire", "hash", "missing"] {
            db.collection::<AssistantConversation>(CONVERSATIONS).update_one(doc! {"_id": &row.id},
                doc! {"$set": {"credential_api_key_id": &credential.api_key_id, "nyxagent_session_id": "old-session"}}).await.unwrap();
            match action {
                "revoke" => key_service::delete_api_key(db, &owner, &credential.api_key_id)
                    .await
                    .unwrap(),
                "expire" => {
                    db.collection::<ApiKey>(KEYS)
                        .update_one(
                            doc! {"_id": &credential.api_key_id},
                            doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
                        )
                        .await
                        .unwrap();
                }
                "hash" => {
                    db.collection::<ApiKey>(KEYS)
                        .update_one(
                            doc! {"_id": &credential.api_key_id},
                            doc! {"$set": {"key_hash": "different"}},
                        )
                        .await
                        .unwrap();
                }
                _ => {
                    db.collection::<ApiKey>(KEYS)
                        .delete_one(doc! {"_id": &credential.api_key_id})
                        .await
                        .unwrap();
                }
            }
            let replacement = load_or_provision(db, &state.encryption_keys, &owner, &row.id)
                .await
                .unwrap();
            assert_ne!(replacement.api_key_id, credential.api_key_id, "{action}");
            let saved = engine::get(db, &owner, &row.id).await.unwrap();
            assert!(saved.nyxagent_session_id.is_none(), "{action}");
            assert_eq!(
                saved.context_reset_reason.as_deref(),
                Some("credential_replaced")
            );
            let key = key_service::get_api_key(db, &owner, &replacement.api_key_id)
                .await
                .unwrap();
            assert!(key.allowed_service_ids.is_empty());
            assert!(key.allow_all_nodes && key.allow_all_services);
            assert_eq!(key.scopes, format!("proxy {ASSISTANT_ACCOUNT_SCOPE}"));
            assert_eq!(
                load_for_conversation(db, &state.encryption_keys, &owner, &other.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .api_key_id,
                other.credential_api_key_id
            );
            credential = replacement;
        }
    }

    #[tokio::test]
    async fn account_disable_and_delete_remove_assistant_ciphertext() {
        let (state, owner, row) = fixture("nyxa_owner_cleanup").await;
        let db = &state.db;
        let other = new_chat(&state, &owner).await;
        crate::services::admin_user_service::set_user_active(db, "admin", &owner, false)
            .await
            .unwrap();
        assert!(!exists(db, &owner).await.unwrap());
        for row in [&row, &other] {
            assert_eq!(
                engine::get(db, &owner, &row.id)
                    .await
                    .unwrap()
                    .context_reset_reason
                    .as_deref(),
                Some("credential_replaced")
            );
        }
        crate::services::admin_user_service::set_user_active(db, "admin", &owner, true)
            .await
            .unwrap();
        load_or_provision(db, &state.encryption_keys, &owner, &row.id)
            .await
            .unwrap();
        crate::test_utils::accept_and_process_admin_erasure(db, "admin", &owner)
            .await
            .unwrap();
        for collection in [
            CREDENTIALS,
            CONVERSATIONS,
            crate::models::assistant_message::COLLECTION_NAME,
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        ] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {"user_id": &owner})
                    .await
                    .unwrap(),
                0
            );
        }
    }

    #[tokio::test]
    async fn rotation_adopts_the_successor_without_minting_or_disclosing_another_key() {
        let (state, owner, row) = fixture("nyxa_rotation").await;
        let db = &state.db;
        let old = load_for_conversation(db, &state.encryption_keys, &owner, &row.id)
            .await
            .unwrap()
            .unwrap();
        db.collection::<AssistantConversation>(CONVERSATIONS)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"nyxagent_session_id": "old"}},
            )
            .await
            .unwrap();
        // Prior acknowledgements must not survive a credential generation change.
        mutations::update_one(
            db,
            doc! {"_id": &old.api_key_id},
            doc! {"$set": {"scopes": "proxy assistant:account", "allow_all_nodes": true}},
            None,
        )
        .await
        .unwrap();
        let successor =
            key_service::rotate_api_key(db, &state.encryption_keys, &owner, &old.api_key_id)
                .await
                .unwrap();
        assert!(successor.full_key.is_empty());
        let current = load_or_provision(db, &state.encryption_keys, &owner, &row.id)
            .await
            .unwrap();
        assert_eq!(current.api_key_id, successor.id);
        assert_ne!(current.raw_key.as_str(), old.raw_key.as_str());
        let key = key_service::get_api_key(db, &owner, &successor.id)
            .await
            .unwrap();
        assert_eq!(key.key_hash, hash_token(&current.raw_key));
        assert_eq!(key.scopes, format!("proxy {ASSISTANT_ACCOUNT_SCOPE}"));
        assert!(key.allow_all_nodes && key.allow_all_services);
        assert!(key.allowed_service_ids.is_empty());
        assert_eq!(
            db.collection::<ApiKey>(KEYS)
                .count_documents(doc! {"user_id": &owner})
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            db.collection::<ApiKey>(KEYS)
                .count_documents(doc! {"user_id": &owner, "is_active": true})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<AssistantAgentCredential>(CREDENTIALS)
                .count_documents(doc! {"user_id": &owner})
                .await
                .unwrap(),
            1
        );
        let saved = engine::get(db, &owner, &row.id).await.unwrap();
        assert!(saved.nyxagent_session_id.is_none());
        assert_eq!(
            saved.context_reset_reason.as_deref(),
            Some("credential_replaced")
        );
    }

    #[tokio::test]
    async fn live_provision_rotate_and_read_support_staging_then_strict_contextual_storage() {
        for staged in [true, false] {
            let (mut state, owner, _) = fixture("assistant_credential_cutover").await;
            let mut keys = crate::test_utils::test_encryption_keys();
            keys.set_contextual_record_writes(!staged);
            state.encryption_keys = std::sync::Arc::new(keys);
            let thread = new_chat(&state, &owner).await;
            let before =
                load_for_conversation(&state.db, &state.encryption_keys, &owner, &thread.id)
                    .await
                    .unwrap()
                    .unwrap();
            let raw = state
                .db
                .collection::<bson::Document>(CREDENTIALS)
                .find_one(doc! {"conversation_id":&thread.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                super::super::record_field_service::stored_versions(&raw)
                    .unwrap()
                    .contains_key("key_ciphertext"),
                !staged
            );
            // Migration is enabled only after the reader-compatible writer
            // cutover. Keep the initial staged write/read assertion above.
            let mut migration_keys = crate::test_utils::test_encryption_keys();
            migration_keys.set_contextual_record_writes(true);
            super::super::assistant_record_service::migrate_credential_snapshot(
                &state.db,
                &migration_keys,
                &raw,
            )
            .await
            .unwrap();
            let mut strict = crate::test_utils::test_encryption_keys();
            strict.set_contextual_record_writes(true);
            strict.set_allow_legacy_record_reads(false);
            state.encryption_keys = std::sync::Arc::new(strict);
            let current =
                load_for_conversation(&state.db, &state.encryption_keys, &owner, &thread.id)
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(before.raw_key.as_str(), current.raw_key.as_str());
            let successor = key_service::rotate_api_key(
                &state.db,
                &state.encryption_keys,
                &owner,
                &before.api_key_id,
            )
            .await
            .unwrap();
            assert!(successor.full_key.is_empty());
            let rotated = load_or_provision(&state.db, &state.encryption_keys, &owner, &thread.id)
                .await
                .unwrap();
            assert_eq!(rotated.api_key_id, successor.id);
            assert_ne!(rotated.raw_key.as_str(), before.raw_key.as_str());
            let raw = state
                .db
                .collection::<bson::Document>(CREDENTIALS)
                .find_one(doc! {"conversation_id":&thread.id})
                .await
                .unwrap()
                .unwrap();
            super::super::assistant_record_service::verify_current_credential(
                &state.db,
                &state.encryption_keys,
                &raw,
            )
            .await
            .unwrap();
            state.db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn models_select_latest_valid_credential_and_conversation_delete_revokes_only_its_key() {
        let (state, owner, first) = fixture("nyxa_latest_delete").await;
        let db = &state.db;
        let second = new_chat(&state, &owner).await;
        db.collection::<AssistantAgentCredential>(CREDENTIALS).update_one(doc! {"conversation_id": &second.id},
            doc! {"$set": {"last_used_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(1))}}).await.unwrap();
        assert_eq!(
            load_existing(db, &state.encryption_keys, &owner)
                .await
                .unwrap()
                .unwrap()
                .api_key_id,
            second.credential_api_key_id
        );
        db.collection::<ApiKey>(KEYS)
            .update_one(
                doc! {"_id": &second.credential_api_key_id},
                doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert_eq!(
            load_existing(db, &state.encryption_keys, &owner)
                .await
                .unwrap()
                .unwrap()
                .api_key_id,
            first.credential_api_key_id
        );
        db.collection::<AssistantConversation>(CONVERSATIONS)
            .update_one(
                doc! {"_id": &first.id},
                doc! {"$set": {"active_turn": bson::Bson::Null}},
            )
            .await
            .unwrap();
        engine::delete(db, &owner, &first.id).await.unwrap();
        assert!(
            key_service::get_api_key(db, &owner, &first.credential_api_key_id)
                .await
                .is_err()
        );
        assert!(
            load_for_conversation(db, &state.encryption_keys, &owner, &first.id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(engine::get(db, &owner, &second.id).await.is_ok());
    }
}
