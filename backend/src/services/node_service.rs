use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::audit_outbox::{AuditOwner, AuditPrincipal, AuditStage};
use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetadata, NodeStatus};
use crate::models::node_pending_credential::{
    COLLECTION_NAME as NODE_PENDING_CREDENTIALS, NodePendingCredential,
};
use crate::models::node_registration_token::{
    COLLECTION_NAME as NODE_REG_TOKENS, NodeRegistrationToken,
};
use crate::models::node_service_binding::{
    COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
};
use crate::models::org_membership::OrgRole;
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::redaction::RedactedLen;
use crate::services::org_service::{self, OwnerAccess};
use crate::services::{
    audit_outbox_service::{self, AuditEvent},
    audit_runtime::{AuditRuntime, intent::EffectReservation, policy::AuditTier},
};

/// A verified actor's logical mutation. Keep this context across transaction
/// retries; it shares the serving runtime rather than creating another one.
#[derive(Clone)]
pub struct NodeMutationAudit {
    runtime: std::sync::Arc<AuditRuntime>,
    actor: String,
    principal: AuditPrincipal,
    operation_id: uuid::Uuid,
    event_id: uuid::Uuid,
    destination_event_id: uuid::Uuid,
    intent_id: uuid::Uuid,
    terminal_id: uuid::Uuid,
    disconnect: bool,
    disconnect_owner: Option<super::node_owner_service::InvalidatedNodeOwner>,
}

impl NodeMutationAudit {
    pub fn new(runtime: &std::sync::Arc<AuditRuntime>, actor: &str) -> Self {
        Self {
            runtime: runtime.clone(),
            actor: actor.to_owned(),
            principal: AuditPrincipal::Person {
                id: actor.to_owned(),
            },
            operation_id: uuid::Uuid::new_v4(),
            event_id: uuid::Uuid::new_v4(),
            destination_event_id: uuid::Uuid::new_v4(),
            intent_id: uuid::Uuid::new_v4(),
            terminal_id: uuid::Uuid::new_v4(),
            disconnect: false,
            disconnect_owner: None,
        }
    }

    /// Only call with identity from the authenticated request, never a stored
    /// resource owner. The actor continues to drive the existing owner ACL.
    pub fn with_verified_principal(mut self, principal: AuditPrincipal) -> Self {
        self.principal = principal;
        self
    }

    async fn require_uncommitted(
        &self,
        db: &mongodb::Database,
        session: &mut mongodb::ClientSession,
    ) -> AppResult<()> {
        if db
            .collection::<bson::Document>(crate::models::audit_outbox::PENDING_COLLECTION_NAME)
            .find_one(doc! {"_id": self.event_id.to_string()})
            .session(session)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict("node_mutation_already_committed".into()));
        }
        Ok(())
    }

    pub async fn with_disconnect(
        mut self,
        db: &mongodb::Database,
        node_id: &str,
    ) -> AppResult<Self> {
        let node = load_active_node(db, node_id).await?;
        self.disconnect_owner =
            node.connection_owner
                .map(|owner| super::node_owner_service::InvalidatedNodeOwner {
                    fence: super::node_owner_service::NodeOwnerFence::from_owner(node_id, &owner),
                    internal_base_url: owner.internal_base_url,
                });
        self.disconnect = true;
        Ok(self)
    }

    async fn record(
        &self,
        db: &mongodb::Database,
        session: &mut mongodb::ClientSession,
        owner: AuditOwner,
        action: &'static str,
        target: &str,
        tier: AuditTier,
    ) -> AppResult<()> {
        let event = AuditEvent {
            operation_id: self.operation_id.to_string(),
            stage: AuditStage::Committed,
            principal: self.principal.clone(),
            owner,
            action,
            target_id: Some(target.to_owned()),
        };
        audit_outbox_service::enqueue_admitted_in_session(
            db,
            session,
            &self.runtime,
            tier,
            &self.event_id,
            &event,
        )
        .await?;
        if self.disconnect {
            if self
                .disconnect_owner
                .as_ref()
                .is_some_and(|owner| owner.fence.node_id != target)
            {
                return Err(AppError::AuditIntegrityConflict);
            }
            let mut filter = self.disconnect_owner.as_ref().map_or_else(
                || doc! {"_id": target, "connection_owner": bson::Bson::Null},
                |owner| owner.fence.filter(),
            );
            filter.insert("_id", target);
            let invalidated = db
                .collection::<Node>(NODES)
                .update_one(
                    filter,
                    doc! {
                        "$unset": {"connection_owner": ""}, "$set": {"status": "offline"},
                    },
                )
                .session(&mut *session)
                .await?;
            if invalidated.matched_count != 1 {
                return Err(AppError::Conflict("node_connection_changed".into()));
            }
        }
        if self.disconnect_owner.is_some() {
            self.runtime
                .begin_effect_in_session(
                    db,
                    session,
                    tier,
                    &EffectReservation {
                        event_id: self.intent_id,
                        terminal_event_id: self.terminal_id,
                        event: AuditEvent {
                            stage: AuditStage::Intent,
                            action: "node_disconnect",
                            ..event
                        },
                    },
                )
                .await?;
        }
        Ok(())
    }

    /// Local revocation has already committed. Witness refusal leaves its
    /// durable disconnect intent pending; it cannot undo that revocation.
    pub async fn disconnect_after_commit(
        &self,
        db: &mongodb::Database,
        dispatch: &super::node_dispatch::NodeDispatch,
        node_id: &str,
        code: u16,
        reason: &'static str,
    ) -> AppResult<bool> {
        if !self.disconnect {
            return Err(AppError::AuditIntegrityConflict);
        }
        let Some(owner) = &self.disconnect_owner else {
            return Ok(false);
        };
        if owner.fence.node_id != node_id {
            return Err(AppError::AuditIntegrityConflict);
        }
        let operation = self.operation_id.to_string();
        let permit = match self.runtime.authorize_effect(db, &operation).await {
            Ok(permit) => permit,
            Err(AppError::AuditAdmissionClosed { .. } | AppError::AuditWitnessDiverged) => {
                tracing::warn!(operation_id = %operation, "node_disconnect_deferred");
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let result = permit
            .run(|| dispatch.disconnect_invalidated(owner.clone(), code, reason))
            .await?;
        let stage = if result.is_ok() {
            AuditStage::Succeeded
        } else {
            AuditStage::Uncertain
        };
        let context = (db, &self.runtime, &operation, stage);
        super::audit_runtime::transaction::run(
            db,
            &context,
            |session, (db, runtime, operation, stage)| {
                Box::pin(runtime.finish_effect_in_session(db, session, operation, stage.clone()))
            },
        )
        .await?;
        result
    }

    pub async fn create_registration_token(
        &self,
        db: &mongodb::Database,
        owner: &str,
        name: &str,
        max_nodes: u32,
        ttl_secs: i64,
    ) -> AppResult<(String, String, DateTime<Utc>)> {
        self.create_registration_token_with_id(
            db,
            owner,
            name,
            (max_nodes, ttl_secs),
            uuid::Uuid::new_v4(),
        )
        .await
    }

    pub async fn create_registration_token_with_id(
        &self,
        db: &mongodb::Database,
        owner: &str,
        name: &str,
        limits: (u32, i64),
        id: uuid::Uuid,
    ) -> AppResult<(String, String, DateTime<Utc>)> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(AppError::ValidationError(
                "Node name must be 1-64 lowercase letters, digits, or hyphens".into(),
            ));
        }
        let raw = Zeroizing::new(format!(
            "nyx_nreg_{}",
            hex::encode(rand::random::<[u8; 32]>())
        ));
        let now = Utc::now();
        let token = NodeRegistrationToken {
            id: id.to_string(),
            user_id: owner.to_owned(),
            token_hash: hash_token(&raw),
            name: name.to_owned(),
            used: false,
            expires_at: now + Duration::seconds(limits.1),
            created_at: now,
        };
        let change = HumanNodeChange::Registration {
            token,
            max_nodes: limits.0,
        };
        self.apply(db, owner, &change).await?;
        let HumanNodeChange::Registration { token, .. } = change else {
            unreachable!()
        };
        Ok((token.id, raw.to_string(), token.expires_at))
    }

    pub async fn rotate_auth_token(
        &self,
        db: &mongodb::Database,
        keys: &EncryptionKeys,
        node_id: &str,
    ) -> AppResult<(String, String)> {
        let material = prepare_managed_credentials(keys).await?;
        let change = HumanNodeChange::Rotation {
            node_id: node_id.to_owned(),
            auth_hash: hash_token(&material.auth_token),
            signing_hash: hash_token(&material.signing_secret),
            encrypted: material.signing_secret_encrypted,
        };
        let owner = load_active_node(db, node_id).await?.user_id;
        self.apply(db, &owner, &change).await?;
        Ok((
            material.auth_token.to_string(),
            material.signing_secret.to_string(),
        ))
    }

    pub async fn create_binding(
        &self,
        db: &mongodb::Database,
        node_id: &str,
        service_id: &str,
    ) -> AppResult<NodeServiceBinding> {
        let node = load_active_node(db, node_id).await?;
        let now = Utc::now();
        let binding = NodeServiceBinding {
            id: uuid::Uuid::new_v4().to_string(),
            node_id: node_id.into(),
            user_id: node.user_id.clone(),
            service_id: service_id.into(),
            is_active: true,
            priority: 0,
            created_at: now,
            updated_at: now,
        };
        self.apply(
            db,
            &node.user_id,
            &HumanNodeChange::BindingCreate(binding.clone()),
        )
        .await?;
        Ok(binding)
    }

    pub async fn update_binding_priority(
        &self,
        db: &mongodb::Database,
        node_id: &str,
        binding_id: &str,
        priority: i32,
    ) -> AppResult<()> {
        let owner = load_active_node(db, node_id).await?.user_id;
        self.apply(
            db,
            &owner,
            &HumanNodeChange::BindingUpdate {
                node_id: node_id.into(),
                binding_id: binding_id.into(),
                priority,
            },
        )
        .await
    }

    pub async fn delete_binding(
        &self,
        db: &mongodb::Database,
        node_id: &str,
        binding_id: &str,
    ) -> AppResult<()> {
        let owner = load_active_node(db, node_id).await?.user_id;
        self.apply(
            db,
            &owner,
            &HumanNodeChange::BindingDelete {
                node_id: node_id.into(),
                binding_id: binding_id.into(),
            },
        )
        .await
    }

    pub async fn admin_delete_node(&self, db: &mongodb::Database, node_id: &str) -> AppResult<()> {
        let owner = load_active_node(db, node_id).await?.user_id;
        self.apply(
            db,
            &owner,
            &HumanNodeChange::AdminDelete {
                node_id: node_id.into(),
            },
        )
        .await
    }

    pub async fn admin_disconnect_node(
        &self,
        db: &mongodb::Database,
        node_id: &str,
    ) -> AppResult<()> {
        let owner = load_active_node(db, node_id).await?.user_id;
        self.apply(
            db,
            &owner,
            &HumanNodeChange::AdminDisconnect {
                node_id: node_id.into(),
            },
        )
        .await
    }

    pub async fn transfer_node_owner(
        &self,
        db: &mongodb::Database,
        node_id: &str,
        new_owner: &str,
        max_nodes: u32,
        expected_version: Option<i64>,
    ) -> AppResult<TransferNodeResult> {
        transfer_node_owner_impl(
            db,
            &self.actor,
            node_id,
            new_owner,
            max_nodes,
            expected_version,
            Some(self),
        )
        .await
    }

    pub async fn delete_node(
        &self,
        db: &mongodb::Database,
        node_id: &str,
        expected_version: Option<i64>,
    ) -> AppResult<()> {
        delete_node_impl(db, &self.actor, node_id, expected_version, Some(self)).await
    }

    async fn apply(
        &self,
        db: &mongodb::Database,
        owner: &str,
        change: &HumanNodeChange,
    ) -> AppResult<()> {
        let admin = matches!(
            change,
            HumanNodeChange::AdminDelete { .. } | HumanNodeChange::AdminDisconnect { .. }
        );
        if !admin
            && !org_service::resolve_owner_access(db, &self.actor, owner)
                .await?
                .can_write()
        {
            return Err(AppError::NodeNotFound("Node not found".into()));
        }
        let now = bson::DateTime::from_chrono(Utc::now());
        let context = (db, self, owner, change, now);
        super::audit_runtime::transaction::run(
            db,
            &context,
            |session, (db, audit, owner, change, now)| {
                Box::pin(change.apply_in_session(db, session, audit, owner, *now))
            },
        )
        .await
    }
}

enum HumanNodeChange {
    Registration {
        token: NodeRegistrationToken,
        max_nodes: u32,
    },
    Rotation {
        node_id: String,
        auth_hash: String,
        signing_hash: String,
        encrypted: Vec<u8>,
    },
    BindingCreate(NodeServiceBinding),
    BindingUpdate {
        node_id: String,
        binding_id: String,
        priority: i32,
    },
    BindingDelete {
        node_id: String,
        binding_id: String,
    },
    AdminDelete {
        node_id: String,
    },
    AdminDisconnect {
        node_id: String,
    },
}

impl HumanNodeChange {
    async fn apply_in_session(
        &self,
        db: &mongodb::Database,
        session: &mut mongodb::ClientSession,
        audit: &NodeMutationAudit,
        owner: &str,
        now: bson::DateTime,
    ) -> AppResult<()> {
        let admin = matches!(
            self,
            Self::AdminDelete { .. } | Self::AdminDisconnect { .. }
        );
        let audit_owner = node_mutation_owner_in_session(db, session, audit, owner, admin).await?;
        audit.require_uncommitted(db, session).await?;
        if let HumanNodeChange::Registration { token, max_nodes } = self {
            if db
                .collection::<Node>(NODES)
                .count_documents(doc! {"user_id": owner, "is_active": true})
                .session(&mut *session)
                .await?
                >= u64::from(*max_nodes)
            {
                return Err(AppError::BadRequest(format!(
                    "Maximum of {max_nodes} nodes per user reached"
                )));
            }
            if db
                .collection::<Node>(NODES)
                .find_one(doc! {"user_id": owner, "name": &token.name, "is_active": true})
                .session(&mut *session)
                .await?
                .is_some()
            {
                return Err(AppError::Conflict(
                    "A node with that name already exists".into(),
                ));
            }
            db.collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
                .insert_one(token)
                .session(&mut *session)
                .await?;
            return audit
                .record(
                    db,
                    session,
                    audit_owner,
                    "node_registration_token_created",
                    &token.id,
                    AuditTier::Standard,
                )
                .await;
        }
        let node_id = self.node_id();
        let node = db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id": node_id, "user_id": owner, "is_active": true})
            .session(&mut *session)
            .await?
            .ok_or_else(|| AppError::NodeNotFound("Node not found".into()))?;
        if matches!(
            self,
            HumanNodeChange::Rotation { .. }
                | HumanNodeChange::AdminDelete { .. }
                | HumanNodeChange::AdminDisconnect { .. }
        ) {
            guard_human_node_in_session(db, session, &node).await?;
        }
        let (action, target, tier) = match self {
            HumanNodeChange::Rotation {
                auth_hash,
                signing_hash,
                encrypted,
                ..
            } => {
                db.collection::<Node>(NODES).update_one(doc! {"_id": node_id, "user_id": owner, "is_active": true}, doc! {"$set": {
                    "auth_token_hash": auth_hash, "signing_secret_hash": signing_hash,
                    "signing_secret_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted.clone()}, "updated_at": now,
                }, "$inc": {"access_revision": 1_i64, "state_version": 1_i64}}).session(&mut *session).await?;
                ("node_token_rotated", node_id, AuditTier::Standard)
            }
            HumanNodeChange::BindingCreate(binding) => {
                if db
                    .collection::<bson::Document>(
                        crate::models::downstream_service::COLLECTION_NAME,
                    )
                    .find_one(doc! {"_id": &binding.service_id, "is_active": true})
                    .session(&mut *session)
                    .await?
                    .is_none()
                {
                    return Err(AppError::NotFound("Service not found".into()));
                }
                if db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS).find_one(doc! {"node_id": node_id, "user_id": owner, "service_id": &binding.service_id, "is_active": true}).session(&mut *session).await?.is_some() {
                    return Err(AppError::Conflict("Binding already exists for this node and service".into()));
                }
                db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                    .insert_one(binding)
                    .session(&mut *session)
                    .await?;
                (
                    "node_binding_created",
                    binding.id.as_str(),
                    AuditTier::Standard,
                )
            }
            HumanNodeChange::BindingUpdate {
                binding_id,
                priority,
                ..
            } => {
                let updated = db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS).update_one(doc! {"_id": binding_id, "node_id": node_id, "user_id": owner, "is_active": true}, doc! {"$set": {"priority": priority, "updated_at": now}}).session(&mut *session).await?;
                if updated.matched_count != 1 {
                    return Err(AppError::NotFound("Binding not found".into()));
                }
                (
                    "node_binding_updated",
                    binding_id.as_str(),
                    AuditTier::Standard,
                )
            }
            HumanNodeChange::BindingDelete { binding_id, .. } => {
                let updated = db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS).update_one(doc! {"_id": binding_id, "node_id": node_id, "user_id": owner, "is_active": true}, doc! {"$set": {"is_active": false, "updated_at": now}}).session(&mut *session).await?;
                if updated.matched_count != 1 {
                    return Err(AppError::NotFound("Binding not found".into()));
                }
                (
                    "node_binding_deleted",
                    binding_id.as_str(),
                    AuditTier::Narrowing,
                )
            }
            HumanNodeChange::AdminDelete { .. } => {
                db.collection::<Node>(NODES).update_one(doc! {"_id": node_id, "user_id": owner, "is_active": true}, doc! {"$set": {"is_active": false, "status": "offline", "updated_at": now}, "$inc": {"state_version": 1_i64, "access_revision": 1_i64}}).session(&mut *session).await?;
                db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                    .update_many(
                        doc! {"node_id": node_id},
                        doc! {"$set": {"is_active": false, "updated_at": now}},
                    )
                    .session(&mut *session)
                    .await?;
                ("admin_node_deleted", node_id, AuditTier::Narrowing)
            }
            HumanNodeChange::AdminDisconnect { .. } => {
                ("admin_node_disconnected", node_id, AuditTier::Narrowing)
            }
            HumanNodeChange::Registration { .. } => unreachable!(),
        };
        audit
            .record(db, session, audit_owner, action, target, tier)
            .await
    }

    fn node_id(&self) -> &str {
        match self {
            Self::Registration { token, .. } => &token.id,
            Self::BindingCreate(binding) => &binding.node_id,
            Self::Rotation { node_id, .. }
            | Self::BindingUpdate { node_id, .. }
            | Self::BindingDelete { node_id, .. }
            | Self::AdminDelete { node_id }
            | Self::AdminDisconnect { node_id } => node_id,
        }
    }
}

async fn node_mutation_owner_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    audit: &NodeMutationAudit,
    owner: &str,
    admin: bool,
) -> AppResult<AuditOwner> {
    let actor = &audit.actor;
    let actor_filter = match &audit.principal {
        AuditPrincipal::Person { id } if id == actor => {
            doc! {"_id": actor, "is_active": true, "user_type": "person"}
        }
        AuditPrincipal::ApiKey { owner_id, .. } if owner_id == actor && !admin => {
            doc! {"_id": actor, "is_active": true}
        }
        _ => return Err(AppError::Forbidden("node_person_principal_required".into())),
    };
    let person = db
        .collection::<User>(USERS)
        .find_one(actor_filter)
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Forbidden("node_person_principal_required".into()))?;
    if admin {
        let role = db.collection::<crate::models::role::Role>(crate::models::role::COLLECTION_NAME).find_one(doc! {"slug": crate::models::role::PLATFORM_ADMIN_ROLE_SLUG, "is_system": true, "client_id": bson::Bson::Null}).session(&mut *session).await?.ok_or_else(|| AppError::Forbidden("Admin access required".into()))?;
        if !person.role_ids.contains(&role.id) {
            return Err(AppError::Forbidden("Admin access required".into()));
        }
    }
    if !admin
        && !org_service::resolve_owner_access_in_session(db, session, actor, owner)
            .await?
            .can_write()
    {
        return Err(AppError::NodeNotFound("Node not found".into()));
    }
    let owner_row = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": owner})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NodeNotFound("Node owner not found".into()))?;
    Ok(if owner_row.user_type.is_org() {
        AuditOwner::Org { id: owner.into() }
    } else {
        AuditOwner::Person { id: owner.into() }
    })
}

async fn guard_human_node_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    node: &Node,
) -> AppResult<()> {
    if db
        .collection::<bson::Document>(crate::models::remote_machine::COLLECTION_NAME)
        .find_one(doc! {"_id": &node.id})
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "managed_node_mutation_requires_lifecycle".into(),
        ));
    }
    if node
        .metadata
        .as_ref()
        .and_then(|m| m.provisioning_source.as_deref())
        .is_some_and(|s| matches!(s, "remote_machine" | "managed_machine" | "agent_sandbox"))
    {
        return Err(AppError::Conflict(
            "managed_node_mapping_missing_reconciliation_required".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeOwnerKind {
    User,
    Org,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeOwnerInfo {
    pub kind: NodeOwnerKind,
    pub id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeWithOwner {
    pub node: Node,
    pub owner: NodeOwnerInfo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeAdminRole {
    Owner,
    Admin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAdminInfo {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub role: NodeAdminRole,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferNodeResult {
    pub node_id: String,
    pub previous_owner_user_id: String,
    pub new_owner_user_id: String,
    pub deactivated_bindings_count: u64,
    pub cleared_user_service_count: u64,
    pub deactivated_pending_credentials_count: u64,
}

#[derive(Clone)]
pub struct DeviceNodeInput<'a> {
    pub user_id: &'a str,
    pub api_key_id: &'a str,
    pub hw_id: &'a str,
    pub label: &'a str,
    pub device_pubkey: Option<&'a [u8; 32]>,
    pub provisioning_source: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeAuthorizationLifecycle {
    RegistrationPending,
    Active,
}

impl NodeAuthorizationLifecycle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RegistrationPending => "registration_pending",
            Self::Active => "active",
        }
    }
}

#[derive(Clone, Debug)]
pub struct NodeAuthorizationState {
    pub id: String,
    pub owner_user_id: String,
    pub lifecycle: NodeAuthorizationLifecycle,
    pub is_active: bool,
    pub state_version: i64,
    pub access_revision: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub registration_expires_at: Option<DateTime<Utc>>,
}

impl fmt::Debug for DeviceNodeInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceNodeInput")
            .field("user_id", &self.user_id)
            .field("api_key_id", &RedactedLen(self.api_key_id.len()))
            .field("hw_id", &self.hw_id)
            .field("label", &self.label)
            .field(
                "device_pubkey",
                &self.device_pubkey.map(|pubkey| RedactedLen(pubkey.len())),
            )
            .field("provisioning_source", &self.provisioning_source)
            .finish()
    }
}

pub const DEVICE_CODE_PROVISIONING_SOURCE: &str = "device-code";
pub const DEVICE_ONBOARD_PROVISIONING_SOURCE: &str = "device-onboard";
const DEVICE_PROVISIONING_SOURCES: &[&str] = &[
    DEVICE_CODE_PROVISIONING_SOURCE,
    DEVICE_ONBOARD_PROVISIONING_SOURCE,
];

/// Create a one-time registration token for a new node.
/// Returns (token_id, raw_token, expires_at). The raw token is shown once and never stored.
#[cfg(test)]
pub async fn create_registration_token(
    db: &mongodb::Database,
    user_id: &str,
    name: &str,
    max_nodes_per_user: u32,
    ttl_secs: i64,
) -> AppResult<(String, String, DateTime<Utc>)> {
    create_registration_token_with_id(
        db,
        user_id,
        name,
        max_nodes_per_user,
        ttl_secs,
        &uuid::Uuid::new_v4().to_string(),
    )
    .await
}

/// Create a one-time registration token whose id is reserved by the caller.
/// Assistant effects use the same UUID as the eventual node id so their safe
/// `node` resource remains authoritative before and after token redemption.
pub async fn create_registration_token_with_id(
    db: &mongodb::Database,
    user_id: &str,
    name: &str,
    max_nodes_per_user: u32,
    ttl_secs: i64,
    token_id: &str,
) -> AppResult<(String, String, DateTime<Utc>)> {
    let token_id = uuid::Uuid::parse_str(token_id)
        .map_err(|_| AppError::ValidationError("token_id must be a UUID".to_string()))?
        .to_string();
    // Validate name
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::ValidationError(
            "Node name must be 1-64 characters".to_string(),
        ));
    }

    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(AppError::ValidationError(
            "Node name must contain only lowercase letters, digits, and hyphens".to_string(),
        ));
    }

    // Check max nodes limit
    let existing_count = db
        .collection::<Node>(NODES)
        .count_documents(doc! { "user_id": user_id, "is_active": true })
        .await?;

    if existing_count >= max_nodes_per_user as u64 {
        return Err(AppError::BadRequest(format!(
            "Maximum of {max_nodes_per_user} nodes per user reached"
        )));
    }

    // Check if node name already exists for this user
    let existing_name = db
        .collection::<Node>(NODES)
        .find_one(doc! { "user_id": user_id, "name": name, "is_active": true })
        .await?;

    if existing_name.is_some() {
        return Err(AppError::Conflict(format!(
            "A node with name '{name}' already exists"
        )));
    }

    // Generate token
    let raw_token = format!("nyx_nreg_{}", hex::encode(rand::random::<[u8; 32]>()));
    let token_hash = hash_token(&raw_token);
    let now = Utc::now();
    let expires_at = now + Duration::seconds(ttl_secs);
    let token = NodeRegistrationToken {
        id: token_id.clone(),
        user_id: user_id.to_string(),
        token_hash,
        name: name.to_string(),
        used: false,
        expires_at,
        created_at: now,
    };

    db.collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
        .insert_one(&token)
        .await?;

    Ok((token_id, raw_token, expires_at))
}

/// Consume a registration token and create a new Node record.
/// Returns (Node, raw_auth_token, raw_signing_secret).
pub async fn register_node(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    raw_token: &str,
    metadata: Option<NodeMetadata>,
) -> AppResult<(Node, String, String)> {
    let token_hash = hash_token(raw_token);
    let now = Utc::now();

    // Find and consume the token atomically
    let token = db
        .collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
        .find_one_and_update(
            doc! {
                "token_hash": &token_hash,
                "used": false,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            },
            doc! {
                "$set": { "used": true },
            },
        )
        .await?
        .ok_or_else(|| {
            AppError::NodeRegistrationFailed("Invalid or expired registration token".to_string())
        })?;

    // Validate metadata fields if present
    if let Some(ref meta) = metadata {
        validate_node_metadata(meta)?;
    }

    // Generate auth token for the node
    let raw_auth_token = format!("nyx_nauth_{}", hex::encode(rand::random::<[u8; 32]>()));
    let auth_token_hash = hash_token(&raw_auth_token);

    // Generate HMAC signing secret
    let raw_signing_secret = hex::encode(rand::random::<[u8; 32]>());
    let signing_secret_encrypted = Some(
        encryption_keys
            .encrypt(raw_signing_secret.as_bytes())
            .await?,
    );
    let signing_secret_hash = hash_token(&raw_signing_secret);

    let node = Node {
        machine: None,
        machine_confirm: Default::default(),
        allow_single_user_saved_logins: false,
        // Registration-token ids are reserved as the future node identity.
        // This gives callers one stable, secret-free resource reference for
        // both the pending registration and the registered node.
        id: token.id,
        user_id: token.user_id,
        name: token.name,
        status: NodeStatus::Online,
        auth_token_hash,
        signing_secret_encrypted,
        signing_secret_hash,
        last_heartbeat_at: Some(now),
        connected_at: Some(now),
        metadata,
        metrics: crate::models::node::NodeMetrics::default(),
        connection_owner: None,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    db.collection::<Node>(NODES).insert_one(&node).await?;

    tracing::info!(
        node_id = %node.id,
        user_id = %node.user_id,
        name = %node.name,
        "Node registered"
    );

    Ok((node, raw_auth_token, raw_signing_secret))
}

/// Create a Node row for an API-key-provisioned headless device.
///
/// Device-code and QR-onboarded devices receive a scoped NyxID API key and
/// authenticate through the proxy API, not the node WebSocket protocol. The row
/// still carries valid random node auth material so it satisfies the Node model
/// invariants and cannot collide on an empty auth hash.
pub async fn create_for_device(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    input: DeviceNodeInput<'_>,
) -> AppResult<Node> {
    if input.hw_id.trim().is_empty() || input.hw_id.len() > 256 {
        return Err(AppError::ValidationError(
            "hw_id must be between 1 and 256 characters".to_string(),
        ));
    }
    if input.label.trim().is_empty() || input.label.len() > 200 {
        return Err(AppError::ValidationError(
            "Device label must be between 1 and 200 characters".to_string(),
        ));
    }
    if input.provisioning_source.trim().is_empty() || input.provisioning_source.len() > 64 {
        return Err(AppError::ValidationError(
            "Device provisioning source must be between 1 and 64 characters".to_string(),
        ));
    }

    let now = Utc::now();
    let node_id = uuid::Uuid::new_v4().to_string();
    let pubkey_fingerprint = input.device_pubkey.map(|device_pubkey| {
        let pubkey_digest = Sha256::digest(device_pubkey);
        hex::encode(&pubkey_digest[..8])
    });
    let raw_auth_token = Zeroizing::new(format!(
        "nyx_nauth_{}",
        hex::encode(rand::random::<[u8; 32]>())
    ));
    let auth_token_hash = hash_token(raw_auth_token.as_str());
    let raw_signing_secret = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
    let signing_secret_encrypted = encryption_keys
        .encrypt(raw_signing_secret.as_bytes())
        .await?;
    let signing_secret_hash = hash_token(raw_signing_secret.as_str());

    let node = Node {
        machine: None,
        machine_confirm: Default::default(),
        allow_single_user_saved_logins: false,
        id: node_id.clone(),
        user_id: input.user_id.to_string(),
        name: device_node_name(input.label, &node_id),
        status: NodeStatus::Offline,
        auth_token_hash,
        signing_secret_encrypted: Some(signing_secret_encrypted),
        signing_secret_hash,
        last_heartbeat_at: None,
        connected_at: None,
        metadata: Some(NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: Some(input.provisioning_source.to_string()),
        }),
        metrics: crate::models::node::NodeMetrics::default(),
        connection_owner: None,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    db.collection::<Node>(NODES).insert_one(&node).await?;

    tracing::info!(
        node_id = %node.id,
        user_id = %node.user_id,
        api_key_id = %input.api_key_id,
        hw_id = %input.hw_id,
        device_pubkey_fingerprint = pubkey_fingerprint.as_deref(),
        provisioning_source = %input.provisioning_source,
        "Device node record created"
    );

    Ok(node)
}

/// Get a single node by ID without ownership check.
/// Used internally (e.g., heartbeat sweep).
pub async fn get_node_by_id(db: &mongodb::Database, node_id: &str) -> AppResult<Option<Node>> {
    let node = db
        .collection::<Node>(NODES)
        .find_one(doc! { "_id": node_id, "is_active": true })
        .await?;
    Ok(node)
}

/// Get multiple active nodes by their IDs without ownership checks.
pub async fn get_nodes_by_ids(db: &mongodb::Database, node_ids: &[String]) -> AppResult<Vec<Node>> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }
    let filter = doc! {
        "_id": { "$in": node_ids },
        "is_active": true
    };
    let cursor = db.collection::<Node>(NODES).find(filter).await?;
    let nodes: Vec<Node> = cursor.try_collect().await?;
    Ok(nodes)
}

/// Decrypt and decode a node's HMAC signing secret.
pub async fn get_node_signing_secret(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_id: &str,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let Some(node) = get_node_by_id(db, node_id).await? else {
        return Err(AppError::NodeNotFound(format!(
            "Node {node_id} not found during request signing"
        )));
    };

    signing_secret_from_node(encryption_keys, &node).await
}

/// Machine tools already batch-load their live nodes. Reusing that snapshot
/// avoids a second database read solely to decrypt the signing key.
pub(crate) async fn signing_secret_from_node(
    encryption_keys: &EncryptionKeys,
    node: &Node,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let node_id = &node.id;

    let Some(encrypted_secret) = node.signing_secret_encrypted.as_deref() else {
        return Err(AppError::NodeOffline(format!(
            "Node {node_id} is missing its signing secret"
        )));
    };

    let decrypted_secret = Zeroizing::new(
        encryption_keys
            .decrypt(encrypted_secret)
            .await
            .map_err(|e| {
                AppError::Internal(format!(
                    "Failed to decrypt node signing secret for {node_id}: {e}"
                ))
            })?,
    );

    decode_node_signing_secret(&decrypted_secret, node_id)
}

/// Get a single node by ID, verifying ownership.
pub async fn get_node(db: &mongodb::Database, user_id: &str, node_id: &str) -> AppResult<Node> {
    let node = load_active_node(db, node_id).await?;
    ensure_node_readable_by_access(db, user_id, &node).await?;
    Ok(node)
}

/// Return the secret-free authority state for a node resource. A freshly
/// minted registration token is addressable by the same UUID that redemption
/// will assign to the node, so one typed resource remains valid across the
/// registration lifecycle. Used or expired registration rows never mask an
/// absent/deleted node.
pub async fn get_node_authorization_state(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
) -> AppResult<NodeAuthorizationState> {
    if let Some(node) = db
        .collection::<Node>(NODES)
        .find_one(doc! { "_id": node_id, "is_active": true })
        .await?
    {
        ensure_node_readable_by_access(db, actor_user_id, &node).await?;
        let authority = db
            .collection::<bson::Document>(NODES)
            .find_one(doc! { "_id": node_id, "is_active": true })
            .await?
            .ok_or_else(|| AppError::NodeNotFound("Node not found".to_string()))?;
        let state_version = authority.get_i64("state_version").unwrap_or(1).max(1);
        let access_revision = authority.get_i64("access_revision").unwrap_or(0).max(0);
        return Ok(NodeAuthorizationState {
            id: node.id,
            owner_user_id: node.user_id,
            lifecycle: NodeAuthorizationLifecycle::Active,
            is_active: true,
            state_version,
            access_revision,
            created_at: node.created_at,
            updated_at: node.updated_at,
            registration_expires_at: None,
        });
    }

    let registration = db
        .collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
        .find_one(doc! { "_id": node_id, "used": false })
        .await?
        .ok_or_else(|| AppError::NodeNotFound("Node not found".to_string()))?;
    let access =
        org_service::resolve_owner_access(db, actor_user_id, &registration.user_id).await?;
    if !node_access_can_read(&access) {
        return Err(AppError::NodeNotFound("Node not found".to_string()));
    }

    Ok(NodeAuthorizationState {
        id: registration.id,
        owner_user_id: registration.user_id,
        lifecycle: NodeAuthorizationLifecycle::RegistrationPending,
        is_active: registration.expires_at > Utc::now(),
        state_version: 1,
        access_revision: 0,
        created_at: registration.created_at,
        updated_at: registration.created_at,
        registration_expires_at: Some(registration.expires_at),
    })
}

/// Look up a node and verify the actor has write access to it -- either as
/// the direct owner or as an admin of the org that owns it.
///
/// Used by `user_service_service::create_user_service` and
/// `update_user_service` so that an admin can route an org-owned service
/// through their personal node (where they're the direct owner) without
/// having to also re-register the node under the org. The check is
/// actor-based rather than service-owner based: it's the human (or API
/// key) making the request who needs node access, not the service's
/// effective owner.
///
/// Returns `NodeNotFound` for any of: missing row, inactive node, or
/// actor without write access (no metadata leak).
pub async fn ensure_node_writable_by_actor(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
) -> AppResult<Node> {
    let node = db
        .collection::<Node>(NODES)
        .find_one(doc! { "_id": node_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NodeNotFound("Node not found".to_string()))?;

    ensure_node_writable_by_access(db, actor_user_id, &node).await?;
    Ok(node)
}

/// List all active nodes reachable by a user, including org-owned nodes for
/// orgs where the actor is an admin or member.
pub async fn list_user_nodes(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<NodeWithOwner>> {
    let memberships = org_service::list_memberships_for_member(db, user_id, false).await?;
    let mut owner_ids = vec![user_id.to_string()];
    for membership in memberships {
        if membership.role.can_proxy() && !owner_ids.iter().any(|id| id == &membership.org_user_id)
        {
            owner_ids.push(membership.org_user_id);
        }
    }

    let owner_id_array: bson::Array = owner_ids
        .iter()
        .map(|id| bson::Bson::String(id.clone()))
        .collect();
    let nodes: Vec<Node> = db
        .collection::<Node>(NODES)
        .find(doc! { "user_id": { "$in": owner_id_array }, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;
    attach_owner_info(db, nodes).await
}

/// Transfer an active node to another owner and detach owner-scoped routing state.
#[cfg(test)]
pub async fn transfer_node_owner(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    new_owner_user_id: &str,
    max_nodes_per_user: u32,
) -> AppResult<TransferNodeResult> {
    transfer_node_owner_with_expected_state_version(
        db,
        actor_user_id,
        node_id,
        new_owner_user_id,
        max_nodes_per_user,
        None,
    )
    .await
}

/// Transfer a node while atomically fencing the mutation on the authority
/// projection's expected state version. Missing `state_version` is treated as
/// the legacy version-one row, matching the authorization projection.
pub async fn transfer_node_owner_with_expected_state_version(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    new_owner_user_id: &str,
    max_nodes_per_user: u32,
    expected_state_version: Option<i64>,
) -> AppResult<TransferNodeResult> {
    transfer_node_owner_impl(
        db,
        actor_user_id,
        node_id,
        new_owner_user_id,
        max_nodes_per_user,
        expected_state_version,
        None,
    )
    .await
}

async fn transfer_node_owner_impl(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    new_owner_user_id: &str,
    max_nodes_per_user: u32,
    expected_state_version: Option<i64>,
    audit: Option<&NodeMutationAudit>,
) -> AppResult<TransferNodeResult> {
    super::remote_machine_service::guard_legacy_node_mutation(db, node_id, "transfer").await?;
    let node = load_active_node(db, node_id).await?;
    ensure_node_writable_by_access(db, actor_user_id, &node).await?;

    if new_owner_user_id == node.user_id {
        return Err(AppError::BadRequest(
            "node already belongs to that owner".to_string(),
        ));
    }

    let destination_access =
        org_service::resolve_owner_access(db, actor_user_id, new_owner_user_id).await?;
    if !destination_access.can_write() {
        return Err(AppError::Forbidden(
            "You must be the destination owner or an admin of the destination org".to_string(),
        ));
    }

    let db = db.clone();
    let node_id = node_id.to_string();
    let previous_owner_user_id = node.user_id;
    let node_name = node.name;
    let new_owner_user_id = new_owner_user_id.to_string();
    let audit = audit.cloned();
    let now = bson::DateTime::from_chrono(Utc::now());
    crate::services::service_history::transaction::run(&db.clone(), async move |session| {
            let operation: AppResult<TransferNodeResult> = async {
                let current_filter = doc! {
                    "_id": &node_id,
                    "user_id": &previous_owner_user_id,
                    "is_active": true,
                };
                let current = db
                    .collection::<Node>(NODES)
                    .find_one(current_filter)
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NodeNotFound("Node not found".to_string()))?;

                let audit_owners = if let Some(audit) = audit.as_ref() {
                    audit.require_uncommitted(&db, session).await?;
                    let previous = node_mutation_owner_in_session(&db, session, audit, &previous_owner_user_id, false).await?;
                    let destination = node_mutation_owner_in_session(&db, session, audit, &new_owner_user_id, false).await?;
                    guard_human_node_in_session(&db, session, &current).await?;
                    Some((previous, destination))
                } else { None };

                let destination_name_collision = db
                    .collection::<Node>(NODES)
                    .find_one(doc! {
                        "_id": { "$ne": &node_id },
                        "user_id": &new_owner_user_id,
                        "name": &node_name,
                        "is_active": true,
                    })
                    .session(&mut *session)
                    .await?;
                if destination_name_collision.is_some() {
                    return Err(AppError::BadRequest(format!(
                        "An active node named '{}' already exists for the destination owner. Rename one of them and retry.",
                        current.name
                    )));
                }

                let destination_node_count = db
                    .collection::<Node>(NODES)
                    .count_documents(doc! {
                        "user_id": &new_owner_user_id,
                        "is_active": true,
                    })
                    .session(&mut *session)
                    .await?;
                if destination_node_count >= max_nodes_per_user as u64 {
                    return Err(AppError::BadRequest(format!(
                        "Maximum of {max_nodes_per_user} nodes per user reached"
                    )));
                }

                let pending_credential_result = db
                    .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
                    .update_many(
                        doc! { "node_id": &node_id, "is_active": true },
                        doc! { "$set": { "is_active": false } },
                    )
                    .session(&mut *session)
                    .await?;
                let service_result = crate::services::service_history::collection::<UserService>(&db, USER_SERVICES)
                    .update_many(
                        doc! {
                            "node_id": &node_id,
                            "user_id": { "$ne": &new_owner_user_id },
                            "is_active": true,
                        },
                        doc! {
                            "$unset": { "node_id": "" },
                            "$set": { "updated_at": &now },
                        },
                    )
                    .session(&mut *session)
                    .await?;
                let binding_result = db
                    .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                    .update_many(
                        doc! { "node_id": &node_id, "is_active": true },
                        doc! { "$set": { "is_active": false, "updated_at": &now } },
                    )
                    .session(&mut *session)
                    .await?;
                let mut node_filter = doc! {
                    "_id": &node_id,
                    "user_id": &previous_owner_user_id,
                    "is_active": true,
                };
                if let Some(expected) = expected_state_version {
                    node_filter.insert(
                        "$or",
                        vec![
                            doc! { "state_version": expected },
                            doc! { "state_version": { "$exists": false } },
                        ],
                    );
                }
                let state_version_fallback = expected_state_version.unwrap_or(0);
                let update_result = db
                    .collection::<Node>(NODES)
                    .update_one(
                        node_filter,
                        vec![doc! {
                            "$set": {
                                "user_id": &new_owner_user_id,
                                "updated_at": &now,
                                "state_version": {
                                    "$add": [
                                        { "$ifNull": ["$state_version", state_version_fallback] },
                                        1_i64,
                                    ]
                                },
                            }
                        }],
                    )
                    .session(&mut *session)
                    .await?;
                if update_result.matched_count == 0 {
                    if expected_state_version.is_some() {
                        return Err(AppError::Conflict(
                            "the node changed since this action was prepared".to_string(),
                        ));
                    }
                    return Err(AppError::NodeNotFound("Node not found".to_string()));
                }

                if let (Some(audit), Some((previous, destination))) = (audit.as_ref(), audit_owners) {
                    audit.record(&db, session, previous, "node_transferred", &node_id, AuditTier::Standard).await?;
                    // Both tenants retain visibility of the authority transition.
                    // The receiving owner's record has its own stable operation:
                    // replay identity is (operation, stage), including any close outcome.
                    let destination_event = AuditEvent {
                        operation_id: audit.destination_event_id.to_string(), stage: AuditStage::Committed,
                        principal: audit.principal.clone(), owner: destination,
                        action: "node_transfer_received", target_id: Some(node_id.clone()),
                    };
                    audit_outbox_service::enqueue_admitted_in_session(&db, session, &audit.runtime, AuditTier::Standard, &audit.destination_event_id, &destination_event).await?;
                }

                Ok(TransferNodeResult {
                    node_id: node_id.clone(),
                    previous_owner_user_id: previous_owner_user_id.clone(),
                    new_owner_user_id: new_owner_user_id.clone(),
                    deactivated_bindings_count: binding_result.modified_count,
                    cleared_user_service_count: service_result.modified_count,
                    deactivated_pending_credentials_count: pending_credential_result.modified_count,
                })
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

/// Soft-delete a node and its bindings.
pub async fn delete_node(db: &mongodb::Database, user_id: &str, node_id: &str) -> AppResult<()> {
    delete_node_with_expected_state_version(db, user_id, node_id, None).await
}

/// Soft-delete a node while atomically fencing the mutation on the expected
/// authority state version. Legacy rows without `state_version` are treated as
/// version one and advance to the next version in the same write.
pub async fn delete_node_with_expected_state_version(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    expected_state_version: Option<i64>,
) -> AppResult<()> {
    delete_node_impl(db, user_id, node_id, expected_state_version, None).await
}

async fn delete_node_impl(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    expected_state_version: Option<i64>,
    audit: Option<&NodeMutationAudit>,
) -> AppResult<()> {
    super::remote_machine_service::guard_legacy_node_mutation(db, node_id, "delete").await?;
    let node = load_active_node(db, node_id).await?;
    ensure_node_writable_by_access(db, user_id, &node).await?;

    let db = db.clone();
    let node_id = node_id.to_string();
    let tracing_node_id = node_id.clone();
    let owner_user_id = node.user_id;
    let audit = audit.cloned();
    let now = bson::DateTime::from_chrono(Utc::now());
    crate::services::service_history::transaction::run(&db.clone(), async move |session| {
        let operation: AppResult<()> = async {
            let audit_owner = if let Some(audit) = audit.as_ref() {
                audit.require_uncommitted(&db, session).await?;
                let current = db
                    .collection::<Node>(NODES)
                    .find_one(doc! {"_id": &node_id, "user_id": &owner_user_id, "is_active": true})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NodeNotFound("Node not found".into()))?;
                guard_human_node_in_session(&db, session, &current).await?;
                Some(
                    node_mutation_owner_in_session(&db, session, audit, &owner_user_id, false)
                        .await?,
                )
            } else {
                None
            };
            let mut node_filter = doc! {
                "_id": &node_id,
                "user_id": &owner_user_id,
                "is_active": true,
            };
            if let Some(expected) = expected_state_version {
                node_filter.insert(
                    "$or",
                    vec![
                        doc! { "state_version": expected },
                        doc! { "state_version": { "$exists": false } },
                    ],
                );
            }
            let state_version_fallback = expected_state_version.unwrap_or(0);
            let result = db
                .collection::<Node>(NODES)
                .update_one(
                    node_filter,
                    vec![doc! {
                        "$set": {
                            "is_active": false,
                            "status": NodeStatus::Offline.as_str(),
                            "updated_at": &now,
                            "state_version": {
                                "$add": [
                                    { "$ifNull": ["$state_version", state_version_fallback] },
                                    1_i64,
                                ]
                            },
                        }
                    }],
                )
                .session(&mut *session)
                .await?;
            if result.matched_count == 0 {
                if expected_state_version.is_some() {
                    return Err(AppError::Conflict(
                        "the node changed since this action was prepared".to_string(),
                    ));
                }
                return Err(AppError::NodeNotFound("Node not found".to_string()));
            }

            db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                .update_many(
                    doc! { "node_id": &node_id, "is_active": true },
                    doc! { "$set": { "is_active": false, "updated_at": &now } },
                )
                .session(&mut *session)
                .await?;
            db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
                .delete_many(doc! { "node_id": &node_id })
                .session(&mut *session)
                .await?;
            crate::services::service_history::collection::<UserService>(&db, USER_SERVICES)
                .update_many(
                    doc! { "node_id": &node_id, "is_active": true },
                    doc! {
                        "$unset": { "node_id": "" },
                        "$set": { "updated_at": &now },
                    },
                )
                .session(&mut *session)
                .await?;
            if let (Some(audit), Some(owner)) = (audit.as_ref(), audit_owner) {
                audit
                    .record(
                        &db,
                        session,
                        owner,
                        "node_deleted",
                        &node_id,
                        AuditTier::Narrowing,
                    )
                    .await?;
            }
            Ok(())
        }
        .await;
        crate::services::api_key_mutation_service::transaction_result(operation)
    })
    .await
    .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;

    tracing::info!(node_id = %tracing_node_id, "Node deleted");
    Ok(())
}

/// Validate NodeMetadata string field lengths (M4).
fn validate_node_metadata(meta: &NodeMetadata) -> AppResult<()> {
    const MAX_METADATA_FIELD_LEN: usize = 64;
    if meta
        .agent_version
        .as_ref()
        .is_some_and(|v| v.len() > MAX_METADATA_FIELD_LEN)
    {
        return Err(AppError::ValidationError(
            "agent_version must be 64 characters or fewer".to_string(),
        ));
    }
    if meta
        .os
        .as_ref()
        .is_some_and(|v| v.len() > MAX_METADATA_FIELD_LEN)
    {
        return Err(AppError::ValidationError(
            "os must be 64 characters or fewer".to_string(),
        ));
    }
    if meta
        .arch
        .as_ref()
        .is_some_and(|v| v.len() > MAX_METADATA_FIELD_LEN)
    {
        return Err(AppError::ValidationError(
            "arch must be 64 characters or fewer".to_string(),
        ));
    }
    if let Some(ref ip) = meta.ip_address
        && ip.parse::<std::net::IpAddr>().is_err()
    {
        return Err(AppError::ValidationError(
            "Invalid IP address format".to_string(),
        ));
    }
    Ok(())
}

fn device_node_name(label: &str, node_id: &str) -> String {
    let suffix: String = node_id.chars().take(8).collect();
    let max_base_len = 64_usize.saturating_sub(suffix.len() + 1);
    let mut base = String::with_capacity(max_base_len);
    let mut last_dash = false;

    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            if base.len() == max_base_len {
                break;
            }
            base.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !base.is_empty() {
            if base.len() == max_base_len {
                break;
            }
            base.push('-');
            last_dash = true;
        }
    }

    let base = base.trim_matches('-');
    let base = if base.is_empty() { "device" } else { base };
    format!("{base}-{suffix}")
}

/// Update last_heartbeat_at and optionally metadata.
pub async fn update_heartbeat(
    db: &mongodb::Database,
    node_id: &str,
    metadata: Option<NodeMetadata>,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let mut update = doc! {
        "$set": {
            "last_heartbeat_at": &now,
            "updated_at": &now,
        },
    };

    if let Some(ref meta) = metadata {
        validate_node_metadata(meta)?;
    }

    if let Some(meta) = metadata {
        let meta_doc = bson::to_document(&meta)
            .map_err(|e| AppError::Internal(format!("Failed to serialize metadata: {e}")))?;
        update
            .get_document_mut("$set")
            .unwrap()
            .insert("metadata", meta_doc);
    }

    db.collection::<Node>(NODES)
        .update_one(doc! { "_id": node_id, "is_active": true }, update)
        .await?;

    Ok(())
}

/// Persist the agent version advertised by a node's WebSocket `status_update`.
/// Existing metadata fields are preserved so the status handshake cannot erase
/// registration-time OS/arch/IP/provisioning metadata.
pub async fn update_node_agent_version(
    db: &mongodb::Database,
    node_id: &str,
    agent_version: &str,
) -> AppResult<()> {
    let agent_version = agent_version.trim();
    if agent_version.is_empty() {
        return Ok(());
    }

    let node = get_node_by_id(db, node_id).await?.ok_or_else(|| {
        AppError::NodeNotFound(format!("Node {node_id} not found during status update"))
    })?;
    let mut metadata = node.metadata.unwrap_or(NodeMetadata {
        agent_version: None,
        os: None,
        arch: None,
        ip_address: None,
        provisioning_source: None,
    });
    if metadata.agent_version.as_deref() == Some(agent_version) {
        update_heartbeat(db, node_id, None).await?;
        return Ok(());
    }
    metadata.agent_version = Some(agent_version.to_string());
    update_heartbeat(db, node_id, Some(metadata)).await
}

/// Set node status.
pub async fn set_node_status(
    db: &mongodb::Database,
    node_id: &str,
    status: NodeStatus,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let mut update_fields = doc! {
        "status": status.as_str(),
        "updated_at": &now,
    };

    if status == NodeStatus::Online {
        update_fields.insert("connected_at", now);
        update_fields.insert("last_heartbeat_at", now);
    }

    db.collection::<Node>(NODES)
        .update_one(
            doc! { "_id": node_id, "is_active": true },
            doc! { "$set": update_fields },
        )
        .await?;

    Ok(())
}

/// Rotate the node's auth token and signing secret. Invalidates old values immediately.
/// Returns (raw_auth_token, raw_signing_secret).
pub async fn rotate_auth_token(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    node_id: &str,
) -> AppResult<(String, String)> {
    super::remote_machine_service::guard_legacy_node_mutation(db, node_id, "rotate").await?;
    let _node = ensure_node_writable_by_actor(db, user_id, node_id).await?;

    let raw_token = format!("nyx_nauth_{}", hex::encode(rand::random::<[u8; 32]>()));
    let token_hash = hash_token(&raw_token);
    let raw_signing_secret = hex::encode(rand::random::<[u8; 32]>());
    let signing_secret_encrypted = encryption_keys
        .encrypt(raw_signing_secret.as_bytes())
        .await?;
    let signing_secret_hash = hash_token(&raw_signing_secret);
    let now = bson::DateTime::from_chrono(Utc::now());

    let result = db
        .collection::<Node>(NODES)
        .update_one(
            doc! { "_id": node_id, "is_active": true },
            doc! { "$set": {
                "auth_token_hash": &token_hash,
                "signing_secret_encrypted": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: signing_secret_encrypted,
                },
                "signing_secret_hash": &signing_secret_hash,
                "updated_at": &now,
            }, "$inc": {
                "access_revision": 1_i64,
                "state_version": 1_i64,
            } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NodeNotFound("Node not found".to_string()));
    }

    tracing::info!(node_id = %node_id, "Node auth token and signing secret rotated");
    Ok((raw_token, raw_signing_secret))
}

/// Prepared using the same Node credential format as ordinary registration.
/// Only managed bootstrap may activate it, in its lifecycle-fenced transaction.
pub(crate) struct ManagedNodeCredentialMaterial {
    pub auth_token: Zeroizing<String>,
    pub signing_secret: Zeroizing<String>,
    pub signing_secret_encrypted: Vec<u8>,
}

pub(crate) async fn prepare_managed_credentials(
    encryption_keys: &EncryptionKeys,
) -> AppResult<ManagedNodeCredentialMaterial> {
    let auth_token = Zeroizing::new(format!(
        "nyx_nauth_{}",
        hex::encode(rand::random::<[u8; 32]>())
    ));
    let signing_secret = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
    let signing_secret_encrypted = encryption_keys.encrypt(signing_secret.as_bytes()).await?;
    Ok(ManagedNodeCredentialMaterial {
        auth_token,
        signing_secret,
        signing_secret_encrypted,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn activate_managed_credentials(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    node_id: &str,
    owner_id: &str,
    auth_hash: &str,
    signing_hash: &str,
    signing_secret_encrypted: &[u8],
    now: DateTime<Utc>,
) -> AppResult<()> {
    let updated = db.collection::<Node>(NODES).update_one(
        doc! {"_id": node_id, "user_id": owner_id, "metadata.provisioning_source": "remote_machine"},
        doc! {"$set": {
            "auth_token_hash": auth_hash, "signing_secret_hash": signing_hash,
            "signing_secret_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: signing_secret_encrypted.to_vec() },
            "is_active": true, "status": NodeStatus::Offline.as_str(),
            "connection_owner": bson::Bson::Null, "updated_at": bson::DateTime::from_chrono(now),
        }, "$inc": {"access_revision": 1_i64, "state_version": 1_i64}},
    ).session(session).await?;
    if updated.modified_count != 1 {
        return Err(AppError::Conflict(
            "managed_node_identity_unavailable".into(),
        ));
    }
    Ok(())
}

/// Admin: list active operational nodes (no user filter).
///
/// Device-code provisioning creates node-shaped ownership records, but those
/// devices authenticate with API keys and are not operational node WebSocket
/// agents, so the operational admin list excludes them.
pub async fn list_all_nodes(
    db: &mongodb::Database,
    page: u64,
    per_page: u64,
    status_filter: Option<&str>,
    user_id_filter: Option<&str>,
) -> AppResult<(Vec<Node>, u64)> {
    let mut filter = doc! {
        "is_active": true,
        "metadata.provisioning_source": { "$nin": DEVICE_PROVISIONING_SOURCES },
    };
    if let Some(status) = status_filter {
        filter.insert("status", status);
    }
    if let Some(uid) = user_id_filter {
        filter.insert("user_id", uid);
    }

    let total = db
        .collection::<Node>(NODES)
        .count_documents(filter.clone())
        .await?;

    let offset = (page - 1) * per_page;
    let nodes: Vec<Node> = db
        .collection::<Node>(NODES)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    Ok((nodes, total))
}

/// Validate a raw auth token. Returns the Node if valid.
pub async fn validate_auth_token(db: &mongodb::Database, raw_token: &str) -> AppResult<Node> {
    let token_hash = hash_token(raw_token);

    db.collection::<Node>(NODES)
        .find_one(doc! { "auth_token_hash": &token_hash, "is_active": true })
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid node auth token".to_string()))
}

fn decode_node_signing_secret(
    secret_hex_bytes: &[u8],
    node_id: &str,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let secret_hex = std::str::from_utf8(secret_hex_bytes).map_err(|e| {
        AppError::Internal(format!(
            "Node signing secret for {node_id} is not valid UTF-8: {e}"
        ))
    })?;

    let secret = hex::decode(secret_hex).map_err(|e| {
        AppError::Internal(format!(
            "Node signing secret for {node_id} is not valid hex: {e}"
        ))
    })?;

    Ok(Zeroizing::new(secret))
}

// --- Binding operations ---

/// Create a service binding for a node.
/// Auto-sync `NodeServiceBinding` when `UserService.node_id` changes.
///
/// Call this after creating or updating a `UserService` with a node routing change.
/// Creates a binding when `node_id` is set, deactivates the old one when it changes.
///
/// `user_id` is the *binding owner* -- for org-shared services this is the
/// org's user_id, so that proxy-time routing (which queries bindings by the
/// effective service owner) finds it. `actor_user_id` is the human (or API
/// key) that actually owns the node and is performing the operation; it is
/// used to validate that the actor is allowed to bind this node. Both are
/// the same value for personal services. For org services they differ:
/// the actor is an org admin and the node is their personal node.
pub async fn sync_node_binding_for_user_service(
    db: &mongodb::Database,
    user_id: &str,
    actor_user_id: &str,
    catalog_service_id: Option<&str>,
    new_node_id: Option<&str>,
    old_node_id: Option<&str>,
) -> AppResult<()> {
    let Some(service_id) = catalog_service_id else {
        return Ok(());
    };

    // Validate the new node before mutating bindings so an invalid update does not
    // tear down the previous route. The node is owned by the *actor*, not the
    // binding owner -- a personal node may be referenced by an org-owned service
    // when the actor is an admin of that org. Use the actor-based access check
    // so the actor's org admin role on the node owner is honored.
    if let Some(new_nid) = new_node_id.filter(|nid| !nid.is_empty()) {
        ensure_node_writable_by_actor(db, actor_user_id, new_nid).await?;
    }

    // Deactivate old binding if the node changed or was cleared.
    if let Some(old_nid) = old_node_id {
        let changed = match new_node_id {
            Some(new_nid) if !new_nid.is_empty() => new_nid != old_nid,
            _ => true, // cleared
        };
        if changed && !has_active_user_service_for_node(db, user_id, service_id, old_nid).await? {
            deactivate_binding_by_node_and_service(db, user_id, old_nid, service_id).await?;
        }
    }

    // Create binding if new node_id is set.
    if let Some(new_nid) = new_node_id.filter(|nid| !nid.is_empty()) {
        ensure_binding_exists(db, user_id, new_nid, service_id).await?;
    }

    Ok(())
}

async fn has_active_user_service_for_node(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    node_id: &str,
) -> AppResult<bool> {
    let count =
        crate::services::service_history::collection::<mongodb::bson::Document>(db, USER_SERVICES)
            .count_documents(doc! {
                "user_id": user_id,
                "catalog_service_id": service_id,
                "node_id": node_id,
                "is_active": true,
            })
            .await?;

    Ok(count > 0)
}

/// Create a `NodeServiceBinding` if one does not already exist for this node + service.
/// Uses insert-first with duplicate-key handling to avoid race conditions.
async fn ensure_binding_exists(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let existing = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find_one(doc! {
            "node_id": node_id,
            "service_id": service_id,
            "user_id": user_id,
            "is_active": true,
        })
        .await?;

    if existing.is_some() {
        return Ok(());
    }

    let now = Utc::now();
    let binding = NodeServiceBinding {
        id: uuid::Uuid::new_v4().to_string(),
        node_id: node_id.to_string(),
        user_id: user_id.to_string(),
        service_id: service_id.to_string(),
        is_active: true,
        priority: 0,
        created_at: now,
        updated_at: now,
    };

    match db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .insert_one(&binding)
        .await
    {
        Ok(_) => {
            tracing::info!(
                binding_id = %binding.id,
                node_id = %node_id,
                service_id = %service_id,
                "Auto-created node service binding from UserService.node_id"
            );
        }
        Err(e) => {
            // Duplicate key error (E11000) means a concurrent request already created the
            // binding -- treat as success (idempotent).
            let is_dup = e.kind.as_ref().to_string().contains("E11000");
            if !is_dup {
                return Err(e.into());
            }
        }
    }

    Ok(())
}

/// Deactivate a binding by node + service (not by binding ID).
async fn deactivate_binding_by_node_and_service(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let result = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .update_many(
            doc! {
                "node_id": node_id,
                "service_id": service_id,
                "user_id": user_id,
                "is_active": true,
            },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?;

    if result.modified_count > 0 {
        tracing::info!(
            node_id = %node_id,
            service_id = %service_id,
            count = result.modified_count,
            "Auto-deactivated node service binding(s) from UserService.node_id change"
        );
    }

    Ok(())
}

/// Fixture seeding only. All serving binding mutations use NodeMutationAudit.
#[cfg(test)]
pub async fn create_binding(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    service_id: &str,
) -> AppResult<NodeServiceBinding> {
    let node = ensure_node_writable_by_actor(db, user_id, node_id).await?;

    // Check for existing binding (same node + service)
    let existing = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find_one(doc! {
            "node_id": node_id,
            "service_id": service_id,
            "user_id": &node.user_id,
            "is_active": true,
        })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "Binding already exists for this node and service".to_string(),
        ));
    }

    let now = Utc::now();
    let binding = NodeServiceBinding {
        id: uuid::Uuid::new_v4().to_string(),
        node_id: node_id.to_string(),
        user_id: node.user_id.clone(),
        service_id: service_id.to_string(),
        is_active: true,
        priority: 0,
        created_at: now,
        updated_at: now,
    };

    db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .insert_one(&binding)
        .await?;

    tracing::info!(
        binding_id = %binding.id,
        node_id = %node_id,
        service_id = %service_id,
        "Node service binding created"
    );

    Ok(binding)
}

/// List all active bindings for a node.
pub async fn list_bindings(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
) -> AppResult<Vec<NodeServiceBinding>> {
    let node = get_node(db, user_id, node_id).await?;

    let bindings: Vec<NodeServiceBinding> = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find(doc! { "node_id": node_id, "user_id": &node.user_id, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    Ok(bindings)
}

async fn load_active_node(db: &mongodb::Database, node_id: &str) -> AppResult<Node> {
    db.collection::<Node>(NODES)
        .find_one(doc! { "_id": node_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NodeNotFound("Node not found".to_string()))
}

async fn ensure_node_readable_by_access(
    db: &mongodb::Database,
    actor_user_id: &str,
    node: &Node,
) -> AppResult<OwnerAccess> {
    let access = org_service::resolve_owner_access(db, actor_user_id, &node.user_id).await?;
    if node_access_can_read(&access) {
        Ok(access)
    } else {
        Err(AppError::NodeNotFound("Node not found".to_string()))
    }
}

async fn ensure_node_writable_by_access(
    db: &mongodb::Database,
    actor_user_id: &str,
    node: &Node,
) -> AppResult<OwnerAccess> {
    let access = org_service::resolve_owner_access(db, actor_user_id, &node.user_id).await?;
    if access.can_write() {
        Ok(access)
    } else {
        Err(AppError::NodeNotFound("Node not found".to_string()))
    }
}

pub(crate) fn node_access_can_read(access: &OwnerAccess) -> bool {
    match access {
        OwnerAccess::Direct | OwnerAccess::AsOrgAdmin { .. } => true,
        OwnerAccess::AsOrgMember { role, .. } => matches!(role, OrgRole::Member),
        OwnerAccess::Forbidden => false,
    }
}

async fn attach_owner_info(
    db: &mongodb::Database,
    nodes: Vec<Node>,
) -> AppResult<Vec<NodeWithOwner>> {
    if nodes.is_empty() {
        return Ok(vec![]);
    }

    let mut owner_ids = Vec::<String>::new();
    for node in &nodes {
        if !owner_ids.iter().any(|id| id == &node.user_id) {
            owner_ids.push(node.user_id.clone());
        }
    }

    let owner_id_array: bson::Array = owner_ids
        .iter()
        .map(|id| bson::Bson::String(id.clone()))
        .collect();
    let owners: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": owner_id_array } })
        .await?
        .try_collect()
        .await?;
    let owner_map: HashMap<String, User> = owners.into_iter().map(|u| (u.id.clone(), u)).collect();

    Ok(nodes
        .into_iter()
        .map(|node| {
            let owner = owner_info_from_user_id(&node.user_id, owner_map.get(&node.user_id));
            NodeWithOwner { node, owner }
        })
        .collect())
}

pub async fn owner_info_for_node(db: &mongodb::Database, node: &Node) -> AppResult<NodeOwnerInfo> {
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &node.user_id })
        .await?;
    Ok(owner_info_from_user_id(&node.user_id, owner.as_ref()))
}

pub async fn list_node_admins(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
) -> AppResult<Vec<NodeAdminInfo>> {
    let node = get_node(db, actor_user_id, node_id).await?;
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &node.user_id })
        .await?;

    if !matches!(owner.as_ref().map(|u| u.user_type), Some(UserType::Org)) {
        return Ok(vec![admin_info_from_user(
            &node.user_id,
            owner.as_ref(),
            NodeAdminRole::Owner,
        )]);
    }

    let memberships = org_service::list_members_for_org(db, &node.user_id, false).await?;
    let admin_user_ids: Vec<String> = memberships
        .into_iter()
        .filter(|membership| matches!(membership.role, OrgRole::Admin))
        .map(|membership| membership.member_user_id)
        .collect();
    if admin_user_ids.is_empty() {
        return Ok(vec![]);
    }

    let admin_id_array: bson::Array = admin_user_ids
        .iter()
        .map(|id| bson::Bson::String(id.clone()))
        .collect();
    let users: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": admin_id_array } })
        .await?
        .try_collect()
        .await?;
    let user_map: HashMap<String, User> = users.into_iter().map(|u| (u.id.clone(), u)).collect();

    Ok(admin_user_ids
        .into_iter()
        .map(|user_id| admin_info_from_user(&user_id, user_map.get(&user_id), NodeAdminRole::Admin))
        .collect())
}

fn owner_info_from_user_id(owner_id: &str, owner: Option<&User>) -> NodeOwnerInfo {
    let kind = match owner.map(|u| u.user_type) {
        Some(UserType::Org) => NodeOwnerKind::Org,
        _ => NodeOwnerKind::User,
    };
    let display_name = owner
        .and_then(|u| u.display_name.clone())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            owner
                .map(|u| u.email.clone())
                .filter(|email| !email.trim().is_empty())
        })
        .unwrap_or_else(|| owner_id.to_string());

    NodeOwnerInfo {
        kind,
        id: owner_id.to_string(),
        display_name,
    }
}

fn admin_info_from_user(user_id: &str, user: Option<&User>, role: NodeAdminRole) -> NodeAdminInfo {
    NodeAdminInfo {
        user_id: user_id.to_string(),
        display_name: user.and_then(|u| u.display_name.clone()),
        email: user.map(|u| u.email.clone()),
        role,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::NodeMetrics;
    use crate::models::node_service_binding::COLLECTION_NAME as NODE_SERVICE_BINDINGS;
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::test_utils::{
        connect_test_database, test_encryption_keys, test_membership, test_user,
    };
    use uuid::Uuid;

    #[derive(Clone, Copy, Debug)]
    enum AccessCase {
        Direct,
        AsOrgAdmin,
        AsOrgMember,
        NoAccess,
    }

    impl AccessCase {
        fn all() -> [Self; 4] {
            [
                Self::Direct,
                Self::AsOrgAdmin,
                Self::AsOrgMember,
                Self::NoAccess,
            ]
        }

        fn label(self) -> &'static str {
            match self {
                Self::Direct => "direct",
                Self::AsOrgAdmin => "org_admin",
                Self::AsOrgMember => "org_member",
                Self::NoAccess => "no_access",
            }
        }

        fn can_read(self) -> bool {
            matches!(self, Self::Direct | Self::AsOrgAdmin | Self::AsOrgMember)
        }

        fn can_write(self) -> bool {
            matches!(self, Self::Direct | Self::AsOrgAdmin)
        }
    }

    #[test]
    fn device_node_name_sanitizes_and_keeps_uuid_suffix() {
        let node_id = "12345678-aaaa-bbbb-cccc-123456789012";

        assert_eq!(
            device_node_name("Kitchen Camera 01", node_id),
            "kitchen-camera-01-12345678"
        );
        assert_eq!(device_node_name("!!!", node_id), "device-12345678");
        assert!(device_node_name(&"A".repeat(200), node_id).len() <= 64);
    }

    #[tokio::test]
    async fn create_for_device_creates_device_node_with_valid_auth_material() {
        let Some(db) = connect_test_database("node_device_code_stub").await else {
            return;
        };
        let owner_id = Uuid::new_v4().to_string();

        let node = create_for_device(
            &db,
            &test_encryption_keys(),
            DeviceNodeInput {
                user_id: &owner_id,
                api_key_id: "api-key-1",
                hw_id: "esp32-p4-cam",
                label: "Kitchen Camera",
                device_pubkey: Some(&[7u8; 32]),
                provisioning_source: DEVICE_CODE_PROVISIONING_SOURCE,
            },
        )
        .await
        .expect("create device stub");

        assert_eq!(node.auth_token_hash.len(), 64);
        assert_ne!(node.auth_token_hash, hash_token(""));
        assert_eq!(node.signing_secret_hash.len(), 64);
        assert_ne!(node.signing_secret_hash, hash_token(""));
        assert!(node.signing_secret_encrypted.is_some());
        assert_eq!(
            node.metadata
                .as_ref()
                .and_then(|metadata| metadata.provisioning_source.as_deref()),
            Some(DEVICE_CODE_PROVISIONING_SOURCE)
        );
    }

    #[tokio::test]
    async fn list_all_nodes_excludes_device_code_stubs_from_operational_view() {
        let Some(db) = connect_test_database("node_device_code_stub_admin_list").await else {
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        let operational_node = make_node(&owner_id, "operational-node");
        db.collection::<Node>(NODES)
            .insert_one(&operational_node)
            .await
            .expect("insert operational node");
        create_for_device(
            &db,
            &test_encryption_keys(),
            DeviceNodeInput {
                user_id: &owner_id,
                api_key_id: "api-key-1",
                hw_id: "esp32-p4-cam",
                label: "Kitchen Camera",
                device_pubkey: Some(&[7u8; 32]),
                provisioning_source: DEVICE_CODE_PROVISIONING_SOURCE,
            },
        )
        .await
        .expect("create device stub");

        let (nodes, total) = list_all_nodes(&db, 1, 50, None, None)
            .await
            .expect("list nodes");

        assert_eq!(total, 1);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, operational_node.id);
    }

    #[tokio::test]
    async fn update_node_agent_version_preserves_existing_metadata() {
        let Some(db) = connect_test_database("node_agent_version_metadata").await else {
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        let mut node = make_node(&owner_id, "versioned-node");
        node.metadata = Some(NodeMetadata {
            agent_version: None,
            os: Some("linux".to_string()),
            arch: Some("x86_64".to_string()),
            ip_address: Some("127.0.0.1".to_string()),
            provisioning_source: Some("manual".to_string()),
        });
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");

        update_node_agent_version(&db, &node.id, "0.7.1")
            .await
            .expect("update agent version");

        let stored = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node.id })
            .await
            .unwrap()
            .expect("node exists");
        let metadata = stored.metadata.expect("metadata exists");
        assert_eq!(metadata.agent_version.as_deref(), Some("0.7.1"));
        assert_eq!(metadata.os.as_deref(), Some("linux"));
        assert_eq!(metadata.arch.as_deref(), Some("x86_64"));
        assert_eq!(metadata.ip_address.as_deref(), Some("127.0.0.1"));
        assert_eq!(metadata.provisioning_source.as_deref(), Some("manual"));
        assert!(stored.last_heartbeat_at.is_some());
    }

    struct NodeAclFixture {
        actor_id: String,
        owner_id: String,
        node: Node,
        binding: NodeServiceBinding,
    }

    fn make_node(owner_id: &str, name: &str) -> Node {
        let now = Utc::now();
        Node {
            machine: None,
            machine_confirm: Default::default(),
            allow_single_user_saved_logins: false,
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            name: name.to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: "auth-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            connection_owner: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_binding(owner_id: &str, node_id: &str, service_id: &str) -> NodeServiceBinding {
        let now = Utc::now();
        NodeServiceBinding {
            id: Uuid::new_v4().to_string(),
            node_id: node_id.to_string(),
            user_id: owner_id.to_string(),
            service_id: service_id.to_string(),
            is_active: true,
            priority: 0,
            created_at: now,
            updated_at: now,
        }
    }

    async fn seed_acl_fixture(db: &mongodb::Database, case: AccessCase) -> NodeAclFixture {
        let actor_id = Uuid::new_v4().to_string();
        let owner_id = match case {
            AccessCase::Direct => actor_id.clone(),
            AccessCase::AsOrgAdmin | AccessCase::AsOrgMember | AccessCase::NoAccess => {
                Uuid::new_v4().to_string()
            }
        };
        let service_id = Uuid::new_v4().to_string();
        let node = make_node(&owner_id, &format!("node-{}", case.label()));
        let binding = make_binding(&owner_id, &node.id, &service_id);

        let mut users = vec![test_user(&actor_id, UserType::Person)];
        if owner_id != actor_id {
            users.push(test_user(&owner_id, UserType::Org));
        }
        db.collection::<User>(USERS)
            .insert_many(users)
            .await
            .expect("insert users");

        match case {
            AccessCase::AsOrgAdmin => {
                db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
                    .insert_one(test_membership(&owner_id, &actor_id, OrgRole::Admin, None))
                    .await
                    .expect("insert admin membership");
            }
            AccessCase::AsOrgMember => {
                db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
                    .insert_one(test_membership(&owner_id, &actor_id, OrgRole::Member, None))
                    .await
                    .expect("insert member membership");
            }
            AccessCase::Direct | AccessCase::NoAccess => {}
        }

        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(binding.clone())
            .await
            .expect("insert binding");

        NodeAclFixture {
            actor_id,
            owner_id,
            node,
            binding,
        }
    }

    fn assert_node_not_found<T: std::fmt::Debug>(result: AppResult<T>, case: AccessCase) {
        let err = result.expect_err("operation should fail");
        assert!(
            matches!(err, AppError::NodeNotFound(_)),
            "case {case:?} returned unexpected error: {err}"
        );
    }

    #[test]
    fn decodes_node_signing_secret_hex() {
        let secret = decode_node_signing_secret(b"616263", "node-1").expect("valid secret");
        assert_eq!(secret.as_slice(), b"abc");
    }

    #[test]
    fn rejects_node_signing_secret_invalid_utf8() {
        let error = decode_node_signing_secret(&[0xff], "node-1").expect_err("invalid utf8");
        assert!(error.to_string().contains("not valid UTF-8"));
    }

    #[test]
    fn rejects_node_signing_secret_invalid_hex() {
        let error = decode_node_signing_secret(b"not-hex", "node-1").expect_err("invalid hex");
        assert!(error.to_string().contains("not valid hex"));
    }

    #[tokio::test]
    async fn get_node_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_get_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result = get_node(&db, &fixture.actor_id, &fixture.node.id).await;
            if case.can_read() {
                let node = result.expect("readable case should load node");
                assert_eq!(node.id, fixture.node.id, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn list_bindings_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_list_bindings_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result = list_bindings(&db, &fixture.actor_id, &fixture.node.id).await;
            if case.can_read() {
                let bindings = result.expect("readable case should list bindings");
                assert_eq!(bindings.len(), 1, "case {case:?}");
                assert_eq!(bindings[0].id, fixture.binding.id, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn delete_node_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_delete_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result = delete_node(&db, &fixture.actor_id, &fixture.node.id).await;
            if case.can_write() {
                result.expect("writable case should delete node");
                let stored = get_node_by_id(&db, &fixture.node.id)
                    .await
                    .expect("query node");
                assert!(stored.is_none(), "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn delete_node_with_expected_state_version_rejects_stale_mutation_atomically() {
        let Some(db) = connect_test_database("node_delete_expected_version_atomic").await else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert actor");
        let node = make_node(&actor_id, "atomic-delete");
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");
        db.collection::<mongodb::bson::Document>(NODES)
            .update_one(
                doc! { "_id": &node.id },
                doc! { "$set": { "state_version": 2_i64 } },
            )
            .await
            .expect("bump state version");

        let error = delete_node_with_expected_state_version(&db, &actor_id, &node.id, Some(1))
            .await
            .expect_err("stale delete must conflict");
        assert!(matches!(error, AppError::Conflict(_)));
        let stored = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node.id })
            .await
            .expect("load node")
            .expect("node exists");
        assert!(stored.is_active, "stale fence must leave node active");
    }

    #[tokio::test]
    async fn transfer_node_with_expected_state_version_rejects_stale_mutation_atomically() {
        let Some(db) = connect_test_database("node_transfer_expected_version_atomic").await else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        let destination_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&destination_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(
                &destination_id,
                &actor_id,
                OrgRole::Admin,
                None,
            ))
            .await
            .expect("insert destination membership");
        let node = make_node(&actor_id, "atomic-transfer");
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");
        db.collection::<mongodb::bson::Document>(NODES)
            .update_one(
                doc! { "_id": &node.id },
                doc! { "$set": { "state_version": 2_i64 } },
            )
            .await
            .expect("bump state version");

        let error = transfer_node_owner_with_expected_state_version(
            &db,
            &actor_id,
            &node.id,
            &destination_id,
            10,
            Some(1),
        )
        .await
        .expect_err("stale transfer must conflict");
        assert!(matches!(error, AppError::Conflict(_)));
        let stored = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node.id })
            .await
            .expect("load node")
            .expect("node exists");
        assert_eq!(stored.user_id, actor_id, "stale fence must preserve owner");
        assert!(stored.is_active);
    }

    #[tokio::test]
    async fn rotate_auth_token_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_rotate_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result =
                rotate_auth_token(&db, &encryption_keys, &fixture.actor_id, &fixture.node.id).await;
            if case.can_write() {
                let (auth_token, signing_secret) =
                    result.expect("writable case should rotate credentials");
                assert!(auth_token.starts_with("nyx_nauth_"), "case {case:?}");
                assert_eq!(signing_secret.len(), 64, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn update_binding_priority_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_update_binding_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let audit = NodeMutationAudit::new(&node_audit_runtime(false), &fixture.actor_id);
            let result = audit
                .update_binding_priority(&db, &fixture.node.id, &fixture.binding.id, 5)
                .await;
            if case.can_write() {
                result.expect("writable case should update binding");
                let stored = db
                    .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                    .find_one(doc! { "_id": &fixture.binding.id })
                    .await
                    .expect("query binding")
                    .expect("binding exists");
                assert_eq!(stored.priority, 5, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn create_binding_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_create_binding_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let service_id = Uuid::new_v4().to_string();
            db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
                .insert_one(doc! {"_id": &service_id, "is_active": true})
                .await
                .unwrap();
            let audit = NodeMutationAudit::new(&node_audit_runtime(false), &fixture.actor_id);
            let result = audit
                .create_binding(&db, &fixture.node.id, &service_id)
                .await;
            if case.can_write() {
                let binding = result.expect("writable case should create binding");
                assert_eq!(binding.user_id, fixture.owner_id, "case {case:?}");
                assert_eq!(binding.node_id, fixture.node.id, "case {case:?}");
                assert_eq!(binding.service_id, service_id, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn delete_binding_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_delete_binding_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let audit = NodeMutationAudit::new(&node_audit_runtime(false), &fixture.actor_id);
            let result = audit
                .delete_binding(&db, &fixture.node.id, &fixture.binding.id)
                .await;
            if case.can_write() {
                result.expect("writable case should delete binding");
                let stored = db
                    .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                    .find_one(doc! { "_id": &fixture.binding.id })
                    .await
                    .expect("query binding")
                    .expect("binding exists");
                assert!(!stored.is_active, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn list_user_nodes_includes_personal_and_member_org_nodes_with_owner_metadata() {
        let Some(db) = connect_test_database("node_list_owner_info").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        let viewer_org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
                test_user(&viewer_org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_id, &actor_id, OrgRole::Member, None),
                test_membership(&viewer_org_id, &actor_id, OrgRole::Viewer, None),
            ])
            .await
            .expect("insert memberships");

        let personal_node = make_node(&actor_id, "personal-node");
        let org_node = make_node(&org_id, "org-node");
        let viewer_org_node = make_node(&viewer_org_id, "viewer-org-node");
        db.collection::<Node>(NODES)
            .insert_many([
                personal_node.clone(),
                org_node.clone(),
                viewer_org_node.clone(),
            ])
            .await
            .expect("insert nodes");

        let listed = list_user_nodes(&db, &actor_id)
            .await
            .expect("list reachable nodes");
        let ids: Vec<&str> = listed.iter().map(|entry| entry.node.id.as_str()).collect();
        assert!(ids.contains(&personal_node.id.as_str()));
        assert!(ids.contains(&org_node.id.as_str()));
        assert!(!ids.contains(&viewer_org_node.id.as_str()));

        let personal = listed
            .iter()
            .find(|entry| entry.node.id == personal_node.id)
            .expect("personal node listed");
        assert_eq!(personal.owner.kind, NodeOwnerKind::User);
        assert_eq!(personal.owner.id, actor_id);
        assert_eq!(personal.owner.display_name, "Test User");

        let org = listed
            .iter()
            .find(|entry| entry.node.id == org_node.id)
            .expect("org node listed");
        assert_eq!(org.owner.kind, NodeOwnerKind::Org);
        assert_eq!(org.owner.id, org_id);
        assert_eq!(org.owner.display_name, "Test Org");
    }

    #[tokio::test]
    async fn personal_user_node_access_remains_unchanged() {
        let Some(db) = connect_test_database("node_personal_regression").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&other_id, UserType::Person),
            ])
            .await
            .expect("insert users");
        let owned = make_node(&actor_id, "owned-node");
        let other = make_node(&other_id, "other-node");
        db.collection::<Node>(NODES)
            .insert_many([owned.clone(), other.clone()])
            .await
            .expect("insert nodes");

        let loaded = get_node(&db, &actor_id, &owned.id)
            .await
            .expect("owner can get personal node");
        assert_eq!(loaded.id, owned.id);
        assert_node_not_found(
            get_node(&db, &actor_id, &other.id).await,
            AccessCase::NoAccess,
        );

        let listed = list_user_nodes(&db, &actor_id)
            .await
            .expect("list personal nodes");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].node.id, owned.id);
        assert_eq!(listed[0].owner.kind, NodeOwnerKind::User);
        assert_eq!(listed[0].owner.id, actor_id);
    }

    // ────────────────────────────────────────────────────────────────────
    // Pure-function tests (no MongoDB required)
    // ────────────────────────────────────────────────────────────────────

    // --- validate_node_metadata ---

    #[test]
    fn validate_node_metadata_accepts_all_none_fields() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("all-None metadata should be valid");
    }

    #[test]
    fn validate_node_metadata_accepts_valid_fields() {
        let meta = NodeMetadata {
            agent_version: Some("1.2.3".to_string()),
            os: Some("linux".to_string()),
            arch: Some("x86_64".to_string()),
            ip_address: Some("192.168.1.1".to_string()),
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("valid metadata should pass");
    }

    #[test]
    fn validate_node_metadata_accepts_ipv6_address() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: Some("::1".to_string()),
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("IPv6 loopback should be valid");
    }

    #[test]
    fn validate_node_metadata_accepts_full_ipv6() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: Some("2001:0db8:85a3:0000:0000:8a2e:0370:7334".to_string()),
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("full IPv6 should be valid");
    }

    #[test]
    fn validate_node_metadata_rejects_overlong_agent_version() {
        let meta = NodeMetadata {
            agent_version: Some("v".repeat(65)),
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("agent_version too long");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("agent_version")));
    }

    #[test]
    fn validate_node_metadata_accepts_exactly_64_char_agent_version() {
        let meta = NodeMetadata {
            agent_version: Some("v".repeat(64)),
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("64-char agent_version should pass");
    }

    #[test]
    fn validate_node_metadata_rejects_overlong_os() {
        let meta = NodeMetadata {
            agent_version: None,
            os: Some("x".repeat(65)),
            arch: None,
            ip_address: None,
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("os too long");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("os")));
    }

    #[test]
    fn validate_node_metadata_rejects_overlong_arch() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: Some("a".repeat(65)),
            ip_address: None,
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("arch too long");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("arch")));
    }

    #[test]
    fn validate_node_metadata_rejects_invalid_ip_address() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: Some("not-an-ip".to_string()),
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("invalid IP");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("Invalid IP")));
    }

    #[test]
    fn validate_node_metadata_rejects_empty_ip_address() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: Some(String::new()),
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("empty IP");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("Invalid IP")));
    }

    // --- node_access_can_read ---

    #[test]
    fn node_access_can_read_allows_direct() {
        assert!(node_access_can_read(&OwnerAccess::Direct));
    }

    #[test]
    fn node_access_can_read_allows_org_admin() {
        let access = OwnerAccess::AsOrgAdmin {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            allowed_service_ids: None,
        };
        assert!(node_access_can_read(&access));
    }

    #[test]
    fn node_access_can_read_allows_org_member() {
        let access = OwnerAccess::AsOrgMember {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            role: OrgRole::Member,
            allowed_service_ids: None,
        };
        assert!(node_access_can_read(&access));
    }

    #[test]
    fn node_access_can_read_denies_org_viewer() {
        let access = OwnerAccess::AsOrgMember {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            role: OrgRole::Viewer,
            allowed_service_ids: None,
        };
        assert!(!node_access_can_read(&access));
    }

    #[test]
    fn node_access_can_read_denies_forbidden() {
        assert!(!node_access_can_read(&OwnerAccess::Forbidden));
    }

    // --- owner_info_from_user_id ---

    #[test]
    fn owner_info_from_user_id_with_person_user() {
        let mut user = test_user("user-1", UserType::Person);
        user.display_name = Some("Alice".to_string());
        let info = owner_info_from_user_id("user-1", Some(&user));
        assert_eq!(info.kind, NodeOwnerKind::User);
        assert_eq!(info.id, "user-1");
        assert_eq!(info.display_name, "Alice");
    }

    #[test]
    fn owner_info_from_user_id_with_org_user() {
        let mut user = test_user("org-1", UserType::Org);
        user.display_name = Some("Acme Corp".to_string());
        let info = owner_info_from_user_id("org-1", Some(&user));
        assert_eq!(info.kind, NodeOwnerKind::Org);
        assert_eq!(info.display_name, "Acme Corp");
    }

    #[test]
    fn owner_info_from_user_id_falls_back_to_email_when_display_name_blank() {
        let mut user = test_user("user-1", UserType::Person);
        user.email = "bob@example.com".to_string();
        user.display_name = Some("   ".to_string());
        let info = owner_info_from_user_id("user-1", Some(&user));
        assert_eq!(info.display_name, "bob@example.com");
    }

    #[test]
    fn owner_info_from_user_id_falls_back_to_email_when_display_name_none() {
        let mut user = test_user("user-1", UserType::Person);
        user.email = "bob@example.com".to_string();
        user.display_name = None;
        let info = owner_info_from_user_id("user-1", Some(&user));
        assert_eq!(info.display_name, "bob@example.com");
    }

    #[test]
    fn owner_info_from_user_id_falls_back_to_owner_id_when_no_user() {
        let info = owner_info_from_user_id("unknown-id", None);
        assert_eq!(info.kind, NodeOwnerKind::User);
        assert_eq!(info.id, "unknown-id");
        assert_eq!(info.display_name, "unknown-id");
    }

    #[test]
    fn owner_info_from_user_id_falls_back_to_id_when_email_blank() {
        let mut user = test_user("user-1", UserType::Person);
        user.email = "   ".to_string();
        user.display_name = None;
        let info = owner_info_from_user_id("user-1", Some(&user));
        assert_eq!(info.display_name, "user-1");
    }

    // --- admin_info_from_user ---

    #[test]
    fn admin_info_from_user_with_user_present() {
        let mut user = test_user("user-1", UserType::Person);
        user.email = "admin@example.com".to_string();
        user.display_name = Some("Admin User".to_string());
        let info = admin_info_from_user("user-1", Some(&user), NodeAdminRole::Owner);
        assert_eq!(info.user_id, "user-1");
        assert_eq!(info.display_name, Some("Admin User".to_string()));
        assert_eq!(info.email, Some("admin@example.com".to_string()));
        assert_eq!(info.role, NodeAdminRole::Owner);
    }

    #[test]
    fn admin_info_from_user_without_user() {
        let info = admin_info_from_user("user-1", None, NodeAdminRole::Admin);
        assert_eq!(info.user_id, "user-1");
        assert_eq!(info.display_name, None);
        assert_eq!(info.email, None);
        assert_eq!(info.role, NodeAdminRole::Admin);
    }

    // ────────────────────────────────────────────────────────────────────
    // Integration tests (require MongoDB)
    // ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_node_uses_registration_token_user_id_as_owner() {
        let Some(db) = connect_test_database("node_register_token_owner").await else {
            eprintln!("skipping node service registration test: no local MongoDB available");
            return;
        };

        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .expect("insert org user");

        let (_token_id, raw_token, _expires_at) =
            create_registration_token(&db, &org_id, "org-node", 10, 3600)
                .await
                .expect("create org-owned token");
        let (node, _raw_auth_token, _raw_signing_secret) =
            register_node(&db, &test_encryption_keys(), &raw_token, None)
                .await
                .expect("register node from org token");

        assert_eq!(node.user_id, org_id);
        assert_eq!(node.name, "org-node");
    }
    fn node_audit_runtime(configured: bool) -> std::sync::Arc<AuditRuntime> {
        std::sync::Arc::new(AuditRuntime::new(Default::default(), configured))
    }

    async fn node_audit_events(
        db: &mongodb::Database,
    ) -> Vec<crate::models::audit_outbox::PendingAuditEvent> {
        db.collection(crate::models::audit_outbox::PENDING_COLLECTION_NAME)
            .find(doc! {})
            .sort(doc! {"order": 1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap()
    }

    async fn node_audit_backlog(db: &mongodb::Database, count: i64, age: i64) {
        let oldest = if count == 0 {
            bson::Bson::Null
        } else {
            bson::Bson::DateTime(bson::DateTime::from_chrono(
                Utc::now() - Duration::seconds(age),
            ))
        };
        db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .update_one(doc! {}, doc! {"$set": {"pending_count": count, "oldest_pending_at": oldest}, "$max": {"next_order": count}}).await.unwrap();
    }

    #[tokio::test]
    async fn w8_node_registration_audit_failure_rolls_back_then_commits_safe_person_event() {
        use crate::services::audit_state_service;
        let db = crate::test_utils::connect_transaction_test_database("w8_node_registration").await;
        let fixture = seed_acl_fixture(&db, AccessCase::Direct).await;
        let runtime = node_audit_runtime(false);
        let audit = NodeMutationAudit::new(&runtime, &fixture.actor_id);
        let state = db
            .collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .delete_many(doc! {})
            .await
            .unwrap();
        assert!(
            audit
                .create_registration_token(&db, &fixture.owner_id, "secret-bearing-name", 10, 60)
                .await
                .is_err()
        );
        assert_eq!(
            db.collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert!(node_audit_events(&db).await.is_empty());
        db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .insert_one(state)
            .await
            .unwrap();
        let (id, raw, _) = audit
            .create_registration_token(&db, &fixture.owner_id, "secret-bearing-name", 10, 60)
            .await
            .unwrap();
        let rows = node_audit_events(&db).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].event.principal,
            AuditPrincipal::Person {
                id: fixture.actor_id.clone()
            }
        );
        assert_eq!(
            rows[0].event.owner,
            AuditOwner::Person {
                id: fixture.owner_id.clone()
            }
        );
        assert_eq!(rows[0].event.target_id.as_deref(), Some(id.as_str()));
        let serialized = serde_json::to_string(&rows).unwrap();
        assert!(!serialized.contains(&raw));
        assert!(!serialized.contains("secret-bearing-name"));
        assert_eq!(
            audit_state_service::load(&db).await.unwrap().pending_count,
            1
        );
        assert!(matches!(
            audit
                .create_registration_token(&db, &fixture.owner_id, "another-name", 10, 60)
                .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            db.collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn w8_node_rotation_rollback_preserves_secrets_and_replay_cannot_rotate_again() {
        let db = crate::test_utils::connect_transaction_test_database("w8_node_rotation").await;
        let fixture = seed_acl_fixture(&db, AccessCase::Direct).await;
        let runtime = node_audit_runtime(true);
        node_audit_backlog(&db, 100_000, 1).await;
        let audit = NodeMutationAudit::new(&runtime, &fixture.actor_id);
        assert!(matches!(
            audit
                .rotate_auth_token(&db, &test_encryption_keys(), &fixture.node.id)
                .await,
            Err(AppError::AuditAdmissionClosed { .. })
        ));
        let unchanged = load_active_node(&db, &fixture.node.id).await.unwrap();
        assert_eq!(unchanged.auth_token_hash, fixture.node.auth_token_hash);
        assert_eq!(
            unchanged.signing_secret_hash,
            fixture.node.signing_secret_hash
        );
        assert!(unchanged.signing_secret_encrypted.is_none());
        assert!(node_audit_events(&db).await.is_empty());
        node_audit_backlog(&db, 0, 0).await;
        let (auth, signing) = audit
            .rotate_auth_token(&db, &test_encryption_keys(), &fixture.node.id)
            .await
            .unwrap();
        let updated = load_active_node(&db, &fixture.node.id).await.unwrap();
        assert_eq!(updated.auth_token_hash, hash_token(&auth));
        assert_eq!(updated.signing_secret_hash, hash_token(&signing));
        assert!(updated.signing_secret_encrypted.is_some());
        assert!(matches!(
            audit
                .rotate_auth_token(&db, &test_encryption_keys(), &fixture.node.id)
                .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            load_active_node(&db, &fixture.node.id)
                .await
                .unwrap()
                .auth_token_hash,
            hash_token(&auth)
        );
        let rows = node_audit_events(&db).await;
        assert_eq!(rows.len(), 1);
        let serialized = serde_json::to_string(&rows).unwrap();
        assert!(!serialized.contains(&auth));
        assert!(!serialized.contains(&signing));
        assert!(!serialized.contains("encrypted"));
    }

    #[tokio::test]
    async fn w8_node_binding_authority_matrix_and_live_revocation() {
        let db = crate::test_utils::connect_transaction_test_database("w8_node_binding_acl").await;
        let runtime = node_audit_runtime(false);
        let mut admitted = 0;
        for case in AccessCase::all() {
            let f = seed_acl_fixture(&db, case).await;
            let audit = NodeMutationAudit::new(&runtime, &f.actor_id);
            let result = audit
                .update_binding_priority(&db, &f.node.id, &f.binding.id, 42)
                .await;
            if case.can_write() {
                result.unwrap();
                admitted += 1;
                let row = node_audit_events(&db).await.pop().unwrap();
                assert_eq!(
                    row.event.principal,
                    AuditPrincipal::Person {
                        id: f.actor_id.clone()
                    }
                );
                assert_eq!(
                    row.event.owner,
                    if f.actor_id == f.owner_id {
                        AuditOwner::Person {
                            id: f.owner_id.clone(),
                        }
                    } else {
                        AuditOwner::Org {
                            id: f.owner_id.clone(),
                        }
                    }
                );
            } else {
                assert_node_not_found(result, case);
            }
            if matches!(case, AccessCase::AsOrgAdmin) {
                // The prepared request cannot freeze a revoked membership.
                let audit = NodeMutationAudit::new(&runtime, &f.actor_id);
                db.collection::<bson::Document>(ORG_MEMBERSHIPS)
                    .update_one(
                        doc! {"member_user_id": &f.actor_id},
                        doc! {"$set": {"revoked_at": bson::DateTime::now()}},
                    )
                    .await
                    .unwrap();
                assert!(
                    audit
                        .delete_binding(&db, &f.node.id, &f.binding.id)
                        .await
                        .is_err()
                );
                assert!(
                    db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                        .find_one(doc! {"_id": &f.binding.id})
                        .await
                        .unwrap()
                        .unwrap()
                        .is_active
                );
            }
        }
        assert_eq!(node_audit_events(&db).await.len(), admitted);
    }

    #[tokio::test]
    async fn w8_node_standard_and_narrowing_tier_cells() {
        let db = crate::test_utils::connect_transaction_test_database("w8_node_tier_cells").await;
        let runtime = node_audit_runtime(true); // no fabricated witness contact
        for (count, age, allowed) in [
            (0, 0, true),
            (10_000, 30, true),
            (99_999, 1, true),
            (100_000, 1, false),
            (1, 3600, false),
            (1_000_000, 86_400, false),
        ] {
            let f = seed_acl_fixture(&db, AccessCase::Direct).await;
            node_audit_backlog(&db, count, age).await;
            let audit = NodeMutationAudit::new(&runtime, &f.actor_id);
            let result = audit
                .update_binding_priority(&db, &f.node.id, &f.binding.id, 20)
                .await;
            assert_eq!(result.is_ok(), allowed, "count={count}, age={age}");
            if !allowed {
                assert!(matches!(result, Err(AppError::AuditAdmissionClosed { .. })));
            }
            NodeMutationAudit::new(&runtime, &f.actor_id)
                .delete_binding(&db, &f.node.id, &f.binding.id)
                .await
                .unwrap();
        }
        let f = seed_acl_fixture(&db, AccessCase::Direct).await;
        node_audit_backlog(&db, 0, 0).await;
        let state = crate::services::audit_state_service::load(&db)
            .await
            .unwrap();
        db.collection::<bson::Document>(crate::models::audit_witness_delivery::COLLECTION_NAME).insert_one(doc! {"_id": &state.id, "divergence": {"seq": 1_i64, "local_hash": "a".repeat(64), "witnessed_hash": "b".repeat(64)}}).await.unwrap();
        assert!(matches!(
            NodeMutationAudit::new(&runtime, &f.actor_id)
                .update_binding_priority(&db, &f.node.id, &f.binding.id, 20)
                .await,
            Err(AppError::AuditWitnessDiverged)
        ));
        NodeMutationAudit::new(&runtime, &f.actor_id)
            .delete_binding(&db, &f.node.id, &f.binding.id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn w8_node_transfer_two_people_two_orgs_and_revision_rollback() {
        let db = crate::test_utils::connect_transaction_test_database("w8_node_transfer").await;
        let runtime = node_audit_runtime(false);
        let a = seed_acl_fixture(&db, AccessCase::AsOrgAdmin).await;
        let b = seed_acl_fixture(&db, AccessCase::AsOrgAdmin).await;
        // The destination's unrelated node must not shadow the revision fence
        // with the independently tested node-name collision refusal.
        db.collection::<Node>(NODES)
            .update_one(
                doc! {"_id": &b.node.id},
                doc! {"$set": {"name": "destination-existing-node"}},
            )
            .await
            .unwrap();
        let audit = NodeMutationAudit::new(&runtime, &a.actor_id);
        assert!(matches!(
            audit
                .transfer_node_owner(&db, &a.node.id, &b.owner_id, 10, None)
                .await,
            Err(AppError::Forbidden(_))
        ));
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(
                &b.owner_id,
                &a.actor_id,
                OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        db.collection::<bson::Document>(NODES)
            .update_one(
                doc! {"_id": &a.node.id},
                doc! {"$set": {"state_version": 7_i64}},
            )
            .await
            .unwrap();
        let stale = audit
            .transfer_node_owner(&db, &a.node.id, &b.owner_id, 10, Some(6))
            .await;
        assert!(matches!(stale, Err(AppError::Conflict(_))), "{stale:?}");
        assert!(node_audit_events(&db).await.is_empty());
        assert!(
            db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                .find_one(doc! {"_id": &a.binding.id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        let result = audit
            .transfer_node_owner(&db, &a.node.id, &b.owner_id, 10, Some(7))
            .await
            .unwrap();
        assert_eq!(result.previous_owner_user_id, a.owner_id);
        assert_eq!(result.new_owner_user_id, b.owner_id);
        assert_eq!(result.deactivated_bindings_count, 1);
        let rows = node_audit_events(&db).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].event.principal,
            AuditPrincipal::Person {
                id: a.actor_id.clone()
            }
        );
        assert_eq!(
            rows[0].event.owner,
            AuditOwner::Org {
                id: a.owner_id.clone()
            }
        );
        assert_eq!(
            rows[1].event.owner,
            AuditOwner::Org {
                id: b.owner_id.clone()
            }
        );
        assert!(
            NodeMutationAudit::new(&runtime, &a.actor_id)
                .delete_binding(&db, &b.node.id, &a.binding.id)
                .await
                .is_err()
        );
        assert!(
            NodeMutationAudit::new(&runtime, &b.actor_id)
                .delete_node(&db, &a.node.id, Some(7))
                .await
                .is_err()
        );
        assert_eq!(
            load_active_node(&db, &a.node.id).await.unwrap().user_id,
            b.owner_id
        );
    }

    #[tokio::test]
    async fn w8_node_delete_rolls_back_cleanup_without_audit_state() {
        let db =
            crate::test_utils::connect_transaction_test_database("w8_node_delete_atomic").await;
        let f = seed_acl_fixture(&db, AccessCase::Direct).await;
        let runtime = node_audit_runtime(false);
        let audit = NodeMutationAudit::new(&runtime, &f.actor_id);
        let state = db
            .collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .delete_many(doc! {})
            .await
            .unwrap();
        assert!(audit.delete_node(&db, &f.node.id, None).await.is_err());
        assert!(load_active_node(&db, &f.node.id).await.unwrap().is_active);
        assert!(
            db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                .find_one(doc! {"_id": &f.binding.id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .insert_one(state)
            .await
            .unwrap();
        audit.delete_node(&db, &f.node.id, None).await.unwrap();
        assert!(
            !db.collection::<Node>(NODES)
                .find_one(doc! {"_id": &f.node.id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        assert_eq!(node_audit_events(&db).await.len(), 1);
    }
    async fn node_audit_connected_fixture(
        db: &mongodb::Database,
        f: &NodeAclFixture,
        state: &crate::AppState,
    ) -> String {
        let connection = Uuid::new_v4().to_string();
        let now = Utc::now();
        let owner = crate::models::node::NodeConnectionOwner {
            instance_name: state.replica_identity.instance_name.clone(),
            generation_id: state.replica_identity.generation_id.clone(),
            internal_base_url: state.replica_identity.internal_base_url.clone(),
            connection_id: connection.clone(),
            claimed_at: now,
            renewed_at: now,
            expires_at: now + Duration::minutes(5),
            http_cancellation: false,
            http_signature_v2: false,
            proxy_upload_v1: false,
            credential_ack_correlation: false,
            remote_credential_crypto_v1: false,
            proxy_max_body_size: None,
            capabilities_resolved: true,
        };
        db.collection::<bson::Document>(NODES).update_one(doc! {"_id": &f.node.id}, doc! {"$set": {"connection_owner": bson::to_bson(&owner).unwrap(), "status": "online"}}).await.unwrap();
        connection
    }

    #[tokio::test]
    async fn w8_node_disconnect_defers_effect_after_local_revocation_and_consumes_once() {
        use crate::models::audit_effect_intent::{AuditEffectIntent, COLLECTION_NAME as INTENTS};
        let db = crate::test_utils::connect_transaction_test_database("w8_node_disconnect").await;
        let f = seed_acl_fixture(&db, AccessCase::Direct).await;
        let state = crate::test_utils::test_app_state(db.clone());
        state
            .audit_runtime
            .bind_effects(
                Uuid::new_v4(),
                std::sync::Arc::new(Zeroizing::new([2; 32])),
                None,
            )
            .unwrap();
        let connection = node_audit_connected_fixture(&db, &f, &state).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        state
            .node_ws_manager
            .register_connection_with_id(&f.node.id, connection, tx);
        let audit = NodeMutationAudit::new(&state.audit_runtime, &f.actor_id)
            .with_disconnect(&db, &f.node.id)
            .await
            .unwrap();
        audit
            .rotate_auth_token(&db, &test_encryption_keys(), &f.node.id)
            .await
            .unwrap();
        assert!(
            load_active_node(&db, &f.node.id)
                .await
                .unwrap()
                .connection_owner
                .is_none()
        );
        assert!(
            !audit
                .disconnect_after_commit(
                    &db,
                    &state.node_dispatch,
                    &f.node.id,
                    4002,
                    "node credentials rotated"
                )
                .await
                .unwrap()
        );
        assert!(
            rx.try_recv().is_err(),
            "unmaterialized intent sends nothing"
        );
        assert_eq!(
            crate::services::audit_state_service::load(&db)
                .await
                .unwrap()
                .pending_count,
            3
        );
        while audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
        {}
        assert!(
            audit
                .disconnect_after_commit(
                    &db,
                    &state.node_dispatch,
                    &f.node.id,
                    4002,
                    "node credentials rotated"
                )
                .await
                .unwrap()
        );
        assert!(rx.recv().await.is_some());
        assert!(
            !audit
                .disconnect_after_commit(
                    &db,
                    &state.node_dispatch,
                    &f.node.id,
                    4002,
                    "node credentials rotated"
                )
                .await
                .unwrap()
        );
        let intent = db
            .collection::<AuditEffectIntent>(INTENTS)
            .find_one(doc! {"_id": audit.operation_id.to_string()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(intent.terminal_stage, Some(AuditStage::Succeeded));
        let rows = node_audit_events(&db).await;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].event.stage, AuditStage::Intent);
        assert_eq!(rows[2].event.stage, AuditStage::Succeeded);
    }

    #[tokio::test]
    async fn w8_node_connection_replacement_rolls_back_rotation_and_fenced_close_spares_successor()
    {
        let db =
            crate::test_utils::connect_transaction_test_database("w8_node_connection_fence").await;
        let f = seed_acl_fixture(&db, AccessCase::Direct).await;
        let state = crate::test_utils::test_app_state(db.clone());
        let old = node_audit_connected_fixture(&db, &f, &state).await;
        let audit = NodeMutationAudit::new(&state.audit_runtime, &f.actor_id)
            .with_disconnect(&db, &f.node.id)
            .await
            .unwrap();
        let successor = node_audit_connected_fixture(&db, &f, &state).await;
        assert_ne!(old, successor);
        assert!(matches!(
            audit
                .rotate_auth_token(&db, &test_encryption_keys(), &f.node.id)
                .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            load_active_node(&db, &f.node.id)
                .await
                .unwrap()
                .auth_token_hash,
            f.node.auth_token_hash
        );
        assert!(node_audit_events(&db).await.is_empty());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        state
            .node_ws_manager
            .register_connection_with_id(&f.node.id, successor, tx);
        assert!(
            !state
                .node_dispatch
                .disconnect_invalidated(
                    audit.disconnect_owner.unwrap(),
                    4002,
                    "node credentials rotated"
                )
                .await
                .unwrap()
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn w8_node_claimed_disconnect_is_uncertain_on_recovery_never_resent() {
        use crate::models::audit_effect_intent::{AuditEffectIntent, COLLECTION_NAME as INTENTS};
        let db =
            crate::test_utils::connect_transaction_test_database("w8_node_disconnect_recovery")
                .await;
        let f = seed_acl_fixture(&db, AccessCase::Direct).await;
        let state = crate::test_utils::test_app_state(db.clone());
        state
            .audit_runtime
            .bind_effects(
                Uuid::new_v4(),
                std::sync::Arc::new(Zeroizing::new([2; 32])),
                None,
            )
            .unwrap();
        let connection = node_audit_connected_fixture(&db, &f, &state).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        state
            .node_ws_manager
            .register_connection_with_id(&f.node.id, connection, tx);
        let audit = NodeMutationAudit::new(&state.audit_runtime, &f.actor_id)
            .with_disconnect(&db, &f.node.id)
            .await
            .unwrap();
        audit.delete_node(&db, &f.node.id, None).await.unwrap();
        while audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
        {}
        let paused = state
            .audit_runtime
            .authorize_effect(&db, &audit.operation_id.to_string())
            .await
            .unwrap();
        let recovered = node_audit_runtime(false);
        recovered
            .bind_effects(
                Uuid::new_v4(),
                std::sync::Arc::new(Zeroizing::new([2; 32])),
                None,
            )
            .unwrap();
        let mut recovered_audit = audit.clone();
        recovered_audit.runtime = recovered;
        assert!(
            !recovered_audit
                .disconnect_after_commit(
                    &db,
                    &state.node_dispatch,
                    &f.node.id,
                    4006,
                    "node deleted"
                )
                .await
                .unwrap()
        );
        assert!(
            paused
                .run(|| state.node_dispatch.disconnect_invalidated(
                    audit.disconnect_owner.clone().unwrap(),
                    4006,
                    "node deleted"
                ))
                .await
                .is_err()
        );
        assert!(
            rx.try_recv().is_err(),
            "recovered/paused attempts cannot close twice"
        );
        let intent = db
            .collection::<AuditEffectIntent>(INTENTS)
            .find_one(doc! {"_id": audit.operation_id.to_string()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(intent.terminal_stage, Some(AuditStage::Uncertain));
    }

    #[tokio::test]
    async fn w8_node_registration_key_principal_retains_verified_owner_without_human_impersonation()
    {
        let db =
            crate::test_utils::connect_transaction_test_database("w8_node_key_attribution").await;
        let f = seed_acl_fixture(&db, AccessCase::AsOrgAdmin).await;
        let runtime = node_audit_runtime(false);
        let principal = AuditPrincipal::ApiKey {
            id: Uuid::new_v4().to_string(),
            owner_id: f.owner_id.clone(),
        };
        let audit = NodeMutationAudit::new(&runtime, &f.owner_id)
            .with_verified_principal(principal.clone());
        audit
            .create_registration_token(&db, &f.owner_id, "org-key-node", 10, 60)
            .await
            .unwrap();
        let rows = node_audit_events(&db).await;
        assert_eq!(rows[0].event.principal, principal);
        assert_eq!(rows[0].event.owner, AuditOwner::Org { id: f.owner_id });
        audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap();
        let row = db
            .collection::<crate::models::audit_log::AuditLog>(
                crate::models::audit_log::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": &rows[0].id})
            .await
            .unwrap()
            .unwrap();
        assert!(row.user_id.is_none());
        assert!(row.api_key_id.is_some());
        assert!(row.api_key_name.is_none());
    }

    #[tokio::test]
    async fn w8_node_admin_mutation_checks_live_role_and_narrows_during_backlog() {
        let db = crate::test_utils::connect_transaction_test_database("w8_node_admin").await;
        let f = seed_acl_fixture(&db, AccessCase::Direct).await;
        let admin_id = Uuid::new_v4().to_string();
        let mut admin = test_user(&admin_id, UserType::Person);
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .unwrap();
        admin.role_ids.push(roles.admin.clone());
        db.collection::<User>(USERS)
            .insert_one(&admin)
            .await
            .unwrap();
        let runtime = node_audit_runtime(true);
        node_audit_backlog(&db, 1_000_000, 86_400).await;
        assert!(
            NodeMutationAudit::new(&runtime, &f.actor_id)
                .admin_delete_node(&db, &f.node.id)
                .await
                .is_err()
        );
        let audit = NodeMutationAudit::new(&runtime, &admin_id)
            .with_disconnect(&db, &f.node.id)
            .await
            .unwrap();
        db.collection::<User>(USERS)
            .update_one(doc! {"_id": &admin_id}, doc! {"$set": {"role_ids": []}})
            .await
            .unwrap();
        assert!(audit.admin_delete_node(&db, &f.node.id).await.is_err());
        assert!(load_active_node(&db, &f.node.id).await.is_ok());
        db.collection::<User>(USERS)
            .update_one(
                doc! {"_id": &admin_id},
                doc! {"$set": {"role_ids": [&roles.admin]}},
            )
            .await
            .unwrap();
        audit.admin_delete_node(&db, &f.node.id).await.unwrap();
        let rows = node_audit_events(&db).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].event.principal,
            AuditPrincipal::Person { id: admin_id }
        );
        assert_eq!(rows[0].event.owner, AuditOwner::Person { id: f.owner_id });
        assert!(
            !db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                .find_one(doc! {"_id": &f.binding.id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
    }

    #[tokio::test]
    async fn w8_node_rotation_real_write_conflict_and_unknown_commit_keep_prepared_material() {
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{Arc, Mutex};
        let base = crate::test_utils::connect_transaction_test_database("w8_node_retry").await;
        let f = seed_acl_fixture(&base, AccessCase::Direct).await;
        let app_name = format!("w8-node-{}", Uuid::new_v4());
        let commands = Arc::new(Mutex::new(Vec::new()));
        let capture = commands.clone();
        let mut options = mongodb::options::ClientOptions::parse(
            std::env::var("NYXID_TEST_DATABASE_URL").unwrap(),
        )
        .await
        .unwrap();
        options.app_name = Some(app_name.clone());
        options.command_event_handler = Some(EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && matches!(event.command_name.as_str(), "update" | "commitTransaction")
            {
                capture
                    .lock()
                    .unwrap()
                    .push((event.command_name, event.command));
            }
        }));
        let db = mongodb::Client::with_options(options)
            .unwrap()
            .database(base.name());
        let runtime = node_audit_runtime(false);
        for labeled in [false, true] {
            commands.lock().unwrap().clear();
            base.client()
                .database("admin")
                .run_command(
                    doc! {"configureFailPoint": "failCommand", "mode": {"times": 2}, "data": {
                        "failCommands": ["update"], "appName": &app_name, "errorCode": 112,
                        "errorLabels": if labeled {vec!["TransientTransactionError"]} else {vec![]},
                    }},
                )
                .await
                .unwrap();
            let result = NodeMutationAudit::new(&runtime, &f.actor_id)
                .rotate_auth_token(&db, &test_encryption_keys(), &f.node.id)
                .await;
            base.client()
                .database("admin")
                .run_command(doc! {"configureFailPoint": "failCommand", "mode": "off"})
                .await
                .unwrap();
            let (auth, _) = result.unwrap();
            let updates: Vec<_> = commands
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(name, cmd)| {
                    if name != "update" || cmd.get_str("update").ok() != Some(NODES) {
                        return None;
                    }
                    Some(
                        cmd.get_array("updates").unwrap()[0]
                            .as_document()
                            .unwrap()
                            .get_document("u")
                            .unwrap()
                            .get_document("$set")
                            .unwrap()
                            .get_str("auth_token_hash")
                            .unwrap()
                            .to_owned(),
                    )
                })
                .collect();
            assert_eq!(
                updates.len(),
                3,
                "two definite conflicts retry the complete rotation"
            );
            assert!(
                updates.iter().all(|hash| hash == &hash_token(&auth)),
                "prepared material remains identical on every attempt"
            );
        }
        commands.lock().unwrap().clear();
        base.client().database("admin").run_command(doc! {"configureFailPoint": "failCommand", "mode": {"times": 1}, "data": {
            "failCommands": ["commitTransaction"], "appName": &app_name,
            "writeConcernError": {"code": 64, "errmsg": "injected uncertain commit"}, "errorLabels": ["UnknownTransactionCommitResult"],
        }}).await.unwrap();
        let result = NodeMutationAudit::new(&runtime, &f.actor_id)
            .rotate_auth_token(&db, &test_encryption_keys(), &f.node.id)
            .await;
        base.client()
            .database("admin")
            .run_command(doc! {"configureFailPoint": "failCommand", "mode": "off"})
            .await
            .unwrap();
        let (auth, _) = result.unwrap();
        {
            let commands = commands.lock().unwrap();
            let commits: Vec<_> = commands
                .iter()
                .filter(|(name, _)| name == "commitTransaction")
                .map(|(_, command)| command)
                .collect();
            assert_eq!(commits.len(), 2);
            assert_eq!(commits[0].get("lsid"), commits[1].get("lsid"));
            assert_eq!(commits[0].get("txnNumber"), commits[1].get("txnNumber"));
            assert_eq!(
                commands
                    .iter()
                    .filter(
                        |(name, cmd)| name == "update" && cmd.get_str("update").ok() == Some(NODES)
                    )
                    .count(),
                1,
                "unknown commit cannot rerun rotation"
            );
        }
        assert_eq!(
            load_active_node(&db, &f.node.id)
                .await
                .unwrap()
                .auth_token_hash,
            hash_token(&auth)
        );
        assert_eq!(node_audit_events(&db).await.len(), 3);
    }
    #[tokio::test]
    async fn w8_node_human_mutation_refuses_invented_machine_or_system_person() {
        let db =
            crate::test_utils::connect_transaction_test_database("w8_node_principal_fence").await;
        let f = seed_acl_fixture(&db, AccessCase::Direct).await;
        let runtime = node_audit_runtime(false);
        for principal in [
            AuditPrincipal::Node {
                id: f.actor_id.clone(),
            },
            AuditPrincipal::ServiceAccount {
                id: f.actor_id.clone(),
            },
            AuditPrincipal::System {
                actor: crate::models::audit_outbox::SystemAuditActor::Maintenance,
            },
            AuditPrincipal::Person {
                id: Uuid::new_v4().to_string(),
            },
        ] {
            let audit =
                NodeMutationAudit::new(&runtime, &f.actor_id).with_verified_principal(principal);
            assert!(matches!(
                audit
                    .update_binding_priority(&db, &f.node.id, &f.binding.id, 99)
                    .await,
                Err(AppError::Forbidden(_))
            ));
        }
        assert!(node_audit_events(&db).await.is_empty());
        assert_eq!(
            db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                .find_one(doc! {"_id": &f.binding.id})
                .await
                .unwrap()
                .unwrap()
                .priority,
            0
        );
    }
    #[tokio::test]
    async fn w8_node_binding_creation_requires_live_service_and_rolls_back_at_standard_ceiling() {
        let db =
            crate::test_utils::connect_transaction_test_database("w8_node_binding_create").await;
        let f = seed_acl_fixture(&db, AccessCase::AsOrgAdmin).await;
        let service = Uuid::new_v4().to_string();
        let runtime = node_audit_runtime(true);
        let audit = NodeMutationAudit::new(&runtime, &f.actor_id);
        assert!(matches!(
            audit.create_binding(&db, &f.node.id, &service).await,
            Err(AppError::NotFound(_))
        ));
        db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(doc! {"_id": &service, "is_active": true})
            .await
            .unwrap();
        node_audit_backlog(&db, 100_000, 1).await;
        assert!(matches!(
            audit.create_binding(&db, &f.node.id, &service).await,
            Err(AppError::AuditAdmissionClosed { .. })
        ));
        assert_eq!(
            db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                .count_documents(doc! {"service_id": &service})
                .await
                .unwrap(),
            0
        );
        node_audit_backlog(&db, 0, 0).await;
        let binding = audit
            .create_binding(&db, &f.node.id, &service)
            .await
            .unwrap();
        assert_eq!(binding.user_id, f.owner_id);
        assert!(matches!(
            NodeMutationAudit::new(&runtime, &f.actor_id)
                .create_binding(&db, &f.node.id, &service)
                .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(node_audit_events(&db).await.len(), 1);
    }

    #[tokio::test]
    async fn w8_node_transfer_requires_the_complete_two_owner_audit_bundle() {
        let db =
            crate::test_utils::connect_transaction_test_database("w8_node_transfer_bundle").await;
        let f = seed_acl_fixture(&db, AccessCase::AsOrgAdmin).await;
        let runtime = node_audit_runtime(true);
        node_audit_backlog(&db, 99_999, 1).await;
        let audit = NodeMutationAudit::new(&runtime, &f.actor_id);
        assert!(matches!(
            audit
                .transfer_node_owner(&db, &f.node.id, &f.actor_id, 10, None)
                .await,
            Err(AppError::AuditAdmissionClosed { .. })
        ));
        assert_eq!(
            load_active_node(&db, &f.node.id).await.unwrap().user_id,
            f.owner_id
        );
        assert!(node_audit_events(&db).await.is_empty());
        assert!(
            db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                .find_one(doc! {"_id": &f.binding.id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        node_audit_backlog(&db, 99_998, 1).await;
        audit
            .transfer_node_owner(&db, &f.node.id, &f.actor_id, 10, None)
            .await
            .unwrap();
        assert_eq!(node_audit_events(&db).await.len(), 2);
        assert_eq!(
            crate::services::audit_state_service::load(&db)
                .await
                .unwrap()
                .pending_count,
            100_000
        );
    }

    #[tokio::test]
    async fn w8_node_online_transfer_preserves_destination_event_and_disconnect_terminal() {
        use crate::models::audit_effect_intent::{AuditEffectIntent, COLLECTION_NAME as INTENTS};
        let db =
            crate::test_utils::connect_transaction_test_database("w8_node_transfer_close").await;
        let f = seed_acl_fixture(&db, AccessCase::AsOrgAdmin).await;
        let state = crate::test_utils::test_app_state(db.clone());
        state
            .audit_runtime
            .bind_effects(
                Uuid::new_v4(),
                std::sync::Arc::new(Zeroizing::new([2; 32])),
                None,
            )
            .unwrap();
        let connection = node_audit_connected_fixture(&db, &f, &state).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        state
            .node_ws_manager
            .register_connection_with_id(&f.node.id, connection, tx);
        let audit = NodeMutationAudit::new(&state.audit_runtime, &f.actor_id)
            .with_disconnect(&db, &f.node.id)
            .await
            .unwrap();
        audit
            .transfer_node_owner(&db, &f.node.id, &f.actor_id, 10, None)
            .await
            .unwrap();
        let received = node_audit_events(&db)
            .await
            .into_iter()
            .find(|row| row.event.action == "node_transfer_received")
            .unwrap();
        let intent = db
            .collection::<AuditEffectIntent>(INTENTS)
            .find_one(doc! {"_id": audit.operation_id.to_string()})
            .await
            .unwrap()
            .unwrap();
        assert_ne!(received.id, intent.terminal_event_id);
        assert_ne!(received.event.operation_id, intent.id);
        assert_eq!(received.event.stage, AuditStage::Committed);
        assert_eq!(received.event.owner, AuditOwner::Person { id: f.actor_id });
        while audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
        {}
        assert!(
            audit
                .disconnect_after_commit(
                    &db,
                    &state.node_dispatch,
                    &f.node.id,
                    4007,
                    "node ownership transferred"
                )
                .await
                .unwrap()
        );
        assert!(rx.recv().await.is_some());
        while audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
        {}
        let rows = node_audit_events(&db).await;
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|row| row.materialized_seq.is_some()));
        assert!(
            rows.iter()
                .any(|row| { row.id == received.id && row.event == received.event })
        );
        assert!(rows.iter().any(|row| {
            row.id == intent.terminal_event_id
                && row.event.action == "node_disconnect"
                && row.event.stage == AuditStage::Succeeded
        }));
    }

    #[tokio::test]
    async fn w8_node_delete_narrows_with_three_reserved_slots_while_close_defers() {
        let db =
            crate::test_utils::connect_transaction_test_database("w8_node_delete_deferred").await;
        let f = seed_acl_fixture(&db, AccessCase::Direct).await;
        let mut state = crate::test_utils::test_app_state(db.clone());
        state.audit_runtime = node_audit_runtime(true);
        state
            .audit_runtime
            .bind_effects(
                Uuid::new_v4(),
                std::sync::Arc::new(Zeroizing::new([2; 32])),
                None,
            )
            .unwrap();
        let connection = node_audit_connected_fixture(&db, &f, &state).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        state
            .node_ws_manager
            .register_connection_with_id(&f.node.id, connection, tx);
        let audit = NodeMutationAudit::new(&state.audit_runtime, &f.actor_id)
            .with_disconnect(&db, &f.node.id)
            .await
            .unwrap();
        node_audit_backlog(&db, 1_000_000, 86_400).await;
        audit.delete_node(&db, &f.node.id, None).await.unwrap();
        assert!(
            !audit
                .disconnect_after_commit(
                    &db,
                    &state.node_dispatch,
                    &f.node.id,
                    4006,
                    "node deleted"
                )
                .await
                .unwrap()
        );
        assert!(rx.try_recv().is_err());
        let node = db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id": &f.node.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!node.is_active);
        assert!(node.connection_owner.is_none());
        assert_eq!(
            node_audit_events(&db).await.len(),
            2,
            "terminal slot is reserved without inventing a terminal outcome"
        );
        assert_eq!(
            crate::services::audit_state_service::load(&db)
                .await
                .unwrap()
                .pending_count,
            1_000_003
        );
    }
}
