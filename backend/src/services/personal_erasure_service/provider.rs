//! Preserve exact revocation capability before removing source secrets. Token
//! scope and grant scope preserve the ordinary disconnect contract; potentially
//! grant-wide effects use a single attempt with a bounded start window.
use super::*;
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    models::provider_config::ProviderConfig,
    services::{
        oauth_revocation::{self, TokenOutcome},
        user_credentials_service::ResolvedOAuthCredentials,
    },
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub(crate) const EFFECTS: &str = "personal_erasure_effects";
pub(super) const RECEIPTS: &str = super::super::personal_erasure_registry::REVOCATION_RECEIPTS;
const MAX_CAPABILITY: usize = 256 * 1024;

#[derive(Serialize, Deserialize)]
pub(crate) struct RevocationMaterial {
    provider: Option<ProviderConfig>,
    #[serde(default)]
    scope: Option<oauth_revocation::RevocationScope>,
    #[serde(default)]
    unavailable_family: Option<String>,
    #[serde(default)]
    app_lineage: Option<String>,
    #[serde(default)]
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    client_id: Option<Zeroizing<String>>,
    client_secret: Option<Zeroizing<String>>,
    credential_user_id: Option<String>,
    byo: bool,
    access_token: Option<Zeroizing<String>>,
    refresh_token: Option<Zeroizing<String>>,
}
crate::errors::redacted_debug!(RevocationMaterial);
fn strip_provider_secrets(provider: &mut ProviderConfig) {
    provider.client_id_encrypted = None;
    provider.client_secret_encrypted = None;
    provider.secret_context_versions.clear();
}

impl RevocationMaterial {
    fn single_attempt(&self) -> bool {
        self.scope == Some(oauth_revocation::RevocationScope::Grant)
            || self
                .provider
                .as_ref()
                .and_then(oauth_revocation::effective_revocation)
                .is_some_and(|config| !oauth_revocation::verified_token_only(&config))
    }
    pub(crate) fn new(
        mut provider: ProviderConfig,
        credentials: Option<ResolvedOAuthCredentials>,
        access_token: Option<Zeroizing<String>>,
        refresh_token: Option<Zeroizing<String>>,
    ) -> Self {
        // Revocation uses the separately captured client material below. Do not
        // retain redundant encrypted source fields or hidden old-KEK snapshots.
        strip_provider_secrets(&mut provider);
        let (client_id, client_secret, credential_user_id, byo) = match credentials {
            Some(c) => (
                Some(c.client_id),
                c.client_secret,
                c.credential_user_id,
                c.app_source == super::super::oauth_app_source::OAuthAppSource::Byo,
            ),
            None => (None, None, None, false),
        };
        Self {
            provider: Some(provider),
            scope: None,
            unavailable_family: None,
            app_lineage: Some(if byo {
                use sha2::{Digest, Sha256};
                format!(
                    "byo:{}",
                    hex::encode(Sha256::digest(
                        client_id
                            .as_ref()
                            .map(|id| id.as_bytes())
                            .unwrap_or_default()
                    ))
                )
            } else {
                "platform".to_owned()
            }),
            expires_at: None,
            client_id,
            client_secret,
            credential_user_id,
            byo,
            access_token,
            refresh_token,
        }
    }
    pub(crate) fn unattributed_oauth(
        access_token: Option<Zeroizing<String>>,
        refresh_token: Option<Zeroizing<String>>,
    ) -> Self {
        Self {
            provider: None,
            scope: None,
            unavailable_family: Some("unattributed_oauth".into()),
            app_lineage: None,
            expires_at: None,
            client_id: None,
            client_secret: None,
            credential_user_id: None,
            byo: false,
            access_token,
            refresh_token,
        }
    }
    pub(crate) fn gcp_service_account(
        access: Zeroizing<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Self {
        Self {
            provider: None,
            scope: None,
            unavailable_family: Some("gcp_service_account".into()),
            app_lineage: None,
            expires_at,
            client_id: None,
            client_secret: None,
            credential_user_id: None,
            byo: false,
            access_token: Some(access),
            refresh_token: None,
        }
    }
    fn skip_reason(&self) -> Option<&'static str> {
        let provider = self.provider.as_ref()?;
        let credentials = self.client_id.as_ref().map(|id| ResolvedOAuthCredentials {
            app_source: if self.byo {
                super::super::oauth_app_source::OAuthAppSource::Byo
            } else {
                super::super::oauth_app_source::OAuthAppSource::Platform
            },
            client_id: id.clone(),
            client_secret: self.client_secret.clone(),
            credential_user_id: self.credential_user_id.clone(),
        });
        super::super::unified_key_service::known_remote_skip_reason(
            provider,
            self.scope
                .unwrap_or(oauth_revocation::RevocationScope::Token),
            credentials.as_ref(),
            self.access_token.as_ref(),
            self.refresh_token.as_ref(),
            false,
        )
    }
    async fn audit(
        &self,
        db: &Database,
        effect: &Document,
        outcome: &str,
        access: Option<&str>,
        refresh: Option<&str>,
    ) {
        if effect.get_str("kind").ok() != Some("oauth_disconnect") {
            return;
        }
        let (Some(provider), Ok(actor)) =
            (self.provider.as_ref(), effect.get_document("audit_actor"))
        else {
            return;
        };
        let Ok(user_id) = actor.get_str("user_id") else {
            return;
        };
        let optional = |field| actor.get_str(field).ok().map(str::to_owned);
        let actor = super::super::audit_service::AuditActor {
            user_id: user_id.into(),
            ip_address: optional("ip_address"),
            user_agent: optional("user_agent"),
            api_key_id: optional("api_key_id"),
            api_key_name: optional("api_key_name"),
        };
        super::super::unified_key_service::write_remote_audit(
            db.clone(),
            &actor,
            effect.get_str("user_id").unwrap_or(""),
            provider,
            self.scope
                .unwrap_or(oauth_revocation::RevocationScope::Token),
            usize::try_from(effect.get_i64("cascade_record_count").unwrap_or(0)).unwrap_or(0),
            outcome,
            access,
            refresh,
        )
        .await;
    }
    fn authority(&self) -> AppResult<String> {
        #[derive(Serialize)]
        struct Authority<'a> {
            provider_id: Option<&'a str>,
            config: Option<crate::models::provider_config::RevocationConfig>,
            scope: oauth_revocation::RevocationScope,
            client_id: Option<&'a str>,
            client_secret: Option<&'a str>,
        }
        let bytes = encode_capability(&Authority {
            provider_id: self.provider.as_ref().map(|p| p.id.as_str()),
            config: self
                .provider
                .as_ref()
                .and_then(oauth_revocation::effective_revocation),
            scope: self
                .scope
                .unwrap_or(oauth_revocation::RevocationScope::Token),
            client_id: self.client_id.as_ref().map(|s| s.as_str()),
            client_secret: self.client_secret.as_ref().map(|s| s.as_str()),
        })?;
        super::super::audit_service::keyed_fingerprint(
            super::super::audit_service::audit_chain_hmac_key(),
            b"oauth-revocation-authority-v1",
            &bytes,
        )
        .ok_or_else(|| AppError::Internal("Revocation authority is unavailable".into()))
    }
    async fn revoke(
        &self,
        access_token: Option<Zeroizing<String>>,
        refresh_token: Option<Zeroizing<String>>,
    ) -> Option<oauth_revocation::RevocationOutcome> {
        let provider = self.provider.as_ref()?;
        let creds = self.client_id.as_ref().map(|id| ResolvedOAuthCredentials {
            app_source: if self.byo {
                super::super::oauth_app_source::OAuthAppSource::Byo
            } else {
                super::super::oauth_app_source::OAuthAppSource::Platform
            },
            client_id: id.clone(),
            client_secret: self.client_secret.clone(),
            credential_user_id: self.credential_user_id.clone(),
        });
        Some(
            oauth_revocation::revoke_remote(oauth_revocation::RevocationRequest {
                provider,
                scope: self
                    .scope
                    .unwrap_or(oauth_revocation::RevocationScope::Token),
                creds,
                access_token,
                refresh_token,
            })
            .await,
        )
    }
}

/// Prepared outside database retries, then inserted by the exact source claim
/// transaction. Failed persistence rolls back local removal as well.
pub(crate) async fn prepare_disconnect_effect(
    keys: &EncryptionKeys,
    owner: &str,
    mut material: RevocationMaterial,
    scope: oauth_revocation::RevocationScope,
    source_collection: &str,
    source_id: &str,
    shared_effect_id: Option<&str>,
) -> AppResult<Document> {
    material.scope = Some(scope);
    let id = shared_effect_id
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let (ciphertext, version) =
        encrypt_capability(keys, &id, owner, &encode_capability(&material)?).await?;
    Ok(doc! {"_id":id,"user_id":owner,"kind":"oauth_disconnect",
    "source_collection":source_collection,"source_id":source_id,"status":"pending",
    "capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext},
    "context_version":version,"scope":scope.as_str(),"single_attempt":material.single_attempt(),
    "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()})
}

/// Receipts are metadata in a dedicated collection, keyed by the
/// owner, token SHA-256 and the captured revocation authority. They carry no
/// capability and are not independent remote obligations.
fn receipt_id(owner: &str, token: &str, authority: &str) -> (String, String) {
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    (
        uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            format!("revocation-receipt:{owner}:{hash}:{authority}").as_bytes(),
        )
        .to_string(),
        hash,
    )
}
enum ReceiptClaim {
    Confirmed,
    Claimed(Document),
    Busy,
    Unconfirmed,
}
async fn claim_receipt(
    db: &Database,
    effect: &Document,
    token: &str,
    authority: &str,
    single: bool,
) -> AppResult<ReceiptClaim> {
    let db = super::super::transaction_service::durable_database(db);
    let result = super::super::transaction_service::run_db_only(
        &db,
        &(&db, effect, token, authority, single),
        |tx, context| {
            let &(db, effect, token, authority, single) = context;
            Box::pin(claim_receipt_in_session(
                db, effect, token, authority, single, tx,
            ))
        },
    )
    .await;
    match result {
        // A concurrent first claimant can win the unique receipt identity.
        // Its definite duplicate-key abort must not commit our parent fence.
        Err(AppError::DatabaseError(error))
            if matches!(error.kind.as_ref(),mongodb::error::ErrorKind::Command(command) if command.code==11000)
                || matches!(error.kind.as_ref(),mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code==11000) =>
        {
            Ok(ReceiptClaim::Busy)
        }
        other => other,
    }
}
async fn claim_receipt_in_session(
    db: &Database,
    effect: &Document,
    token: &str,
    authority: &str,
    single: bool,
    tx: &mut ClientSession,
) -> AppResult<ReceiptClaim> {
    let owner = effect
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid revocation owner".into()))?;
    let effect_id = effect
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid revocation effect".into()))?;
    // Serialize receipt creation with effect completion/reconciliation. A
    // stale dispatch must not recreate owned metadata after final cleanup.
    if db
        .collection::<Document>(EFFECTS)
        .update_one(
            doc! {
                "_id":effect_id,"user_id":owner,"status":"pending",
                "lease_token":effect.get("lease_token").cloned(),
                "$expr":{"$gt":["$lease_expires_at","$$NOW"]}
            },
            doc! {"$inc":{"receipt_claim_revision":1_i64}},
        )
        .session(&mut *tx)
        .await?
        .matched_count
        != 1
    {
        return Ok(ReceiptClaim::Busy);
    }
    let (id, hash) = receipt_id(owner, token, authority);
    let receipt = db
        .collection::<Document>(RECEIPTS)
        .find_one(doc! {"_id":&id})
        .session(&mut *tx)
        .await?;
    if receipt
        .as_ref()
        .is_some_and(|row| row.get_bool("confirmed") == Ok(true))
    {
        return Ok(ReceiptClaim::Confirmed);
    }
    let generation = retry_generation(effect);
    if let Some(receipt) = receipt.as_ref()
        && let Ok(original) = receipt.get_str("effect_id")
        && original != effect_id
    {
        let previous = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":original,"user_id":owner})
            .session(&mut *tx)
            .await?;
        if previous
            .as_ref()
            .is_some_and(|row| row.get_str("status") == Ok("pending"))
        {
            return Ok(ReceiptClaim::Busy);
        }
        // A duplicate cannot silently bypass another effect's exhausted
        // budget. An explicit operator retry may take over its receipt lease.
        if generation == 0 {
            return Ok(ReceiptClaim::Unconfirmed);
        }
    }
    if single
        && let Some(receipt) = receipt.as_ref()
        && receipt.get_bool("attempted") == Ok(true)
        && !(generation > 0
            && (receipt.get_str("effect_id") != Ok(effect_id)
                || retry_generation(receipt) < generation))
    {
        return Ok(ReceiptClaim::Unconfirmed);
    }
    let mut predicate = match receipt.as_ref() {
        Some(row) => super::super::record_storage::exact_document_predicate(row),
        // MongoDB forbids $expr in an upsert predicate. The missing-kind
        // condition makes a concurrent first claimant lose to the unique _id
        // instead of overwriting the winner's live lease.
        None => doc! {"_id":&id,"kind":{"$exists":false}},
    };
    if receipt.is_some() {
        let available = doc! {"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]};
        if let Some(exact) = predicate.remove("$expr") {
            predicate.insert("$expr", doc! {"$and":[exact,available]});
        } else {
            predicate.insert("$expr", available);
        }
    }
    let result=db.collection::<Document>(RECEIPTS).find_one_and_update(predicate,
        vec![doc! {"$set":{"kind":"oauth_revocation_receipt","user_id":owner,"token_sha256":hash,"authority_digest":authority,
            "effect_id":effect_id,"retry_generation":generation,"attempted":false,
            "lease_token":effect.get("lease_token").cloned(),"lease_expires_at":effect.get("lease_expires_at").cloned(),
            "updated_at":"$$NOW","created_at":{"$ifNull":["$created_at","$$NOW"]}}}])
        .upsert(receipt.is_none()).return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await;
    match result {
        Ok(Some(row)) => Ok(ReceiptClaim::Claimed(row)),
        Ok(None) => Ok(ReceiptClaim::Busy),
        Err(error) => Err(error.into()),
    }
}
fn retry_generation(effect: &Document) -> i64 {
    effect
        .get_i64("retry_generation")
        .unwrap_or_else(|_| i64::from(effect.get_i32("retry_generation").unwrap_or(0)))
}
async fn record_receipt(db: &Database, receipt: &Document, confirmed: bool) -> AppResult<()> {
    db.collection::<Document>(RECEIPTS).update_one(doc! {"_id":receipt.get("_id").cloned(),
        "lease_token":receipt.get("lease_token").cloned(),"$expr":{"$gt":["$lease_expires_at","$$NOW"]}},
        doc! {"$set":{"confirmed":confirmed},"$unset":{"lease_token":"","lease_expires_at":""},"$currentDate":{"updated_at":true}}).await?;
    Ok(())
}
enum RevocationDelivery {
    Outcome(oauth_revocation::RevocationOutcome),
    Busy,
    Unconfirmed,
}
fn confirmed(outcome: &Option<TokenOutcome>, scope: oauth_revocation::RevocationScope) -> bool {
    matches!(outcome, Some(TokenOutcome::Delivered))
        || (scope == oauth_revocation::RevocationScope::Token
            && matches!(outcome, Some(TokenOutcome::NotFound)))
}
async fn revoke_with_receipts(
    db: &Database,
    effect: &Document,
    material: &RevocationMaterial,
) -> AppResult<RevocationDelivery> {
    let authority = material.authority()?;
    let owner = effect
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid revocation owner".into()))?;
    let hashes: std::collections::BTreeSet<_> = [
        material.access_token.as_ref(),
        material.refresh_token.as_ref(),
    ]
    .into_iter()
    .flatten()
    .map(|token| receipt_id(owner, token, &authority).1)
    .collect();
    // Keep the per-owner dedup identity on the obligation itself. Receipt
    // children coordinate delivery of the same identity across local copies.
    let recorded = db.collection::<Document>(EFFECTS).update_one(doc! {
        "_id":effect.get("_id").cloned(),"user_id":owner,"status":"pending",
        "lease_token":effect.get("lease_token").cloned(),
        "$expr":{"$gt":["$lease_expires_at","$$NOW"]}
    },doc! {"$set":{"authority_digest":&authority,"token_sha256":hashes.into_iter().collect::<Vec<_>>()}}).await?;
    if recorded.matched_count != 1 {
        return Ok(RevocationDelivery::Busy);
    }
    let mut claims = Vec::new();
    let mut access = None;
    let mut refresh = None;
    let mut result = oauth_revocation::RevocationOutcome {
        style: material
            .provider
            .as_ref()
            .and_then(oauth_revocation::effective_revocation)
            .map(|config| config.style),
        scope: material
            .scope
            .unwrap_or(oauth_revocation::RevocationScope::Token),
        access: None,
        refresh: None,
    };
    for (is_access, token) in [
        (true, material.access_token.as_ref()),
        (false, material.refresh_token.as_ref()),
    ] {
        let Some(token) = token else {
            continue;
        };
        // One opaque token in both slots is one upstream capability.
        if !is_access
            && material
                .access_token
                .as_ref()
                .is_some_and(|access| access.as_str() == token.as_str())
        {
            if result.access.is_some() {
                result.refresh = result.access.clone();
            }
            continue;
        }
        match claim_receipt(db, effect, token, &authority, material.single_attempt()).await? {
            ReceiptClaim::Confirmed => {
                if is_access {
                    result.access = Some(TokenOutcome::Delivered)
                } else {
                    result.refresh = Some(TokenOutcome::Delivered)
                }
            }
            ReceiptClaim::Claimed(receipt) => {
                claims.push((is_access, receipt));
                if is_access {
                    access = Some(token.clone())
                } else {
                    refresh = Some(token.clone())
                }
            }
            other => {
                for (_, receipt) in &claims {
                    record_receipt(db, receipt, false).await?;
                }
                return Ok(if matches!(other, ReceiptClaim::Busy) {
                    RevocationDelivery::Busy
                } else {
                    RevocationDelivery::Unconfirmed
                });
            }
        }
    }
    if access.is_some() || refresh.is_some() {
        for (_, receipt) in &claims {
            let changed = db.collection::<Document>(RECEIPTS).update_one(doc! {"_id":receipt.get("_id").cloned(),
                "lease_token":receipt.get("lease_token").cloned(),"$expr":{"$gt":["$lease_expires_at","$$NOW"]}},
                doc! {"$set":{"attempted":true}}).await?;
            if changed.matched_count != 1 {
                return Ok(RevocationDelivery::Unconfirmed);
            }
        }
        let response = material
            .revoke(access, refresh)
            .await
            .ok_or_else(|| AppError::Internal("Invalid revocation descriptor".into()))?;
        let grant_confirmed = material.scope == Some(oauth_revocation::RevocationScope::Grant)
            && (confirmed(&response.access, result.scope)
                || confirmed(&response.refresh, result.scope));
        for (is_access, receipt) in claims {
            let outcome = if grant_confirmed {
                Some(TokenOutcome::Delivered)
            } else if is_access {
                response.access.clone()
            } else {
                response.refresh.clone()
            };
            record_receipt(db, &receipt, confirmed(&outcome, result.scope)).await?;
            if is_access {
                result.access = outcome
            } else {
                result.refresh = outcome
            }
        }
    }
    if material.access_token.is_some() && material.access_token == material.refresh_token {
        result.refresh = result.access.clone();
    }
    Ok(RevocationDelivery::Outcome(result))
}

// Allocate once; serialization cannot free earlier plaintext allocations during
// growth. The buffer and all deserialized string owners clear on every exit.
pub(super) fn encode_capability<T: Serialize>(value: &T) -> AppResult<Zeroizing<Vec<u8>>> {
    struct Writer(Zeroizing<Vec<u8>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_CAPABILITY - self.0.len() {
                return Err(std::io::Error::other("revocation capability size limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer(Zeroizing::new(Vec::with_capacity(MAX_CAPABILITY)));
    serde_json::to_writer(&mut writer, value)
        .map_err(|_| AppError::Internal("Revocation capability encoding failed".into()))?;
    Ok(writer.0)
}

fn capability_aad(id: &str, owner: &str) -> Vec<u8> {
    let mut aad = b"nyxid:erasure-capability:v1\0".to_vec();
    for part in [EFFECTS, id, owner, "capability_encrypted"] {
        aad.extend_from_slice(&(part.len() as u32).to_be_bytes());
        aad.extend_from_slice(part.as_bytes());
    }
    aad
}
pub(super) async fn encrypt_capability(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    bytes: &[u8],
) -> AppResult<(Vec<u8>, i32)> {
    if keys.contextual_record_writes() {
        Ok((
            keys.encrypt_record(
                bytes,
                &RecordContext::new(EFFECTS, id, owner, &["capability_encrypted"])?,
            )
            .await?,
            3,
        ))
    } else {
        Ok((
            keys.encrypt_with_aad(bytes, &capability_aad(id, owner))
                .await?,
            2,
        ))
    }
}
async fn decrypt_capability(
    keys: &EncryptionKeys,
    effect: &Document,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let id = effect
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid effect identity".into()))?;
    let owner = effect
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid effect owner".into()))?;
    let bytes = effect
        .get_binary_generic("capability_encrypted")
        .map_err(|_| AppError::Internal("Missing preserved capability".into()))?;
    match effect.get("context_version") {
        // Historical contextual capabilities omitted the marker. Absence has
        // this one documented meaning; present malformed markers never select
        // a decoder, including an otherwise numerically equal BSON int64.
        None | Some(bson::Bson::Int32(3)) => {
            keys.decrypt_record(
                bytes,
                &RecordContext::new(EFFECTS, id, owner, &["capability_encrypted"])?,
            )
            .await
        }
        Some(bson::Bson::Int32(2)) if keys.allow_legacy_record_reads() => keys
            .decrypt_with_aad(bytes, &capability_aad(id, owner))
            .await
            .map(Zeroizing::new),
        _ => Err(AppError::Internal(
            "Unsupported preserved capability format".into(),
        )),
    }
}

pub(super) enum Pass {
    Complete,
    CompleteWithException(&'static str),
    Pending,
    Action(&'static str),
}

/// One external effect or one source row per pass.
/// Verified token-only effects retry within a bounded budget. Grant-capable
/// effects use one near-immediate attempt. Completion requires a receipt.
pub(super) async fn pass(
    db: &Database,
    keys: &EncryptionKeys,
    job: &PersonalErasureJob,
) -> AppResult<Pass> {
    let durable = super::super::transaction_service::durable_database(db);
    let db = &durable;
    // Dependency gate also protects already-retained jobs with older domain
    // array order. Never revoke the connection needed to remove its webhook.
    if db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"user_id":&job.user_id,"personal_erasure_cleanup_complete":{"$ne":true}})
        .projection(doc! {"_id":1})
        .await?
        .is_some()
    {
        return Ok(Pass::Pending);
    }
    if let Some(effect) = db
        .collection::<Document>(EFFECTS)
        .find_one(doc! {"user_id":&job.user_id,"status":"operator_action"})
        .await?
    {
        let _ = effect;
        return Ok(Pass::Action("provider_revocation_requires_reconciliation"));
    }
    if let Ok(_permit) = oauth_revocation::try_acquire_revocation_permit()
        && let Some(effect) = claim_effect(db, Some(&job.user_id)).await?
    {
        deliver(db, keys, &effect).await?;
        return Ok(Pass::Pending);
    }
    if db
        .collection::<Document>(EFFECTS)
        .find_one(doc! {"user_id":&job.user_id,
        "status":"pending"})
        .await?
        .is_some()
    {
        return Ok(Pass::Pending);
    }
    // Process canonically owned service accounts as personal resources, not
    // arbitrary accounts for which this person was only the initiating actor.
    if let Some(account)=db.collection::<Document>(crate::models::service_account::COLLECTION_NAME)
        .find_one(doc! {"$or":[{"owner_user_id":&job.user_id},{"owner_user_id":bson::Bson::Null,"created_by":&job.user_id}],"personal_erasure_credentials_preserved":{"$ne":true}}).sort(doc! {"_id":1}).await? {
        let subject=account.get_str("_id").map_err(|_|AppError::Internal("Invalid service account owner".into()))?;
        for collection in [crate::models::user_api_key::COLLECTION_NAME,crate::models::user_provider_token::COLLECTION_NAME] {
            if let Some(source)=db.collection::<Document>(collection).find_one(doc! {"user_id":subject,"personal_erasure_preserved":{"$ne":true}}).await? {
                preserve(db,keys,job,collection,&source).await?; return Ok(Pass::Pending);
            }
        }
        let mut tx=db.client().start_session().await?;
        tx.start_transaction().read_concern(ReadConcern::snapshot()).write_concern(WriteConcern::majority()).await?;
        let result:AppResult<()>=async {
            if db.collection::<Document>(super::super::erasure_admission::COLLECTION)
                .update_one(doc! {"_id":&job.user_id,"erasure_accepted":true},doc! {"$inc":{"cleanup_revision":1_i64}})
                .session(&mut tx).await?.matched_count!=1 {return Err(AppError::Conflict("Personal cleanup barrier changed".into()));}
            for collection in [crate::models::user_api_key::COLLECTION_NAME,crate::models::user_provider_token::COLLECTION_NAME] {
                if db.collection::<Document>(collection).find_one(doc! {"user_id":subject,"personal_erasure_preserved":{"$ne":true}}).session(&mut tx).await?.is_some() {
                    return Err(AppError::Conflict("Service-account credentials changed during preservation".into()));
                }
            }
            if db.collection::<Document>(crate::models::service_account::COLLECTION_NAME).update_one(
                doc! {"_id":subject,"$or":[{"owner_user_id":&job.user_id},{"owner_user_id":bson::Bson::Null,"created_by":&job.user_id}]},
                doc! {"$set":{"personal_erasure_credentials_preserved":true},"$inc":{"owner_lifecycle_fence":1_i64}}).session(&mut tx).await?.matched_count!=1 {
                return Err(AppError::Conflict("Service-account owner changed during preservation".into()));
            }
            Ok(())
        }.await;
        super::super::org_membership_authority::finish(tx,result).await?;
        return Ok(Pass::Pending);
    }
    for collection in [
        crate::models::user_api_key::COLLECTION_NAME,
        crate::models::user_provider_token::COLLECTION_NAME,
    ] {
        if let Some(source) = db
            .collection::<Document>(collection)
            .find_one(doc! {"user_id":&job.user_id,"personal_erasure_preserved":{"$ne":true}})
            .sort(doc! {"_id":1})
            .await?
        {
            preserve(db, keys, job, collection, &source).await?;
            return Ok(Pass::Pending);
        }
    }
    if db
        .collection::<Document>(EFFECTS)
        .find_one(doc! {"user_id":&job.user_id,"$or":[{"status":{"$ne":"complete"}},{"capability_encrypted":{"$exists":true}}]})
        .await?
        .is_some()
    {
        return Ok(Pass::Pending);
    }
    if db
        .collection::<Document>(EFFECTS)
        .find_one(doc! {"user_id":&job.user_id,"status":"complete",
        "completion_basis":"operator_recorded_revocation_unavailable"})
        .await?
        .is_some()
    {
        return Ok(Pass::CompleteWithException(
            "remote_revocation_unavailable_policy_exception",
        ));
    }
    Ok(Pass::Complete)
}

/// Re-open a stale completed provider checkpoint before exposing it to a reader.
pub(super) async fn refresh(
    db: &Database,
    mut job: PersonalErasureJob,
) -> AppResult<PersonalErasureJob> {
    if job.status != ErasureStatus::Complete
        && !job.domains.iter().any(|domain| {
            domain.domain == "provider_credentials" && domain.status == DomainStatus::Complete
        })
    {
        return Ok(job);
    }
    let outstanding = db
        .collection::<Document>(EFFECTS)
        .find_one(doc! {
            "user_id":&job.user_id,
            "$or":[{"status":{"$ne":"complete"}},{"capability_encrypted":{"$exists":true}}]
        })
        .await?;
    let Some(effect) = outstanding else {
        return Ok(job);
    };
    let original = bson::to_bson(&job.domains)
        .map_err(|_| AppError::Internal("Erasure evidence encoding failed".into()))?;
    let action = effect.get_str("status") == Ok("operator_action");
    if let Some(domain) = job
        .domains
        .iter_mut()
        .find(|domain| domain.domain == "provider_credentials")
    {
        domain.status = if action {
            DomainStatus::OperatorAction
        } else {
            DomainStatus::Pending
        };
        domain.completed_at = None;
        domain.reason = Some("provider_revocation_requires_reconciliation".into());
    }
    job.status = if action {
        ErasureStatus::OperatorAction
    } else {
        ErasureStatus::Pending
    };
    job.completed_at = None;
    let updated = bson::to_bson(&job.domains)
        .map_err(|_| AppError::Internal("Erasure evidence encoding failed".into()))?;
    db.collection::<Document>(JOBS).update_one(doc! {"_id":&job.id,"domains":original,
        "$expr":{"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}},
        vec![doc! {"$set":{"domains":{"$literal":updated},"status":if action {"operator_action"} else {"pending"},
            "completed_at":bson::Bson::Null,"updated_at":"$$NOW","retry_at":"$$NOW","lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null}}]).await?;
    Ok(job)
}

async fn preserve(
    db: &Database,
    keys: &EncryptionKeys,
    job: &PersonalErasureJob,
    collection: &str,
    source: &Document,
) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    let owner = source
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid credential owner".into()))?;
    let source_id = source
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid revocation source".into()))?;
    let plan = super::preparation::source_plan(db, &job.user_id, collection, source_id).await?;
    let marker = plan.as_ref();
    // Preserve the strongest live member before a shared grant effect can be
    // dispatched. The plan contains metadata only, never credential material.
    if let Some(marker) = marker.filter(|row| row.get_str("scope") == Ok("grant")) {
        let representative_collection = marker
            .get_str("representative_collection")
            .map_err(|_| AppError::Internal("Missing grant representative".into()))?;
        let representative_id = marker
            .get_str("representative_id")
            .map_err(|_| AppError::Internal("Missing grant representative".into()))?;
        if (representative_collection != collection || representative_id != source_id)
            && let Some(representative) = db.collection::<Document>(representative_collection)
                .find_one(doc! {"_id":representative_id,"user_id":owner,"personal_erasure_preserved":{"$ne":true}}).await? {
            return Box::pin(preserve(db, keys, job, representative_collection, &representative)).await;
        }
    }
    let material = Box::pin(
        super::super::unified_key_service::prepare_erasure_revocation(
            db, keys, owner, collection, source,
        ),
    )
    .await?;
    let scope = if marker.and_then(|row| row.get_str("scope").ok()) == Some("grant") {
        oauth_revocation::RevocationScope::Grant
    } else {
        oauth_revocation::RevocationScope::Token
    };
    let identity = if scope == oauth_revocation::RevocationScope::Grant {
        format!(
            "{}:{owner}:grant:{}",
            job.id,
            marker
                .and_then(|row| row.get_str("group").ok())
                .ok_or_else(|| AppError::Internal("Erasure grant group is unavailable".into()))?
        )
    } else {
        format!(
            "{}:{collection}:{source_id}:{}",
            job.id,
            hex::encode(Sha256::digest(bson::to_vec(source).map_err(|_| {
                AppError::Internal("Invalid revocation source encoding".into())
            })?))
        )
    };
    let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, identity.as_bytes()).to_string();
    let effect = match material {
        Some(mut material) => {
            material.scope = Some(scope);
            if let Some(lineage) = marker.and_then(|row| row.get_str("app_lineage").ok()) {
                material.app_lineage = Some(lineage.to_owned());
            }
            let (encrypted, version) =
                encrypt_capability(keys, &id, &job.user_id, &encode_capability(&material)?).await?;
            Some(
                doc! {"_id":&id,"user_id":&job.user_id,"target_owner_id":owner,"source_collection":collection,"source_id":source_id,
                "provider_config_id":material.provider.as_ref().map(|provider|provider.id.as_str()),
                "kind":"oauth_token_revocation","status":"pending","capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
                "context_version":version,"scope":scope.as_str(),"single_attempt":material.single_attempt(),
                "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()},
            )
        }
        None => None,
    };
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<()>=async {
        if db.collection::<Document>(USERS).update_one(doc! {"_id":&job.user_id,"is_active":false,"platform_personal_deleting":true},doc! {"$inc":{"erasure_effect_fence":1_i64}}).session(&mut tx).await?.matched_count!=1 {
            return Err(AppError::Conflict("Erasure source authority changed".into()));
        }
        if db.collection::<Document>(collection).update_one(super::super::record_storage::exact_document_predicate(source),
            doc! {"$set":{"personal_erasure_preserved":true,"status":"revoked","error_message":bson::Bson::Null},
                "$unset":{"access_token_encrypted":"","refresh_token_encrypted":"","credential_encrypted":"","api_key_encrypted":"",
                    "user_oauth_client_id_encrypted":"","user_oauth_client_secret_encrypted":""}}).session(&mut tx).await?.matched_count!=1 {
            return Err(AppError::Conflict("Revocation source changed".into()));
        }
        if let Some(effect)=effect.as_ref() {
            db.collection::<Document>(EFFECTS).update_one(doc! {"_id":&id},doc! {"$setOnInsert":effect}).upsert(true).session(&mut tx).await?;
        }
        Ok(())
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

pub(crate) async fn schedule_disconnect(
    db: &Database,
    keys: &EncryptionKeys,
    id: &str,
) -> AppResult<bool> {
    let durable = super::super::transaction_service::durable_database(db);
    let Ok(permit) = oauth_revocation::try_acquire_revocation_permit() else {
        return Ok(false);
    };
    let Some(effect) = claim_effect_matching(&durable, None, Some(id)).await? else {
        return Ok(false);
    };
    let plaintext = decrypt_capability(keys, &effect).await?;
    let material: RevocationMaterial = serde_json::from_slice(&plaintext)
        .map_err(|_| AppError::Internal("Invalid disconnect capability".into()))?;
    if let Some(reason) = preflight(&durable, keys, &effect, &material).await? {
        finish_effect(&durable, &effect, false, Some(reason)).await?;
        return Ok(false);
    }
    if let Some(reason) = material.skip_reason() {
        material.audit(&durable, &effect, reason, None, None).await;
        finish_effect(&durable, &effect, false, Some(reason)).await?;
        return Ok(false);
    }
    tokio::spawn(async move {
        let _permit = permit;
        let _ = deliver_material(&durable, &effect, material).await;
    });
    Ok(true)
}
async fn deliver(db: &Database, keys: &EncryptionKeys, effect: &Document) -> AppResult<()> {
    let material = match decrypt_capability(keys, effect).await {
        Ok(plaintext) => serde_json::from_slice::<RevocationMaterial>(&plaintext).ok(),
        Err(_) => None,
    };
    let Some(material) = material else {
        return finish_effect(
            db,
            effect,
            false,
            Some("preserved_capability_unavailable_or_invalid"),
        )
        .await;
    };
    if let Some(reason) = preflight(db, keys, effect, &material).await? {
        return finish_effect(db, effect, false, Some(reason)).await;
    }
    deliver_material(db, effect, material).await
}
const GRANT_REVOCATION_START_WINDOW: chrono::Duration = chrono::Duration::minutes(5);
fn window_origin(effect: &Document) -> Option<chrono::DateTime<chrono::Utc>> {
    let created = effect.get_datetime("created_at").ok()?.to_chrono();
    Some(
        effect
            .get_datetime("retry_armed_at")
            .map(|time| time.to_chrono().max(created))
            .unwrap_or(created),
    )
}
async fn successor_present(
    db: &Database,
    keys: &EncryptionKeys,
    effect: &Document,
    material: &RevocationMaterial,
    origin: chrono::DateTime<chrono::Utc>,
) -> AppResult<bool> {
    use futures::TryStreamExt;
    let Some(provider) = material.provider.as_ref() else {
        return Ok(false);
    };
    let owner = effect
        .get_str("target_owner_id")
        .or_else(|_| effect.get_str("user_id"))
        .map_err(|_| AppError::Internal("Invalid revocation owner".into()))?;
    let expected = material.app_lineage.as_deref().unwrap_or(if material.byo {
        "unknown"
    } else {
        "platform"
    });
    for collection in [
        crate::models::user_api_key::COLLECTION_NAME,
        crate::models::user_provider_token::COLLECTION_NAME,
    ] {
        let mut cursor = db
            .collection::<Document>(collection)
            .find(doc! {"user_id":owner,"provider_config_id":&provider.id,
            "status":{"$ne":"revoked"},"personal_erasure_preserved":{"$ne":true},
            "created_at":{"$gt":bson::DateTime::from_chrono(origin)}})
            .await?;
        while let Some(source) = cursor.try_next().await? {
            let group = super::super::unified_key_service::erasure_credential_group(
                db, keys, collection, &source,
            )
            .await?;
            if group.get_str("app_lineage") == Ok(expected) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
async fn deliver_material(
    db: &Database,
    effect: &Document,
    material: RevocationMaterial,
) -> AppResult<()> {
    if material.provider.is_none() {
        let reason = match material.unavailable_family.as_deref() {
            Some("gcp_service_account") => {
                "gcp_service_account_access_token_has_no_individual_revocation"
            }
            Some("unattributed_oauth") => "oauth_provider_unknown_requires_external_reconciliation",
            _ => "preserved_capability_unavailable_or_invalid",
        };
        return finish_effect(db, effect, false, Some(reason)).await;
    }
    if let Some(reason) = material.skip_reason() {
        material.audit(db, effect, reason, None, None).await;
        return finish_effect(db, effect, false, Some(reason)).await;
    }
    let single = material.single_attempt();
    let outcome = match revoke_with_receipts(db, effect, &material).await? {
        RevocationDelivery::Outcome(outcome) => outcome,
        RevocationDelivery::Busy => return defer_effect(db, effect).await,
        RevocationDelivery::Unconfirmed => {
            return finish_effect(db, effect, false, Some("grant_scope_unconfirmed")).await;
        }
    };
    material
        .audit(
            db,
            effect,
            super::super::unified_key_service::aggregate_revocation_outcome(&outcome),
            outcome
                .access
                .as_ref()
                .map(super::super::unified_key_service::token_outcome_name),
            outcome
                .refresh
                .as_ref()
                .map(super::super::unified_key_service::token_outcome_name),
        )
        .await;
    let complete = if material.scope == Some(oauth_revocation::RevocationScope::Grant) {
        // Request-scoped grant endpoints (GitHub, Meta) have one access-token
        // receipt; RFC7009 reports both supplied token outcomes.
        confirmed(&outcome.access, outcome.scope) || confirmed(&outcome.refresh, outcome.scope)
    } else {
        (material.access_token.is_none() || confirmed(&outcome.access, outcome.scope))
            && (material.refresh_token.is_none() || confirmed(&outcome.refresh, outcome.scope))
    };
    let permanent = [&outcome.access, &outcome.refresh]
        .into_iter()
        .any(|outcome| matches!(outcome, Some(TokenOutcome::PermanentFailure)));
    let action = if !complete && permanent {
        Some("provider_revocation_permanent_failure")
    } else if !complete && single {
        Some("grant_scope_unconfirmed")
    } else if [&outcome.access, &outcome.refresh]
        .into_iter()
        .any(|value| matches!(value, Some(TokenOutcome::Skipped(_))))
    {
        Some("provider_revocation_not_supported_or_credentials_missing")
    } else {
        None
    };
    finish_effect(db, effect, complete, action).await
}
async fn defer_effect(db: &Database, effect: &Document) -> AppResult<()> {
    db.collection::<Document>(EFFECTS).update_one(doc! {"_id":effect.get("_id").cloned(),"status":"pending",
        "lease_token":effect.get("lease_token").cloned(),"$expr":{"$gt":["$lease_expires_at","$$NOW"]}},
        vec![doc! {"$set":{"reason":"equivalent_revocation_in_flight",
            "attempts":attempts(effect).saturating_sub(1).max(0),"lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,
            "retry_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":10}},"updated_at":"$$NOW"}}]).await?;
    Ok(())
}

async fn preflight(
    db: &Database,
    keys: &EncryptionKeys,
    effect: &Document,
    material: &RevocationMaterial,
) -> AppResult<Option<&'static str>> {
    if material.single_attempt() {
        // Claim increments attempts. A reclaimed dispatch can have completed at
        // the provider before its receipt was committed, so it cannot be sent again.
        let reason = if attempts(effect) > 1 {
            Some("grant_scope_unconfirmed")
        } else if let Some(origin) = window_origin(effect) {
            if chrono::Utc::now() > origin + GRANT_REVOCATION_START_WINDOW {
                Some("grant_scope_window_elapsed")
            } else if successor_present(db, keys, effect, material, origin).await? {
                Some("grant_scope_successor_present")
            } else {
                None
            }
        } else {
            Some("grant_scope_window_elapsed")
        };
        return Ok(reason);
    }
    Ok(None)
}

const MAX_DELIVERY_ATTEMPTS: i64 = 8;
fn attempts(effect: &Document) -> i64 {
    effect
        .get_i64("attempts")
        .unwrap_or_else(|_| i64::from(effect.get_i32("attempts").unwrap_or(0)))
}
async fn claim_effect(db: &Database, owner: Option<&str>) -> AppResult<Option<Document>> {
    claim_effect_matching(db, owner, None).await
}
async fn claim_effect_matching(
    db: &Database,
    owner: Option<&str>,
    id: Option<&str>,
) -> AppResult<Option<Document>> {
    let mut filter = doc! {"status":"pending","$expr":{"$and":[
        {"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]},
        {"$lte":[{"$ifNull":["$retry_at",bson::DateTime::from_millis(0)]},"$$NOW"]}
    ]}};
    if let Some(owner) = owner {
        filter.insert("user_id", owner);
    }
    if let Some(id) = id {
        filter.insert("_id", id);
    }
    Ok(db
        .collection::<Document>(EFFECTS)
        .find_one_and_update(
            filter,
            vec![doc! {"$set":{
                "status":"pending",
                "lease_token":uuid::Uuid::new_v4().to_string(),
                "lease_expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":90}},
                "attempts":{"$add":[{"$ifNull":["$attempts",0_i64]},1_i64]},
                "updated_at":"$$NOW"
            }}],
        )
        .sort(doc! {"retry_at":1,"updated_at":1,"_id":1})
        .return_document(mongodb::options::ReturnDocument::After)
        .await?)
}
async fn finish_effect(
    db: &Database,
    effect: &Document,
    complete: bool,
    action_reason: Option<&str>,
) -> AppResult<()> {
    let exhausted = attempts(effect) >= MAX_DELIVERY_ATTEMPTS;
    let action = action_reason.is_some() || (!complete && exhausted);
    let reason = if complete {
        "provider_revocation_confirmed"
    } else if let Some(reason) = action_reason {
        reason
    } else if exhausted {
        "provider_revocation_retry_exhausted"
    } else {
        "provider_revocation_unconfirmed"
    };
    let delay = (5_i64 * (1_i64 << attempts(effect).clamp(0, 6))).min(300);
    let mut set = doc! {"status":if complete {"complete"} else if action {"operator_action"} else {"pending"},
    "reason":reason,"attempts":attempts(effect),"lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,
    "updated_at":"$$NOW",
    "retry_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":delay}}};
    if complete {
        set.insert("completed_at", "$$NOW");
    }
    let mut stages = vec![doc! {"$set":set}];
    if complete {
        stages.push(doc! {"$unset":"capability_encrypted"});
    }
    db.collection::<Document>(EFFECTS).update_one(doc! {
        "_id":effect.get_str("_id").map_err(|_|AppError::Internal("Invalid effect identity".into()))?,
        "status":"pending","lease_token":effect.get_str("lease_token").map_err(|_|AppError::Internal("Invalid erasure lease".into()))?,
        "$expr":{"$gt":["$lease_expires_at","$$NOW"]}
    }, stages).await?;
    Ok(())
}

fn unavailable_resolution_allowed(reason: &str) -> bool {
    matches!(
        reason,
        "preserved_capability_unavailable_or_invalid"
            | "provider_revocation_retry_exhausted"
            | "provider_revocation_not_supported_or_credentials_missing"
            | "provider_revocation_permanent_failure"
            | "gcp_service_account_access_token_has_no_individual_revocation"
            | "skipped_decrypt_failed"
            | "skipped_not_configured"
            | "skipped_no_token"
            | "skipped_no_credentials"
            | "skipped_unsupported_auth"
            | "skipped_grant_only"
            | "skipped_unsupported_style"
    )
}

/// One explicit operator disposition, bound to the round shown by the metadata
/// endpoint. A recorded request is idempotent even after that round exhausts.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EffectReconciliationRequest {
    pub effect_id: String,
    pub action: String,
    pub evidence_reference: String,
    pub expected_retry_generation: i64,
    pub idempotency_key: String,
}

/// Authenticated admin/operator layer supplies actor authority. Evidence is a
/// bounded reference hash, not credential text. External confirmation is an
/// audited operator attestation, including when the capability cannot be read.
pub(crate) async fn reconcile_effect(
    db: &Database,
    keys: &EncryptionKeys,
    actor: &str,
    user: &str,
    request: &EffectReconciliationRequest,
) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    let effect_id = request.effect_id.as_str();
    let action = request.action.as_str();
    let evidence_reference = request.evidence_reference.as_str();
    if request.expected_retry_generation < 0
        || uuid::Uuid::parse_str(&request.idempotency_key).is_err()
    {
        return Err(AppError::ValidationError(
            "Invalid effect retry identity or generation".into(),
        ));
    }
    if evidence_reference.trim().is_empty()
        || evidence_reference.len() > 1024
        || evidence_reference.chars().any(char::is_control)
    {
        return Err(AppError::ValidationError(
            "A bounded evidence reference is required".into(),
        ));
    }
    if !matches!(
        action,
        "retry"
            | "retry_with_current_provider_config"
            | "record_remote_revocation_unavailable"
            | "record_external_revocation_confirmed"
    ) {
        return Err(AppError::ValidationError(
            "Unsupported revocation reconciliation action".into(),
        ));
    }
    let durable = super::super::transaction_service::durable_database(db);
    let db = &durable;
    let key_hash = hex::encode(Sha256::digest(request.idempotency_key.as_bytes()));
    let request_digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&serde_json::json!({
            "actor":actor,"owner":user,"effect_id":effect_id,"action":action,
            "retry_generation":request.expected_retry_generation,
            "evidence_sha256":hex::encode(Sha256::digest(evidence_reference.as_bytes())),
        }))
        .map_err(|_| AppError::Internal("Effect reconciliation binding failed".into()))?,
    ));
    let effect = db
        .collection::<Document>(EFFECTS)
        .find_one(doc! {"_id":effect_id,"user_id":user})
        .await?
        .ok_or_else(|| AppError::Conflict("Effect is not awaiting reconciliation".into()))?;
    // Do not decrypt, consume another budget, append another audit event or
    // wake a parent for a previously accepted HTTP request.
    if effect.get_str("last_reconciliation_key_hash").ok() == Some(key_hash.as_str()) {
        if effect.get_str("last_reconciliation_request_digest").ok()
            != Some(request_digest.as_str())
        {
            return Err(AppError::Conflict(
                "Effect reconciliation idempotency key was reused".into(),
            ));
        }
        return Ok(());
    }
    let generation = match effect.get("retry_generation") {
        None => 0,
        Some(bson::Bson::Int64(value)) => *value,
        Some(bson::Bson::Int32(value)) => i64::from(*value),
        _ => return Err(AppError::Internal("Invalid effect retry generation".into())),
    };
    if effect.get_str("status").ok() != Some("operator_action")
        || generation != request.expected_retry_generation
    {
        return Err(AppError::Conflict(
            "Effect reconciliation round changed; reload current evidence".into(),
        ));
    }
    let next_generation = generation
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("Effect retry generation exhausted".into()))?;
    let mut set = doc! {"status":"pending","reason":bson::Bson::Null,"lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,"attempts":0_i64,"retry_at":bson::Bson::Null,
    "retry_armed_at":bson::DateTime::now(),"retry_generation":next_generation,"last_reconciliation_key_hash":&key_hash,
    "last_reconciliation_request_digest":&request_digest};
    if action == "record_external_revocation_confirmed" {
        set.extend(doc! {"status":"complete","reason":"external_revocation_confirmed",
            "completion_basis":"operator_confirmed_external_revocation","remote_revocation_confirmed":true});
    } else if action == "record_remote_revocation_unavailable" {
        // GCP service-account mint receipts retain their issuance reason. Their
        // captured family still proves there is no individual token-revocation
        // endpoint, including when the encrypted response cannot be read.
        if !unavailable_resolution_allowed(effect.get_str("reason").unwrap_or(""))
            && effect.get_str("credential_family") != Ok("gcp_service_account")
        {
            return Err(AppError::Conflict(
                "This effect requires external confirmation or an explicit retry".into(),
            ));
        }
        set.extend(doc! {"status":"complete","reason":"remote_revocation_unavailable",
            "completion_basis":"operator_recorded_revocation_unavailable","remote_revocation_confirmed":false});
    } else if action == "retry_with_current_provider_config" {
        let plaintext = decrypt_capability(keys, &effect).await?;
        let mut material: RevocationMaterial = serde_json::from_slice(&plaintext)
            .map_err(|_| AppError::Internal("Preserved capability is invalid".into()))?;
        let current = if let Some(provider) = material.provider.as_ref() {
            db.collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
                .find_one(doc! {"_id":&provider.id})
                .await?
        } else {
            None
        };
        let current = current
            .ok_or_else(|| AppError::Conflict("Provider configuration is missing".into()))?;
        // BYO credentials remain bound to their historical lineage. Platform
        // credentials may rotate only after proving the same client identity.
        if !material.byo {
            let resolved = super::super::user_credentials_service::decrypt_provider_credentials(
                keys, &current,
            )
            .await?;
            if material
                .client_id
                .as_ref()
                .is_none_or(|old| old.as_bytes() != resolved.client_id.as_bytes())
            {
                return Err(AppError::Conflict("Current provider names a different OAuth app; reconcile original app credentials".into()));
            }
            material.client_secret = resolved.client_secret;
        }
        material.provider = Some(current);
        set.insert("single_attempt", material.single_attempt());
        if let Some(provider) = material.provider.as_mut() {
            strip_provider_secrets(provider);
        }
        let (encrypted, version) =
            encrypt_capability(keys, effect_id, user, &encode_capability(&material)?).await?;
        set.insert("context_version", version);
        set.insert(
            "capability_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: encrypted,
            },
        );
    }
    let context = (
        &durable,
        actor,
        user,
        action,
        effect_id,
        generation,
        &effect,
        &set,
        &request_digest,
        evidence_reference,
    );
    super::super::transaction_service::run_db_only(db, &context, |tx, context| {
        let &(db, actor, user, action, effect_id, generation, effect, set, request_digest, evidence_reference) = context;
        Box::pin(async move {
    let result:AppResult<()>=async {
        let mut predicate = super::super::record_storage::exact_document_predicate(effect);
        predicate.insert("$expr",doc! {"$and":[
            {"$eq":["$$ROOT",{"$literal":&effect}]},
            {"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}
        ]});
        let mut update=doc! {"$set":set.clone()};
        if matches!(action,"record_remote_revocation_unavailable" | "record_external_revocation_confirmed") {update.insert("$unset",doc! {"capability_encrypted":""});}
        let changed=db.collection::<Document>(EFFECTS).update_one(predicate,update).session(&mut *tx).await?;
        if changed.matched_count!=1 {return Err(AppError::Conflict("Revocation effect changed; reload evidence".into()));}
        let key=super::super::audit_service::audit_chain_hmac_key().ok_or_else(||AppError::Internal("Audit chain is unavailable".into()))?;
        super::super::audit_service::log_actor_event_in_session(db,&mut *tx,key,&super::super::audit_service::AuditActor {user_id:actor.to_owned(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},"user.account.erasure_provider_reconciled",serde_json::json!({"target_user_id":user,"effect_id":effect_id,"action":action,"expected_retry_generation":generation,"request_digest":request_digest,"evidence_sha256":hex::encode(Sha256::digest(evidence_reference.as_bytes())),"remote_revocation_confirmed":action=="record_external_revocation_confirmed"})).await?;
        // Requeue the parent in the same transaction. Only the domain adapter
        // evaluates resulting evidence; reconciliation never completes it.
        if let Some(job)=db.collection::<PersonalErasureJob>(JOBS).find_one(doc! {"user_id":user}).session(&mut *tx).await? {
            let mut domains=job.domains;
            if let Some(domain)=domains.iter_mut().find(|domain|domain.domain=="provider_credentials") {domain.status=DomainStatus::Pending;domain.reason=None;domain.completed_at=None;}
            let changed = db.collection::<Document>(JOBS).update_one(doc! {"_id":job.id,"lease_token":bson::Bson::Null},vec![doc! {"$set":{"domains":{"$literal":bson::to_bson(&domains).map_err(|_|AppError::Internal("Erasure checkpoint encoding failed".into()))?},"status":"pending","completed_at":bson::Bson::Null,"retry_at":"$$NOW","updated_at":"$$NOW"}}]).session(&mut *tx).await?;
            if changed.matched_count != 1 { return Err(AppError::Conflict("Erasure worker owns this intent; retry reconciliation after its checkpoint".into())); }
        }
        Ok(())
    }.await;
    result
        })
    }).await
}

#[cfg(test)]
mod reconciliation_tests {
    use super::*;

    async fn fixture() -> (Database, String, EncryptionKeys, String) {
        let (db, user) = super::super::tests::fixture().await;
        super::super::accept(&db, &user, &user, None).await.unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let provider: ProviderConfig = bson::from_document(doc! {
            "_id":"provider", "slug":"provider", "name":"Provider", "provider_type":"oauth2",
            "is_active":true, "created_by":"admin", "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
        }).unwrap();
        let material = RevocationMaterial::new(
            provider,
            Some(ResolvedOAuthCredentials {
                app_source: super::super::super::oauth_app_source::OAuthAppSource::Platform,
                client_id: "original-app".to_owned().into(),
                client_secret: Some("old-secret".to_owned().into()),
                credential_user_id: None,
            }),
            Some(Zeroizing::new("access-token".into())),
            None,
        );
        let id = uuid::Uuid::new_v4().to_string();
        let encrypted = keys
            .encrypt_record(
                &encode_capability(&material).unwrap(),
                &RecordContext::new(EFFECTS, &id, &user, &["capability_encrypted"]).unwrap(),
            )
            .await
            .unwrap();
        db.collection::<Document>(EFFECTS).insert_one(doc! {"_id":&id,"user_id":&user,"status":"operator_action",
            "capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}).await.unwrap();
        (db, user, keys, id)
    }

    #[tokio::test]
    async fn active_parent_lease_aborts_child_reconciliation_then_idle_retry_wakes_parent() {
        let (db, user, keys, id) = fixture().await;
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"user_id":&user},
                doc! {"$set":{"lease_token":"held-worker"}},
            )
            .await
            .unwrap();
        let before = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            reconcile_effect(
                &db,
                &keys,
                "operator",
                &user,
                &crate::services::personal_erasure_service::provider::EffectReconciliationRequest {
                    effect_id: id.to_string(),
                    action: ("retry").to_string(),
                    evidence_reference: ("ticket-123").to_string(),
                    expected_retry_generation: 0,
                    idempotency_key: uuid::Uuid::new_v4().to_string()
                }
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            db.collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap(),
            before
        );
        assert_eq!(
            db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type":"user.account.erasure_provider_reconciled"})
                .await
                .unwrap(),
            0
        );
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"user_id":&user},
                doc! {"$set":{"lease_token":bson::Bson::Null,"status":"operator_action"}},
            )
            .await
            .unwrap();
        reconcile_effect(
            &db,
            &keys,
            "operator",
            &user,
            &crate::services::personal_erasure_service::provider::EffectReconciliationRequest {
                effect_id: id.to_string(),
                action: ("retry").to_string(),
                evidence_reference: ("ticket-123").to_string(),
                expected_retry_generation: 0,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            db.collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "pending"
        );
        assert_eq!(
            super::super::status(&db, &user)
                .await
                .unwrap()
                .unwrap()
                .status,
            ErasureStatus::Pending
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn same_platform_app_uses_rotated_secret_but_different_app_is_rejected() {
        for client_id in ["original-app", "different-app"] {
            let (db, user, keys, id) = fixture().await;
            let mut provider:ProviderConfig=bson::from_document(doc! {"_id":"provider","slug":"provider","name":"Provider","provider_type":"oauth2",
                "is_active":true,"created_by":"admin","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
            provider.client_id_encrypted = Some(keys.encrypt(client_id.as_bytes()).await.unwrap());
            provider.client_secret_encrypted = Some(keys.encrypt(b"rotated-secret").await.unwrap());
            db.collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
                .insert_one(&provider)
                .await
                .unwrap();
            let result = reconcile_effect(
                &db,
                &keys,
                "operator",
                &user,
                &crate::services::personal_erasure_service::provider::EffectReconciliationRequest {
                    effect_id: id.to_string(),
                    action: ("retry_with_current_provider_config").to_string(),
                    evidence_reference: ("rotation-ticket").to_string(),
                    expected_retry_generation: 0,
                    idempotency_key: uuid::Uuid::new_v4().to_string(),
                },
            )
            .await;
            let effect = db
                .collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap();
            if client_id == "original-app" {
                result.unwrap();
                let decoded = keys
                    .decrypt_record(
                        effect.get_binary_generic("capability_encrypted").unwrap(),
                        &RecordContext::new(EFFECTS, &id, &user, &["capability_encrypted"])
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let material: RevocationMaterial = serde_json::from_slice(&decoded).unwrap();
                assert_eq!(
                    material
                        .client_secret
                        .as_deref()
                        .map(|secret| secret.as_str()),
                    Some("rotated-secret")
                );
                assert!(!material.byo);
            } else {
                assert!(matches!(result, Err(AppError::Conflict(_))));
                assert_eq!(effect.get_str("status").unwrap(), "operator_action");
            }
            db.drop().await.unwrap();
        }
    }
}

/// Independently drain authenticated requeues, including rejected reconnects
/// with no personal-erasure intent. Remote effects never require inventing one.
pub(crate) async fn drain_one(db: &Database, keys: &EncryptionKeys) -> AppResult<()> {
    let db = super::super::transaction_service::durable_database(db);
    let Ok(_permit) = oauth_revocation::try_acquire_revocation_permit() else {
        return Ok(());
    };
    if let Some(effect) = claim_effect(&db, None).await? {
        deliver(&db, keys, &effect).await?;
    }
    Ok(())
}

/// Storage-only retirement adapter, including pending and operator-action
/// capabilities introduced by remediation. Effect/attempt identity and retry
/// state never change. A format marker is insufficient: authenticate under
/// the exact expected owner/record and prove the primary current provider.
pub(crate) async fn verify_current_capability_snapshot(
    keys: &EncryptionKeys,
    effect: &Document,
) -> AppResult<()> {
    let invalid = || AppError::Internal("Invalid preserved capability storage".into());
    let id = effect.get_str("_id").map_err(|_| invalid())?;
    let owner = effect.get_str("user_id").map_err(|_| invalid())?;
    if effect.get_i32("context_version").ok() != Some(3) {
        return Err(invalid());
    }
    let ciphertext = effect
        .get_binary_generic("capability_encrypted")
        .map_err(|_| invalid())?;
    keys.authenticate_current_record(
        ciphertext,
        &RecordContext::new(EFFECTS, id, owner, &["capability_encrypted"])?,
    )
    .await?;
    let plain = decrypt_capability(keys, effect).await?;
    let material: RevocationMaterial = serde_json::from_slice(&plain).map_err(|_| invalid())?;
    if material.provider.as_ref().is_some_and(|provider| {
        provider.client_id_encrypted.is_some()
            || provider.client_secret_encrypted.is_some()
            || !provider.secret_context_versions.is_empty()
    }) || (material.provider.is_none()
        && !matches!(
            material.unavailable_family.as_deref(),
            Some("gcp_service_account" | "unattributed_oauth")
        ))
        || (material.access_token.is_none() && material.refresh_token.is_none())
    {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) async fn rewrap_capability_snapshot(
    db: &Database,
    keys: &EncryptionKeys,
    effect: &Document,
) -> AppResult<bool> {
    if verify_current_capability_snapshot(keys, effect)
        .await
        .is_ok()
    {
        return Ok(false);
    }
    let id = effect
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid effect identity".into()))?;
    let owner = effect
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid effect owner".into()))?;
    let plaintext = decrypt_capability(keys, effect).await?;
    let mut material: RevocationMaterial = serde_json::from_slice(&plaintext)
        .map_err(|_| AppError::Internal("Malformed preserved capability".into()))?;
    let context = RecordContext::new(EFFECTS, id, owner, &["capability_encrypted"])?;
    if let Some(provider) = material.provider.as_mut() {
        strip_provider_secrets(provider);
    }
    let plaintext = encode_capability(&material)?;
    let encrypted = keys.encrypt_record(&plaintext, &context).await?;
    keys.authenticate_current_record(&encrypted, &context)
        .await?;
    let changed=super::super::transaction_service::durable_database(db).collection::<Document>(EFFECTS).update_one(
        doc! {"_id":id,"user_id":owner,"capability_encrypted":effect.get("capability_encrypted").cloned().ok_or_else(||AppError::Internal("Missing effect ciphertext".into()))?,
            "context_version":effect.get("context_version").cloned().unwrap_or(bson::Bson::Null),
            "lease_token":effect.get("lease_token").cloned().unwrap_or(bson::Bson::Null),
            "$expr":{"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}},
        doc! {"$set":{"capability_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},"context_version":3}}
    ).await?;
    Ok(changed.matched_count == 1)
}

#[cfg(test)]
mod scheduling_tests {
    use super::*;
    use axum::{Router, http::StatusCode, routing::post};
    use futures::TryStreamExt;

    fn mock_provider(id: &str, url: String) -> ProviderConfig {
        bson::from_document(doc! {
            "_id":id,"slug":id,"name":id,"provider_type":"oauth2","is_active":true,
            "created_by":"admin","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            "revocation":{"style":"rfc7009","url":url,"auth":"none","request_encoding":"form",
                "revokes_grant":false,"effect_scope":"token_only"},
        }).unwrap()
    }
    async fn queue_token_effect(
        db: &Database,
        keys: &EncryptionKeys,
        owner: &str,
        provider: ProviderConfig,
        access: &str,
        refresh: Option<&str>,
    ) -> String {
        let material = RevocationMaterial::new(
            provider,
            None,
            Some(access.to_owned().into()),
            refresh.map(|s| s.to_owned().into()),
        );
        let effect = prepare_disconnect_effect(
            keys,
            owner,
            material,
            oauth_revocation::RevocationScope::Token,
            "user_api_keys",
            "removed-source",
            None,
        )
        .await
        .unwrap();
        let id = effect.get_str("_id").unwrap().to_owned();
        db.collection::<Document>(EFFECTS)
            .insert_one(effect)
            .await
            .unwrap();
        id
    }
    async fn deliver_id(db: &Database, keys: &EncryptionKeys, id: &str) {
        db.collection::<Document>(EFFECTS)
            .update_one(doc! {"_id":id}, doc! {"$unset":{"retry_at":""}})
            .await
            .unwrap();
        let effect = claim_effect_matching(db, None, Some(id))
            .await
            .unwrap()
            .unwrap();
        deliver(db, keys, &effect).await.unwrap();
    }

    #[tokio::test]
    async fn simultaneous_first_receipt_claims_keep_one_live_dispatch_lease() {
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let provider = mock_provider("receipt-race", "http://127.0.0.1/revoke".into());
        let mut effects = Vec::new();
        for _ in 0..2 {
            let id =
                queue_token_effect(&db, &keys, &owner, provider.clone(), "same-token", None).await;
            effects.push(
                claim_effect_matching(&db, None, Some(&id))
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        let (first, second) = tokio::join!(
            claim_receipt(&db, &effects[0], "same-token", "same-authority", false),
            claim_receipt(&db, &effects[1], "same-token", "same-authority", false)
        );
        let winner = match (first.unwrap(), second.unwrap()) {
            (ReceiptClaim::Claimed(row), ReceiptClaim::Busy)
            | (ReceiptClaim::Busy, ReceiptClaim::Claimed(row)) => row,
            _ => panic!("equivalent obligations must have exactly one receipt lease"),
        };
        let receipts: Vec<Document> = db
            .collection::<Document>(RECEIPTS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(receipts, vec![winner]);
        assert!(receipts[0].get_datetime("lease_expires_at").unwrap() > &bson::DateTime::now());
        let parent = effects
            .iter()
            .find(|effect| effect.get_str("_id") == receipts[0].get_str("effect_id"))
            .unwrap();
        assert_eq!(
            receipts[0].get_datetime("lease_expires_at"),
            parent.get_datetime("lease_expires_at")
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn malformed_present_capability_versions_retain_valid_ciphertext_without_dispatch() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/revoke", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/revoke",
                    post(move || {
                        let count = count.clone();
                        async move {
                            count.fetch_add(1, Ordering::SeqCst);
                            StatusCode::OK
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let provider = mock_provider("malformed-marker", url);
        for marker in [
            bson::Bson::Boolean(true),
            bson::Bson::String("3".into()),
            bson::Bson::Int64(3),
            bson::Bson::Null,
        ] {
            let id = queue_token_effect(
                &db,
                &keys,
                &owner,
                provider.clone(),
                "retained-marker-token",
                None,
            )
            .await;
            let original = db
                .collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap();
            assert!(decrypt_capability(&keys, &original).await.is_ok());
            let mut historical = original.clone();
            historical.remove("context_version");
            assert!(decrypt_capability(&keys, &historical).await.is_ok());
            db.collection::<Document>(EFFECTS)
                .update_one(
                    doc! {"_id":&id},
                    doc! {"$set":{"context_version":marker.clone()}},
                )
                .await
                .unwrap();
            deliver_id(&db, &keys, &id).await;
            let retained = db
                .collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(retained.get_str("status"), Ok("operator_action"));
            assert_eq!(
                retained.get_str("reason"),
                Ok("preserved_capability_unavailable_or_invalid")
            );
            assert_eq!(retained.get("context_version"), Some(&marker));
            assert_eq!(
                retained.get("capability_encrypted"),
                original.get("capability_encrypted")
            );
        }
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn confirmed_results_are_authority_scoped_and_partial_success_survives_duplicate_effects()
    {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let a = Arc::new(AtomicUsize::new(0));
        let b = Arc::new(AtomicUsize::new(0));
        let access = Arc::new(AtomicUsize::new(0));
        let refresh = Arc::new(AtomicUsize::new(0));
        let accept_refresh = Arc::new(AtomicBool::new(false));
        let ca = a.clone();
        let cb = b.clone();
        let caccess = access.clone();
        let crefresh = refresh.clone();
        let allow = accept_refresh.clone();
        let app = Router::new()
            .route(
                "/a",
                post(move || {
                    ca.fetch_add(1, Ordering::SeqCst);
                    async { StatusCode::OK }
                }),
            )
            .route(
                "/b",
                post(move || {
                    cb.fetch_add(1, Ordering::SeqCst);
                    async { StatusCode::OK }
                }),
            )
            .route(
                "/partial",
                post(move |body: String| {
                    let access = caccess.clone();
                    let refresh = crefresh.clone();
                    let allow = allow.clone();
                    async move {
                        if body.contains("refresh-token") {
                            refresh.fetch_add(1, Ordering::SeqCst);
                            if !allow.load(Ordering::SeqCst) {
                                return StatusCode::SERVICE_UNAVAILABLE;
                            }
                        } else {
                            access.fetch_add(1, Ordering::SeqCst);
                        }
                        StatusCode::OK
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        // B deliberately returns RFC7009's indistinguishable success even
        // though only A actually owns this token. That result must stay at B.
        let pb = mock_provider("provider-b", format!("{base}/b"));
        let pa = mock_provider("provider-a", format!("{base}/a"));
        let eb = queue_token_effect(&db, &keys, &owner, pb, "same-bytes", None).await;
        deliver_id(&db, &keys, &eb).await;
        let ea = queue_token_effect(&db, &keys, &owner, pa.clone(), "same-bytes", None).await;
        deliver_id(&db, &keys, &ea).await;
        let duplicate = queue_token_effect(&db, &keys, &owner, pa, "same-bytes", None).await;
        deliver_id(&db, &keys, &duplicate).await;
        assert_eq!(a.load(Ordering::SeqCst), 1);
        assert_eq!(b.load(Ordering::SeqCst), 1);
        let unrelated = queue_token_effect(
            &db,
            &keys,
            &owner,
            mock_provider("provider-c", format!("{base}/a")),
            "same-bytes",
            None,
        )
        .await;
        deliver_id(&db, &keys, &unrelated).await;

        let held = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&unrelated})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(held.get_str("status"), Ok("complete"));
        assert!(!held.contains_key("capability_encrypted"));
        assert_eq!(a.load(Ordering::SeqCst), 2);
        let pp = mock_provider("partial", format!("{base}/partial"));
        let first = queue_token_effect(
            &db,
            &keys,
            &owner,
            pp.clone(),
            "access-token",
            Some("refresh-token"),
        )
        .await;
        deliver_id(&db, &keys, &first).await;
        let second = queue_token_effect(
            &db,
            &keys,
            &owner,
            pp,
            "access-token",
            Some("refresh-token"),
        )
        .await;
        deliver_id(&db, &keys, &second).await;
        let blocked = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&second})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(blocked.get_str("status"), Ok("pending"));
        assert!(blocked.contains_key("capability_encrypted"));
        accept_refresh.store(true, Ordering::SeqCst);
        db.collection::<Document>(EFFECTS)
            .update_one(doc! {"_id":&first}, doc! {"$unset":{"retry_at":""}})
            .await
            .unwrap();
        deliver_id(&db, &keys, &first).await;
        deliver_id(&db, &keys, &second).await;
        assert_eq!(
            access.load(Ordering::SeqCst),
            1,
            "confirmed access must survive refresh failure/retry"
        );
        assert_eq!(refresh.load(Ordering::SeqCst), 2);
        for id in [first, second] {
            let effect = db
                .collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(effect.get_str("status"), Ok("complete"));
            assert!(!effect.contains_key("capability_encrypted"));
        }
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_duplicate_effect_waits_for_original_confirmation_without_second_request() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let sends = Arc::new(AtomicUsize::new(0));
        let count = sends.clone();
        let entered = Arc::new(tokio::sync::Notify::new());
        let seen = entered.clone();
        let release = Arc::new(tokio::sync::Notify::new());
        let held = release.clone();
        let app = Router::new().route(
            "/held",
            post(move || {
                let count = count.clone();
                let seen = seen.clone();
                let held = held.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    seen.notify_one();
                    held.notified().await;
                    StatusCode::OK
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = mock_provider(
            "held",
            format!("http://{}/held", listener.local_addr().unwrap()),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let first = queue_token_effect(
            &db,
            &keys,
            &owner,
            provider.clone(),
            "duplicate-token",
            None,
        )
        .await;
        let second =
            queue_token_effect(&db, &keys, &owner, provider, "duplicate-token", None).await;
        let task_db = db.clone();
        let task_id = first.clone();
        let delivery = tokio::spawn(async move {
            let task_keys = crate::test_utils::test_encryption_keys();
            deliver_id(&task_db, &task_keys, &task_id).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
            .await
            .unwrap();
        deliver_id(&db, &keys, &second).await;
        let waiting = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&second})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(waiting.get_str("status"), Ok("pending"));
        assert_eq!(attempts(&waiting), 0);
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        let pending = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&second})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.get_str("status"), Ok("pending"));
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(10), delivery)
            .await
            .unwrap()
            .unwrap();

        deliver_id(&db, &keys, &second).await;
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        assert_eq!(
            db.collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&second})
                .await
                .unwrap()
                .unwrap()
                .get_str("status"),
            Ok("complete")
        );
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn unverified_token_revocation_attempts_once_then_requires_scoped_external_evidence() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let sent = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = sent.clone();
        let app = Router::new().route(
            "/revoke",
            post(move || {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { StatusCode::SERVICE_UNAVAILABLE }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/revoke", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for (index,configuration) in [
            doc! {"revocation":{"style":"rfc7009","url":&url,"auth":"none","request_encoding":"form","revokes_grant":true}},
            doc! {"revocation":{"style":"rfc7009","url":&url,"auth":"none","request_encoding":"form","revokes_grant":false}},
            doc! {"revocation":{"style":"rfc7009","url":&url,"auth":"none","request_encoding":"form"}},
            doc! {"revocation_url":&url},
        ].into_iter().enumerate() {
            let mut source = doc! {
                "_id":"grant-provider","slug":"grant-provider","name":"Grant Provider","provider_type":"oauth2",
                "is_active":true,"created_by":"admin","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            };
            source.extend(configuration);
            let provider: ProviderConfig = bson::from_document(source).unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let material = RevocationMaterial::new(
                provider,
                None,
                Some(format!("independent-access-{index}").into()),
                None,
            );
            let (encrypted, version) =
                encrypt_capability(&keys, &id, &user, &encode_capability(&material).unwrap())
                    .await
                    .unwrap();
            db.collection::<Document>(EFFECTS).insert_one(doc! {"_id":&id,"user_id":&user,"status":"pending","created_at":bson::DateTime::now(),"context_version":version,
            "capability_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},"updated_at":bson::DateTime::now()}).await.unwrap();
            drain_one(&db, &keys).await.unwrap();
            let effect = db
                .collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(effect.get_str("status"), Ok("operator_action"));
            assert_eq!(
                effect.get_str("reason"),
                Ok("grant_scope_unconfirmed")
            );
            assert!(effect.contains_key("capability_encrypted"));
            assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), index+1);
            assert!(
                reconcile_effect(&db, &keys, "operator", &uuid::Uuid::new_v4().to_string(), &crate::services::personal_erasure_service::provider::EffectReconciliationRequest { effect_id: id.to_string(), action: ("record_external_revocation_confirmed").to_string(), evidence_reference: ("support-ticket").to_string(), expected_retry_generation: 0, idempotency_key: uuid::Uuid::new_v4().to_string() })
                .await
                .is_err()
            );
            reconcile_effect(
                &db,
                &keys,
                "operator",
                &user,
                &crate::services::personal_erasure_service::provider::EffectReconciliationRequest {
                    effect_id: id.to_string(),
                    action: ("record_external_revocation_confirmed").to_string(),
                    evidence_reference: ("support-ticket").to_string(),
                    expected_retry_generation: 0,
                    idempotency_key: uuid::Uuid::new_v4().to_string(),
                },
            )
            .await
            .unwrap();
            let effect = db
                .collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(effect.get_str("status"), Ok("complete"));
            assert_eq!(
                effect.get_str("completion_basis"),
                Ok("operator_confirmed_external_revocation")
            );
            assert!(!effect.contains_key("capability_encrypted"));
            assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), index+1);
        }
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn failing_owner_backs_off_other_owner_drains_and_corrupt_capability_is_actionable() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/revoke", listener.local_addr().unwrap());
        let sends = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_sends = sends.clone();
        let app = Router::new().route(
            "/revoke",
            post(move |body: String| {
                let sends = observed_sends.clone();
                async move {
                    sends.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if body.contains("failing-token") {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::OK
                    }
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider: ProviderConfig = bson::from_document(doc! {
            "_id":"provider", "slug":"provider", "name":"Provider", "provider_type":"oauth2",
            "is_active":true, "created_by":"admin", "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
            "revocation":{"style":"rfc7009","url":url,"auth":"none","request_encoding":"form","revokes_grant":false,"effect_scope":"token_only"}
        }).unwrap();
        db.collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
            .insert_one(&provider)
            .await
            .unwrap();
        let mut ids = Vec::new();
        for (owner, token) in [
            (&user, "failing-token"),
            (&uuid::Uuid::new_v4().to_string(), "successful-token"),
        ] {
            let id = uuid::Uuid::new_v4().to_string();
            let material = RevocationMaterial::new(
                provider.clone(),
                Some(ResolvedOAuthCredentials {
                    app_source: crate::services::oauth_app_source::OAuthAppSource::Byo,
                    client_id: "captured-original-app".to_owned().into(),
                    client_secret: None,
                    credential_user_id: Some(owner.clone()),
                }),
                Some(token.to_owned().into()),
                None,
            );
            let (encrypted, version) =
                encrypt_capability(&keys, &id, owner, &encode_capability(&material).unwrap())
                    .await
                    .unwrap();
            db.collection::<Document>(EFFECTS).insert_one(doc! {"_id":&id,"user_id":owner,"status":"pending","context_version":version,
                "capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
                "updated_at":bson::DateTime::from_millis(ids.len() as i64)}).await.unwrap();
            ids.push(id);
        }
        drain_one(&db, &keys).await.unwrap();
        let first = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&ids[0]})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.get_str("status").unwrap(), "pending");
        assert_eq!(attempts(&first), 1);
        assert!(
            first.get_datetime("retry_at").unwrap() > first.get_datetime("updated_at").unwrap()
        );
        drain_one(&db, &keys).await.unwrap();
        let second = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&ids[1]})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.get_str("status").unwrap(), "complete");
        assert!(!second.contains_key("capability_encrypted"));
        drain_one(&db, &keys).await.unwrap();
        assert_eq!(
            attempts(
                &db.collection::<Document>(EFFECTS)
                    .find_one(doc! {"_id":&ids[0]})
                    .await
                    .unwrap()
                    .unwrap()
            ),
            1
        );
        let corrupt = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>(EFFECTS).insert_one(doc! {"_id":&corrupt,"user_id":&user,"status":"pending","context_version":3,
            "capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![0,1,2]}}).await.unwrap();
        drain_one(&db, &keys).await.unwrap();
        let broken = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&corrupt})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(broken.get_str("status").unwrap(), "operator_action");
        assert_eq!(
            broken.get_str("reason").unwrap(),
            "preserved_capability_unavailable_or_invalid"
        );
        assert!(broken.contains_key("capability_encrypted"));
        // Repair restores the same authenticated effect; retry neither invents a
        // deletion intent nor discards the preserved obligation/audit identity.
        let material = RevocationMaterial::new(
            provider,
            None,
            Some("repaired-token".to_owned().into()),
            None,
        );
        let (encrypted, version) = encrypt_capability(
            &keys,
            &corrupt,
            &user,
            &encode_capability(&material).unwrap(),
        )
        .await
        .unwrap();
        db.collection::<Document>(EFFECTS).update_one(doc! {"_id":&corrupt,"status":"operator_action"}, doc! {"$set":{
            "context_version":version,"capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}
        }}).await.unwrap();
        reconcile_effect(
            &db,
            &keys,
            &user,
            &user,
            &crate::services::personal_erasure_service::provider::EffectReconciliationRequest {
                effect_id: corrupt.to_string(),
                action: ("retry").to_string(),
                evidence_reference: ("repair-evidence-123").to_string(),
                expected_retry_generation: 0,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        drain_one(&db, &keys).await.unwrap();
        assert_eq!(
            db.collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&corrupt})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "complete"
        );
        assert!(super::super::status(&db, &user).await.unwrap().is_none());
        for expected_attempt in 2..=MAX_DELIVERY_ATTEMPTS {
            db.collection::<Document>(EFFECTS)
                .update_one(
                    doc! {"_id":&ids[0],"status":"pending"},
                    vec![doc! {"$set":{"retry_at":"$$NOW"}}],
                )
                .await
                .unwrap();
            drain_one(&db, &keys).await.unwrap();
            let failed = db
                .collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&ids[0]})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(attempts(&failed), expected_attempt);
            assert!(failed.contains_key("capability_encrypted"));
            assert_eq!(
                failed.get_str("status").unwrap(),
                if expected_attempt == MAX_DELIVERY_ATTEMPTS {
                    "operator_action"
                } else {
                    "pending"
                }
            );
        }
        let exhausted = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&ids[0]})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            exhausted.get_str("reason").unwrap(),
            "provider_revocation_retry_exhausted"
        );
        drain_one(&db, &keys).await.unwrap();
        assert_eq!(
            db.collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&ids[0]})
                .await
                .unwrap()
                .unwrap(),
            exhausted
        );
        for (generation, action) in ["retry", "retry_with_current_provider_config"]
            .into_iter()
            .enumerate()
        {
            let request = EffectReconciliationRequest {
                effect_id: ids[0].clone(),
                action: action.to_owned(),
                evidence_reference: format!("transport-repaired-round-{generation}"),
                expected_retry_generation: generation as i64,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            };
            reconcile_effect(&db, &keys, &user, &user, &request)
                .await
                .unwrap();
            let reset = db
                .collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&ids[0]})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(attempts(&reset), 0);
            assert_eq!(reset.get_str("status"), Ok("pending"));
            assert_eq!(reset.get_i64("retry_generation"), Ok(generation as i64 + 1));
            assert!(reset.contains_key("capability_encrypted"));
            assert!(!reset.contains_key("completed_at"));
            // Advance only durable server eligibility, without sleeping or
            // fabricating delivery outcomes. All eight sends really fail.
            for expected in 1..=MAX_DELIVERY_ATTEMPTS {
                db.collection::<Document>(EFFECTS)
                    .update_one(
                        doc! {"_id":&ids[0],"status":"pending"},
                        vec![doc! {"$set":{"retry_at":"$$NOW"}}],
                    )
                    .await
                    .unwrap();
                drain_one(&db, &keys).await.unwrap();
                let row = db
                    .collection::<Document>(EFFECTS)
                    .find_one(doc! {"_id":&ids[0]})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(attempts(&row), expected);
            }
            let exhausted = db
                .collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&ids[0]})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(exhausted.get_str("status"), Ok("operator_action"));
            assert!(exhausted.contains_key("capability_encrypted"));
            let sent = sends.load(std::sync::atomic::Ordering::SeqCst);
            let audited = db
                .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type":"user.account.erasure_provider_reconciled"})
                .await
                .unwrap();
            // Replaying the accepted HTTP body cannot renew the later round,
            // append audit or wake the parent. A new key with stale generation
            // must also fail, rather than authorize a new retry.
            reconcile_effect(&db, &keys, &user, &user, &request)
                .await
                .unwrap();
            let stale = EffectReconciliationRequest {
                idempotency_key: uuid::Uuid::new_v4().to_string(),
                ..request
            };
            assert!(matches!(
                reconcile_effect(&db, &keys, &user, &user, &stale).await,
                Err(AppError::Conflict(_))
            ));
            drain_one(&db, &keys).await.unwrap();
            assert_eq!(sends.load(std::sync::atomic::Ordering::SeqCst), sent);
            assert_eq!(
                db.collection::<Document>(EFFECTS)
                    .find_one(doc! {"_id":&ids[0]})
                    .await
                    .unwrap()
                    .unwrap(),
                exhausted
            );
            assert_eq!(
                db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                    .count_documents(doc! {"event_type":"user.account.erasure_provider_reconciled"})
                    .await
                    .unwrap(),
                audited
            );
        }
        server.abort();
        db.drop().await.unwrap();
    }
    async fn queue_scoped_effect(
        db: &Database,
        keys: &EncryptionKeys,
        owner: &str,
        provider: ProviderConfig,
        scope: oauth_revocation::RevocationScope,
        token: &str,
    ) -> String {
        let credentials = ResolvedOAuthCredentials {
            app_source: super::super::super::oauth_app_source::OAuthAppSource::Platform,
            client_id: "test-app".to_owned().into(),
            client_secret: None,
            credential_user_id: None,
        };
        let effect = prepare_disconnect_effect(
            keys,
            owner,
            RevocationMaterial::new(
                provider,
                Some(credentials),
                Some(token.to_owned().into()),
                None,
            ),
            scope,
            "user_api_keys",
            "removed-source",
            None,
        )
        .await
        .unwrap();
        let id = effect.get_str("_id").unwrap().to_owned();
        db.collection::<Document>(EFFECTS)
            .insert_one(effect)
            .await
            .unwrap();
        id
    }
    async fn effect_row(db: &Database, id: &str) -> Document {
        db.collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap()
    }
    async fn counting_server() -> (
        String,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sent = count.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/revoke", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/revoke",
                    post(move || {
                        sent.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        async { StatusCode::SERVICE_UNAVAILABLE }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        (url, count, server)
    }
    #[tokio::test]
    async fn grant_and_default_rfc7009_each_get_one_attempt_per_explicit_retry_round() {
        use std::sync::atomic::Ordering;
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let (url, sent, server) = counting_server().await;
        for (index, scope) in [
            oauth_revocation::RevocationScope::Grant,
            oauth_revocation::RevocationScope::Token,
        ]
        .into_iter()
        .enumerate()
        {
            let mut provider = mock_provider(&format!("single-{index}"), url.clone());
            provider.revocation.as_mut().unwrap().effect_scope = None;
            provider.client_id_encrypted = Some(keys.encrypt(b"test-app").await.unwrap());
            db.collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
                .insert_one(&provider)
                .await
                .unwrap();
            let id = queue_scoped_effect(
                &db,
                &keys,
                &owner,
                provider,
                scope,
                &format!("single-token-{index}"),
            )
            .await;
            let before = sent.load(Ordering::SeqCst);
            deliver_id(&db, &keys, &id).await;
            let row = effect_row(&db, &id).await;
            assert_eq!(row.get_str("status"), Ok("operator_action"));
            assert_eq!(row.get_str("reason"), Ok("grant_scope_unconfirmed"));
            assert_eq!(attempts(&row), 1);
            assert_eq!(sent.load(Ordering::SeqCst), before + 1);
            assert!(
                claim_effect_matching(&db, None, Some(&id))
                    .await
                    .unwrap()
                    .is_none()
            );
            for (generation, action) in ["retry", "retry_with_current_provider_config"]
                .into_iter()
                .enumerate()
            {
                // The old source is outside the automatic start window. Only
                // the explicit operator action opens a fresh, bounded window.
                db.collection::<Document>(EFFECTS).update_one(doc! {"_id":&id},doc! {"$set":{"created_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::hours(1))}}).await.unwrap();
                reconcile_effect(
                    &db,
                    &keys,
                    &owner,
                    &owner,
                    &EffectReconciliationRequest {
                        effect_id: id.clone(),
                        action: action.into(),
                        expected_retry_generation: generation as i64,
                        idempotency_key: uuid::Uuid::new_v4().to_string(),
                        evidence_reference: "fixture://operator-reviewed-successor-risk".into(),
                    },
                )
                .await
                .unwrap();
                let armed = effect_row(&db, &id).await;
                assert_eq!(attempts(&armed), 0);
                assert_eq!(armed.get_str("status"), Ok("pending"));
                assert!(
                    armed.get_datetime("retry_armed_at").unwrap()
                        > armed.get_datetime("created_at").unwrap()
                );
                assert_eq!(
                    window_origin(&armed),
                    Some(armed.get_datetime("retry_armed_at").unwrap().to_chrono())
                );
                deliver_id(&db, &keys, &id).await;
                let finished = effect_row(&db, &id).await;
                assert_eq!(finished.get_str("status"), Ok("operator_action"));
                assert_eq!(finished.get_str("reason"), Ok("grant_scope_unconfirmed"));
                assert_eq!(attempts(&finished), 1);
                assert_eq!(sent.load(Ordering::SeqCst), before + generation + 2);
                assert!(
                    claim_effect_matching(&db, None, Some(&id))
                        .await
                        .unwrap()
                        .is_none()
                );
            }
        }
        server.abort();
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn single_attempt_window_successor_and_reclaimed_claim_prevent_provider_calls() {
        use std::sync::atomic::Ordering;
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let (url, sent, server) = counting_server().await;
        for scope in [
            oauth_revocation::RevocationScope::Grant,
            oauth_revocation::RevocationScope::Token,
        ] {
            for reason in [
                "grant_scope_window_elapsed",
                "grant_scope_successor_present",
                "grant_scope_unconfirmed",
            ] {
                let provider_id = uuid::Uuid::new_v4().to_string();
                let mut provider = mock_provider(&provider_id, url.clone());
                provider.revocation.as_mut().unwrap().effect_scope = None;
                db.collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
                    .insert_one(&provider)
                    .await
                    .unwrap();
                let id = queue_scoped_effect(
                    &db,
                    &keys,
                    &owner,
                    provider,
                    scope,
                    &uuid::Uuid::new_v4().to_string(),
                )
                .await;
                if reason == "grant_scope_window_elapsed" {
                    db.collection::<Document>(EFFECTS).update_one(doc! {"_id":&id},doc! {"$set":{"created_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::minutes(6))}}).await.unwrap();
                } else if reason == "grant_scope_successor_present" {
                    let origin = chrono::Utc::now() - chrono::Duration::seconds(1);
                    db.collection::<Document>(EFFECTS)
                        .update_one(
                            doc! {"_id":&id},
                            doc! {"$set":{"created_at":bson::DateTime::from_chrono(origin)}},
                        )
                        .await
                        .unwrap();
                    db.collection::<Document>(crate::models::user_provider_token::COLLECTION_NAME).insert_one(doc! {
                        "_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"provider_config_id":&provider_id,
                        "token_type":"oauth2","status":"active","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
                    }).await.unwrap();
                } else {
                    let claimed = claim_effect_matching(&db, None, Some(&id))
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(attempts(&claimed), 1);
                    db.collection::<Document>(EFFECTS)
                        .update_one(
                            doc! {"_id":&id},
                            doc! {"$set":{"lease_expires_at":bson::DateTime::from_millis(0)}},
                        )
                        .await
                        .unwrap();
                }
                deliver_id(&db, &keys, &id).await;
                let row = effect_row(&db, &id).await;
                assert_eq!(row.get_str("status"), Ok("operator_action"));
                assert_eq!(row.get_str("reason"), Ok(reason));
                assert!(row.contains_key("capability_encrypted"));
                assert_eq!(sent.load(Ordering::SeqCst), 0);
            }
        }
        server.abort();
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn identical_tokens_of_different_owners_do_not_share_receipts() {
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let (url, sent, server) = counting_server().await;
        let provider = mock_provider("owner-dedup", url);
        for owner in [&owner, "another-owner"] {
            let id =
                queue_token_effect(&db, &keys, owner, provider.clone(), "identical-token", None)
                    .await;
            deliver_id(&db, &keys, &id).await;
            let row = effect_row(&db, &id).await;
            assert_eq!(row.get_str("status"), Ok("pending"));
            assert_eq!(row.get_str("user_id"), Ok(owner));
            assert_eq!(row.get_array("token_sha256").unwrap().len(), 1);
            assert!(row.get_str("authority_digest").is_ok());
        }
        assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            db.collection::<Document>(RECEIPTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
        server.abort();
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn completed_erasure_cannot_hide_unresolved_or_secret_holding_effects() {
        let (db, owner) = super::super::tests::fixture().await;
        let job = super::super::accept(&db, &owner, &owner, None)
            .await
            .unwrap();
        for (state, capability) in [
            ("pending", false),
            ("operator_action", false),
            ("complete", true),
        ] {
            let mut completed = job.clone();
            completed.status = ErasureStatus::Complete;
            completed.completed_at = Some(chrono::Utc::now());
            for domain in &mut completed.domains {
                domain.status = DomainStatus::Complete;
            }
            db.collection::<PersonalErasureJob>(JOBS)
                .replace_one(doc! {"_id":&job.id}, &completed)
                .await
                .unwrap();
            let mut effect = doc! {"_id":"unresolved-effect","user_id":&owner,"kind":"oauth_disconnect","status":state};
            if capability {
                effect.insert(
                    "capability_encrypted",
                    bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: vec![1, 2],
                    },
                );
            }
            db.collection::<Document>(EFFECTS)
                .replace_one(doc! {"_id":"unresolved-effect"}, effect)
                .upsert(true)
                .await
                .unwrap();
            let refreshed = refresh(&db, completed).await.unwrap();
            assert_ne!(refreshed.status, ErasureStatus::Complete);
            assert!(refreshed.completed_at.is_none());
            assert_ne!(
                db.collection::<Document>(JOBS)
                    .find_one(doc! {"_id":&job.id})
                    .await
                    .unwrap()
                    .unwrap()
                    .get_str("status"),
                Ok("complete")
            );
        }
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn confirmed_revocation_receipts_do_not_block_final_personal_erasure() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let (db, owner) = super::super::tests::fixture().await;
        let state = crate::test_utils::test_app_state(db.clone());
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = sent.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/revoke", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/revoke",
                    post(move || {
                        counter.fetch_add(1, Ordering::SeqCst);
                        async { StatusCode::OK }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let id = queue_token_effect(
            &db,
            &state.encryption_keys,
            &owner,
            mock_provider("complete", url),
            "complete-token",
            None,
        )
        .await;
        super::super::accept(&db, &owner, &owner, None)
            .await
            .unwrap();
        let stale_dispatch = claim_effect_matching(&db, None, Some(&id))
            .await
            .unwrap()
            .unwrap();
        deliver(&db, &state.encryption_keys, &stale_dispatch)
            .await
            .unwrap();
        let completed = effect_row(&db, &id).await;
        assert_eq!(completed.get_str("status"), Ok("complete"));
        let authority = completed.get_str("authority_digest").unwrap();
        assert_eq!(sent.load(Ordering::SeqCst), 1);
        assert_eq!(
            db.collection::<Document>(RECEIPTS)
                .count_documents(doc! {"user_id":&owner,"confirmed":true})
                .await
                .unwrap(),
            1
        );
        for _ in 0..240 {
            db.collection::<Document>(JOBS)
                .update_many(doc! {}, doc! {"$set":{"retry_at":bson::DateTime::now()}})
                .await
                .unwrap();
            super::super::process_one(&state).await.unwrap();
            if db
                .collection::<Document>(USERS)
                .find_one(doc! {"_id":&owner})
                .await
                .unwrap()
                .is_none()
                && crate::services::telemetry_erasure_service::completion_evidence(&db, &owner)
                    .await
                    .unwrap()
                    .is_none()
            {
                crate::services::telemetry_erasure_service::drain_once(
                    &db,
                    &crate::telemetry::erasure::ErasureClients::default(),
                )
                .await
                .unwrap();
                crate::services::telemetry_erasure_service::operator_reconcile_empty_inventory(
                    &db,
                    &owner,
                    "operator",
                    "fixture://history",
                    "Synthetic deployment never configured telemetry",
                )
                .await
                .unwrap();
            }
            if super::super::status(&db, &owner)
                .await
                .unwrap()
                .unwrap()
                .status
                == ErasureStatus::Complete
            {
                break;
            }
        }
        assert_eq!(
            super::super::status(&db, &owner)
                .await
                .unwrap()
                .unwrap()
                .status,
            ErasureStatus::Complete
        );
        assert!(
            db.collection::<Document>(USERS)
                .find_one(doc! {"_id":&owner})
                .await
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            claim_receipt(&db, &stale_dispatch, "complete-token", authority, false)
                .await
                .unwrap(),
            ReceiptClaim::Busy
        ));
        assert_eq!(
            db.collection::<Document>(RECEIPTS)
                .count_documents(doc! {"user_id":&owner})
                .await
                .unwrap(),
            0
        );
        assert_eq!(sent.load(Ordering::SeqCst), 1);
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn github_not_found_confirms_only_token_scope_including_receipt_reuse() {
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/applications/{{client_id}}/{{scope}}",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().fallback(|| async { StatusCode::NOT_FOUND }),
            )
            .await
            .unwrap();
        });
        for scope in [
            oauth_revocation::RevocationScope::Grant,
            oauth_revocation::RevocationScope::Token,
        ] {
            for _ in 0..2 {
                let mut provider = mock_provider("github-confirmation", url.clone());
                let config = provider.revocation.as_mut().unwrap();
                config.style = "github".into();
                config.revokes_grant = true;
                let material = RevocationMaterial::new(
                    provider,
                    Some(ResolvedOAuthCredentials {
                        app_source: super::super::super::oauth_app_source::OAuthAppSource::Platform,
                        client_id: "app".to_string().into(),
                        client_secret: Some("secret".to_string().into()),
                        credential_user_id: None,
                    }),
                    Some("missing-token".to_string().into()),
                    None,
                );
                let effect = prepare_disconnect_effect(
                    &keys,
                    &owner,
                    material,
                    scope,
                    "user_api_keys",
                    "removed",
                    None,
                )
                .await
                .unwrap();
                let id = effect.get_str("_id").unwrap().to_owned();
                db.collection::<Document>(EFFECTS)
                    .insert_one(effect)
                    .await
                    .unwrap();
                deliver_id(&db, &keys, &id).await;
                let effect = effect_row(&db, &id).await;
                assert_eq!(
                    effect.get_str("status"),
                    Ok(if scope == oauth_revocation::RevocationScope::Grant {
                        "operator_action"
                    } else {
                        "complete"
                    })
                );
                if scope == oauth_revocation::RevocationScope::Grant {
                    assert_eq!(effect.get_str("reason"), Ok("grant_scope_unconfirmed"));
                }
            }
        }
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn every_operator_reason_has_audited_terminal_resolution_without_decryption() {
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        for reason in [
            "grant_scope_unconfirmed",
            "grant_scope_window_elapsed",
            "grant_scope_successor_present",
            "preserved_capability_unavailable_or_invalid",
            "provider_revocation_retry_exhausted",
            "provider_revocation_not_supported_or_credentials_missing",
            "provider_revocation_permanent_failure",
            "oauth_provider_unknown_requires_external_reconciliation",
            "gcp_service_account_access_token_has_no_individual_revocation",
            "skipped_decrypt_failed",
            "skipped_not_configured",
            "skipped_no_token",
            "skipped_no_credentials",
            "skipped_unsupported_auth",
            "skipped_grant_only",
            "skipped_unsupported_style",
            "provider_issuance_not_installed",
            "provider_outcome_uncertain",
        ] {
            for action in [
                "record_external_revocation_confirmed",
                "record_remote_revocation_unavailable",
            ] {
                let id = uuid::Uuid::new_v4().to_string();
                let mut effect = doc! {"_id":&id,"user_id":&owner,"status":"operator_action","reason":reason,
                "capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![0_u8;4]}};
                if matches!(
                    reason,
                    "provider_issuance_not_installed" | "provider_outcome_uncertain"
                ) {
                    effect.insert("credential_family", "gcp_service_account");
                }
                db.collection::<Document>(EFFECTS)
                    .insert_one(effect)
                    .await
                    .unwrap();
                let request = EffectReconciliationRequest {
                    effect_id: id.clone(),
                    action: action.into(),
                    evidence_reference: "evidence://operator-reviewed".into(),
                    expected_retry_generation: 0,
                    idempotency_key: uuid::Uuid::new_v4().to_string(),
                };
                let result = reconcile_effect(&db, &keys, "operator", &owner, &request).await;
                if action == "record_remote_revocation_unavailable"
                    && (reason.starts_with("grant_scope_")
                        || reason == "oauth_provider_unknown_requires_external_reconciliation")
                {
                    assert!(matches!(result, Err(AppError::Conflict(_))));
                    db.collection::<Document>(EFFECTS)
                        .delete_one(doc! {"_id":id})
                        .await
                        .unwrap();
                } else {
                    result.unwrap();
                    let effect = effect_row(&db, &id).await;
                    assert_eq!(
                        effect.get_str("status"),
                        Ok("complete"),
                        "{reason}: {action}"
                    );
                    assert!(!effect.contains_key("capability_encrypted"));
                    reconcile_effect(&db, &keys, "operator", &owner, &request)
                        .await
                        .unwrap();
                    assert_eq!(db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME).count_documents(doc! {"event_type":"user.account.erasure_provider_reconciled","event_data.effect_id":&id}).await.unwrap(),1);
                }
            }
        }
        for status in ["pending", "complete", "operator_action"] {
            let id = uuid::Uuid::new_v4().to_string();
            db.collection::<Document>(EFFECTS)
                .insert_one(doc! {"_id":&id,"user_id":&owner,"status":status,"lease_expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(5)),"lease_token":"held-dispatch"})
                .await
                .unwrap();
            assert!(matches!(
                reconcile_effect(
                    &db,
                    &keys,
                    "operator",
                    &owner,
                    &EffectReconciliationRequest {
                        effect_id: id,
                        action: "record_external_revocation_confirmed".into(),
                        evidence_reference: "ticket".into(),
                        expected_retry_generation: 0,
                        idempotency_key: uuid::Uuid::new_v4().to_string()
                    }
                )
                .await,
                Err(AppError::Conflict(_))
            ));
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn supported_permanent_failure_has_an_unavailable_resolution() {
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/revoke", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/revoke", post(|| async { StatusCode::UNAUTHORIZED })),
            )
            .await
            .unwrap();
        });
        let id = queue_token_effect(
            &db,
            &keys,
            &owner,
            mock_provider("permanent", url),
            "obsolete-app-token",
            None,
        )
        .await;
        deliver_id(&db, &keys, &id).await;
        assert_eq!(
            effect_row(&db, &id).await.get_str("reason"),
            Ok("provider_revocation_permanent_failure")
        );
        reconcile_effect(
            &db,
            &keys,
            "operator",
            &owner,
            &EffectReconciliationRequest {
                effect_id: id.clone(),
                action: "record_remote_revocation_unavailable".into(),
                evidence_reference: "evidence://app-removed".into(),
                expected_retry_generation: 0,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        let effect = effect_row(&db, &id).await;
        assert_eq!(effect.get_str("status"), Ok("complete"));
        assert_eq!(effect.get_bool("remote_revocation_confirmed"), Ok(false));
        assert!(!effect.contains_key("capability_encrypted"));
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn bulk_retry_rearms_only_explicitly_verified_token_effects() {
        let (db, owner) = super::super::tests::fixture().await;
        let mut job = super::super::accept(&db, &owner, &owner, None)
            .await
            .unwrap();
        for checkpoint in &mut job.domains {
            if checkpoint.domain == "provider_credentials" {
                checkpoint.status = DomainStatus::OperatorAction;
            }
        }
        db.collection::<Document>(JOBS).update_one(doc! {"_id":&job.id},doc! {"$set":{"status":"operator_action","domains":bson::to_bson(&job.domains).unwrap()}}).await.unwrap();
        for (id, single) in [
            ("single", Some(true)),
            ("bounded", Some(false)),
            ("legacy", None),
        ] {
            let mut effect = doc! {"_id":id,"user_id":&owner,"kind":"oauth_token_revocation","status":"operator_action","reason":"grant_scope_window_elapsed","attempts":1_i64,
            "capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![0_u8;4]}};
            if let Some(single) = single {
                effect.insert("single_attempt", single);
            }
            db.collection::<Document>(EFFECTS)
                .insert_one(effect)
                .await
                .unwrap();
        }
        let single = effect_row(&db, "single").await;
        let legacy = effect_row(&db, "legacy").await;
        super::super::worker::retry_domain(
            &db,
            "operator",
            &owner,
            &super::super::worker::DomainRetryRequest {
                domain: "provider_credentials".into(),
                job_id: job.id,
                expected_retry_generation: 0,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(effect_row(&db, "single").await, single);
        assert_eq!(effect_row(&db, "legacy").await, legacy);
        let bounded = effect_row(&db, "bounded").await;
        assert_eq!(bounded.get_str("status"), Ok("pending"));
        assert_eq!(attempts(&bounded), 0);
        assert_eq!(retry_generation(&bounded), 1);
        db.drop().await.unwrap();
    }
}
