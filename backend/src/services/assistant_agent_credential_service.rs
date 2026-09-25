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
        assistant_agent_credential::{AssistantAgentCredential, COLLECTION_NAME as CREDENTIALS},
        assistant_conversation::{
            AccessMode, AssistantConversation, COLLECTION_NAME as CONVERSATIONS,
        },
    },
    services::{api_key_mutation_service as mutations, key_service},
};

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
    let exists = collection
        .find_one(filter.clone())
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await
        .map_err(|_| unavailable())?
        .is_some();
    if exists {
        let ciphertext = keys
            .encrypt(successor_key.as_bytes())
            .await
            .map_err(|_| unavailable())?;
        collection
            .update_one(
                filter,
                doc! {"$set": {
                    "api_key_id": successor_id,
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

/// Called inside the turn transaction: the conversation, key and ciphertext
/// become durable together. Touching the key fences concurrent revocation.
pub async fn load_or_provision_in_session(
    db: &Database,
    keys: &EncryptionKeys,
    user_id: &str,
    conversation_id: &str,
    access_mode: AccessMode,
    session: &mut ClientSession,
) -> AppResult<AssistantCredential> {
    let old = db
        .collection::<AssistantAgentCredential>(CREDENTIALS)
        .find_one(doc! {"user_id": user_id, "conversation_id": conversation_id})
        .session(&mut *session)
        .await
        .map_err(|_| unavailable())?;
    let mut revoked_children = Vec::new();
    if let Some(old) = old {
        let raw = decrypt(keys, &old).await?;
        let valid = db
            .collection::<ApiKey>(KEYS)
            .find_one(valid_key_filter(user_id, &old.api_key_id, &raw))
            .session(&mut *session)
            .await?
            .is_some();
        if valid {
            mutations::update_one(
                db,
                doc! {"_id": &old.api_key_id, "user_id": user_id},
                // Service consent covers its node route, including for existing keys.
                doc! {"$set": {"last_used_at": bson::DateTime::now(), "allow_all_nodes": true}},
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
        Some(user_id),
        None,
        &name,
        ASSISTANT_SCOPES,
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(access_mode == AccessMode::Full),
        Some(true),
        Some(true),
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
    if access_mode == AccessMode::Full {
        super::assistant_access_mode_service::apply_key_mode(
            db,
            user_id,
            &created.id,
            access_mode,
            session,
        )
        .await?;
    }
    let raw = Zeroizing::new(created.full_key);
    let now = Utc::now();
    let row = AssistantAgentCredential {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.into(),
        conversation_id: conversation_id.into(),
        api_key_id: created.id.clone(),
        key_ciphertext: keys
            .encrypt(raw.as_bytes())
            .await
            .map_err(|_| unavailable())?,
        created_at: now,
        last_used_at: now,
    };
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

/// Recovery only; first-use provisioning belongs to begin_turn's transaction.
pub async fn load_or_provision(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    user_id: &str,
    conversation_id: &str,
) -> AppResult<AssistantCredential> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let keys = keys.clone();
    let user_id = user_id.to_owned();
    let conversation_id = conversation_id.to_owned();
    let audit_db = db.clone();
    let audit_user = user_id.clone();
    let audit_conversation = conversation_id.clone();
    let credential = session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = async {
                // Write the conversation as well to fence concurrent deletion.
                let result = db
                    .collection::<AssistantConversation>(CONVERSATIONS)
                    .update_one(
                        doc! {"_id": &conversation_id, "user_id": &user_id},
                        doc! {"$inc": {"credential_fence": 1}},
                    )
                    .session(&mut *session)
                    .await?;
                if result.matched_count != 1 {
                    return Err(AppError::NotFound("Conversation not found".into()));
                }
                let conversation = db
                    .collection::<AssistantConversation>(CONVERSATIONS)
                    .find_one(doc! {"_id": &conversation_id, "user_id": &user_id})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Conversation not found".into()))?;
                load_or_provision_in_session(
                    &db,
                    &keys,
                    &user_id,
                    &conversation_id,
                    conversation.access_mode,
                    session,
                )
                .await
            }
            .await;
            mutations::transaction_result(operation)
        })
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
    match key_service::delete_api_key(db, user_id, old_key_id).await {
        Ok(()) => {}
        Err(AppError::NotFound(_)) => invalidate_for_key(db, user_id, old_key_id, None).await?,
        Err(error) => return Err(error),
    }
    load_or_provision(db, keys, user_id, conversation_id).await
}

fn valid_key_filter(user_id: &str, key_id: &str, raw: &str) -> bson::Document {
    doc! {
        "_id": key_id, "user_id": user_id, "key_hash": hash_token(raw), "is_active": true,
        "$or": [{"expires_at": bson::Bson::Null}, {"expires_at": {"$gt": bson::DateTime::now()}}],
    }
}

async fn decrypt(
    keys: &EncryptionKeys,
    row: &AssistantAgentCredential,
) -> AppResult<Zeroizing<String>> {
    let bytes = Zeroizing::new(
        keys.decrypt(&row.key_ciphertext)
            .await
            .map_err(|_| unavailable())?,
    );
    Ok(Zeroizing::new(
        String::from_utf8(bytes.to_vec()).map_err(|_| unavailable())?,
    ))
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
            state
                .encryption_keys
                .decrypt(&stored.key_ciphertext)
                .await
                .unwrap(),
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
        assert!(
            !key.allow_all_services && key.allow_all_nodes && key.allow_auto_connected_services
        );
        assert!(key.allowed_service_ids.is_empty() && key.allowed_node_ids.is_empty());
        assert!(key.expires_at.is_none());
        assert_eq!(key.scopes, "proxy");
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
    async fn existing_conversation_key_restores_node_access_without_widening_services() {
        let (state, owner, row) = fixture("nyxa_restore_nodes").await;
        let db = &state.db;
        db.collection::<ApiKey>(KEYS)
            .update_one(
                doc! {"_id": &row.credential_api_key_id},
                doc! {"$set": {"allow_all_nodes": false}},
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
        assert!(key.allow_all_nodes && !key.allow_all_services);
        assert!(key.allowed_service_ids.is_empty());
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
            assert!(key.allow_all_nodes && !key.allow_all_services);
            assert_eq!(key.scopes, "proxy");
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
        crate::services::admin_user_service::delete_user_cascade(db, "admin", &owner)
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
        assert_eq!(key.scopes, "proxy");
        assert!(key.allow_all_nodes && !key.allow_all_services);
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
