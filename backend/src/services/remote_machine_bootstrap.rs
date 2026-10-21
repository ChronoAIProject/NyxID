//! Credential bootstrap for the existing stable Node identity. Workload
//! authentication and current native lifecycle admission are independent gates.
//! A user token or caller-supplied `verified` flag cannot satisfy either gate.

use chrono::{DateTime, Utc};
use mongodb::{
    Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};
use nyxid_machine::managed::Binding;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{
    crypto::{aes::EncryptionKeys, token::hash_token},
    errors::{AppError, AppResult},
    models::{
        remote_machine::{COLLECTION_NAME as MACHINES, ManagedMachine},
        remote_machine_bootstrap::{COLLECTION_NAME as DELIVERIES, ManagedBootstrapDelivery},
    },
};

use super::{api_key_mutation_service, node_service};

pub const MAX_TICKET_BYTES: usize = 64 * 1024;

/// Output of a trusted provider implementation after verifying its signed,
/// short-lived workload ticket. This type is not deserializable from HTTP.
/// Implementations must authenticate the actual workload, not just decode
/// caller-provided claims. The binding includes the one-use delivery id.
pub struct VerifiedBootstrapTicket {
    binding: Binding,
    recipient_key_digest: [u8; 32],
    expires_at: DateTime<Utc>,
}

#[async_trait::async_trait]
pub trait BootstrapTicketVerifier: Send + Sync {
    async fn verify(&self, ticket: &Value, binding: &Binding)
    -> AppResult<VerifiedBootstrapTicket>;
}

/// Fail-closed fallback when workload authentication is unavailable. Ordinary
/// native deployments use the Kubernetes TokenReview/live-Pod verifier; an
/// unsigned caller claim never substitutes for that authentication.
pub struct UnavailableBootstrapTicketVerifier;

#[async_trait::async_trait]
impl BootstrapTicketVerifier for UnavailableBootstrapTicketVerifier {
    async fn verify(
        &self,
        _ticket: &Value,
        _binding: &Binding,
    ) -> AppResult<VerifiedBootstrapTicket> {
        Err(AppError::NodeOffline(
            "managed_bootstrap_provider_unavailable".into(),
        ))
    }
}

impl VerifiedBootstrapTicket {
    /// This constructor is available to server-side provider implementations,
    /// never an HTTP claim or configuration flag. Validation is repeated at the
    /// admission boundary and inside the credential transaction.
    pub fn from_provider_verified_claims(
        binding: Binding,
        recipient_key_digest: [u8; 32],
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            binding,
            recipient_key_digest,
            expires_at,
        }
    }
}

pub struct BootstrapInput {
    pub binding: Binding,
    pub ticket: Value,
    pub recipient_public_key: String,
}

pub struct BootstrapResult {
    pub binding: Binding,
    pub sealed: nyxid_machine::bootstrap::Envelope,
    #[cfg(test)]
    pub replayed: bool,
}

impl std::fmt::Debug for BootstrapInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapInput")
            .field("request", &"[REDACTED]")
            .finish()
    }
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    let deliveries = db.collection::<ManagedBootstrapDelivery>(DELIVERIES);
    deliveries.create_index(IndexModel::builder()
        .keys(doc! {"binding.node_id": 1, "binding.runtime_generation": 1, "binding.credential_epoch": 1})
        .options(IndexOptions::builder().unique(true).build()).build()).await?;
    // Ciphertext is only replayable during the provider ticket lifetime.
    // Expiry never deletes the Machine, Node identity or paused workspace.
    deliveries
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
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

fn rekey_required() -> AppError {
    AppError::Conflict("managed_bootstrap_rekey_required".into())
}

fn validate_binding(machine: &ManagedMachine, binding: &Binding) -> AppResult<()> {
    binding
        .validate()
        .map_err(|_| AppError::ValidationError("invalid managed bootstrap binding".into()))?;
    if machine.node_id != binding.node_id
        || machine.sandbox_uid.as_deref() != Some(&binding.sandbox_uid)
        || machine.pod_uid.as_deref() != Some(&binding.pod_uid)
        || u64::try_from(machine.runtime_generation).ok() != Some(binding.runtime_generation)
        || u64::try_from(machine.credential_epoch).ok() != Some(binding.credential_epoch)
        || machine.writer_fence != binding.writer_fence
        || machine.active_operation_id.as_deref() != Some(&binding.operation_id)
        || machine
            .bootstrap_delivery_id
            .as_deref()
            .is_some_and(|id| id != binding.delivery_id)
    {
        return Err(rekey_required());
    }
    if !super::remote_machine_service::runtime_fence_allows_bootstrap(machine) {
        return Err(AppError::Conflict(
            "managed_runtime_writer_evidence_required".into(),
        ));
    }
    Ok(())
}

/// Resolve the live target before a protected supervisor requests credential
/// delivery. Kubernetes authenticates this exact Pod; recipient possession by
/// itself never authorizes target selection. No credential or ticket is stored.
pub async fn prepare_workload(
    db: &Database,
    verifier: &dyn BootstrapTicketVerifier,
    node_id: &str,
    pod_uid: &str,
    ticket: &Value,
    recipient_public_key: &str,
) -> AppResult<Binding> {
    if uuid::Uuid::parse_str(node_id).is_err()
        || uuid::Uuid::parse_str(pod_uid).is_err()
        || serde_json::to_vec(ticket)
            .map_err(|_| AppError::MachineNotAllowed)?
            .len()
            > MAX_TICKET_BYTES
    {
        return Err(AppError::MachineNotAllowed);
    }
    let machine = db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id":node_id})
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    let binding = Binding {
        node_id: machine.node_id.clone(),
        sandbox_uid: machine
            .sandbox_uid
            .clone()
            .ok_or(AppError::MachineAuthorityStale)?,
        pod_uid: machine
            .pod_uid
            .clone()
            .filter(|uid| uid == pod_uid)
            .ok_or(AppError::MachineAuthorityStale)?,
        runtime_generation: u64::try_from(machine.runtime_generation)
            .map_err(|_| AppError::MachineAuthorityStale)?,
        credential_epoch: u64::try_from(machine.credential_epoch)
            .map_err(|_| AppError::MachineAuthorityStale)?,
        writer_fence: machine.writer_fence.clone(),
        operation_id: machine
            .active_operation_id
            .clone()
            .ok_or(AppError::MachineAuthorityStale)?,
        // The writer fence is a server-created UUIDv4 unique to this epoch.
        // A repeated prepare, even after a lost response, resolves one delivery.
        delivery_id: machine
            .bootstrap_delivery_id
            .clone()
            .unwrap_or_else(|| machine.writer_fence.clone()),
    };
    validate_binding(&machine, &binding)?;
    let digest = nyxid_machine::bootstrap::recipient_key_digest(recipient_public_key)
        .map_err(|_| AppError::MachineNotAllowed)?;
    let verified = verifier.verify(ticket, &binding).await?;
    if verified.binding != binding
        || verified.recipient_key_digest != digest
        || verified.expires_at <= Utc::now()
    {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(binding)
}

fn digest_envelope(envelope: &nyxid_machine::bootstrap::Envelope) -> AppResult<String> {
    let encoded = serde_json::to_vec(envelope)
        .map_err(|_| AppError::Internal("bootstrap envelope serialization failed".into()))?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

/// Activate credentials and their replay envelope atomically. Every replay
/// authenticates the provider ticket again and live-checks the full machine
/// fence. A new recipient/delivery cannot recover or rotate an existing epoch.
pub async fn bootstrap(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    verifier: &dyn BootstrapTicketVerifier,
    input: BootstrapInput,
) -> AppResult<BootstrapResult> {
    input
        .binding
        .validate()
        .map_err(|_| AppError::ValidationError("invalid managed bootstrap binding".into()))?;
    if serde_json::to_vec(&input.ticket)
        .map_err(|_| AppError::ValidationError("invalid provider ticket".into()))?
        .len()
        > MAX_TICKET_BYTES
    {
        return Err(AppError::ValidationError(
            "managed bootstrap ticket exceeds 65536 bytes".into(),
        ));
    }
    let recipient_key_digest = nyxid_machine::bootstrap::recipient_key_digest(
        &input.recipient_public_key,
    )
    .map_err(|_| AppError::ValidationError("invalid bootstrap recipient public key".into()))?;
    let verified = verifier.verify(&input.ticket, &input.binding).await?;
    let now = Utc::now();
    if verified.binding != input.binding
        || verified.recipient_key_digest != recipient_key_digest
        || verified.expires_at <= now
        || verified.expires_at > now + chrono::Duration::minutes(15)
    {
        return Err(AppError::Unauthorized(
            "invalid or expired managed workload ticket".into(),
        ));
    }
    // No raw auth token is stored server-side, even temporarily in MongoDB.
    // The existing Node auth/signing format remains authoritative.
    let selected = db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id":&input.binding.node_id})
        .await?
        .ok_or_else(|| AppError::Unauthorized("managed workload is unavailable".into()))?;
    validate_binding(&selected, &input.binding)?;
    let selected_capabilities = selected.capabilities;
    let material = node_service::prepare_managed_credentials(
        encryption_keys,
        &input.binding.node_id,
        &selected.owner_id,
    )
    .await?;
    #[derive(Serialize)]
    struct Secrets<'a> {
        auth_token: &'a str,
        signing_secret: &'a str,
        capabilities: &'a [String],
    }
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&Secrets {
            auth_token: &material.auth_token,
            signing_secret: &material.signing_secret,
            capabilities: &selected_capabilities,
        })
        .map_err(|_| AppError::Internal("bootstrap payload serialization failed".into()))?,
    );
    let sealed =
        nyxid_machine::bootstrap::seal(&plaintext, &input.recipient_public_key, &input.binding)
            .map_err(|_| {
                AppError::ValidationError(
                    "bootstrap recipient key cannot receive credentials".into(),
                )
            })?;
    let recipient_digest = hex::encode(recipient_key_digest);
    let row = ManagedBootstrapDelivery {
        id: input.binding.delivery_id.clone(),
        binding: input.binding.clone(),
        recipient_key_digest: recipient_digest.clone(),
        sealed_digest: digest_envelope(&sealed)?,
        sealed,
        created_at: now,
        expires_at: verified.expires_at,
    };
    let auth_hash = hash_token(&material.auth_token);
    let signing_hash = hash_token(&material.signing_secret);
    let db = db.clone();
    let result = db.client().start_session().await?.start_transaction().and_run2(async move |session| {
        let result: AppResult<BootstrapResult> = async {
            let now = Utc::now();
            if verified.expires_at <= now { return Err(AppError::Unauthorized("managed workload ticket expired".into())); }
            let machine = db.collection::<ManagedMachine>(MACHINES).find_one(doc! {"_id": &input.binding.node_id})
                .session(&mut *session).await?.ok_or_else(|| AppError::Unauthorized("managed workload is unavailable".into()))?;
            validate_binding(&machine, &input.binding)?;
            if machine.capabilities != selected_capabilities {return Err(rekey_required());}
            super::remote_machine_service::ensure_owner_eligible_in_session(&db, session, &machine.owner_id).await?;
            if let Some(existing) = db.collection::<ManagedBootstrapDelivery>(DELIVERIES)
                .find_one(doc! {"_id": &input.binding.delivery_id}).session(&mut *session).await? {
                if existing.binding != input.binding || existing.recipient_key_digest != recipient_digest
                    || existing.expires_at <= now || machine.bootstrap_delivery_id.as_deref() != Some(&existing.id)
                    || machine.bootstrap_digest.as_deref() != Some(&existing.sealed_digest)
                { return Err(rekey_required()); }
                // A write makes replay admission conflict with concurrent
                // pause/delete/rekey, not merely read a stale snapshot.
                let updated = db.collection::<ManagedMachine>(MACHINES).update_one(
                    doc! {"_id": &machine.node_id, "revision": machine.revision},
                    doc! {"$inc": {"revision": 1_i64}, "$set": {"updated_at": bson::DateTime::from_chrono(now)}})
                    .session(&mut *session).await?;
                if updated.modified_count != 1 { return Err(rekey_required()); }
                return Ok(BootstrapResult { binding: existing.binding, sealed: existing.sealed, #[cfg(test)] replayed: true });
            }
            if machine.bootstrap_delivery_id.is_some() { return Err(rekey_required()); }
            let updated = db.collection::<ManagedMachine>(MACHINES).update_one(
                doc! {"_id": &machine.node_id, "revision": machine.revision, "bootstrap_delivery_id": null},
                doc! {"$set": {"bootstrap_delivery_id": &row.id, "bootstrap_digest": &row.sealed_digest, "updated_at": bson::DateTime::from_chrono(now)}, "$inc": {"revision": 1_i64, "bootstrap_epoch": 1_i64}})
                .session(&mut *session).await?;
            if updated.modified_count != 1 { return Err(rekey_required()); }
            node_service::activate_managed_credentials(&db, session, &machine.node_id, &machine.owner_id,
                &auth_hash, &signing_hash, &material.signing_secret_encrypted, material.signing_secret_context_version, now).await?;
            db.collection::<ManagedBootstrapDelivery>(DELIVERIES).insert_one(&row).session(&mut *session).await?;
            let audit_key = super::audit_service::audit_chain_hmac_key();
            #[cfg(test)]
            let audit_key = audit_key.or(Some(&[2u8; 32]));
            let audit_key = audit_key.ok_or_else(|| AppError::Internal("audit chain key is unavailable".into()))?;
            super::audit_service::log_system_event_in_session(&db, session, audit_key,
                "managed_machine_credentials_delivered", serde_json::json!({
                    "owner_id":machine.owner_id,"machine_id":machine.node_id,
                    "operation_id":input.binding.operation_id,"runtime_generation":input.binding.runtime_generation,
                    "credential_epoch":input.binding.credential_epoch,
                })).await?;
            Ok(BootstrapResult { binding: row.binding.clone(), sealed: row.sealed.clone(), #[cfg(test)] replayed: false })
        }.await;
        api_key_mutation_service::transaction_result(result)
    }).await.map_err(api_key_mutation_service::map_transaction_error)?;
    Ok(result)
}

#[cfg(test)]
#[path = "remote_machine_bootstrap_tests.rs"]
pub(super) mod tests;
