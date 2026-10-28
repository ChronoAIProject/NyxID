//! Human consent to use the actor's exact connection. Independent of Full access.
use chrono::{Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    assistant_acknowledgement_service::ChatAuthority,
    shared_agent_service::{self as sharing, RequestAccess},
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_sharing::{AgentConsent, CONSENTS_COLLECTION_NAME, ConsentDecision},
        user_service::UserService,
    },
};

#[derive(Clone)]
pub struct Permit {
    id: String,
    claim: Option<String>,
    revision: i64,
    behavior: String,
}

pub fn behavior(access: &RequestAccess) -> String {
    let a = access.agent();
    super::mcp_service::canonical_sha256(json!({"persona": a.persona, "description": a.description,
        "model": a.model, "skills": a.skills, "skills_revision": a.skills_revision, "grants": a.grants, "operations": a.operation_scopes,
        "sharing_revision": access.revision()}))
}

/// A refusal is a structured, actor-only consent card. No credential values,
/// operation arguments, owner approvals or account switching are implicit.
pub async fn authorize(
    db: &Database,
    chat: &ChatAuthority,
    access: &RequestAccess,
    service: &UserService,
    binding: &str,
    operation: &Value,
) -> AppResult<Result<Permit, Value>> {
    authorize_inner(db, chat, access, service, binding, operation, true).await
}

/// Observe/create the connection card before an ordinary action card consumes
/// anything. Once is claimed only after both human decisions permit execution.
pub async fn prompt(
    db: &Database,
    chat: &ChatAuthority,
    access: &RequestAccess,
    service: &UserService,
    binding: &str,
    operation: &Value,
) -> AppResult<Option<Value>> {
    Ok(
        authorize_inner(db, chat, access, service, binding, operation, false)
            .await?
            .err(),
    )
}

async fn authorize_inner(
    db: &Database,
    chat: &ChatAuthority,
    access: &RequestAccess,
    service: &UserService,
    binding: &str,
    operation: &Value,
    consume_once: bool,
) -> AppResult<Result<Permit, Value>> {
    if !chat.turn_live || chat.turn_stopped || chat.turn_id.is_none() {
        return Err(AppError::AssistantTurnRequired);
    }
    let matches = access.matches(&chat.user_id, access.owner(), &chat.agent_id);
    let actor_service = service.user_id == chat.user_id;
    let connection = access.allows_connection(service);
    let model = matches!(
        operation["kind"].as_str(),
        Some("raw_inference" | "legacy_inference")
    ) && access.allows_model_connection(service);
    if !matches || !actor_service || !connection && !model {
        return Err(sharing::missing());
    }
    let behavior = behavior(access);
    let mut consent_scope = operation.clone();
    let model_scope = operation["kind"] == "raw_inference";
    if model_scope && let Some(scope) = consent_scope.as_object_mut() {
        scope.remove("input_admission");
    }
    let operation_digest = super::mcp_service::canonical_sha256(consent_scope);
    let filter = doc! {"actor_user_id": &chat.user_id, "agent_id": &chat.agent_id,
    "conversation_id": &chat.conversation_id, "api_key_id": &chat.api_key_id,
    "connection_id": &service.id, "policy_revision": access.revision(),
    "behavior_digest": &behavior, "authority_digest": binding, "operation_digest": &operation_digest,
    "$or": [{"expires_at": null}, {"expires_at": {"$gt": bson::DateTime::now()}}],
    "$and": [{"$or": [{"decision": {"$ne": "skip"}}, {"turn_id": &chat.turn_id}]}],
    "decision": {"$in": ["pending", "once", "always", "skip"]}};
    let rows = db.collection::<AgentConsent>(CONSENTS_COLLECTION_NAME);
    if model_scope {
        let mut consumed = filter.clone();
        consumed.insert("decision", "consumed");
        consumed.insert("turn_id", chat.turn_id.as_deref());
        if let Some(row) = rows.find_one(consumed).await? {
            return Ok(Ok(Permit {
                id: row.id,
                claim: row.claim_id,
                revision: access.revision(),
                behavior,
            }));
        }
    }
    if let Some(row) = rows.find_one(filter).sort(doc! {"created_at": -1}).await? {
        match row.decision {
            ConsentDecision::Always => {
                return Ok(Ok(Permit {
                    id: row.id,
                    claim: None,
                    revision: access.revision(),
                    behavior,
                }));
            }
            ConsentDecision::Once => {
                if !consume_once {
                    return Ok(Ok(Permit {
                        id: row.id,
                        claim: None,
                        revision: access.revision(),
                        behavior,
                    }));
                }
                let claim = Uuid::new_v4().to_string();
                let result = rows.update_one(doc! {"_id": &row.id, "decision": "once"},
                    doc! {"$set": {"decision": "consumed", "turn_id": &chat.turn_id, "claim_id": &claim, "updated_at": bson::DateTime::now()}}).await?;
                if result.modified_count != 1 {
                    return Err(AppError::Conflict("Consent was already used".into()));
                }
                return Ok(Ok(Permit {
                    id: row.id,
                    claim: Some(claim),
                    revision: access.revision(),
                    behavior,
                }));
            }
            ConsentDecision::Skip => {
                return Ok(Err(
                    json!({"error": "shared_connection_skipped", "connection_id": service.id}),
                ));
            }
            _ => return Ok(Err(card(&row))),
        }
    }
    let now = Utc::now();
    let scope_key = super::mcp_service::canonical_sha256(json!({
        "actor": chat.user_id, "agent": chat.agent_id, "conversation": chat.conversation_id,
        "key": chat.api_key_id, "connection": service.id, "revision": access.revision(),
        "behavior": behavior, "authority": binding, "operation": operation_digest,
    }));
    let pending = doc! {"scope_key": &scope_key, "decision": "pending"};
    let mut expired = pending.clone();
    expired.insert(
        "expires_at",
        doc! {"$lte": bson::DateTime::from_chrono(now)},
    );
    rows.update_many(expired, doc! {"$set": {"decision": "revoked"}})
        .await?;
    let row = AgentConsent {
        id: Uuid::new_v4().to_string(),
        actor_user_id: chat.user_id.clone(),
        agent_id: chat.agent_id.clone(),
        owner_id: access.owner().into(),
        conversation_id: chat.conversation_id.clone(),
        turn_id: chat
            .turn_id
            .clone()
            .ok_or(AppError::AssistantTurnRequired)?,
        api_key_id: chat.api_key_id.clone(),
        connection_id: service.id.clone(),
        catalog_service_id: service
            .catalog_service_id
            .clone()
            .ok_or_else(sharing::missing)?,
        policy_revision: access.revision(),
        behavior_digest: behavior,
        authority_digest: binding.into(),
        operation_digest,
        scope_key,
        stateless_model: model_scope,
        operation_summary: if model_scope {
            "Stateless model calls for this turn".into()
        } else {
            format!(
                "{} {}",
                operation["method"].as_str().unwrap_or("Operation"),
                operation["path"].as_str().unwrap_or("")
            )
            .chars()
            .take(240)
            .collect()
        },
        decision: ConsentDecision::Pending,
        acknowledgement_id: None,
        expires_at: Some(now + Duration::minutes(25)),
        claim_id: None,
        created_at: now,
        updated_at: now,
    };
    if let Err(error) = rows.insert_one(&row).await {
        if super::service_pool_service::is_duplicate_key(&error) {
            let existing = rows.find_one(pending).await?.ok_or_else(|| {
                AppError::Conflict("Consent changed concurrently; retry the request".into())
            })?;
            return Ok(Err(card(&existing)));
        }
        return Err(error.into());
    }
    Ok(Err(card(&row)))
}

fn card(row: &AgentConsent) -> Value {
    json!({"error": "shared_connection_consent_required", "consent_id": row.id,
        "connection_id": row.connection_id, "message": "Choose Allow once, Always or Skip in your private chat to use your connection. No account has been switched."})
}

impl Permit {
    pub async fn revalidate(
        &self,
        db: &Database,
        access: &RequestAccess,
        actor: &str,
    ) -> AppResult<()> {
        if access.actor() != actor
            || access.revision() != self.revision
            || behavior(access) != self.behavior
        {
            return Err(sharing::missing());
        }
        let mut filter = doc! {"_id": &self.id, "actor_user_id": actor, "agent_id": access.agent_id(),
        "policy_revision": self.revision, "behavior_digest": &self.behavior,
        "$or": [{"expires_at": null}, {"expires_at": {"$gt": bson::DateTime::now()}}]};
        if let Some(claim) = &self.claim {
            filter.insert("decision", "consumed");
            filter.insert("claim_id", claim);
        } else {
            filter.insert("decision", "always");
        }
        if db
            .collection::<AgentConsent>(CONSENTS_COLLECTION_NAME)
            .find_one(filter)
            .await?
            .is_none()
        {
            return Err(AppError::Forbidden(
                "Connection consent was revoked or expired".into(),
            ));
        }
        Ok(())
    }
}

pub async fn decide(
    db: &Database,
    actor: &str,
    id: &str,
    decision: ConsentDecision,
) -> AppResult<()> {
    if !matches!(
        decision,
        ConsentDecision::Once
            | ConsentDecision::Always
            | ConsentDecision::Skip
            | ConsentDecision::Revoked
    ) {
        return Err(AppError::ValidationError(
            "Choose once, always, skip or revoked".into(),
        ));
    }
    let rows = db.collection::<AgentConsent>(CONSENTS_COLLECTION_NAME);
    let row = rows
        .find_one(doc! {"_id": id, "actor_user_id": actor})
        .await?
        .ok_or_else(sharing::missing)?;
    if decision == ConsentDecision::Revoked {
        rows.update_one(
            doc! {"_id": id, "actor_user_id": actor},
            doc! {"$set": {"decision": "revoked", "updated_at": bson::DateTime::now()}},
        )
        .await?;
        super::audit_service::log_actor_event(
            db.clone(),
            &super::audit_service::AuditActor {
                user_id: actor.into(),
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
            },
            "assistant_connection_consent_revoked",
            Some(
                json!({"agent_id": row.agent_id, "agent_owner_id": row.owner_id,
                "execution_mode": "shared", "consent_id": row.id}),
            ),
        )
        .await?;
        return Ok(());
    }
    let access = sharing::load(db, actor, Some(std::slice::from_ref(&row.agent_id)))
        .await?
        .agents
        .remove(&row.agent_id)
        .ok_or_else(sharing::missing)?;
    if row.policy_revision != access.revision()
        || row.behavior_digest != behavior(&access)
        || row.authority_digest
            != super::assistant_service_execution::connection_binding(
                db,
                &row.connection_id,
                &row.api_key_id,
            )
            .await?
    {
        return Err(AppError::Conflict(
            "Agent or connection changed; request fresh consent".into(),
        ));
    }
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    sharing::fence(db, &access, &mut session).await?;
    let expires = if decision == ConsentDecision::Always {
        bson::Bson::Null
    } else {
        bson::Bson::DateTime(bson::DateTime::from_chrono(
            Utc::now() + Duration::minutes(25),
        ))
    };
    let result = rows.update_one(doc! {"_id": id, "actor_user_id": actor, "decision": "pending", "expires_at": {"$gt": bson::DateTime::now()}},
        doc! {"$set": {"decision": bson::to_bson(&decision).map_err(|_| sharing::missing())?, "expires_at": expires, "updated_at": bson::DateTime::now()}})
        .session(&mut session).await?;
    if result.modified_count != 1 {
        return Err(AppError::Conflict(
            "Consent expired or already decided".into(),
        ));
    }
    session.commit_transaction().await?;
    sharing::audit(db, actor, access.agent(), "connection_consent").await;
    Ok(())
}
