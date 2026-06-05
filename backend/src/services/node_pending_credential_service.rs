use std::collections::{HashMap, HashSet};
use std::fmt;

use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, doc};
use mongodb::options::ReturnDocument;
use serde::Deserialize;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::errors::{AppError, AppResult};
use crate::models::node_pending_credential::{
    COLLECTION_NAME as NODE_PENDING_CREDENTIALS, CryptoBundle, FanOutDecryptOutcome,
    FanOutNodeState, InjectionMethod, NodePendingCredential, RemoteCryptoState,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::services::{
    node_fanout_resolver, node_service, org_service, rci_audit_service, url_validation,
};

pub const MAX_CIPHERTEXT_SIZE: usize = 16 * 1024;
pub const MAX_FAN_OUT_TARGETS: usize = 10;
pub const MAX_FAN_OUT_CIPHERTEXT_TOTAL_SIZE: usize = MAX_FAN_OUT_TARGETS * MAX_CIPHERTEXT_SIZE;
pub const MAX_FAN_OUT_HTTP_BODY_BYTES: usize = 384 * 1024;
pub const OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS: i64 = 15 * 60;
pub const MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE: u64 = 5;

pub struct CreatePendingCredentialInput {
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
    pub ttl_secs: i64,
    pub remote_crypto: bool,
}

pub struct CreateFanOutPendingCredentialInput {
    pub owner_user_id: String,
    pub service_id: String,
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
    pub ttl_secs: i64,
    pub remote_crypto: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorePendingCiphertextInput {
    pub admin_pubkey: Zeroizing<String>,
    pub nonce: Zeroizing<String>,
    pub ciphertext: Zeroizing<Vec<u8>>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StoreFanOutCiphertextItemInput {
    pub node_id: String,
    pub generation: i64,
    pub version: String,
    pub admin_pubkey: Zeroizing<String>,
    pub nonce: Zeroizing<String>,
    pub ciphertext: Zeroizing<Vec<u8>>,
}

impl StoreFanOutCiphertextItemInput {
    pub fn new(
        node_id: String,
        generation: i64,
        version: String,
        admin_pubkey: String,
        nonce: String,
        ciphertext: Vec<u8>,
    ) -> Self {
        Self {
            node_id,
            generation,
            version,
            admin_pubkey: Zeroizing::new(admin_pubkey),
            nonce: Zeroizing::new(nonce),
            ciphertext: Zeroizing::new(ciphertext),
        }
    }
}

impl fmt::Debug for StoreFanOutCiphertextItemInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreFanOutCiphertextItemInput")
            .field("node_id", &self.node_id)
            .field("generation", &self.generation)
            .field("version", &self.version)
            .field("admin_pubkey", &"[REDACTED]")
            .field("nonce", &"[REDACTED]")
            .field(
                "ciphertext",
                &format!("[REDACTED; {} bytes]", self.ciphertext.len()),
            )
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct StoreFanOutCiphertextsInput {
    pub fan_out_revision: i64,
    pub items: Vec<StoreFanOutCiphertextItemInput>,
    pub online_node_ids: HashSet<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum PendingCredentialIntegrityVerificationRequest {
    AdminVerified {
        fingerprint_sha384_hex: Option<String>,
        verified_at: Option<String>,
        manifest_url_configured: bool,
    },
    OrgPolicyOptOut {
        fingerprint_sha384_hex: Option<String>,
        verified_at: Option<String>,
        manifest_url_configured: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityVerificationAudit {
    pub mode: &'static str,
    pub fingerprint_sha384_prefix: Option<String>,
    pub verified_at: Option<String>,
    pub manifest_url_configured: bool,
}

impl StorePendingCiphertextInput {
    pub fn new(admin_pubkey: String, nonce: String, ciphertext: Vec<u8>) -> Self {
        Self {
            admin_pubkey: Zeroizing::new(admin_pubkey),
            nonce: Zeroizing::new(nonce),
            ciphertext: Zeroizing::new(ciphertext),
        }
    }
}

impl fmt::Debug for StorePendingCiphertextInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorePendingCiphertextInput")
            .field("admin_pubkey", &"[REDACTED]")
            .field("nonce", &"[REDACTED]")
            .field(
                "ciphertext",
                &format!("[REDACTED; {} bytes]", self.ciphertext.len()),
            )
            .finish()
    }
}

pub async fn owner_integrity_verification_opt_out(
    db: &mongodb::Database,
    owner_user_id: &str,
) -> AppResult<bool> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": owner_user_id })
        .await?;
    Ok(user
        .as_ref()
        .map(org_service::remote_credential_integrity_verification_opt_out)
        .unwrap_or(false))
}

pub async fn validate_integrity_verification_for_owner(
    db: &mongodb::Database,
    owner_user_id: &str,
    release_integrity_manifest_url: Option<&str>,
    verification_ttl_secs: i64,
    request: Option<&PendingCredentialIntegrityVerificationRequest>,
    now: DateTime<Utc>,
) -> AppResult<IntegrityVerificationAudit> {
    let effective_org_opt_out = owner_integrity_verification_opt_out(db, owner_user_id).await?;
    validate_integrity_verification(
        release_integrity_manifest_url,
        verification_ttl_secs,
        effective_org_opt_out,
        request,
        now,
    )
}

fn validate_integrity_verification(
    release_integrity_manifest_url: Option<&str>,
    verification_ttl_secs: i64,
    effective_org_opt_out: bool,
    request: Option<&PendingCredentialIntegrityVerificationRequest>,
    now: DateTime<Utc>,
) -> AppResult<IntegrityVerificationAudit> {
    let manifest_configured = release_integrity_manifest_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();

    let Some(request) = request else {
        if effective_org_opt_out {
            return Ok(IntegrityVerificationAudit {
                mode: "org_policy_opt_out",
                fingerprint_sha384_prefix: None,
                verified_at: None,
                manifest_url_configured: manifest_configured,
            });
        }
        return Err(AppError::ValidationError(
            "integrity_verification is required for remote credential ciphertext submission"
                .to_string(),
        ));
    };

    match request {
        PendingCredentialIntegrityVerificationRequest::AdminVerified {
            fingerprint_sha384_hex,
            verified_at,
            manifest_url_configured,
        } => {
            if !manifest_configured || !manifest_url_configured {
                return Err(AppError::ValidationError(
                    "release integrity manifest URL is not configured".to_string(),
                ));
            }
            let fingerprint = fingerprint_sha384_hex.as_deref().ok_or_else(|| {
                AppError::ValidationError("fingerprint_sha384_hex is required".to_string())
            })?;
            if !is_sha384_hex(fingerprint) {
                return Err(AppError::ValidationError(
                    "fingerprint_sha384_hex must be 96 lowercase hex characters".to_string(),
                ));
            }
            let verified_at_raw = verified_at
                .as_deref()
                .ok_or_else(|| AppError::ValidationError("verified_at is required".to_string()))?;
            let verified_at = DateTime::parse_from_rfc3339(verified_at_raw)
                .map_err(|_| AppError::ValidationError("verified_at must be RFC3339".to_string()))?
                .with_timezone(&Utc);
            if verified_at > now {
                return Err(AppError::ValidationError(
                    "verified_at must not be in the future".to_string(),
                ));
            }
            if now.signed_duration_since(verified_at).num_seconds() > verification_ttl_secs {
                return Err(AppError::ValidationError(
                    "integrity verification has expired".to_string(),
                ));
            }
            Ok(IntegrityVerificationAudit {
                mode: "admin_verified",
                fingerprint_sha384_prefix: Some(fingerprint[..12].to_string()),
                verified_at: Some(verified_at.to_rfc3339()),
                manifest_url_configured: true,
            })
        }
        PendingCredentialIntegrityVerificationRequest::OrgPolicyOptOut {
            fingerprint_sha384_hex,
            verified_at,
            manifest_url_configured,
        } => {
            if !effective_org_opt_out {
                return Err(AppError::ValidationError(
                    "org policy has not opted out of release integrity verification".to_string(),
                ));
            }
            if fingerprint_sha384_hex.is_some() || verified_at.is_some() {
                return Err(AppError::ValidationError(
                    "org_policy_opt_out integrity verification must not include fingerprint or verified_at".to_string(),
                ));
            }
            Ok(IntegrityVerificationAudit {
                mode: "org_policy_opt_out",
                fingerprint_sha384_prefix: None,
                verified_at: None,
                manifest_url_configured: *manifest_url_configured,
            })
        }
    }
}

fn is_sha384_hex(value: &str) -> bool {
    value.len() == 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCredentialAuditSummary {
    pub node_id: String,
    pub pending_credential_id: String,
    pub service_slug: String,
    pub owner_user_id: String,
    pub remote_state: Option<RemoteCryptoState>,
    pub fan_out: bool,
    pub generation: Option<i64>,
    pub pending_created_at: DateTime<Utc>,
    pub pending_expires_at: DateTime<Utc>,
    pub ciphertext_queued_at: Option<DateTime<Utc>>,
    pub ciphertext_expires_at: Option<DateTime<Utc>>,
}

impl PendingCredentialAuditSummary {
    fn from_pending(pending: &NodePendingCredential) -> Self {
        Self {
            node_id: pending.node_id.clone(),
            pending_credential_id: pending.id.clone(),
            service_slug: pending.service_slug.clone(),
            owner_user_id: pending.owner_user_id.clone(),
            remote_state: pending.remote_state.clone(),
            fan_out: false,
            generation: None,
            pending_created_at: pending.created_at,
            pending_expires_at: pending.expires_at,
            ciphertext_queued_at: pending.ciphertext_queued_at,
            ciphertext_expires_at: pending.ciphertext_expires_at,
        }
    }

    pub fn from_fan_out_target(pending: &NodePendingCredential, target: &FanOutNodeState) -> Self {
        Self {
            node_id: target.node_id.clone(),
            pending_credential_id: pending.id.clone(),
            service_slug: pending.service_slug.clone(),
            owner_user_id: pending.owner_user_id.clone(),
            remote_state: target.remote_state.clone(),
            fan_out: true,
            generation: Some(target.generation),
            pending_created_at: pending.created_at,
            pending_expires_at: pending.expires_at,
            ciphertext_queued_at: target.ciphertext_queued_at,
            ciphertext_expires_at: target.ciphertext_expires_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanOutTargetStatus {
    pub node_id: String,
    pub generation: i64,
    pub remote_state: Option<RemoteCryptoState>,
    pub error_code: Option<u32>,
    pub error_kind: Option<String>,
    pub delivery_status: Option<FanOutDeliveryStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FanOutDeliveryStatus {
    Sent,
    Queued,
}

impl FanOutDeliveryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Queued => "queued",
        }
    }
}

#[derive(Clone, Debug)]
pub struct FanOutPendingCredentialResult {
    pub pending: NodePendingCredential,
    pub targets: Vec<FanOutTargetStatus>,
}

#[derive(Clone, Debug)]
pub struct StoreFanOutCiphertextsOutcome {
    pub pending: NodePendingCredential,
    pub targets: Vec<FanOutTargetStatus>,
}

#[derive(Clone)]
pub enum StorePendingCiphertextOutcome {
    StoredForOnlineNode(NodePendingCredential),
    QueuedOffline(NodePendingCredential),
    QueueFull(PendingCredentialAuditSummary),
}

impl fmt::Debug for StorePendingCiphertextOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StoredForOnlineNode(pending) => f
                .debug_struct("StoredForOnlineNode")
                .field("pending_id", &pending.id)
                .field("remote_state", &pending.remote_state)
                .finish(),
            Self::QueuedOffline(pending) => f
                .debug_struct("QueuedOffline")
                .field("pending_id", &pending.id)
                .field("remote_state", &pending.remote_state)
                .finish(),
            Self::QueueFull(summary) => f
                .debug_struct("QueueFull")
                .field("pending_id", &summary.pending_credential_id)
                .field("remote_state", &summary.remote_state)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingCredentialDecryptOutcome {
    Ok,
    Error,
}

pub async fn create_pending_credential(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    input: CreatePendingCredentialInput,
) -> AppResult<NodePendingCredential> {
    validate_service_slug(&input.service_slug)?;
    validate_field_name(&input.field_name, &input.injection_method)?;
    let target_url = clean_optional_string(input.target_url);
    if let Some(url) = target_url.as_deref() {
        url_validation::validate_advisory_http_url(url, "target_url", url_validation::MAX_URL_LEN)?;
    }
    let label = clean_optional_string(input.label);
    if let Some(label) = label.as_deref()
        && label.len() > 128
    {
        return Err(AppError::ValidationError(
            "label must be 128 characters or fewer".to_string(),
        ));
    }

    let node = node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    let existing = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one(doc! {
            "node_id": node_id,
            "service_slug": &input.service_slug,
            "is_active": true,
        })
        .await?;
    if let Some(existing) = existing {
        return Err(AppError::Conflict(format!(
            "A pending credential already exists for service '{}' on this node (id: {})",
            input.service_slug, existing.id
        )));
    }

    let now = Utc::now();
    let expires_at = now + Duration::seconds(input.ttl_secs.max(1));
    let crypto = input.remote_crypto.then(|| CryptoBundle {
        version: "v1".to_string(),
        node_pubkey: String::new(),
        admin_pubkey: None,
        nonce: None,
        ciphertext: None,
    });
    let pending = NodePendingCredential {
        id: Uuid::new_v4().to_string(),
        node_id: node_id.to_string(),
        service_slug: input.service_slug,
        injection_method: input.injection_method,
        field_name: input.field_name,
        target_url,
        label,
        created_by_user_id: actor_user_id.to_string(),
        owner_user_id: node.user_id,
        created_at: now,
        expires_at,
        consumed_at: None,
        declined_at: None,
        crypto,
        remote_state: None,
        ciphertext_queued_at: None,
        ciphertext_expires_at: None,
        is_active: true,
        fan_out_nodes: Vec::new(),
        fan_out_revision: 0,
    };

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .insert_one(&pending)
        .await?;

    Ok(pending)
}

pub async fn create_fan_out_pending_credential(
    db: &mongodb::Database,
    actor_user_id: &str,
    input: CreateFanOutPendingCredentialInput,
) -> AppResult<FanOutPendingCredentialResult> {
    if !input.remote_crypto {
        return Err(AppError::ValidationError(
            "remote_crypto must be true for fan-out credential push".to_string(),
        ));
    }
    validate_service_slug(&input.service_slug)?;
    validate_field_name(&input.field_name, &input.injection_method)?;
    let target_url = clean_optional_string(input.target_url);
    if let Some(url) = target_url.as_deref() {
        url_validation::validate_advisory_http_url(url, "target_url", url_validation::MAX_URL_LEN)?;
    }
    let label = clean_optional_string(input.label);
    if let Some(label) = label.as_deref()
        && label.len() > 128
    {
        return Err(AppError::ValidationError(
            "label must be 128 characters or fewer".to_string(),
        ));
    }

    let targets = node_fanout_resolver::resolve_credential_fan_out_targets(
        db,
        actor_user_id,
        &input.owner_user_id,
        &input.service_id,
    )
    .await?;
    if targets.len() > MAX_FAN_OUT_TARGETS {
        return Err(AppError::ValidationError(format!(
            "fan-out target count must be {MAX_FAN_OUT_TARGETS} or fewer"
        )));
    }
    if targets.len() == 1 {
        let target = targets
            .first()
            .expect("target count checked")
            .node_id
            .clone();
        let pending = create_pending_credential(
            db,
            actor_user_id,
            &target,
            CreatePendingCredentialInput {
                service_slug: input.service_slug,
                injection_method: input.injection_method,
                field_name: input.field_name,
                target_url,
                label,
                ttl_secs: input.ttl_secs,
                remote_crypto: true,
            },
        )
        .await?;
        return Ok(FanOutPendingCredentialResult {
            targets: vec![FanOutTargetStatus {
                node_id: target,
                generation: 0,
                remote_state: pending.remote_state.clone(),
                error_code: None,
                error_kind: None,
                delivery_status: None,
            }],
            pending,
        });
    }

    let node_ids: Vec<String> = targets
        .iter()
        .map(|target| target.node_id.clone())
        .collect();
    preflight_no_active_pending_for_targets(db, &input.service_slug, &node_ids).await?;

    let now = Utc::now();
    let expires_at = now + Duration::seconds(input.ttl_secs.max(1));
    let fan_out_nodes: Vec<FanOutNodeState> = targets
        .iter()
        .map(|target| FanOutNodeState {
            node_id: target.node_id.clone(),
            generation: 0,
            crypto: CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: String::new(),
                admin_pubkey: None,
                nonce: None,
                ciphertext: None,
            },
            remote_state: None,
            decrypt_outcome: None,
            error_code: None,
            error_kind: None,
            pubkey_posted_at: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            consumed_at: None,
            declined_at: None,
            updated_at: now,
        })
        .collect();
    let pending = NodePendingCredential {
        id: Uuid::new_v4().to_string(),
        node_id: node_ids.first().cloned().ok_or_else(|| {
            AppError::Conflict("no active node targets for service fan-out".to_string())
        })?,
        service_slug: input.service_slug,
        injection_method: input.injection_method,
        field_name: input.field_name,
        target_url,
        label,
        created_by_user_id: actor_user_id.to_string(),
        owner_user_id: input.owner_user_id,
        created_at: now,
        expires_at,
        consumed_at: None,
        declined_at: None,
        crypto: None,
        remote_state: None,
        ciphertext_queued_at: None,
        ciphertext_expires_at: None,
        is_active: true,
        fan_out_nodes,
        fan_out_revision: 1,
    };

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .insert_one(&pending)
        .await?;

    Ok(FanOutPendingCredentialResult {
        targets: fan_out_statuses(&pending),
        pending,
    })
}

pub async fn list_pending_credentials_for_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    include_history: bool,
) -> AppResult<Vec<NodePendingCredential>> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;

    let mut filter = doc! {
        "$or": [
            { "node_id": node_id },
            { "fan_out_nodes.node_id": node_id },
        ],
    };
    if !include_history {
        filter.insert("is_active", true);
        filter.insert(
            "expires_at",
            doc! { "$gt": bson::DateTime::from_chrono(Utc::now()) },
        );
    }

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

pub async fn list_pending_credentials_for_node(
    db: &mongodb::Database,
    node_id: &str,
) -> AppResult<Vec<NodePendingCredential>> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(doc! {
            "$or": [
                { "node_id": node_id },
                { "fan_out_nodes.node_id": node_id },
            ],
            "is_active": true,
            "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
        })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

pub async fn record_pending_credential_pubkey(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    version: &str,
    node_pubkey: &str,
) -> AppResult<NodePendingCredential> {
    if version != "v1" {
        return Err(AppError::PendingCredentialVersionUnsupported(
            version.to_string(),
        ));
    }
    let now = Utc::now();
    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "fan_out_nodes.0": { "$exists": false },
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "crypto.version": "v1",
                "crypto.node_pubkey": "",
            },
            doc! {
                "$set": {
                    "crypto.node_pubkey": node_pubkey,
                    "remote_state": remote_state_bson(RemoteCryptoState::PubkeyPosted)?,
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) => Ok(updated),
        None => {
            let current =
                load_active_unexpired_pending_credential(db, node_id, pending_id, now).await?;
            match current.crypto.as_ref() {
                Some(crypto) if crypto.version == "v1" && !crypto.node_pubkey.is_empty() => {
                    Ok(current)
                }
                _ => Err(AppError::NotFound(
                    "Pending credential not found".to_string(),
                )),
            }
        }
    }
}

pub async fn record_fan_out_pubkey(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    version: &str,
    node_pubkey: &str,
) -> AppResult<NodePendingCredential> {
    if version != "v1" {
        return Err(AppError::PendingCredentialVersionUnsupported(
            version.to_string(),
        ));
    }
    let now = Utc::now();
    let now_bson = bson::DateTime::from_chrono(now);
    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "is_active": true,
                "expires_at": { "$gt": now_bson },
                "fan_out_nodes": {
                    "$elemMatch": {
                        "node_id": node_id,
                        "crypto.version": "v1",
                        "crypto.node_pubkey": "",
                    },
                },
            },
            doc! {
                "$set": {
                    "fan_out_nodes.$[target].crypto.node_pubkey": node_pubkey,
                    "fan_out_nodes.$[target].remote_state": remote_state_bson(RemoteCryptoState::PubkeyPosted)?,
                    "fan_out_nodes.$[target].pubkey_posted_at": now_bson,
                    "fan_out_nodes.$[target].error_code": Bson::Null,
                    "fan_out_nodes.$[target].error_kind": Bson::Null,
                    "fan_out_nodes.$[target].updated_at": now_bson,
                },
            },
        )
        .array_filters(vec![doc! { "target.node_id": node_id }])
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) => refresh_fan_out_remote_state(db, updated, now).await,
        None => {
            let current =
                load_active_unexpired_fan_out_pending_credential(db, node_id, pending_id, now)
                    .await?;
            let target = fan_out_target(&current, node_id)
                .ok_or_else(|| AppError::NotFound("Fan-out target not found".to_string()))?;
            if target.crypto.version != "v1" {
                return Err(AppError::PendingCredentialVersionUnsupported(
                    target.crypto.version.clone(),
                ));
            }
            if !target.crypto.node_pubkey.is_empty() {
                Ok(current)
            } else {
                Err(AppError::NotFound(
                    "Pending credential not found".to_string(),
                ))
            }
        }
    }
}

pub async fn get_fan_out_pending_credential_for_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    let pending = load_active_unexpired_pending_by_id(db, pending_id, Utc::now()).await?;
    if pending.fan_out_nodes.is_empty() {
        return Err(AppError::NotFound(
            "Fan-out pending credential not found".to_string(),
        ));
    }
    ensure_actor_can_manage_pending_owner(db, actor_user_id, &pending).await?;
    Ok(pending)
}

pub async fn store_fan_out_ciphertexts_revision_guard(
    db: &mongodb::Database,
    actor_user_id: &str,
    pending_id: &str,
    input: StoreFanOutCiphertextsInput,
    now: DateTime<Utc>,
) -> AppResult<StoreFanOutCiphertextsOutcome> {
    validate_fan_out_ciphertext_input_sizes(&input)?;
    let mut pending = load_active_unexpired_pending_by_id(db, pending_id, now).await?;
    if pending.fan_out_nodes.is_empty() {
        return Err(AppError::NotFound(
            "Fan-out pending credential not found".to_string(),
        ));
    }
    ensure_actor_can_manage_pending_owner(db, actor_user_id, &pending).await?;
    if pending.fan_out_revision != input.fan_out_revision {
        return Err(AppError::Conflict("stale fan_out_revision".to_string()));
    }

    let ready_targets: Vec<&FanOutNodeState> = pending
        .fan_out_nodes
        .iter()
        .filter(|target| fan_out_target_awaiting_ciphertext(target))
        .collect();
    if ready_targets.is_empty() {
        return Err(AppError::Conflict(
            "fan-out ciphertexts were already submitted or no target pubkeys are ready".to_string(),
        ));
    }
    validate_exact_fan_out_item_set(&ready_targets, &input.items)?;

    for item in &input.items {
        if !input.online_node_ids.contains(&item.node_id)
            && active_unexpired_queued_ciphertext_count(db, &item.node_id, now).await?
                >= MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE
        {
            return Err(AppError::PendingCredentialQueueFull(item.node_id.clone()));
        }
    }

    let revision = pending.fan_out_revision;
    let mut items_by_node: HashMap<String, StoreFanOutCiphertextItemInput> = input
        .items
        .into_iter()
        .map(|item| (item.node_id.clone(), item))
        .collect();
    for target in &mut pending.fan_out_nodes {
        let Some(item) = items_by_node.remove(&target.node_id) else {
            continue;
        };
        target.crypto.admin_pubkey = Some(item.admin_pubkey.as_str().to_string());
        target.crypto.nonce = Some(item.nonce.as_str().to_string());
        target.crypto.ciphertext = Some(item.ciphertext.as_slice().to_vec());
        target.updated_at = now;
        target.error_code = None;
        target.error_kind = None;
        if input.online_node_ids.contains(&target.node_id) {
            target.remote_state = Some(RemoteCryptoState::CiphertextReceived);
            target.ciphertext_queued_at = None;
            target.ciphertext_expires_at = None;
        } else {
            target.remote_state = Some(RemoteCryptoState::CiphertextQueued);
            target.ciphertext_queued_at = Some(now);
            target.ciphertext_expires_at =
                Some(now + Duration::seconds(OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS));
        }
    }
    pending.remote_state = aggregate_fan_out_remote_state(&pending.fan_out_nodes);

    let updated = update_fan_out_nodes_with_revision(db, pending, revision, true, now).await?;
    Ok(StoreFanOutCiphertextsOutcome {
        targets: fan_out_statuses(&updated),
        pending: updated,
    })
}

pub async fn retry_failed_fan_out_nodes(
    db: &mongodb::Database,
    actor_user_id: &str,
    pending_id: &str,
    expected_revision: i64,
    now: DateTime<Utc>,
) -> AppResult<FanOutPendingCredentialResult> {
    let mut pending = load_active_unexpired_pending_by_id(db, pending_id, now).await?;
    if pending.fan_out_nodes.is_empty() {
        return Err(AppError::NotFound(
            "Fan-out pending credential not found".to_string(),
        ));
    }
    ensure_actor_can_manage_pending_owner(db, actor_user_id, &pending).await?;
    if pending.fan_out_revision != expected_revision {
        return Err(AppError::Conflict("stale fan_out_revision".to_string()));
    }

    let mut reset_count = 0usize;
    for target in &mut pending.fan_out_nodes {
        if matches!(target.remote_state, Some(RemoteCryptoState::DecryptFailed)) {
            target.generation += 1;
            target.crypto = CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: String::new(),
                admin_pubkey: None,
                nonce: None,
                ciphertext: None,
            };
            target.remote_state = None;
            target.decrypt_outcome = None;
            target.error_code = None;
            target.error_kind = None;
            target.pubkey_posted_at = None;
            target.ciphertext_queued_at = None;
            target.ciphertext_expires_at = None;
            target.consumed_at = None;
            target.declined_at = None;
            target.updated_at = now;
            reset_count += 1;
        }
    }
    if reset_count == 0 {
        return Err(AppError::Conflict(
            "no failed fan-out targets are eligible for retry".to_string(),
        ));
    }
    pending.is_active = true;
    pending.remote_state = aggregate_fan_out_remote_state(&pending.fan_out_nodes);
    pending.consumed_at = None;
    pending.declined_at = None;

    let revision = pending.fan_out_revision;
    let updated = update_fan_out_nodes_with_revision(db, pending, revision, true, now).await?;
    Ok(FanOutPendingCredentialResult {
        targets: fan_out_statuses(&updated),
        pending: updated,
    })
}

pub async fn get_pending_credential_for_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    load_active_unexpired_pending_credential(db, node_id, pending_id, Utc::now()).await
}

pub async fn init_pending_remote_crypto_for_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    let now = Utc::now();
    let pending = load_active_unexpired_pending_credential(db, node_id, pending_id, now).await?;
    if !pending.fan_out_nodes.is_empty() {
        return Err(AppError::ValidationError(
            "fan-out pending credential injection is not supported by this command".to_string(),
        ));
    }
    if pending.crypto.is_some() {
        return Ok(pending);
    }

    let crypto = bson::to_bson(&CryptoBundle {
        version: "v1".to_string(),
        node_pubkey: String::new(),
        admin_pubkey: None,
        nonce: None,
        ciphertext: None,
    })
    .map_err(|err| AppError::Internal(format!("crypto metadata serialization failed: {err}")))?;

    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "fan_out_nodes.0": { "$exists": false },
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "$or": [
                    { "crypto": { "$exists": false } },
                    { "crypto": Bson::Null },
                ],
            },
            doc! {
                "$set": {
                    "crypto": crypto,
                    "remote_state": remote_state_bson(RemoteCryptoState::PubkeyAwaiting)?,
                },
                "$unset": {
                    "ciphertext_queued_at": "",
                    "ciphertext_expires_at": "",
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) => Ok(updated),
        None => {
            let current =
                load_active_unexpired_pending_credential(db, node_id, pending_id, now).await?;
            if current.crypto.is_some() {
                Ok(current)
            } else {
                Err(AppError::Conflict(
                    "pending credential could not be initialized for remote crypto".to_string(),
                ))
            }
        }
    }
}

pub async fn get_pending_credential_audit_summary_for_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    pending_id: &str,
) -> AppResult<PendingCredentialAuditSummary> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    let pending =
        load_active_unexpired_pending_credential(db, node_id, pending_id, Utc::now()).await?;
    Ok(PendingCredentialAuditSummary::from_pending(&pending))
}

pub async fn get_pending_credential_audit_summary_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
) -> AppResult<PendingCredentialAuditSummary> {
    let pending =
        load_active_unexpired_pending_credential(db, node_id, pending_id, Utc::now()).await?;
    Ok(PendingCredentialAuditSummary::from_pending(&pending))
}

pub async fn store_pending_ciphertext_first_writer_wins(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    pending_id: &str,
    input: StorePendingCiphertextInput,
    node_connected: bool,
    now: DateTime<Utc>,
) -> AppResult<StorePendingCiphertextOutcome> {
    if input.ciphertext.len() > MAX_CIPHERTEXT_SIZE {
        return Err(AppError::PendingCredentialCiphertextTooLarge(
            input.ciphertext.len(),
        ));
    }

    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    let pending = load_active_unexpired_pending_credential(db, node_id, pending_id, now).await?;
    if pending_pubkey_missing(&pending) {
        return Err(AppError::PendingCredentialPubkeyAwaiting(
            pending_id.to_string(),
        ));
    }
    if has_ciphertext(&pending) {
        return stored_ciphertext_outcome(pending);
    }

    let state = if node_connected {
        RemoteCryptoState::CiphertextReceived
    } else {
        if active_unexpired_queued_ciphertext_count(db, node_id, now).await?
            >= MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE
        {
            return Ok(StorePendingCiphertextOutcome::QueueFull(
                PendingCredentialAuditSummary::from_pending(&pending),
            ));
        }
        RemoteCryptoState::CiphertextQueued
    };

    let now_bson = bson::DateTime::from_chrono(now);
    let mut set_doc = doc! {
        "crypto.admin_pubkey": input.admin_pubkey.as_str(),
        "crypto.nonce": input.nonce.as_str(),
        "crypto.ciphertext": Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: input.ciphertext.as_slice().to_vec(),
        }),
        "remote_state": remote_state_bson(state.clone())?,
    };
    let mut unset_doc = doc! {};
    if node_connected {
        unset_doc.insert("ciphertext_queued_at", "");
        unset_doc.insert("ciphertext_expires_at", "");
    } else {
        set_doc.insert("ciphertext_queued_at", now_bson);
        set_doc.insert(
            "ciphertext_expires_at",
            bson::DateTime::from_chrono(now + Duration::seconds(OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS)),
        );
    }

    let mut update_doc = doc! { "$set": set_doc };
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }

    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "crypto.node_pubkey": { "$type": "string" },
                "$or": [
                    { "crypto.ciphertext": { "$exists": false } },
                    { "crypto.ciphertext": Bson::Null },
                ],
            },
            update_doc,
        )
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) if node_connected => {
            Ok(StorePendingCiphertextOutcome::StoredForOnlineNode(updated))
        }
        Some(updated) => Ok(StorePendingCiphertextOutcome::QueuedOffline(updated)),
        None => {
            let current =
                load_active_unexpired_pending_credential(db, node_id, pending_id, now).await?;
            if pending_pubkey_missing(&current) {
                Err(AppError::PendingCredentialPubkeyAwaiting(
                    pending_id.to_string(),
                ))
            } else if has_ciphertext(&current) {
                stored_ciphertext_outcome(current)
            } else {
                Err(AppError::PendingCredentialPubkeyAwaiting(
                    pending_id.to_string(),
                ))
            }
        }
    }
}

pub async fn expire_queued_ciphertexts_with_summaries(
    db: &mongodb::Database,
    now: DateTime<Utc>,
) -> AppResult<Vec<PendingCredentialAuditSummary>> {
    let filter = doc! {
        "is_active": true,
        "remote_state": "ciphertext_queued",
        "ciphertext_expires_at": { "$lte": bson::DateTime::from_chrono(now) },
    };
    let summaries: Vec<PendingCredentialAuditSummary> = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(filter.clone())
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .iter()
        .map(PendingCredentialAuditSummary::from_pending)
        .collect();
    let mut summaries = summaries;

    let result = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .update_many(
            filter,
            doc! {
                "$set": {
                    "remote_state": "expired",
                    "is_active": false,
                },
                "$unset": {
                    "crypto.admin_pubkey": "",
                    "crypto.nonce": "",
                    "crypto.ciphertext": "",
                    "ciphertext_queued_at": "",
                    "ciphertext_expires_at": "",
                },
            },
        )
        .await?;

    let modified_count = result.modified_count as usize;
    summaries.truncate(modified_count);

    let fan_out_filter = doc! {
        "is_active": true,
        "fan_out_nodes": {
            "$elemMatch": {
                "remote_state": "ciphertext_queued",
                "ciphertext_expires_at": { "$lte": bson::DateTime::from_chrono(now) },
            }
        }
    };
    let fan_out_pending: Vec<NodePendingCredential> = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(fan_out_filter)
        .await?
        .try_collect()
        .await?;
    for mut pending in fan_out_pending {
        let revision = pending.fan_out_revision;
        let pending_for_summary = pending.clone();
        let mut changed = false;
        let mut local_summaries = Vec::new();
        for target in &mut pending.fan_out_nodes {
            if matches!(
                target.remote_state,
                Some(RemoteCryptoState::CiphertextQueued)
            ) && target
                .ciphertext_expires_at
                .is_some_and(|expires_at| expires_at <= now)
            {
                local_summaries.push(PendingCredentialAuditSummary::from_fan_out_target(
                    &pending_for_summary,
                    target,
                ));
                target.remote_state = Some(RemoteCryptoState::Expired);
                target.error_code = Some(crate::errors::PENDING_CREDENTIAL_NODE_OFFLINE_CODE);
                target.error_kind = Some("pending_credential_node_offline".to_string());
                target.crypto.admin_pubkey = None;
                target.crypto.nonce = None;
                target.crypto.ciphertext = None;
                target.ciphertext_queued_at = None;
                target.ciphertext_expires_at = None;
                target.updated_at = now;
                changed = true;
            }
        }
        if changed {
            pending.remote_state = aggregate_fan_out_remote_state(&pending.fan_out_nodes);
            if matches!(pending.remote_state, Some(RemoteCryptoState::Expired)) {
                pending.is_active = false;
            }
            if let Ok(_updated) =
                update_fan_out_nodes_with_revision(db, pending, revision, true, now).await
            {
                summaries.extend(local_summaries);
            }
        }
    }

    let fan_out_partial_expired_filter = doc! {
        "is_active": true,
        "remote_state": "partial_decrypted",
        "expires_at": { "$lte": bson::DateTime::from_chrono(now) },
        "fan_out_nodes.0": { "$exists": true },
    };
    let fan_out_partial_expired: Vec<NodePendingCredential> = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(fan_out_partial_expired_filter)
        .await?
        .try_collect()
        .await?;
    for mut pending in fan_out_partial_expired {
        let revision = pending.fan_out_revision;
        let pending_for_summary = pending.clone();
        let mut changed = false;
        let mut local_summaries = Vec::new();
        for target in &mut pending.fan_out_nodes {
            if !matches!(
                target.remote_state,
                Some(RemoteCryptoState::Consumed | RemoteCryptoState::Expired)
            ) {
                local_summaries.push(PendingCredentialAuditSummary::from_fan_out_target(
                    &pending_for_summary,
                    target,
                ));
                target.remote_state = Some(RemoteCryptoState::Expired);
                target.error_code = None;
                target.error_kind = None;
                target.crypto.admin_pubkey = None;
                target.crypto.nonce = None;
                target.crypto.ciphertext = None;
                target.ciphertext_queued_at = None;
                target.ciphertext_expires_at = None;
                target.updated_at = now;
                changed = true;
            }
        }
        if changed {
            pending.remote_state = Some(RemoteCryptoState::Expired);
            pending.is_active = false;
            if let Ok(updated) =
                expire_fan_out_nodes_with_revision(db, pending, revision, now).await
            {
                rci_audit_service::log_rci_fan_out_for_node(
                    db.clone(),
                    &updated.owner_user_id,
                    None,
                    None,
                    &rci_audit_service::RciFanOutAuditSubject::from_pending(&updated),
                    rci_audit_service::RciFanOutAuditEventKind::Expired,
                );
                summaries.extend(local_summaries);
            }
        }
    }

    Ok(summaries)
}

pub async fn mark_queued_ciphertext_too_large_after_replay(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "fan_out_nodes.0": { "$exists": false },
                "is_active": true,
                "remote_state": "ciphertext_queued",
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "ciphertext_expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "crypto.ciphertext": { "$exists": true },
            },
            doc! {
                "$set": {
                    "remote_state": remote_state_bson(RemoteCryptoState::DecryptFailed)?,
                    "is_active": false,
                },
                "$unset": {
                    "crypto.admin_pubkey": "",
                    "crypto.nonce": "",
                    "crypto.ciphertext": "",
                    "ciphertext_queued_at": "",
                    "ciphertext_expires_at": "",
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) => Ok(updated),
        None => {
            mark_fan_out_queued_ciphertext_too_large_after_replay(db, node_id, pending_id, now)
                .await
        }
    }
}

async fn mark_fan_out_queued_ciphertext_too_large_after_replay(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let mut pending =
        load_active_unexpired_fan_out_pending_credential(db, node_id, pending_id, now).await?;
    let revision = pending.fan_out_revision;
    let target = fan_out_target(&pending, node_id)
        .ok_or_else(|| AppError::NotFound("Fan-out target not found".to_string()))?;
    if !matches!(
        target.remote_state,
        Some(RemoteCryptoState::CiphertextQueued)
    ) || target
        .ciphertext_expires_at
        .is_none_or(|expires_at| expires_at <= now)
        || target.crypto.ciphertext.is_none()
    {
        return Err(AppError::NotFound(
            "Pending credential not found".to_string(),
        ));
    }
    apply_fan_out_decrypt_result(
        &mut pending,
        node_id,
        PendingCredentialDecryptOutcome::Error,
        Some(crate::errors::PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE),
        now,
    )?;
    update_fan_out_nodes_with_revision(db, pending, revision, true, now).await
}

pub async fn mark_pending_ciphertext_queued_after_send_failure(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "fan_out_nodes.0": { "$exists": false },
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "crypto.admin_pubkey": { "$type": "string" },
                "crypto.nonce": { "$type": "string" },
                "crypto.ciphertext": { "$exists": true },
            },
            doc! {
                "$set": {
                    "remote_state": remote_state_bson(RemoteCryptoState::CiphertextQueued)?,
                    "ciphertext_queued_at": bson::DateTime::from_chrono(now),
                    "ciphertext_expires_at": bson::DateTime::from_chrono(now + Duration::seconds(OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS)),
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) => Ok(updated),
        None => {
            mark_fan_out_ciphertext_queued_after_send_failure(db, node_id, pending_id, now).await
        }
    }
}

pub async fn mark_fan_out_ciphertext_queued_after_send_failure(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let mut pending =
        load_active_unexpired_fan_out_pending_credential(db, node_id, pending_id, now).await?;
    let revision = pending.fan_out_revision;
    let target = fan_out_target_mut(&mut pending, node_id)?;
    if target.crypto.admin_pubkey.is_none()
        || target.crypto.nonce.is_none()
        || target.crypto.ciphertext.is_none()
    {
        return Err(AppError::NotFound(
            "Pending credential not found".to_string(),
        ));
    }
    target.remote_state = Some(RemoteCryptoState::CiphertextQueued);
    target.ciphertext_queued_at = Some(now);
    target.ciphertext_expires_at = Some(now + Duration::seconds(OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS));
    target.updated_at = now;
    pending.remote_state = aggregate_fan_out_remote_state(&pending.fan_out_nodes);
    update_fan_out_nodes_with_revision(db, pending, revision, true, now).await
}

pub async fn mark_queued_ciphertext_sent(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "fan_out_nodes.0": { "$exists": false },
                "is_active": true,
                "remote_state": "ciphertext_queued",
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "ciphertext_expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "crypto.admin_pubkey": { "$type": "string" },
                "crypto.nonce": { "$type": "string" },
                "crypto.ciphertext": { "$exists": true },
            },
            doc! {
                "$set": {
                    "remote_state": remote_state_bson(RemoteCryptoState::CiphertextReceived)?,
                },
                "$unset": {
                    "ciphertext_queued_at": "",
                    "ciphertext_expires_at": "",
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) => Ok(updated),
        None => mark_fan_out_queued_ciphertext_sent(db, node_id, pending_id, now).await,
    }
}

pub async fn mark_fan_out_queued_ciphertext_sent(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let mut pending =
        load_active_unexpired_fan_out_pending_credential(db, node_id, pending_id, now).await?;
    let revision = pending.fan_out_revision;
    let target = fan_out_target_mut(&mut pending, node_id)?;
    if !matches!(
        target.remote_state,
        Some(RemoteCryptoState::CiphertextQueued)
    ) || target
        .ciphertext_expires_at
        .is_none_or(|expires_at| expires_at <= now)
        || target.crypto.admin_pubkey.is_none()
        || target.crypto.nonce.is_none()
        || target.crypto.ciphertext.is_none()
    {
        return Err(AppError::NotFound(
            "Pending credential not found".to_string(),
        ));
    }
    target.remote_state = Some(RemoteCryptoState::CiphertextReceived);
    target.ciphertext_queued_at = None;
    target.ciphertext_expires_at = None;
    target.updated_at = now;
    pending.remote_state = aggregate_fan_out_remote_state(&pending.fan_out_nodes);
    update_fan_out_nodes_with_revision(db, pending, revision, true, now).await
}

pub async fn list_deliverable_queued_ciphertexts_for_node(
    db: &mongodb::Database,
    node_id: &str,
    limit: i64,
    now: DateTime<Utc>,
) -> AppResult<Vec<NodePendingCredential>> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(doc! {
            "is_active": true,
            "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            "$or": [
                {
                    "node_id": node_id,
                    "remote_state": "ciphertext_queued",
                    "ciphertext_expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                    "crypto.version": "v1",
                    "crypto.node_pubkey": { "$type": "string", "$ne": "" },
                    "crypto.admin_pubkey": { "$type": "string" },
                    "crypto.nonce": { "$type": "string" },
                    "crypto.ciphertext": { "$exists": true },
                },
                {
                    "fan_out_nodes": {
                        "$elemMatch": {
                            "node_id": node_id,
                            "remote_state": "ciphertext_queued",
                            "ciphertext_expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                            "crypto.version": "v1",
                            "crypto.node_pubkey": { "$type": "string", "$ne": "" },
                            "crypto.admin_pubkey": { "$type": "string" },
                            "crypto.nonce": { "$type": "string" },
                            "crypto.ciphertext": { "$exists": true },
                        }
                    }
                }
            ],
        })
        .sort(doc! { "ciphertext_queued_at": 1, "created_at": 1 })
        .limit(limit.max(0))
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

pub async fn record_pending_credential_decrypt_result(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    outcome: PendingCredentialDecryptOutcome,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let (state, consumed_at) = match outcome {
        PendingCredentialDecryptOutcome::Ok => (RemoteCryptoState::Consumed, Some(now)),
        PendingCredentialDecryptOutcome::Error => (RemoteCryptoState::DecryptFailed, None),
    };

    let mut set_doc = doc! {
        "remote_state": remote_state_bson(state)?,
        "is_active": false,
    };
    if let Some(consumed_at) = consumed_at {
        set_doc.insert("consumed_at", bson::DateTime::from_chrono(consumed_at));
    }

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "fan_out_nodes.0": { "$exists": false },
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            },
            doc! {
                "$set": set_doc,
                "$unset": {
                    "crypto.admin_pubkey": "",
                    "crypto.nonce": "",
                    "crypto.ciphertext": "",
                    "ciphertext_queued_at": "",
                    "ciphertext_expires_at": "",
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

pub async fn record_fan_out_decrypt_result(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    outcome: PendingCredentialDecryptOutcome,
    error_code: Option<u32>,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let mut pending =
        load_active_unexpired_fan_out_pending_credential(db, node_id, pending_id, now).await?;
    let revision = pending.fan_out_revision;
    apply_fan_out_decrypt_result(&mut pending, node_id, outcome, error_code, now)?;
    update_fan_out_nodes_with_revision(db, pending, revision, true, now).await
}

pub async fn cancel_pending_credential(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;

    // Consume rejects expired pushes because accepting stale setup metadata is
    // correctness-critical. Cancel intentionally remains admin housekeeping:
    // it can deactivate an expired active row so cleanup is idempotent.
    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
            },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

pub async fn consume_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    complete_pending_credential_for_node(db, node_id, pending_id, CompletionKind::Consumed).await
}

pub async fn decline_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    complete_pending_credential_for_node(db, node_id, pending_id, CompletionKind::Declined).await
}

enum CompletionKind {
    Consumed,
    Declined,
}

async fn complete_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    kind: CompletionKind,
) -> AppResult<NodePendingCredential> {
    let now_chrono = Utc::now();
    let now = bson::DateTime::from_chrono(now_chrono);
    let timestamp_field = match kind {
        CompletionKind::Consumed => "consumed_at",
        CompletionKind::Declined => "declined_at",
    };
    let mut set_doc = doc! { "is_active": false };
    set_doc.insert(timestamp_field, now);

    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "fan_out_nodes.0": { "$exists": false },
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now_chrono) },
            },
            doc! { "$set": set_doc },
        )
        .await?;

    match updated {
        Some(updated) => Ok(updated),
        None => {
            complete_fan_out_pending_credential_for_node(db, node_id, pending_id, kind, now_chrono)
                .await
        }
    }
}

async fn complete_fan_out_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    kind: CompletionKind,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let mut pending =
        load_active_unexpired_fan_out_pending_credential(db, node_id, pending_id, now).await?;
    let revision = pending.fan_out_revision;
    match kind {
        CompletionKind::Consumed => {
            apply_fan_out_decrypt_result(
                &mut pending,
                node_id,
                PendingCredentialDecryptOutcome::Ok,
                None,
                now,
            )?;
        }
        CompletionKind::Declined => {
            let target = fan_out_target_mut(&mut pending, node_id)?;
            target.remote_state = Some(RemoteCryptoState::Declined);
            target.decrypt_outcome = Some(FanOutDecryptOutcome::Error);
            target.declined_at = Some(now);
            target.crypto.admin_pubkey = None;
            target.crypto.nonce = None;
            target.crypto.ciphertext = None;
            target.ciphertext_queued_at = None;
            target.ciphertext_expires_at = None;
            target.updated_at = now;
            recompute_fan_out_completion(&mut pending, now);
        }
    }
    update_fan_out_nodes_with_revision(db, pending, revision, true, now).await
}

async fn load_active_unexpired_pending_credential(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one(doc! {
            "_id": pending_id,
            "node_id": node_id,
            "is_active": true,
            "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

async fn load_active_unexpired_pending_by_id(
    db: &mongodb::Database,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one(doc! {
            "_id": pending_id,
            "is_active": true,
            "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

async fn load_active_unexpired_fan_out_pending_credential(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one(doc! {
            "_id": pending_id,
            "is_active": true,
            "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            "fan_out_nodes.node_id": node_id,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

async fn active_unexpired_queued_ciphertext_count(
    db: &mongodb::Database,
    node_id: &str,
    now: DateTime<Utc>,
) -> AppResult<u64> {
    let single_count = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .count_documents(doc! {
            "node_id": node_id,
            "is_active": true,
            "remote_state": "ciphertext_queued",
            "ciphertext_expires_at": { "$gt": bson::DateTime::from_chrono(now) },
        })
        .await?;
    let fan_out_pending: Vec<NodePendingCredential> = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(doc! {
            "is_active": true,
            "fan_out_nodes": {
                "$elemMatch": {
                    "node_id": node_id,
                    "remote_state": "ciphertext_queued",
                    "ciphertext_expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                }
            }
        })
        .await?
        .try_collect()
        .await?;
    let fan_out_count = fan_out_pending
        .iter()
        .flat_map(|pending| pending.fan_out_nodes.iter())
        .filter(|target| {
            target.node_id == node_id
                && matches!(
                    target.remote_state,
                    Some(RemoteCryptoState::CiphertextQueued)
                )
                && target
                    .ciphertext_expires_at
                    .is_some_and(|expires_at| expires_at > now)
        })
        .count() as u64;
    Ok(single_count + fan_out_count)
}

async fn preflight_no_active_pending_for_targets(
    db: &mongodb::Database,
    service_slug: &str,
    node_ids: &[String],
) -> AppResult<()> {
    let node_id_array: bson::Array = node_ids.iter().cloned().map(Bson::String).collect();
    let existing = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one(doc! {
            "service_slug": service_slug,
            "is_active": true,
            "$or": [
                { "node_id": { "$in": node_id_array.clone() } },
                { "fan_out_nodes.node_id": { "$in": node_id_array } },
            ],
        })
        .await?;
    if let Some(existing) = existing {
        return Err(AppError::Conflict(format!(
            "A pending credential already exists for service '{}' on one or more target nodes (id: {})",
            service_slug, existing.id
        )));
    }
    Ok(())
}

async fn ensure_actor_can_manage_pending_owner(
    db: &mongodb::Database,
    actor_user_id: &str,
    pending: &NodePendingCredential,
) -> AppResult<()> {
    let access = crate::services::org_service::resolve_owner_access(
        db,
        actor_user_id,
        &pending.owner_user_id,
    )
    .await?;
    if access.can_write() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Not allowed to manage this pending credential".to_string(),
        ))
    }
}

fn fan_out_target_mut<'a>(
    pending: &'a mut NodePendingCredential,
    node_id: &str,
) -> AppResult<&'a mut FanOutNodeState> {
    pending
        .fan_out_nodes
        .iter_mut()
        .find(|target| target.node_id == node_id)
        .ok_or_else(|| AppError::NotFound("Fan-out target not found".to_string()))
}

pub fn fan_out_target<'a>(
    pending: &'a NodePendingCredential,
    node_id: &str,
) -> Option<&'a FanOutNodeState> {
    pending
        .fan_out_nodes
        .iter()
        .find(|target| target.node_id == node_id)
}

fn fan_out_target_awaiting_ciphertext(target: &FanOutNodeState) -> bool {
    matches!(target.remote_state, Some(RemoteCryptoState::PubkeyPosted))
        && !target.crypto.node_pubkey.is_empty()
        && target.crypto.admin_pubkey.is_none()
        && target.crypto.nonce.is_none()
        && target.crypto.ciphertext.is_none()
}

fn validate_fan_out_ciphertext_input_sizes(input: &StoreFanOutCiphertextsInput) -> AppResult<()> {
    let mut total = 0usize;
    for item in &input.items {
        if item.version != "v1" {
            return Err(AppError::PendingCredentialVersionUnsupported(
                item.version.clone(),
            ));
        }
        let len = item.ciphertext.len();
        if len > MAX_CIPHERTEXT_SIZE {
            return Err(AppError::PendingCredentialCiphertextTooLarge(len));
        }
        total = total.saturating_add(len);
    }
    if total > MAX_FAN_OUT_CIPHERTEXT_TOTAL_SIZE {
        return Err(AppError::PendingCredentialCiphertextTooLarge(total));
    }
    Ok(())
}

fn validate_exact_fan_out_item_set(
    ready_targets: &[&FanOutNodeState],
    items: &[StoreFanOutCiphertextItemInput],
) -> AppResult<()> {
    if ready_targets.len() != items.len() {
        return Err(AppError::Conflict(
            "fan-out ciphertext item set does not match ready targets".to_string(),
        ));
    }
    let expected: HashMap<&str, i64> = ready_targets
        .iter()
        .map(|target| (target.node_id.as_str(), target.generation))
        .collect();
    let mut seen = HashSet::new();
    for item in items {
        if !seen.insert(item.node_id.as_str()) {
            return Err(AppError::Conflict(
                "fan-out ciphertext item contains duplicate node_id".to_string(),
            ));
        }
        match expected.get(item.node_id.as_str()) {
            Some(generation) if *generation == item.generation => {}
            Some(_) => {
                return Err(AppError::Conflict(
                    "fan-out ciphertext item generation is stale".to_string(),
                ));
            }
            None => {
                return Err(AppError::Conflict(
                    "fan-out ciphertext item does not match a ready target".to_string(),
                ));
            }
        }
    }
    Ok(())
}

async fn update_fan_out_nodes_with_revision(
    db: &mongodb::Database,
    pending: NodePendingCredential,
    expected_revision: i64,
    increment_revision: bool,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let fan_out_nodes = bson::to_bson(&pending.fan_out_nodes)
        .map_err(|err| AppError::Internal(format!("fan_out_nodes serialization failed: {err}")))?;
    let mut set_doc = doc! {
        "fan_out_nodes": fan_out_nodes,
        "remote_state": remote_state_option_bson(pending.remote_state.clone())?,
        "is_active": pending.is_active,
    };
    if let Some(consumed_at) = pending.consumed_at {
        set_doc.insert("consumed_at", bson::DateTime::from_chrono(consumed_at));
    }
    if let Some(declined_at) = pending.declined_at {
        set_doc.insert("declined_at", bson::DateTime::from_chrono(declined_at));
    }
    let mut unset_doc = doc! {};
    if pending.consumed_at.is_none() {
        unset_doc.insert("consumed_at", "");
    }
    if pending.declined_at.is_none() {
        unset_doc.insert("declined_at", "");
    }

    let mut update_doc = doc! { "$set": set_doc };
    if increment_revision {
        update_doc.insert("$inc", doc! { "fan_out_revision": 1 });
    }
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }

    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": &pending.id,
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                "fan_out_revision": expected_revision,
            },
            update_doc,
        )
        .return_document(ReturnDocument::After)
        .await?;

    updated.ok_or_else(|| AppError::Conflict("stale fan_out_revision".to_string()))
}

async fn refresh_fan_out_remote_state(
    db: &mongodb::Database,
    mut pending: NodePendingCredential,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let remote_state = aggregate_fan_out_remote_state(&pending.fan_out_nodes);
    if pending.remote_state == remote_state {
        return Ok(pending);
    }

    let updated = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": &pending.id,
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            },
            doc! {
                "$set": {
                    "remote_state": remote_state_option_bson(remote_state.clone())?,
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;

    match updated {
        Some(updated) => Ok(updated),
        None => {
            pending.remote_state = remote_state;
            Ok(pending)
        }
    }
}

async fn expire_fan_out_nodes_with_revision(
    db: &mongodb::Database,
    pending: NodePendingCredential,
    expected_revision: i64,
    now: DateTime<Utc>,
) -> AppResult<NodePendingCredential> {
    let fan_out_nodes = bson::to_bson(&pending.fan_out_nodes)
        .map_err(|err| AppError::Internal(format!("fan_out_nodes serialization failed: {err}")))?;
    let mut unset_doc = doc! {};
    if pending.consumed_at.is_none() {
        unset_doc.insert("consumed_at", "");
    }
    if pending.declined_at.is_none() {
        unset_doc.insert("declined_at", "");
    }

    let mut update_doc = doc! {
        "$set": {
            "fan_out_nodes": fan_out_nodes,
            "remote_state": remote_state_option_bson(pending.remote_state.clone())?,
            "is_active": false,
        },
    };
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": &pending.id,
                "is_active": true,
                "expires_at": { "$lte": bson::DateTime::from_chrono(now) },
                "fan_out_revision": expected_revision,
            },
            update_doc,
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("stale fan_out_revision".to_string()))
}

fn aggregate_fan_out_remote_state(nodes: &[FanOutNodeState]) -> Option<RemoteCryptoState> {
    if nodes.is_empty() {
        return None;
    }
    if nodes
        .iter()
        .all(|target| matches!(target.remote_state, Some(RemoteCryptoState::Consumed)))
    {
        return Some(RemoteCryptoState::Consumed);
    }
    if nodes
        .iter()
        .all(|target| matches!(target.remote_state, Some(RemoteCryptoState::Declined)))
    {
        return Some(RemoteCryptoState::Declined);
    }
    if nodes
        .iter()
        .any(|target| matches!(target.remote_state, Some(RemoteCryptoState::Expired)))
        && nodes.iter().all(|target| {
            matches!(
                target.remote_state,
                Some(
                    RemoteCryptoState::Consumed
                        | RemoteCryptoState::Declined
                        | RemoteCryptoState::DecryptFailed
                        | RemoteCryptoState::Expired
                )
            )
        })
    {
        return Some(RemoteCryptoState::Expired);
    }
    if nodes
        .iter()
        .any(|target| matches!(target.remote_state, Some(RemoteCryptoState::Consumed)))
        && nodes
            .iter()
            .any(|target| !matches!(target.remote_state, Some(RemoteCryptoState::Consumed)))
    {
        return Some(RemoteCryptoState::PartialDecrypted);
    }
    if nodes
        .iter()
        .any(|target| matches!(target.remote_state, Some(RemoteCryptoState::DecryptFailed)))
    {
        return Some(RemoteCryptoState::PartialDecrypted);
    }
    if nodes.iter().all(|target| {
        matches!(
            target.remote_state,
            Some(RemoteCryptoState::CiphertextReceived)
        )
    }) {
        return Some(RemoteCryptoState::CiphertextReceived);
    }
    if nodes.iter().any(|target| {
        matches!(
            target.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        )
    }) {
        return Some(RemoteCryptoState::CiphertextQueued);
    }
    if nodes
        .iter()
        .all(|target| matches!(target.remote_state, Some(RemoteCryptoState::PubkeyPosted)))
    {
        return Some(RemoteCryptoState::PubkeyPosted);
    }
    None
}

fn apply_fan_out_decrypt_result(
    pending: &mut NodePendingCredential,
    node_id: &str,
    outcome: PendingCredentialDecryptOutcome,
    error_code: Option<u32>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let target = fan_out_target_mut(pending, node_id)?;
    match outcome {
        PendingCredentialDecryptOutcome::Ok => {
            target.remote_state = Some(RemoteCryptoState::Consumed);
            target.decrypt_outcome = Some(FanOutDecryptOutcome::Ok);
            target.error_code = None;
            target.error_kind = None;
            target.consumed_at = Some(now);
        }
        PendingCredentialDecryptOutcome::Error => {
            target.remote_state = Some(RemoteCryptoState::DecryptFailed);
            target.decrypt_outcome = Some(FanOutDecryptOutcome::Error);
            target.error_code = error_code;
            target.error_kind = error_code
                .and_then(fan_out_error_kind_name)
                .map(str::to_string);
        }
    }
    target.crypto.admin_pubkey = None;
    target.crypto.nonce = None;
    target.crypto.ciphertext = None;
    target.ciphertext_queued_at = None;
    target.ciphertext_expires_at = None;
    target.updated_at = now;
    recompute_fan_out_completion(pending, now);
    Ok(())
}

fn recompute_fan_out_completion(pending: &mut NodePendingCredential, now: DateTime<Utc>) {
    pending.remote_state = aggregate_fan_out_remote_state(&pending.fan_out_nodes);
    match pending.remote_state {
        Some(RemoteCryptoState::Consumed) => {
            pending.is_active = false;
            pending.consumed_at = Some(now);
        }
        Some(RemoteCryptoState::Declined) => {
            pending.is_active = false;
            pending.declined_at = Some(now);
        }
        Some(RemoteCryptoState::Expired) => {
            pending.is_active = false;
        }
        _ => {
            pending.is_active = true;
        }
    }
}

fn fan_out_error_kind_name(code: u32) -> Option<&'static str> {
    match code {
        crate::errors::PENDING_CREDENTIAL_DECRYPT_FAILED_CODE => {
            Some("pending_credential_decrypt_failed")
        }
        crate::errors::PENDING_CREDENTIAL_VERSION_UNSUPPORTED_CODE => {
            Some("pending_credential_version_unsupported")
        }
        crate::errors::PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE => {
            Some("pending_credential_ciphertext_too_large")
        }
        crate::errors::PENDING_CREDENTIAL_PUBKEY_AWAITING_CODE => {
            Some("pending_credential_pubkey_awaiting")
        }
        crate::errors::PENDING_CREDENTIAL_NODE_OFFLINE_CODE => {
            Some("pending_credential_node_offline")
        }
        crate::errors::PENDING_CREDENTIAL_QUEUE_FULL_CODE => Some("pending_credential_queue_full"),
        _ => None,
    }
}

fn fan_out_statuses(pending: &NodePendingCredential) -> Vec<FanOutTargetStatus> {
    pending
        .fan_out_nodes
        .iter()
        .map(|target| FanOutTargetStatus {
            node_id: target.node_id.clone(),
            generation: target.generation,
            remote_state: target.remote_state.clone(),
            error_code: target.error_code,
            error_kind: target.error_kind.clone(),
            delivery_status: match target.remote_state {
                Some(RemoteCryptoState::CiphertextReceived) => Some(FanOutDeliveryStatus::Sent),
                Some(RemoteCryptoState::CiphertextQueued) => Some(FanOutDeliveryStatus::Queued),
                _ => None,
            },
        })
        .collect()
}

fn has_ciphertext(pending: &NodePendingCredential) -> bool {
    pending
        .crypto
        .as_ref()
        .and_then(|crypto| crypto.ciphertext.as_ref())
        .is_some()
}

fn pending_pubkey_missing(pending: &NodePendingCredential) -> bool {
    match pending.crypto.as_ref() {
        Some(crypto) => crypto.node_pubkey.is_empty(),
        None => true,
    }
}

fn stored_ciphertext_outcome(
    pending: NodePendingCredential,
) -> AppResult<StorePendingCiphertextOutcome> {
    if matches!(
        pending.remote_state.as_ref(),
        Some(RemoteCryptoState::CiphertextQueued)
    ) {
        Ok(StorePendingCiphertextOutcome::QueuedOffline(pending))
    } else {
        Ok(StorePendingCiphertextOutcome::StoredForOnlineNode(pending))
    }
}

fn remote_state_bson(state: RemoteCryptoState) -> AppResult<Bson> {
    bson::to_bson(&state)
        .map_err(|err| AppError::Internal(format!("remote state serialization failed: {err}")))
}

fn remote_state_option_bson(state: Option<RemoteCryptoState>) -> AppResult<Bson> {
    match state {
        Some(state) => remote_state_bson(state),
        None => Ok(Bson::Null),
    }
}

fn validate_service_slug(slug: &str) -> AppResult<()> {
    if slug.is_empty() || slug.len() > 64 {
        return Err(AppError::ValidationError(
            "service_slug must be 1-64 characters".to_string(),
        ));
    }
    let valid = slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && slug
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !valid {
        return Err(AppError::ValidationError(
            "service_slug must be lowercase alphanumeric with optional hyphens, and cannot start or end with hyphen".to_string(),
        ));
    }
    Ok(())
}

fn validate_field_name(field_name: &str, injection_method: &InjectionMethod) -> AppResult<()> {
    if field_name.is_empty() || field_name.len() > 128 {
        return Err(AppError::ValidationError(
            "field_name must be 1-128 characters".to_string(),
        ));
    }

    match injection_method {
        InjectionMethod::Header => {
            for ch in field_name.chars() {
                if !is_http_token_char(ch) {
                    return Err(disallowed_field_char_error("header", ch));
                }
            }
        }
        InjectionMethod::QueryParam => {
            validate_percent_encoding(field_name, "query-param")?;
            for ch in field_name.chars() {
                if ch == '%' {
                    continue;
                }
                if !is_rfc3986_unreserved(ch) {
                    return Err(disallowed_field_char_error("query-param", ch));
                }
            }
        }
        InjectionMethod::PathPrefix => {
            validate_percent_encoding(field_name, "path-prefix")?;
            for ch in field_name.chars() {
                if ch == '%' {
                    continue;
                }
                if ch.is_control() || ch.is_whitespace() || matches!(ch, '?' | '#') {
                    return Err(disallowed_field_char_error("path-prefix", ch));
                }
                if !ch.is_ascii() {
                    return Err(disallowed_field_char_error("path-prefix", ch));
                }
            }
        }
    }

    Ok(())
}

fn is_http_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '.'
                | '^'
                | '_'
                | '`'
                | '|'
                | '~'
        )
}

fn is_rfc3986_unreserved(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~')
}

fn validate_percent_encoding(value: &str, method: &str) -> AppResult<()> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let valid = index + 2 < bytes.len()
                && bytes[index + 1].is_ascii_hexdigit()
                && bytes[index + 2].is_ascii_hexdigit();
            if !valid {
                return Err(AppError::ValidationError(format!(
                    "field_name for {method} contains invalid percent-encoding"
                )));
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn disallowed_field_char_error(method: &str, ch: char) -> AppError {
    let display = match ch {
        ' ' => "space".to_string(),
        '\t' => "tab".to_string(),
        '\n' => "newline".to_string(),
        '\r' => "carriage return".to_string(),
        _ => ch.to_string(),
    };
    AppError::ValidationError(format!(
        "field_name for {method} contains disallowed character '{display}'"
    ))
}

fn clean_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::node_service_binding::{
        COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
    };
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::node_service;
    use crate::test_utils::{connect_test_database, test_membership, test_user};

    async fn test_db(prefix: &str) -> mongodb::Database {
        connect_test_database(prefix)
            .await
            .expect("local MongoDB required for pending credential tests")
    }

    fn test_node(owner_id: &str, name: &str) -> Node {
        let now = Utc::now();
        Node {
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
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn credential_input(service_slug: &str) -> CreatePendingCredentialInput {
        CreatePendingCredentialInput {
            service_slug: service_slug.to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: Some("Production".to_string()),
            ttl_secs: 86_400,
            remote_crypto: false,
        }
    }

    fn remote_credential_input(service_slug: &str) -> CreatePendingCredentialInput {
        CreatePendingCredentialInput {
            remote_crypto: true,
            ..credential_input(service_slug)
        }
    }

    fn ciphertext_input(
        admin_pubkey: impl Into<String>,
        nonce: impl Into<String>,
        ciphertext: Vec<u8>,
    ) -> StorePendingCiphertextInput {
        StorePendingCiphertextInput::new(admin_pubkey.into(), nonce.into(), ciphertext)
    }

    fn fan_out_item(
        node_id: &str,
        generation: i64,
        ciphertext: Vec<u8>,
    ) -> StoreFanOutCiphertextItemInput {
        StoreFanOutCiphertextItemInput::new(
            node_id.to_string(),
            generation,
            "v1".to_string(),
            "admin-pubkey".to_string(),
            "nonce".to_string(),
            ciphertext,
        )
    }

    fn fan_out_node_state(
        node_id: &str,
        generation: i64,
        remote_state: Option<RemoteCryptoState>,
    ) -> FanOutNodeState {
        let now = Utc::now();
        FanOutNodeState {
            node_id: node_id.to_string(),
            generation,
            crypto: crate::models::node_pending_credential::CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: "node-pubkey".to_string(),
                admin_pubkey: None,
                nonce: None,
                ciphertext: None,
            },
            remote_state,
            decrypt_outcome: None,
            error_code: None,
            error_kind: None,
            pubkey_posted_at: Some(now),
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            consumed_at: None,
            declined_at: None,
            updated_at: now,
        }
    }

    async fn insert_users(db: &mongodb::Database, users: Vec<User>) {
        db.collection::<User>(USERS)
            .insert_many(users)
            .await
            .expect("insert users");
    }

    async fn insert_membership(db: &mongodb::Database, membership: OrgMembership) {
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(membership)
            .await
            .expect("insert membership");
    }

    async fn insert_node(db: &mongodb::Database, node: &Node) {
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");
    }

    async fn load_pending(db: &mongodb::Database, pending_id: &str) -> NodePendingCredential {
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .find_one(doc! { "_id": pending_id })
            .await
            .expect("query pending credential")
            .expect("pending credential exists")
    }

    async fn insert_fan_out_pending(
        db: &mongodb::Database,
        owner_id: &str,
        node_ids: &[String],
        service_slug: &str,
    ) -> NodePendingCredential {
        let now = Utc::now();
        let pending = NodePendingCredential {
            id: Uuid::new_v4().to_string(),
            node_id: node_ids.first().expect("at least one node").clone(),
            service_slug: service_slug.to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: Some("Production".to_string()),
            created_by_user_id: owner_id.to_string(),
            owner_user_id: owner_id.to_string(),
            created_at: now,
            expires_at: now + Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: None,
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: node_ids
                .iter()
                .map(|node_id| FanOutNodeState {
                    node_id: node_id.clone(),
                    generation: 0,
                    crypto: CryptoBundle {
                        version: "v1".to_string(),
                        node_pubkey: String::new(),
                        admin_pubkey: None,
                        nonce: None,
                        ciphertext: None,
                    },
                    remote_state: None,
                    decrypt_outcome: None,
                    error_code: None,
                    error_kind: None,
                    pubkey_posted_at: None,
                    ciphertext_queued_at: None,
                    ciphertext_expires_at: None,
                    consumed_at: None,
                    declined_at: None,
                    updated_at: now,
                })
                .collect(),
            fan_out_revision: 1,
        };
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .insert_one(&pending)
            .await
            .expect("insert fan-out pending credential");
        pending
    }

    fn target<'a>(pending: &'a NodePendingCredential, node_id: &str) -> &'a FanOutNodeState {
        fan_out_target(pending, node_id).expect("fan-out target exists")
    }

    async fn record_fan_out_pubkeys(
        db: &mongodb::Database,
        pending: &NodePendingCredential,
        node_ids: &[String],
    ) -> NodePendingCredential {
        for (index, node_id) in node_ids.iter().enumerate() {
            record_fan_out_pubkey(
                db,
                node_id,
                &pending.id,
                "v1",
                &format!("node-pubkey-{index}"),
            )
            .await
            .expect("record fan-out pubkey");
        }
        load_pending(db, &pending.id).await
    }

    fn fan_out_ciphertext_input(
        revision: i64,
        items: Vec<StoreFanOutCiphertextItemInput>,
        online_node_ids: &[&str],
    ) -> StoreFanOutCiphertextsInput {
        StoreFanOutCiphertextsInput {
            fan_out_revision: revision,
            items,
            online_node_ids: online_node_ids
                .iter()
                .map(|node_id| (*node_id).to_string())
                .collect(),
        }
    }

    fn test_binding(owner_id: &str, node_id: &str, service_id: &str) -> NodeServiceBinding {
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

    fn assert_pubkey_only_pending(pending: &NodePendingCredential, expected_node_pubkey: &str) {
        assert!(pending.is_active);
        assert_eq!(pending.remote_state, Some(RemoteCryptoState::PubkeyPosted));
        assert!(pending.ciphertext_queued_at.is_none());
        assert!(pending.ciphertext_expires_at.is_none());
        let crypto = pending.crypto.as_ref().expect("crypto metadata");
        assert_eq!(crypto.version, "v1");
        assert_eq!(crypto.node_pubkey, expected_node_pubkey);
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());
    }

    fn assert_invalid_field_name(method: InjectionMethod, field_name: &str, expected: &str) {
        let err = validate_field_name(field_name, &method).expect_err("field name should fail");
        assert!(
            matches!(err, AppError::ValidationError(ref message) if message.contains(expected)),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn store_pending_ciphertext_input_debug_redacts_material() {
        let input = ciphertext_input("admin-pubkey-secret", "nonce-secret", vec![1, 2, 3]);

        let debug = format!("{input:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("admin-pubkey-secret"));
        assert!(!debug.contains("nonce-secret"));
        assert!(!debug.contains("[1, 2, 3]"));
    }

    #[test]
    fn fan_out_ciphertext_size_caps_are_enforced_per_element_and_aggregate() {
        let per_element = StoreFanOutCiphertextsInput {
            fan_out_revision: 1,
            items: vec![fan_out_item("node-1", 0, vec![0; MAX_CIPHERTEXT_SIZE + 1])],
            online_node_ids: HashSet::new(),
        };
        assert!(matches!(
            validate_fan_out_ciphertext_input_sizes(&per_element),
            Err(AppError::PendingCredentialCiphertextTooLarge(size))
                if size == MAX_CIPHERTEXT_SIZE + 1
        ));

        let aggregate = StoreFanOutCiphertextsInput {
            fan_out_revision: 1,
            items: (0..MAX_FAN_OUT_TARGETS)
                .map(|idx| fan_out_item(&format!("node-{idx}"), 0, vec![0; MAX_CIPHERTEXT_SIZE]))
                .chain(std::iter::once(fan_out_item("overflow", 0, vec![1])))
                .collect(),
            online_node_ids: HashSet::new(),
        };
        assert!(matches!(
            validate_fan_out_ciphertext_input_sizes(&aggregate),
            Err(AppError::PendingCredentialCiphertextTooLarge(size))
                if size == MAX_FAN_OUT_CIPHERTEXT_TOTAL_SIZE + 1
        ));
    }

    #[test]
    fn fan_out_exact_item_set_rejects_stale_generation_and_duplicates() {
        let ready_a = fan_out_node_state("node-a", 2, Some(RemoteCryptoState::PubkeyPosted));
        let ready_b = fan_out_node_state("node-b", 0, Some(RemoteCryptoState::PubkeyPosted));
        let ready = vec![&ready_a, &ready_b];

        assert!(matches!(
            validate_exact_fan_out_item_set(
                &ready,
                &[fan_out_item("node-a", 1, vec![1]), fan_out_item("node-b", 0, vec![1])]
            ),
            Err(AppError::Conflict(message)) if message.contains("generation")
        ));
        assert!(matches!(
            validate_exact_fan_out_item_set(
                &ready,
                &[fan_out_item("node-a", 2, vec![1]), fan_out_item("node-a", 2, vec![1])]
            ),
            Err(AppError::Conflict(message)) if message.contains("duplicate")
        ));
    }

    #[test]
    fn fan_out_aggregate_states_cover_all_consumed_partial_and_expired() {
        assert_eq!(
            aggregate_fan_out_remote_state(&[
                fan_out_node_state("node-a", 0, Some(RemoteCryptoState::Consumed)),
                fan_out_node_state("node-b", 0, Some(RemoteCryptoState::Consumed)),
            ]),
            Some(RemoteCryptoState::Consumed)
        );
        assert_eq!(
            aggregate_fan_out_remote_state(&[
                fan_out_node_state("node-a", 0, Some(RemoteCryptoState::Consumed)),
                fan_out_node_state("node-b", 0, Some(RemoteCryptoState::DecryptFailed)),
            ]),
            Some(RemoteCryptoState::PartialDecrypted)
        );
        assert_eq!(
            aggregate_fan_out_remote_state(&[
                fan_out_node_state("node-a", 0, Some(RemoteCryptoState::Consumed)),
                fan_out_node_state("node-b", 0, Some(RemoteCryptoState::Expired)),
            ]),
            Some(RemoteCryptoState::Expired)
        );
    }

    #[test]
    fn store_pending_ciphertext_outcome_debug_redacts_pending_crypto() {
        let now = Utc::now();
        let pending = NodePendingCredential {
            id: "pending-id".to_string(),
            node_id: "node-id".to_string(),
            service_slug: "openclaw".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-id".to_string(),
            owner_user_id: "user-id".to_string(),
            created_at: now,
            expires_at: now + Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: Some(crate::models::node_pending_credential::CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: "node-pubkey-secret".to_string(),
                admin_pubkey: Some("admin-pubkey-secret".to_string()),
                nonce: Some("nonce-secret".to_string()),
                ciphertext: Some(vec![1, 2, 3]),
            }),
            remote_state: Some(RemoteCryptoState::CiphertextQueued),
            ciphertext_queued_at: Some(now),
            ciphertext_expires_at: Some(now + Duration::minutes(15)),
            is_active: true,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };

        let debug = format!(
            "{:?}",
            StorePendingCiphertextOutcome::QueuedOffline(pending)
        );

        assert!(debug.contains("pending-id"));
        assert!(!debug.contains("node-pubkey-secret"));
        assert!(!debug.contains("admin-pubkey-secret"));
        assert!(!debug.contains("nonce-secret"));
        assert!(!debug.contains("[1, 2, 3]"));
    }

    #[test]
    fn validates_header_field_name_as_http_token() {
        validate_field_name("X-API-Key", &InjectionMethod::Header).expect("valid header");
        validate_field_name("X_Custom!#$%&'*+-.^`|~", &InjectionMethod::Header)
            .expect("valid token chars");

        assert_invalid_field_name(InjectionMethod::Header, "X API Key", "space");
        assert_invalid_field_name(InjectionMethod::Header, "X:API-Key", ":");
        assert_invalid_field_name(InjectionMethod::Header, "X,API-Key", ",");
        assert_invalid_field_name(InjectionMethod::Header, "X-ÄPI-Key", "Ä");
    }

    #[test]
    fn validates_query_param_field_name_as_url_safe() {
        validate_field_name("api_key", &InjectionMethod::QueryParam).expect("valid param");
        validate_field_name("api-key.%7E", &InjectionMethod::QueryParam)
            .expect("valid percent-encoded param");

        assert_invalid_field_name(InjectionMethod::QueryParam, "api key", "space");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api&key", "&");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api=key", "=");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api?key", "?");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api#key", "#");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api%key", "percent-encoding");
    }

    #[test]
    fn validates_path_prefix_field_name_as_path_component() {
        validate_field_name("v1/api/%2Ftenant", &InjectionMethod::PathPrefix)
            .expect("valid path prefix");

        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/api key", "space");
        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/api?key", "?");
        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/api#key", "#");
        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/%key", "percent-encoding");
    }

    #[tokio::test]
    async fn admin_push_creates_pending_credential_with_acl_fields() {
        let db = test_db("pending_credential_push").await;

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &admin_id, OrgRole::Admin, None),
        )
        .await;
        let node = test_node(&org_id, "org-node");
        insert_node(&db, &node).await;

        let pending =
            create_pending_credential(&db, &admin_id, &node.id, credential_input("openclaw"))
                .await
                .expect("admin can push");

        assert_eq!(pending.node_id, node.id);
        assert_eq!(pending.service_slug, "openclaw");
        assert_eq!(pending.created_by_user_id, admin_id);
        assert_eq!(pending.owner_user_id, org_id);
        assert!(pending.is_active);

        let listed = list_pending_credentials_for_admin(&db, &admin_id, &node.id, false)
            .await
            .expect("admin can list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, pending.id);
    }

    #[tokio::test]
    async fn create_remote_crypto_false_keeps_legacy_crypto_none() {
        let db = test_db("pending_credential_remote_false").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;

        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        assert!(pending.crypto.is_none());
        assert!(pending.remote_state.is_none());
    }

    #[tokio::test]
    async fn init_pending_remote_crypto_upgrades_legacy_metadata_only() {
        let db = test_db("pending_credential_init_legacy_remote").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;

        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("legacy push succeeds");
        assert!(pending.crypto.is_none());
        assert!(pending.remote_state.is_none());

        let upgraded = init_pending_remote_crypto_for_admin(&db, &actor_id, &node.id, &pending.id)
            .await
            .expect("metadata-only init succeeds");

        let crypto = upgraded.crypto.expect("crypto metadata initialized");
        assert_eq!(crypto.version, "v1");
        assert!(crypto.node_pubkey.is_empty());
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());
        assert_eq!(
            upgraded.remote_state,
            Some(RemoteCryptoState::PubkeyAwaiting)
        );

        let stored = get_pending_credential_for_admin(&db, &actor_id, &node.id, &pending.id)
            .await
            .expect("stored pending remains readable");
        assert!(stored.crypto.is_some());
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::PubkeyAwaiting));
    }

    #[tokio::test]
    async fn init_pending_remote_crypto_rejects_fan_out_without_initializing() {
        let db = test_db("pending_credential_init_fanout_rejected").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let first = test_node(&actor_id, "fanout-init-first");
        let second = test_node(&actor_id, "fanout-init-second");
        insert_node(&db, &first).await;
        insert_node(&db, &second).await;
        let pending = insert_fan_out_pending(
            &db,
            &actor_id,
            &[first.id.clone(), second.id.clone()],
            "fanout-init",
        )
        .await;
        assert!(pending.crypto.is_none());
        assert!(pending.remote_state.is_none());

        let err = init_pending_remote_crypto_for_admin(&db, &actor_id, &first.id, &pending.id)
            .await
            .expect_err("fan-out init path is deferred");

        assert!(matches!(
            err,
            AppError::ValidationError(message)
                if message == "fan-out pending credential injection is not supported by this command"
        ));
        let stored = load_pending(&db, &pending.id).await;
        assert!(stored.crypto.is_none());
        assert!(stored.remote_state.is_none());
        assert_eq!(stored.fan_out_nodes.len(), 2);
        assert!(
            stored
                .fan_out_nodes
                .iter()
                .all(|target| target.remote_state.is_none()
                    && target.crypto.node_pubkey.is_empty()
                    && target.crypto.admin_pubkey.is_none()
                    && target.crypto.nonce.is_none()
                    && target.crypto.ciphertext.is_none())
        );
    }

    #[tokio::test]
    async fn create_remote_crypto_true_initializes_v1_without_pubkey() {
        let db = test_db("pending_credential_remote_true").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;

        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("remote push succeeds");

        let crypto = pending.crypto.expect("remote crypto metadata");
        assert_eq!(crypto.version, "v1");
        assert!(crypto.node_pubkey.is_empty());
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());
        assert!(pending.remote_state.is_none());
    }

    #[tokio::test]
    async fn member_cannot_push_pending_credential_for_org_node() {
        let db = test_db("pending_credential_member_denied").await;

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &member_id, OrgRole::Member, None),
        )
        .await;
        let node = test_node(&org_id, "org-node");
        insert_node(&db, &node).await;

        let err =
            create_pending_credential(&db, &member_id, &node.id, credential_input("openclaw"))
                .await
                .expect_err("member cannot push");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn push_for_nonexistent_node_returns_not_found() {
        let db = test_db("pending_credential_missing_node").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;

        let err = create_pending_credential(
            &db,
            &actor_id,
            &Uuid::new_v4().to_string(),
            credential_input("openclaw"),
        )
        .await
        .expect_err("missing node should fail");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn duplicate_pending_slug_returns_conflict_with_existing_id() {
        let db = test_db("pending_credential_duplicate").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;

        let first =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("first push succeeds");
        let err = create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
            .await
            .expect_err("duplicate push should fail");

        match err {
            AppError::Conflict(message) => {
                assert!(message.contains(&first.id));
            }
            other => panic!("expected conflict, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn push_accepts_internal_target_url() {
        let db = test_db("pending_credential_internal_url").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let mut input = credential_input("openclaw");
        input.target_url = Some("http://127.0.0.1:8080".to_string());

        let pending = create_pending_credential(&db, &actor_id, &node.id, input)
            .await
            .expect("internal URL is node-local advisory metadata");
        assert_eq!(pending.target_url.as_deref(), Some("http://127.0.0.1:8080"));
    }

    #[tokio::test]
    async fn node_consumes_own_pending_credential() {
        let db = test_db("pending_credential_consume").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let returned = consume_pending_credential_for_node(&db, &node.id, &pending.id)
            .await
            .expect("node consumes own pending");
        assert_eq!(returned.id, pending.id);

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert!(stored.consumed_at.is_some());
        assert!(stored.declined_at.is_none());
    }

    #[tokio::test]
    async fn node_cannot_consume_another_nodes_pending_credential() {
        let db = test_db("pending_credential_wrong_node").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node_a = test_node(&actor_id, "node-a");
        let node_b = test_node(&actor_id, "node-b");
        insert_node(&db, &node_a).await;
        insert_node(&db, &node_b).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node_a.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let err = consume_pending_credential_for_node(&db, &node_b.id, &pending.id)
            .await
            .expect_err("other node cannot consume");
        assert!(matches!(err, AppError::NotFound(_)));

        let stored = load_pending(&db, &pending.id).await;
        assert!(stored.is_active);
        assert!(stored.consumed_at.is_none());
    }

    #[tokio::test]
    async fn node_declines_pending_credential() {
        let db = test_db("pending_credential_decline").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        decline_pending_credential_for_node(&db, &node.id, &pending.id)
            .await
            .expect("node declines");

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert!(stored.declined_at.is_some());
        assert!(stored.consumed_at.is_none());
    }

    #[tokio::test]
    async fn admin_cancel_prevents_later_consume() {
        let db = test_db("pending_credential_cancel").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        cancel_pending_credential(&db, &actor_id, &node.id, &pending.id)
            .await
            .expect("admin cancels");

        let err = consume_pending_credential_for_node(&db, &node.id, &pending.id)
            .await
            .expect_err("canceled row is not consumable");
        assert!(matches!(err, AppError::NotFound(_)));

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert!(stored.consumed_at.is_none());
    }

    #[tokio::test]
    async fn expired_pending_credentials_are_not_listed() {
        let db = test_db("pending_credential_expired").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let now = Utc::now();
        let expired = NodePendingCredential {
            id: Uuid::new_v4().to_string(),
            node_id: node.id.clone(),
            service_slug: "expired".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: actor_id.clone(),
            owner_user_id: actor_id.clone(),
            created_at: now - Duration::hours(2),
            expires_at: now - Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: None,
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .insert_one(&expired)
            .await
            .expect("insert expired pending");

        let admin_list = list_pending_credentials_for_admin(&db, &actor_id, &node.id, false)
            .await
            .expect("admin list succeeds");
        let node_list = list_pending_credentials_for_node(&db, &node.id)
            .await
            .expect("node list succeeds");

        assert!(admin_list.is_empty());
        assert!(node_list.is_empty());
    }

    #[tokio::test]
    async fn get_pending_credential_for_admin_returns_active_unexpired_row() {
        let db = test_db("pending_credential_admin_get").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let returned = get_pending_credential_for_admin(&db, &actor_id, &node.id, &pending.id)
            .await
            .expect("admin can read active pending credential");

        assert_eq!(returned.id, pending.id);
        assert_eq!(returned.node_id, node.id);
        assert!(returned.is_active);
    }

    #[tokio::test]
    async fn get_pending_credential_for_admin_rejects_actor_without_node_access() {
        let db = test_db("pending_credential_admin_get_acl").await;

        let owner_id = Uuid::new_v4().to_string();
        let stranger_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&owner_id, UserType::Person),
                test_user(&stranger_id, UserType::Person),
            ],
        )
        .await;
        let node = test_node(&owner_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &owner_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let err = get_pending_credential_for_admin(&db, &stranger_id, &node.id, &pending.id)
            .await
            .expect_err("stranger cannot read another owner's pending credential");

        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn get_pending_credential_for_admin_filters_inactive_and_expired_rows() {
        let db = test_db("pending_credential_admin_get_filters").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let inactive =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("inactive"))
                .await
                .expect("push succeeds");
        let expired =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("expired"))
                .await
                .expect("push succeeds");
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &inactive.id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("mark inactive");
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &expired.id },
                doc! {
                    "$set": {
                        "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::hours(1)),
                    },
                },
            )
            .await
            .expect("mark expired");

        let inactive_err = get_pending_credential_for_admin(&db, &actor_id, &node.id, &inactive.id)
            .await
            .expect_err("inactive pending credential is filtered");
        let expired_err = get_pending_credential_for_admin(&db, &actor_id, &node.id, &expired.id)
            .await
            .expect_err("expired pending credential is filtered");

        assert!(matches!(inactive_err, AppError::NotFound(_)));
        assert!(matches!(expired_err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn store_ciphertext_first_writer_wins_sets_ciphertext_once() {
        let db = test_db("pending_credential_ciphertext_first_writer").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");

        let now = Utc::now();
        let first = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey-1", "nonce-1", vec![1, 2, 3]),
            true,
            now,
        )
        .await
        .expect("first writer stores ciphertext");
        match first {
            StorePendingCiphertextOutcome::StoredForOnlineNode(stored) => {
                assert_eq!(
                    stored.remote_state,
                    Some(RemoteCryptoState::CiphertextReceived)
                );
                assert_eq!(
                    stored.crypto.and_then(|crypto| crypto.ciphertext),
                    Some(vec![1, 2, 3])
                );
            }
            other => panic!("expected online storage, got {other:?}"),
        }

        let second = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey-2", "nonce-2", vec![9, 9, 9]),
            true,
            now,
        )
        .await
        .expect("second writer observes existing ciphertext");
        match second {
            StorePendingCiphertextOutcome::StoredForOnlineNode(stored) => {
                assert_eq!(
                    stored.crypto.and_then(|crypto| crypto.ciphertext),
                    Some(vec![1, 2, 3])
                );
            }
            other => panic!("expected existing online storage, got {other:?}"),
        }

        let stored = load_pending(&db, &pending.id).await;
        let crypto = stored.crypto.expect("crypto bundle");
        assert_eq!(crypto.admin_pubkey.as_deref(), Some("admin-pubkey-1"));
        assert_eq!(crypto.nonce.as_deref(), Some("nonce-1"));
        assert_eq!(crypto.ciphertext, Some(vec![1, 2, 3]));
    }

    #[tokio::test]
    async fn store_ciphertext_rejects_non_writable_actor_without_state_change() {
        let db = test_db("pending_credential_ciphertext_acl_denied").await;

        let admin_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let stranger_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&member_id, UserType::Person),
                test_user(&stranger_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &admin_id, OrgRole::Admin, None),
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &member_id, OrgRole::Member, None),
        )
        .await;
        let node = test_node(&org_id, "org-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &admin_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("org admin can create pending credential");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let before = load_pending(&db, &pending.id).await;
        assert_pubkey_only_pending(&before, "node-pubkey");

        for denied_actor_id in [&member_id, &stranger_id] {
            let err = store_pending_ciphertext_first_writer_wins(
                &db,
                denied_actor_id,
                &node.id,
                &pending.id,
                ciphertext_input("admin-pubkey", "nonce", vec![1, 2, 3]),
                false,
                Utc::now(),
            )
            .await
            .expect_err("actor without node write access cannot store ciphertext");

            assert!(matches!(err, AppError::NodeNotFound(_)));
            let stored = load_pending(&db, &pending.id).await;
            assert_pubkey_only_pending(&stored, "node-pubkey");
        }
    }

    #[tokio::test]
    async fn record_pubkey_is_first_writer_and_does_not_overwrite() {
        let db = test_db("pending_credential_pubkey_first_writer").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");

        let first = record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-1")
            .await
            .expect("first pubkey records");
        let second = record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-2")
            .await
            .expect("second pubkey returns existing");

        assert_eq!(
            first
                .crypto
                .as_ref()
                .map(|crypto| crypto.node_pubkey.as_str()),
            Some("node-1")
        );
        assert_eq!(
            second
                .crypto
                .as_ref()
                .map(|crypto| crypto.node_pubkey.as_str()),
            Some("node-1")
        );
        assert_eq!(second.remote_state, Some(RemoteCryptoState::PubkeyPosted));
    }

    #[tokio::test]
    async fn send_failure_queue_marking_and_mark_sent_transition() {
        let db = test_db("pending_credential_send_failure_queue").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let now = Utc::now();
        let stored = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey", "nonce", vec![1, 2, 3]),
            true,
            now,
        )
        .await
        .expect("store online");
        assert!(matches!(
            stored,
            StorePendingCiphertextOutcome::StoredForOnlineNode(_)
        ));

        let queued =
            mark_pending_ciphertext_queued_after_send_failure(&db, &node.id, &pending.id, now)
                .await
                .expect("mark queued");
        assert_eq!(
            queued.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        );
        assert!(queued.ciphertext_queued_at.is_some());

        let deliverable = list_deliverable_queued_ciphertexts_for_node(&db, &node.id, 10, now)
            .await
            .expect("list queued");
        assert_eq!(deliverable.len(), 1);
        assert_eq!(deliverable[0].id, pending.id);

        let sent = mark_queued_ciphertext_sent(&db, &node.id, &pending.id, now)
            .await
            .expect("mark sent");
        assert_eq!(
            sent.remote_state,
            Some(RemoteCryptoState::CiphertextReceived)
        );
        assert!(sent.ciphertext_queued_at.is_none());
        assert!(sent.ciphertext_expires_at.is_none());
    }

    #[tokio::test]
    async fn decrypt_result_ok_and_error_clear_ciphertext_without_persisted_error_code() {
        let db = test_db("pending_credential_decrypt_result").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let now = Utc::now();
        for (service_slug, outcome, expected_state, expect_consumed) in [
            (
                "decrypt-ok",
                PendingCredentialDecryptOutcome::Ok,
                RemoteCryptoState::Consumed,
                true,
            ),
            (
                "decrypt-error",
                PendingCredentialDecryptOutcome::Error,
                RemoteCryptoState::DecryptFailed,
                false,
            ),
        ] {
            let pending = create_pending_credential(
                &db,
                &actor_id,
                &node.id,
                remote_credential_input(service_slug),
            )
            .await
            .expect("push succeeds");
            record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
                .await
                .expect("record pubkey");
            store_pending_ciphertext_first_writer_wins(
                &db,
                &actor_id,
                &node.id,
                &pending.id,
                ciphertext_input("admin-pubkey", "nonce", vec![1, 2, 3]),
                true,
                now,
            )
            .await
            .expect("store ciphertext");

            let returned =
                record_pending_credential_decrypt_result(&db, &node.id, &pending.id, outcome, now)
                    .await
                    .expect("record decrypt result");
            assert!(!returned.is_active);
            assert_eq!(returned.remote_state, Some(expected_state));
            assert_eq!(returned.consumed_at.is_some(), expect_consumed);

            let stored = db
                .collection::<bson::Document>(NODE_PENDING_CREDENTIALS)
                .find_one(doc! { "_id": &pending.id })
                .await
                .expect("query raw pending")
                .expect("pending exists");
            let forbidden_field = ["remote", "error", "code"].join("_");
            assert!(stored.get(&forbidden_field).is_none());
            let crypto = stored.get_document("crypto").expect("crypto document");
            assert!(crypto.get("admin_pubkey").is_none());
            assert!(crypto.get("nonce").is_none());
            assert!(crypto.get("ciphertext").is_none());
        }
    }

    #[tokio::test]
    async fn store_ciphertext_offline_returns_queue_full_when_cap_reached() {
        let db = test_db("pending_credential_queue_full").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let now = Utc::now();
        for index in 0..MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE {
            let pending = create_pending_credential(
                &db,
                &actor_id,
                &node.id,
                remote_credential_input(&format!("service-{index}")),
            )
            .await
            .expect("push succeeds");
            record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
                .await
                .expect("record pubkey");
            let outcome = store_pending_ciphertext_first_writer_wins(
                &db,
                &actor_id,
                &node.id,
                &pending.id,
                ciphertext_input(
                    format!("admin-pubkey-{index}"),
                    format!("nonce-{index}"),
                    vec![index as u8],
                ),
                false,
                now,
            )
            .await
            .expect("queue ciphertext offline");
            assert!(matches!(
                outcome,
                StorePendingCiphertextOutcome::QueuedOffline(_)
            ));
        }

        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("service-full"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let outcome = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey-full", "nonce-full", vec![42]),
            false,
            now,
        )
        .await
        .expect("full offline queue returns a business outcome");

        match outcome {
            StorePendingCiphertextOutcome::QueueFull(summary) => {
                assert_eq!(summary.pending_credential_id, pending.id);
                assert_eq!(summary.node_id, node.id);
                assert_eq!(summary.service_slug, "service-full");
                assert_eq!(summary.owner_user_id, actor_id);
            }
            other => panic!("expected queue full, got {other:?}"),
        }
        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::PubkeyPosted));
        assert!(stored.crypto.and_then(|crypto| crypto.ciphertext).is_none());
    }

    #[tokio::test]
    async fn store_ciphertext_rejects_oversized_ciphertext() {
        let db = test_db("pending_credential_ciphertext_too_large").await;

        let err = store_pending_ciphertext_first_writer_wins(
            &db,
            "actor",
            "node",
            "pending",
            ciphertext_input("admin-pubkey", "nonce", vec![0; MAX_CIPHERTEXT_SIZE + 1]),
            true,
            Utc::now(),
        )
        .await
        .expect_err("oversized ciphertext should fail before storing");

        assert!(matches!(
            err,
            AppError::PendingCredentialCiphertextTooLarge(size)
                if size == MAX_CIPHERTEXT_SIZE + 1
        ));
    }

    #[tokio::test]
    async fn store_ciphertext_before_pubkey_returns_pubkey_awaiting() {
        let db = test_db("pending_credential_pubkey_awaiting").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");

        let err = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey", "nonce", vec![1, 2, 3]),
            true,
            Utc::now(),
        )
        .await
        .expect_err("ciphertext cannot be stored before node pubkey");

        assert!(matches!(
            err,
            AppError::PendingCredentialPubkeyAwaiting(id) if id == pending.id
        ));
        let stored = load_pending(&db, &pending.id).await;
        assert!(
            stored
                .crypto
                .as_ref()
                .is_some_and(|crypto| crypto.node_pubkey.is_empty())
        );
    }

    #[tokio::test]
    async fn queue_cap_counts_only_active_unexpired_ciphertexts() {
        let db = test_db("pending_credential_queue_cap").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let now = Utc::now();
        let mut pending_ids = Vec::new();
        for index in 0..MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE {
            let pending = create_pending_credential(
                &db,
                &actor_id,
                &node.id,
                remote_credential_input(&format!("service-{index}")),
            )
            .await
            .expect("push succeeds");
            record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
                .await
                .expect("record pubkey");
            store_pending_ciphertext_first_writer_wins(
                &db,
                &actor_id,
                &node.id,
                &pending.id,
                ciphertext_input(
                    format!("admin-pubkey-{index}"),
                    format!("nonce-{index}"),
                    vec![index as u8],
                ),
                false,
                now,
            )
            .await
            .expect("queue ciphertext offline");
            pending_ids.push(pending.id);
        }

        let count = active_unexpired_queued_ciphertext_count(&db, &node.id, now)
            .await
            .expect("count queued");
        assert_eq!(count, MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE);

        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &pending_ids[0] },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("mark inactive");
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &pending_ids[1] },
                doc! {
                    "$set": {
                        "ciphertext_expires_at": bson::DateTime::from_chrono(now - Duration::seconds(1)),
                    },
                },
            )
            .await
            .expect("mark expired");

        let count = active_unexpired_queued_ciphertext_count(&db, &node.id, now)
            .await
            .expect("count queued");
        assert_eq!(count, MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE - 2);

        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("service-extra"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let outcome = store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey-extra", "nonce-extra", vec![42]),
            false,
            now,
        )
        .await
        .expect("queue should have capacity after inactive and expired rows");
        assert!(matches!(
            outcome,
            StorePendingCiphertextOutcome::QueuedOffline(_)
        ));
    }

    #[tokio::test]
    async fn expire_queued_ciphertexts_returns_metadata_only_summaries() {
        let db = test_db("pending_credential_expire_queued").await;

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending = create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            remote_credential_input("openclaw"),
        )
        .await
        .expect("push succeeds");
        record_pending_credential_pubkey(&db, &node.id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        let now = Utc::now();
        store_pending_ciphertext_first_writer_wins(
            &db,
            &actor_id,
            &node.id,
            &pending.id,
            ciphertext_input("admin-pubkey", "nonce", vec![7, 8, 9]),
            false,
            now,
        )
        .await
        .expect("queue ciphertext offline");

        let summaries = expire_queued_ciphertexts_with_summaries(
            &db,
            now + Duration::seconds(OFFLINE_CIPHERTEXT_QUEUE_TTL_SECS + 1),
        )
        .await
        .expect("expire queued ciphertexts");
        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert_eq!(summary.pending_credential_id, pending.id);
        assert_eq!(summary.node_id, node.id);
        assert_eq!(summary.service_slug, "openclaw");
        assert_eq!(summary.owner_user_id, actor_id);
        assert_eq!(
            summary.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        );
        assert!(summary.ciphertext_queued_at.is_some());
        assert!(summary.ciphertext_expires_at.is_some());
        let summary_debug = format!("{summary:?}");
        assert!(!summary_debug.contains("admin-pubkey"));
        assert!(!summary_debug.contains("nonce"));
        assert!(!summary_debug.contains("[7, 8, 9]"));
        assert!(!summary_debug.contains("node-pubkey"));

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::Expired));
        assert!(stored.ciphertext_queued_at.is_none());
        assert!(stored.ciphertext_expires_at.is_none());
        let crypto = stored
            .crypto
            .expect("crypto bundle remains for pubkey metadata");
        assert_eq!(crypto.version, "v1");
        assert_eq!(crypto.node_pubkey, "node-pubkey");
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());
    }

    #[tokio::test]
    async fn fan_out_pubkey_posts_for_different_nodes_do_not_clobber() {
        let db = test_db("fanout_pubkey_concurrent").await;
        let owner_id = Uuid::new_v4().to_string();
        let node_a = Uuid::new_v4().to_string();
        let node_b = Uuid::new_v4().to_string();
        let pending = insert_fan_out_pending(
            &db,
            &owner_id,
            &[node_a.clone(), node_b.clone()],
            "fanout-pubkey",
        )
        .await;

        let (posted_a, posted_b) = tokio::join!(
            record_fan_out_pubkey(&db, &node_a, &pending.id, "v1", "node-pubkey-a"),
            record_fan_out_pubkey(&db, &node_b, &pending.id, "v1", "node-pubkey-b"),
        );
        posted_a.expect("node a posts pubkey");
        posted_b.expect("node b posts pubkey");

        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(target(&stored, &node_a).crypto.node_pubkey, "node-pubkey-a");
        assert_eq!(target(&stored, &node_b).crypto.node_pubkey, "node-pubkey-b");
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::PubkeyPosted));
        assert_eq!(stored.fan_out_revision, 1);
    }

    #[tokio::test]
    async fn fan_out_stale_revision_rejects_without_partial_write() {
        let db = test_db("fanout_stale_revision_no_write").await;
        let owner_id = Uuid::new_v4().to_string();
        let node_a = Uuid::new_v4().to_string();
        let node_b = Uuid::new_v4().to_string();
        let pending = insert_fan_out_pending(
            &db,
            &owner_id,
            &[node_a.clone(), node_b.clone()],
            "fanout-stale",
        )
        .await;
        record_fan_out_pubkeys(&db, &pending, &[node_a.clone(), node_b.clone()]).await;

        let err = store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &pending.id,
            fan_out_ciphertext_input(
                0,
                vec![
                    fan_out_item(&node_a, 0, vec![1]),
                    fan_out_item(&node_b, 0, vec![2]),
                ],
                &[&node_a, &node_b],
            ),
            Utc::now(),
        )
        .await
        .expect_err("stale revision rejects");

        assert!(matches!(err, AppError::Conflict(message) if message.contains("stale")));
        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(stored.fan_out_revision, 1);
        for node_id in [&node_a, &node_b] {
            let target = target(&stored, node_id);
            assert_eq!(target.remote_state, Some(RemoteCryptoState::PubkeyPosted));
            assert!(target.crypto.admin_pubkey.is_none());
            assert!(target.crypto.nonce.is_none());
            assert!(target.crypto.ciphertext.is_none());
        }
    }

    #[tokio::test]
    async fn fan_out_duplicate_ciphertext_post_rejects_and_preserves_first_write() {
        let db = test_db("fanout_duplicate_post").await;
        let owner_id = Uuid::new_v4().to_string();
        let node_a = Uuid::new_v4().to_string();
        let node_b = Uuid::new_v4().to_string();
        let pending = insert_fan_out_pending(
            &db,
            &owner_id,
            &[node_a.clone(), node_b.clone()],
            "fanout-dupe",
        )
        .await;
        record_fan_out_pubkeys(&db, &pending, &[node_a.clone(), node_b.clone()]).await;

        let first = store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &pending.id,
            fan_out_ciphertext_input(
                1,
                vec![
                    fan_out_item(&node_a, 0, vec![1]),
                    fan_out_item(&node_b, 0, vec![2]),
                ],
                &[&node_a, &node_b],
            ),
            Utc::now(),
        )
        .await
        .expect("first fan-out ciphertext write succeeds");
        assert_eq!(first.pending.fan_out_revision, 2);

        let err = store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &pending.id,
            fan_out_ciphertext_input(
                1,
                vec![
                    fan_out_item(&node_a, 0, vec![9]),
                    fan_out_item(&node_b, 0, vec![9]),
                ],
                &[&node_a, &node_b],
            ),
            Utc::now(),
        )
        .await
        .expect_err("duplicate post rejects");
        assert!(matches!(err, AppError::Conflict(_)));

        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(
            target(&stored, &node_a).crypto.ciphertext.as_deref(),
            Some(&[1][..])
        );
        assert_eq!(
            target(&stored, &node_b).crypto.ciphertext.as_deref(),
            Some(&[2][..])
        );
    }

    #[tokio::test]
    async fn fan_out_retry_resets_only_failed_targets_and_rejects_old_generation() {
        let db = test_db("fanout_retry_generation").await;
        let owner_id = Uuid::new_v4().to_string();
        let node_a = Uuid::new_v4().to_string();
        let node_b = Uuid::new_v4().to_string();
        let pending = insert_fan_out_pending(
            &db,
            &owner_id,
            &[node_a.clone(), node_b.clone()],
            "fanout-retry",
        )
        .await;
        record_fan_out_pubkeys(&db, &pending, &[node_a.clone(), node_b.clone()]).await;
        store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &pending.id,
            fan_out_ciphertext_input(
                1,
                vec![
                    fan_out_item(&node_a, 0, vec![1]),
                    fan_out_item(&node_b, 0, vec![2]),
                ],
                &[&node_a, &node_b],
            ),
            Utc::now(),
        )
        .await
        .expect("store ciphertexts");
        let failed = record_fan_out_decrypt_result(
            &db,
            &node_a,
            &pending.id,
            PendingCredentialDecryptOutcome::Error,
            Some(crate::errors::PENDING_CREDENTIAL_DECRYPT_FAILED_CODE),
            Utc::now(),
        )
        .await
        .expect("record one failed target");

        let retried = retry_failed_fan_out_nodes(
            &db,
            &owner_id,
            &pending.id,
            failed.fan_out_revision,
            Utc::now(),
        )
        .await
        .expect("retry failed target");
        let retried_pending = retried.pending;
        let retried_a = target(&retried_pending, &node_a);
        assert_eq!(retried_a.generation, 1);
        assert!(retried_a.remote_state.is_none());
        assert!(retried_a.crypto.node_pubkey.is_empty());
        assert!(retried_a.crypto.admin_pubkey.is_none());
        assert!(retried_a.crypto.ciphertext.is_none());
        let preserved_b = target(&retried_pending, &node_b);
        assert_eq!(preserved_b.generation, 0);
        assert_eq!(
            preserved_b.remote_state,
            Some(RemoteCryptoState::CiphertextReceived)
        );
        assert_eq!(preserved_b.crypto.ciphertext.as_deref(), Some(&[2][..]));

        record_fan_out_pubkey(&db, &node_a, &pending.id, "v1", "node-pubkey-a-retry")
            .await
            .expect("retry target posts new pubkey");
        let current = load_pending(&db, &pending.id).await;
        let err = store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &pending.id,
            fan_out_ciphertext_input(
                current.fan_out_revision,
                vec![fan_out_item(&node_a, 0, vec![9])],
                &[&node_a],
            ),
            Utc::now(),
        )
        .await
        .expect_err("old generation rejects");
        assert!(matches!(err, AppError::Conflict(message) if message.contains("generation")));
    }

    #[tokio::test]
    async fn fan_out_all_consumed_deactivates_top_level() {
        let db = test_db("fanout_all_consumed").await;
        let owner_id = Uuid::new_v4().to_string();
        let node_a = Uuid::new_v4().to_string();
        let node_b = Uuid::new_v4().to_string();
        let pending = insert_fan_out_pending(
            &db,
            &owner_id,
            &[node_a.clone(), node_b.clone()],
            "fanout-done",
        )
        .await;
        record_fan_out_pubkeys(&db, &pending, &[node_a.clone(), node_b.clone()]).await;
        store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &pending.id,
            fan_out_ciphertext_input(
                1,
                vec![
                    fan_out_item(&node_a, 0, vec![1]),
                    fan_out_item(&node_b, 0, vec![2]),
                ],
                &[&node_a, &node_b],
            ),
            Utc::now(),
        )
        .await
        .expect("store ciphertexts");

        let partial = record_fan_out_decrypt_result(
            &db,
            &node_a,
            &pending.id,
            PendingCredentialDecryptOutcome::Ok,
            None,
            Utc::now(),
        )
        .await
        .expect("first target consumes");
        assert!(partial.is_active);
        assert_eq!(
            partial.remote_state,
            Some(RemoteCryptoState::PartialDecrypted)
        );

        let consumed = record_fan_out_decrypt_result(
            &db,
            &node_b,
            &pending.id,
            PendingCredentialDecryptOutcome::Ok,
            None,
            Utc::now(),
        )
        .await
        .expect("second target consumes");
        assert!(!consumed.is_active);
        assert_eq!(consumed.remote_state, Some(RemoteCryptoState::Consumed));
        assert!(consumed.consumed_at.is_some());
    }

    #[tokio::test]
    async fn fan_out_partial_breakdown_records_success_and_failure_targets() {
        let db = test_db("fanout_partial_breakdown").await;
        let owner_id = Uuid::new_v4().to_string();
        let node_a = Uuid::new_v4().to_string();
        let node_b = Uuid::new_v4().to_string();
        let pending = insert_fan_out_pending(
            &db,
            &owner_id,
            &[node_a.clone(), node_b.clone()],
            "fanout-partial",
        )
        .await;
        record_fan_out_pubkeys(&db, &pending, &[node_a.clone(), node_b.clone()]).await;
        store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &pending.id,
            fan_out_ciphertext_input(
                1,
                vec![
                    fan_out_item(&node_a, 0, vec![1]),
                    fan_out_item(&node_b, 0, vec![2]),
                ],
                &[&node_a, &node_b],
            ),
            Utc::now(),
        )
        .await
        .expect("store ciphertexts");
        record_fan_out_decrypt_result(
            &db,
            &node_a,
            &pending.id,
            PendingCredentialDecryptOutcome::Ok,
            None,
            Utc::now(),
        )
        .await
        .expect("first target consumes");
        let partial = record_fan_out_decrypt_result(
            &db,
            &node_b,
            &pending.id,
            PendingCredentialDecryptOutcome::Error,
            Some(crate::errors::PENDING_CREDENTIAL_DECRYPT_FAILED_CODE),
            Utc::now(),
        )
        .await
        .expect("second target fails");

        assert!(partial.is_active);
        assert_eq!(
            partial.remote_state,
            Some(RemoteCryptoState::PartialDecrypted)
        );
        assert_eq!(
            target(&partial, &node_a).remote_state,
            Some(RemoteCryptoState::Consumed)
        );
        let failed = target(&partial, &node_b);
        assert_eq!(failed.remote_state, Some(RemoteCryptoState::DecryptFailed));
        assert_eq!(
            failed.error_code,
            Some(crate::errors::PENDING_CREDENTIAL_DECRYPT_FAILED_CODE)
        );
        assert_eq!(
            failed.error_kind.as_deref(),
            Some("pending_credential_decrypt_failed")
        );
    }

    #[tokio::test]
    async fn fan_out_partial_expiry_marks_top_expired_and_preserves_consumed_target() {
        let db = test_db("fanout_partial_expiry").await;
        let owner_id = Uuid::new_v4().to_string();
        let node_a = Uuid::new_v4().to_string();
        let node_b = Uuid::new_v4().to_string();
        let pending = insert_fan_out_pending(
            &db,
            &owner_id,
            &[node_a.clone(), node_b.clone()],
            "fanout-expire",
        )
        .await;
        record_fan_out_pubkeys(&db, &pending, &[node_a.clone(), node_b.clone()]).await;
        store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &pending.id,
            fan_out_ciphertext_input(
                1,
                vec![
                    fan_out_item(&node_a, 0, vec![1]),
                    fan_out_item(&node_b, 0, vec![2]),
                ],
                &[&node_a, &node_b],
            ),
            Utc::now(),
        )
        .await
        .expect("store ciphertexts");
        record_fan_out_decrypt_result(
            &db,
            &node_a,
            &pending.id,
            PendingCredentialDecryptOutcome::Ok,
            None,
            Utc::now(),
        )
        .await
        .expect("first target consumes");
        record_fan_out_decrypt_result(
            &db,
            &node_b,
            &pending.id,
            PendingCredentialDecryptOutcome::Error,
            Some(crate::errors::PENDING_CREDENTIAL_DECRYPT_FAILED_CODE),
            Utc::now(),
        )
        .await
        .expect("second target fails");
        let expired_at = Utc::now() - Duration::seconds(1);
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &pending.id },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(expired_at) } },
            )
            .await
            .expect("force top-level expiry");

        let summaries = expire_queued_ciphertexts_with_summaries(&db, Utc::now())
            .await
            .expect("expire partial fan-out");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].pending_credential_id, pending.id);
        assert_eq!(summaries[0].node_id, node_b);

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::Expired));
        let consumed = target(&stored, &node_a);
        assert_eq!(consumed.remote_state, Some(RemoteCryptoState::Consumed));
        assert!(consumed.consumed_at.is_some());
        let expired = target(&stored, &node_b);
        assert_eq!(expired.remote_state, Some(RemoteCryptoState::Expired));
        assert!(expired.crypto.admin_pubkey.is_none());
        assert!(expired.crypto.nonce.is_none());
        assert!(expired.crypto.ciphertext.is_none());
    }

    #[tokio::test]
    async fn fan_out_queue_cap_counts_single_and_fan_out_queued_targets() {
        let db = test_db("fanout_queue_cap_mixed").await;
        let owner_id = Uuid::new_v4().to_string();
        let node_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        for index in 0..(MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE - 1) {
            let pending = NodePendingCredential {
                id: Uuid::new_v4().to_string(),
                node_id: node_id.clone(),
                service_slug: format!("single-{index}"),
                injection_method: InjectionMethod::Header,
                field_name: "X-API-Key".to_string(),
                target_url: None,
                label: None,
                created_by_user_id: owner_id.clone(),
                owner_user_id: owner_id.clone(),
                created_at: now,
                expires_at: now + Duration::hours(1),
                consumed_at: None,
                declined_at: None,
                crypto: Some(CryptoBundle {
                    version: "v1".to_string(),
                    node_pubkey: "node-pubkey".to_string(),
                    admin_pubkey: Some("admin-pubkey".to_string()),
                    nonce: Some("nonce".to_string()),
                    ciphertext: Some(vec![index as u8]),
                }),
                remote_state: Some(RemoteCryptoState::CiphertextQueued),
                ciphertext_queued_at: Some(now),
                ciphertext_expires_at: Some(now + Duration::minutes(15)),
                is_active: true,
                fan_out_nodes: Vec::new(),
                fan_out_revision: 0,
            };
            db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
                .insert_one(&pending)
                .await
                .expect("insert queued single pending");
        }

        let fan_out = insert_fan_out_pending(
            &db,
            &owner_id,
            std::slice::from_ref(&node_id),
            "fanout-queued",
        )
        .await;
        record_fan_out_pubkey(&db, &node_id, &fan_out.id, "v1", "node-pubkey")
            .await
            .expect("record fan-out pubkey");
        store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &fan_out.id,
            fan_out_ciphertext_input(1, vec![fan_out_item(&node_id, 0, vec![42])], &[]),
            now,
        )
        .await
        .expect("queue fan-out target");
        assert_eq!(
            active_unexpired_queued_ciphertext_count(&db, &node_id, now)
                .await
                .expect("count queued ciphertexts"),
            MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE
        );

        let blocked = insert_fan_out_pending(
            &db,
            &owner_id,
            std::slice::from_ref(&node_id),
            "fanout-blocked",
        )
        .await;
        record_fan_out_pubkey(&db, &node_id, &blocked.id, "v1", "node-pubkey")
            .await
            .expect("record blocked pubkey");
        let err = store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &blocked.id,
            fan_out_ciphertext_input(1, vec![fan_out_item(&node_id, 0, vec![43])], &[]),
            now,
        )
        .await
        .expect_err("mixed queue cap rejects next queued target");
        assert!(matches!(err, AppError::PendingCredentialQueueFull(id) if id == node_id));
    }

    #[tokio::test]
    async fn fan_out_queued_nested_expiry_clears_nested_crypto() {
        let db = test_db("fanout_nested_expiry").await;
        let owner_id = Uuid::new_v4().to_string();
        let node_id = Uuid::new_v4().to_string();
        let pending = insert_fan_out_pending(
            &db,
            &owner_id,
            std::slice::from_ref(&node_id),
            "fanout-nested-expiry",
        )
        .await;
        record_fan_out_pubkey(&db, &node_id, &pending.id, "v1", "node-pubkey")
            .await
            .expect("record pubkey");
        store_fan_out_ciphertexts_revision_guard(
            &db,
            &owner_id,
            &pending.id,
            fan_out_ciphertext_input(1, vec![fan_out_item(&node_id, 0, vec![7, 8, 9])], &[]),
            Utc::now(),
        )
        .await
        .expect("queue nested ciphertext");
        let mut queued = load_pending(&db, &pending.id).await;
        target(&queued, &node_id);
        queued.fan_out_nodes[0].ciphertext_expires_at = Some(Utc::now() - Duration::seconds(1));
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .replace_one(doc! { "_id": &pending.id }, &queued)
            .await
            .expect("force nested ciphertext expiry");

        let summaries = expire_queued_ciphertexts_with_summaries(&db, Utc::now())
            .await
            .expect("expire nested queued ciphertext");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].node_id, node_id);
        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::Expired));
        let target = target(&stored, &node_id);
        assert_eq!(target.remote_state, Some(RemoteCryptoState::Expired));
        assert!(target.crypto.admin_pubkey.is_none());
        assert!(target.crypto.nonce.is_none());
        assert!(target.crypto.ciphertext.is_none());
        assert!(target.ciphertext_queued_at.is_none());
        assert!(target.ciphertext_expires_at.is_none());
    }

    #[tokio::test]
    async fn fan_out_creation_with_one_target_uses_single_node_shape() {
        let db = test_db("fanout_single_target_shape").await;
        let owner_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&owner_id, UserType::Person)]).await;
        let node = test_node(&owner_id, "single-fanout-node");
        insert_node(&db, &node).await;
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(test_binding(&owner_id, &node.id, "catalog-svc"))
            .await
            .expect("insert node service binding");

        let result = create_fan_out_pending_credential(
            &db,
            &owner_id,
            CreateFanOutPendingCredentialInput {
                owner_user_id: owner_id.clone(),
                service_id: "catalog-svc".to_string(),
                service_slug: "single-fanout".to_string(),
                injection_method: InjectionMethod::Header,
                field_name: "X-API-Key".to_string(),
                target_url: None,
                label: None,
                ttl_secs: 86_400,
                remote_crypto: true,
            },
        )
        .await
        .expect("single target fan-out delegates to single pending");

        assert_eq!(result.pending.node_id, node.id);
        assert!(result.pending.fan_out_nodes.is_empty());
        assert_eq!(result.pending.fan_out_revision, 0);
        assert!(result.pending.crypto.is_some());
        assert!(result.pending.remote_state.is_none());
        let raw = db
            .collection::<bson::Document>(NODE_PENDING_CREDENTIALS)
            .find_one(doc! { "_id": &result.pending.id })
            .await
            .expect("query raw pending")
            .expect("pending exists");
        assert!(raw.get("fan_out_nodes").is_none());
        assert!(raw.get("fan_out_revision").is_none());
    }

    #[tokio::test]
    async fn transfer_deactivates_pending_credentials_for_node() {
        let db = test_db("pending_credential_transfer").await;

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &actor_id, OrgRole::Admin, None),
        )
        .await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let transfer = node_service::transfer_node_owner(&db, &actor_id, &node.id, &org_id, 10)
            .await
            .expect("transfer succeeds");
        assert_eq!(transfer.deactivated_pending_credentials_count, 1);

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
    }
}
