//! Consent to the declared service authority of one machine command/job.
//! Dynamic HTTP payloads are authorized by that command, never represented as
//! individually approved bytes. Only the server can construct this grant.
use super::{assistant_acknowledgement_service as acks, assistant_service_execution as execution};
use crate::{
    AppState,
    errors::{AppError, AppResult},
};
use bson::{Document, doc};
use chrono::{Duration, Utc};
use serde_json::{Value, json};

pub struct Plan {
    pub id: String,
    pub org_labels: Vec<String>,
    pub unresolved_org_connections: Vec<String>,
    pub legacy_org_policy: bool,
    instruction_version: i32,
    bindings: Vec<Document>,
    command: String,
}
impl Plan {
    pub async fn revalidate(
        &self,
        db: &mongodb::Database,
        chat: &acks::ChatAuthority,
    ) -> AppResult<()> {
        execution::live_authority(db, chat, "machine.job_launch").await?;
        for row in &self.bindings {
            let id = row
                .get_str("connection_id")
                .map_err(|_| AppError::MachineAuthorityStale)?;
            if row.get_str("binding")
                != Ok(execution::connection_binding(db, id, &chat.api_key_id)
                    .await?
                    .as_str())
            {
                return Err(AppError::MachineAuthorityStale);
            }
        }
        Ok(())
    }
    pub fn confirmation(&self) -> Value {
        json!({"job_id": self.id, "command_admission": self.command, "connections": self.bindings})
    }
    pub async fn claim(&self, db: &mongodb::Database) -> AppResult<()> {
        let claim = db
            .collection::<Document>(execution::COLLECTION)
            .update_one(
                doc! {"_id":&self.id,"kind":"job_plan","state":"pending"},
                doc! {"$set":{"state":"claimed"}},
            )
            .await?;
        if claim.modified_count != 1 {
            return Err(AppError::MachineAuthorityStale);
        }
        Ok(())
    }
    pub async fn publish(
        &self,
        db: &mongodb::Database,
        job: &crate::models::machine_job::MachineJob,
    ) -> AppResult<()> {
        if self.id != job.id {
            return Err(AppError::MachineAuthorityStale);
        }
        let updated = db.collection::<Document>("machine_jobs").update_one(doc! {"_id": &job.id,
            "api_key_id": &job.api_key_id, "state":"running", "expires_at":{"$gt":bson::DateTime::now()}},
            doc! {"$set":{"service_use": {"job_id": &self.id, "command_admission": &self.command,
                "bindings": &self.bindings, "api_key_id": &job.api_key_id, "instruction_receipt_version": self.instruction_version}}}).await?;
        if updated.matched_count != 1 {
            return Err(AppError::MachineAuthorityStale);
        }
        Ok(())
    }
    pub async fn bind_card(
        &self,
        db: &mongodb::Database,
        chat: &acks::ChatAuthority,
        mut refusal: Value,
    ) -> AppResult<Value> {
        let mut connections = Vec::new();
        for id in &self.unresolved_org_connections {
            let service = super::user_service_service::find_user_service_by_id(db, id)
                .await?
                .ok_or(AppError::MachineAuthorityStale)?;
            let facts =
                super::assistant_connection_service::facts(db, &chat.user_id, &service).await?;
            let offer =
                super::assistant_connection_service::connect_own(db, &chat.user_id, &service)
                    .await?;
            connections.push(json!({"connection_id":service.id,"connection_slug":service.slug,
                "organization_name":facts["organization_name"],"catalog_service_id":facts["catalog_service_id"],
                "service_slug":facts["catalog_service_slug"],"connection":facts,"connect_own":offer}));
        }
        if !connections.is_empty() {
            // One combined card authorizes the command. Every unresolved identity
            // retains its own setup guidance, with one supported hosted action.
            let primary = connections
                .iter()
                .find(|row| row["connect_own"]["supported"] == true)
                .unwrap_or(&connections[0]);
            for field in [
                "connection_id",
                "connection_slug",
                "organization_name",
                "catalog_service_id",
                "service_slug",
                "connection",
                "connect_own",
            ] {
                refusal[field] = primary[field].clone();
            }
            refusal["organization_connections"] = json!(connections);
        }
        if let Some(id) = refusal["acknowledgement_id"].as_str() {
            db.collection::<Document>(execution::COLLECTION)
                .update_one(
                    doc! {"_id": &self.id, "kind":"job_plan"},
                    doc! {"$set":{"acknowledgement_id":id}},
                )
                .await?;
            if let Some(offer) = refusal.get("connect_own") {
                db.collection::<Document>("assistant_acknowledgements")
                    .update_one(
                        doc! {"_id":id,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
                            "api_key_id":&chat.api_key_id,"confirmation.reason":"org_connection"},
                        doc! {"$set":{"connect_own":bson::to_bson(offer).map_err(|_| AppError::Internal("Connection offer failed".into()))?}},
                    )
                    .await?;
            }
        }
        Ok(refusal)
    }
}

pub async fn prepare(
    state: &AppState,
    chat: &acks::ChatAuthority,
    node: &str,
    arguments: &Value,
    declared: &[super::machine_gateway_service::AvailableService],
) -> AppResult<Result<Plan, Value>> {
    chat.ensure_live_turn(&state.db, None, None, "machine.service_declaration")
        .await?;
    let mut semantic = arguments.clone();
    semantic
        .as_object_mut()
        .ok_or_else(|| AppError::ValidationError("Expected command arguments".into()))?
        .remove("acknowledgement_id");
    let command = super::mcp_service::canonical_sha256(json!({"node":node,"arguments":semantic,
        "declared":super::machine_gateway_service::declared(declared)}));
    let mut filter = doc! {"kind":"job_plan", "user_id":&chat.user_id, "conversation_id":&chat.conversation_id,
    "api_key_id":&chat.api_key_id, "command_admission":&command, "state":"pending", "expires_at":{"$gt":bson::DateTime::now()}};
    let ack = arguments["acknowledgement_id"].as_str();
    let continued = if let Some(id) = ack {
        state.db.collection::<Document>("assistant_acknowledgements").find_one(doc! {"_id":id,
            "user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"api_key_id":&chat.api_key_id,
            "tool_name":"nyx__machine_exec","status":"allowed","decided_by":"user",
            "expires_at":{"$gt":bson::DateTime::now()}}).await?.is_some()
    } else {
        false
    };
    if continued {
        filter.insert("acknowledgement_id", ack);
    } else {
        filter.insert("turn_id", &chat.turn_id);
    }
    if !continued {
        filter.insert("acknowledgement_id", doc! {"$type":"string"});
    }
    let mut existing = state
        .db
        .collection::<Document>(execution::COLLECTION)
        .find_one(filter)
        .await?;
    if !continued && let Some(plan) = existing.as_ref() {
        let pending = state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .find_one(doc! {
            "_id":plan.get_str("acknowledgement_id").unwrap_or_default(),"status":"pending",
            "expires_at":{"$gt":bson::DateTime::now()}})
            .await?
            .is_some();
        if !pending {
            existing = None;
        }
    }
    let id = existing
        .as_ref()
        .and_then(|r| r.get_str("_id").ok())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut connections = Vec::new();
    for declaration in declared {
        if declaration.user_service {
            connections.push((declaration.id.clone(), None));
        } else if let Some(pool) = state
            .db
            .collection::<crate::models::service_pool::ServicePool>("service_pools")
            .find_one(doc! {"_id":&declaration.id,"is_active":true})
            .await?
        {
            for member in pool.members.into_iter().filter(|m| m.enabled) {
                connections.push((member.user_service_id, Some(pool.id.clone())));
            }
        } // Legacy catalog routes have no organization credential identity.
    }
    connections.sort();
    connections.dedup();
    let mut bindings = Vec::new();
    let mut labels = Vec::new();
    let mut unresolved_org_connections = Vec::new();
    let mut legacy_org_policy = false;
    let instruction_version = if execution::instruction::active(&state.db, &chat.user_id).await? {
        1
    } else {
        0
    };
    let job_id = id.clone();
    for (id, pool) in connections {
        let row = super::user_service_service::find_user_service_by_id(&state.db, &id)
            .await?
            .ok_or_else(|| AppError::NotFound("Declared connection unavailable".into()))?;
        let instruction_covered = execution::instruction::covered(&state.db, chat, &row).await?;
        let binding = execution::connection_binding(&state.db, &id, &chat.api_key_id).await?;
        let snapshot = doc! {"connection_id":&id,"owner":&row.user_id,"binding":&binding};
        if continued {
            let saved = existing
                .as_ref()
                .and_then(|r| r.get_array("bindings").ok())
                .is_some_and(|items| items.iter().any(|v| v.as_document() == Some(&snapshot)));
            if !saved {
                return Err(AppError::MachineAuthorityStale);
            }
        } else {
            // This preflight enforces selection and preference, while the
            // existing machine-exec card combines every consent reason once.
            let op = json!({"kind":"machine_job","job_id":&job_id,"command_admission":&command,"pool_id":pool});
            let result = execution::check(
                &state.db,
                &state.node_ws_manager,
                chat,
                &id,
                &row.user_id,
                &op,
                acks::ActionFacts::of(acks::OwnerActionClass::Read),
                None,
                "Declared command service",
                pool.is_some(),
                None,
                true,
                None,
            )
            .await?;
            if let Some(refusal) = result.refusal {
                return Ok(Err(refusal));
            }
        }
        if row.user_id != chat.user_id {
            let owner = state
                .db
                .collection::<crate::models::user::User>("users")
                .find_one(doc! {"_id":&row.user_id,"is_active":true})
                .await?
                .ok_or(AppError::MachineAuthorityStale)?;
            if owner.user_type == crate::models::user::UserType::Org {
                legacy_org_policy |= !instruction_covered;
                labels.push(format!(
                    "organization connection {} ({}) in {}",
                    super::assistant_nyxagent::identifier(&row.slug),
                    id,
                    super::assistant_nyxagent::identifier(
                        owner
                            .display_name
                            .as_deref()
                            .unwrap_or(owner.slug.as_deref().unwrap_or(&owner.id))
                    )
                ));
            }
        }
        if instruction_covered
            && !execution::instruction::owner_instruction(&state.db, chat, &row).await?
        {
            unresolved_org_connections.push(id.clone());
        }
        bindings.push(snapshot);
    }
    if existing.is_none() {
        state.db.collection::<Document>(execution::COLLECTION).insert_one(doc! {"_id":&id,"kind":"job_plan",
            "user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"api_key_id":&chat.api_key_id,
            "turn_id":&chat.turn_id,"command_admission":&command,"bindings":&bindings,"state":"pending",
            "expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(25))}).await?;
    }
    Ok(Ok(Plan {
        id,
        org_labels: labels,
        unresolved_org_connections,
        legacy_org_policy,
        instruction_version,
        bindings,
        command,
    }))
}

/// Reuse the job's existing detached authority, then check exact service and
/// credential generation. No synthetic live turn and no owner-consent bypass.
pub fn validate<'a>(
    db: &'a mongodb::Database,
    job_id: &'a str,
    connection: &'a str,
    owner: &'a str,
) -> futures::future::BoxFuture<'a, AppResult<crate::mw::auth::AuthUser>> {
    Box::pin(async move {
        let job = db
            .collection::<crate::models::machine_job::MachineJob>("machine_jobs")
            .find_one(doc! {"_id":job_id,
            "state":"running","expires_at":{"$gt":bson::DateTime::now()}})
            .await?
            .ok_or(AppError::MachineAuthorityStale)?;
        let auth = Box::pin(super::machine_gateway_service::job_auth(db, &job)).await?;
        validate_lease(db, &job).await?;
        super::org_agent_service::authorize_execution(db, &auth, Some(connection)).await?;
        if !auth.allow_all_services && !auth.allowed_service_ids.iter().any(|id| id == connection) {
            return Err(AppError::MachinePermissionRevoked);
        }
        let access = super::org_service::resolve_owner_access(db, &job.user_id, owner).await?;
        let can_proxy = match &access {
            super::org_service::OwnerAccess::Direct
            | super::org_service::OwnerAccess::AsOrgAdmin { .. } => true,
            super::org_service::OwnerAccess::AsOrgMember { role, .. } => role.can_proxy(),
            super::org_service::OwnerAccess::Forbidden => false,
        };
        if !can_proxy || !access.allows_resource(connection) {
            return Err(AppError::MachinePermissionRevoked);
        }
        let row = db
            .collection::<Document>("machine_jobs")
            .find_one(doc! {"_id":job_id})
            .await?
            .ok_or(AppError::MachineAuthorityStale)?;
        if let Ok(grant) = row.get_document("service_use") {
            if let Some(chat) = auth.assistant_chat.as_deref()
                && let Some(service) =
                    super::user_service_service::find_user_service_by_id(db, connection).await?
                && execution::instruction::covered(db, chat, &service).await?
                && grant.get_i32("instruction_receipt_version") != Ok(1)
            {
                return Err(AppError::Forbidden("This organization job has no owner instruction receipt. Ask the owner to review a new bounded command card.".into()));
            }
            let binding = execution::connection_binding(db, connection, &job.api_key_id).await?;
            if grant.get_str("job_id") != Ok(job_id)
                || grant.get_str("api_key_id") != Ok(job.api_key_id.as_str())
                || !grant.get_array("bindings").is_ok_and(|rows| {
                    rows.iter().any(|row| {
                        row.as_document().is_some_and(|r| {
                            r.get_str("connection_id") == Ok(connection)
                                && r.get_str("owner") == Ok(owner)
                                && r.get_str("binding") == Ok(binding.as_str())
                        })
                    })
                })
            {
                return Err(AppError::MachineAuthorityStale);
            }
        } else if owner != job.user_id {
            return Err(AppError::MachinePermissionRevoked);
        }
        super::assistant_invocation_service::require_machine_job_active(db, &job, &auth).await?;
        Ok(auth)
    })
}

pub fn validate_catalog<'a>(
    db: &'a mongodb::Database,
    job_id: &'a str,
    catalog: &'a str,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let job = db
            .collection::<crate::models::machine_job::MachineJob>("machine_jobs")
            .find_one(
                doc! {"_id":job_id,"state":"running","expires_at":{"$gt":bson::DateTime::now()}},
            )
            .await?
            .ok_or(AppError::MachineAuthorityStale)?;
        if !job.services.iter().any(|service| service.id == catalog) {
            return Err(AppError::MachinePermissionRevoked);
        }
        let auth = Box::pin(super::machine_gateway_service::job_auth(db, &job)).await?;
        validate_lease(db, &job).await?;
        super::org_agent_service::authorize_execution(db, &auth, Some(catalog)).await?;
        if !auth.allow_all_services && !auth.allowed_service_ids.iter().any(|id| id == catalog) {
            return Err(AppError::MachinePermissionRevoked);
        }
        super::assistant_invocation_service::require_machine_job_active(db, &job, &auth).await?;
        Ok(())
    })
}

async fn validate_lease(
    db: &mongodb::Database,
    job: &crate::models::machine_job::MachineJob,
) -> AppResult<()> {
    let row = db
        .collection::<Document>("machine_jobs")
        .find_one(doc! {"_id":&job.id})
        .await?
        .ok_or(AppError::MachineAuthorityStale)?;
    if let Ok(authority) = row.get_document("machine_authority") {
        let lease = authority
            .get_str("lease_id")
            .map_err(|_| AppError::MachineAuthorityStale)?;
        if db.collection::<Document>(crate::models::machine_access::LEASES).find_one(doc! {
            "_id":lease,"job_id":&job.id,"api_key_id":&job.api_key_id,"node_id":&job.node_id,
            "authority.revision":authority.get_i64("revision").map_err(|_|AppError::MachineAuthorityStale)?,
            "authority.expires_at_ms":{"$gt":Utc::now().timestamp_millis()},
        }).await?.is_none() { return Err(AppError::MachineAuthorityStale); }
    }
    Ok(())
}
