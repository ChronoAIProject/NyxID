//! Preserve exact revocation capability before removing source secrets. Token
//! scope is combined with acceptance dependency guards; RFC7009 token scope
//! alone does not establish that an upstream grant is unshared.
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
const MAX_CAPABILITY: usize = 256 * 1024;

#[derive(Serialize, Deserialize)]
pub(crate) struct RevocationMaterial {
    provider: Option<ProviderConfig>,
    #[serde(default)]
    scope: Option<oauth_revocation::RevocationScope>,
    #[serde(default)]
    unavailable_family: Option<String>,
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
    fn requires_grant_review(&self) -> bool {
        self.provider
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
) -> AppResult<Document> {
    material.scope = Some(scope);
    let id = uuid::Uuid::new_v4().to_string();
    let (ciphertext, version) =
        encrypt_capability(keys, &id, owner, &encode_capability(&material)?).await?;
    Ok(doc! {"_id":id,"user_id":owner,"kind":"oauth_disconnect",
    "source_collection":source_collection,"source_id":source_id,"status":"pending",
    "capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext},
    "context_version":version,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()})
}

async fn revoke_with_receipts(
    db: &Database,
    effect: &Document,
    material: &RevocationMaterial,
) -> AppResult<Option<oauth_revocation::RevocationOutcome>> {
    let id = effect
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid effect identity".into()))?;
    let authority = material.authority()?;
    let tokens: Vec<&str> = [
        material.access_token.as_deref(),
        material.refresh_token.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(String::as_str)
    .collect();
    let closure =
        super::super::token_copy_admission::close_for_dispatch(db, id, &authority, &tokens).await?;
    for owner in &closure.waiting_on {
        let retained = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":owner})
            .await?;
        if retained
            .as_ref()
            .is_none_or(|effect| effect.get_str("status").ok() == Some("operator_action"))
        {
            return Err(AppError::Conflict(
                "Equivalent token revocation requires reconciliation".into(),
            ));
        }
    }
    if !closure.waiting_on.is_empty() {
        return Ok(None);
    }
    let confirmed = |outcome: &Option<TokenOutcome>| {
        matches!(
            outcome,
            Some(TokenOutcome::Delivered | TokenOutcome::NotFound)
        )
    };
    if material.scope == Some(oauth_revocation::RevocationScope::Grant) {
        return Err(AppError::Conflict(
            "Grant revocation requires external reconciliation".into(),
        ));
    }
    let mut result = oauth_revocation::RevocationOutcome {
        style: material
            .provider
            .as_ref()
            .and_then(oauth_revocation::effective_revocation)
            .map(|c| c.style),
        scope: oauth_revocation::RevocationScope::Token,
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
        // Reread after the preceding token receipt; access and refresh may be
        // the same upstream capability. Never resend an already confirmed one.
        let closure = super::super::token_copy_admission::close_for_dispatch(
            db,
            id,
            &authority,
            &[token.as_str()],
        )
        .await?;
        let outcome = if closure.confirmed(token)? {
            Some(TokenOutcome::Delivered)
        } else {
            let response = material
                .revoke(
                    is_access.then(|| token.clone()),
                    (!is_access).then(|| token.clone()),
                )
                .await
                .ok_or_else(|| AppError::Internal("Invalid revocation descriptor".into()))?;
            let outcome = if is_access {
                response.access
            } else {
                response.refresh
            };
            if confirmed(&outcome) {
                super::super::token_copy_admission::confirm_result(db, id, &authority, token)
                    .await?;
            }
            outcome
        };
        if is_access {
            result.access = outcome;
        } else {
            result.refresh = outcome;
        }
    }
    Ok(Some(result))
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

/// One external effect or one source row per pass. Duplicate calls after an
/// uncertain provider result repeat the same token revocation, never a broader
/// grant. Completion requires the provider's supported confirmation semantics.
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
    if let Some(effect) = claim_effect(db, Some(&job.user_id)).await? {
        deliver(db, keys, &effect).await?;
        return Ok(Pass::Pending);
    }
    if db
        .collection::<Document>(EFFECTS)
        .find_one(doc! {"user_id":&job.user_id,
        "status":{"$in":["pending","waiting_token_result"]}})
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
        .find_one(doc! {"user_id":&job.user_id,"status":{"$ne":"complete"}})
        .await?
        .is_some()
    {
        return Ok(Pass::Pending);
    }
    if db
        .collection::<Document>(EFFECTS)
        .find_one(doc! {"user_id":&job.user_id,"status":"complete",
        "completion_basis":"operator_recorded_unsupported_revocation_policy"})
        .await?
        .is_some()
    {
        return Ok(Pass::CompleteWithException(
            "remote_revocation_unavailable_policy_exception",
        ));
    }
    Ok(Pass::Complete)
}

async fn preserve(
    db: &Database,
    keys: &EncryptionKeys,
    job: &PersonalErasureJob,
    collection: &str,
    source: &Document,
) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    let material = super::super::unified_key_service::prepare_erasure_revocation(
        db,
        keys,
        source
            .get_str("user_id")
            .map_err(|_| AppError::Internal("Invalid credential owner".into()))?,
        collection,
        source,
    )
    .await?;
    let source_id = source
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid revocation source".into()))?;
    // Exact source snapshot makes a late successor a distinct effect, while a
    // crash/retry before checkpoint reuses this immutable capability identity.
    let source_bytes = bson::to_vec(source)
        .map_err(|_| AppError::Internal("Invalid revocation source encoding".into()))?;
    let id = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!(
            "{}:{collection}:{source_id}:{}",
            job.id,
            hex::encode(Sha256::digest(&source_bytes))
        )
        .as_bytes(),
    )
    .to_string();
    let encrypted = match material {
        Some(material) => {
            Some(encrypt_capability(keys, &id, &job.user_id, &encode_capability(&material)?).await?)
        }
        None => None,
    };
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<()>=async {
        if db.collection::<Document>(USERS).update_one(doc! {"_id":&job.user_id,"is_active":false,"platform_personal_deleting":true},doc! {"$inc":{"erasure_effect_fence":1_i64}}).session(&mut tx).await?.matched_count!=1 {return Err(AppError::Conflict("Erasure source authority changed".into()));}
        let mut set=doc! {"personal_erasure_preserved":true,"status":"revoked"};
        set.insert("error_message",bson::Bson::Null);
        let changed=db.collection::<Document>(collection).update_one(doc! {"_id":source_id,"user_id":source.get_str("user_id").map_err(|_|AppError::Internal("Invalid credential owner".into()))?,"$expr":{"$eq":["$$ROOT",{"$literal":source}]}},doc! {"$set":set,"$unset":{"access_token_encrypted":"","refresh_token_encrypted":"","credential_encrypted":"","api_key_encrypted":"","user_oauth_client_id_encrypted":"","user_oauth_client_secret_encrypted":"","revocation_provenance":""}}).session(&mut tx).await?;
        if changed.matched_count!=1 {return Err(AppError::Conflict("Revocation source changed".into()));}
        super::super::token_copy_admission::remove_in_session(db, collection, source_id, &mut tx).await?;
        if let Some((encrypted,version))=encrypted {
            db.collection::<Document>(EFFECTS).insert_one(doc! {"_id":&id,"user_id":&job.user_id,"source_collection":collection,"source_id":source_id,"kind":"oauth_token_revocation","status":"pending","capability_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},"context_version":version,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).session(&mut tx).await?;
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
    let Some(effect) = durable
        .collection::<Document>(EFFECTS)
        .find_one(doc! {"_id":id,"kind":"oauth_disconnect","status":"pending"})
        .await?
    else {
        return Ok(false);
    };
    let plaintext = decrypt_capability(keys, &effect).await?;
    let material: RevocationMaterial = serde_json::from_slice(&plaintext)
        .map_err(|_| AppError::Internal("Invalid preserved disconnect capability".into()))?;
    if material.provider.is_none() || effect.get_bool("direct_remote_selected").ok() != Some(true) {
        return Ok(false);
    }
    if material.scope == Some(oauth_revocation::RevocationScope::Grant)
        || material.requires_grant_review()
    {
        if let Some(claimed) = claim_effect_matching(&durable, None, Some(id)).await? {
            finish_effect(
                &durable,
                &claimed,
                false,
                Some("provider_grant_scope_requires_external_reconciliation"),
            )
            .await?;
        }
        return Ok(false);
    }
    if let Some(reason) = material.skip_reason() {
        if let Some(claimed) = claim_effect_matching(&durable, None, Some(id)).await? {
            material.audit(&durable, &claimed, reason, None, None).await;
            finish_effect(&durable, &claimed, false, Some(reason)).await?;
        }
        return Ok(false);
    }
    let tokens: Vec<&str> = [
        material.access_token.as_deref(),
        material.refresh_token.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(String::as_str)
    .collect();
    let authority = material.authority()?;
    match super::super::token_copy_admission::close_for_dispatch(&durable, id, &authority, &tokens)
        .await
    {
        Ok(closure) if closure.waiting_on.is_empty() => {}
        _ => return Ok(false),
    }
    let Ok(permit) = oauth_revocation::try_acquire_revocation_permit() else {
        return Ok(false);
    };
    let Some(claimed) = claim_effect_matching(&durable, None, Some(id)).await? else {
        return Ok(false);
    };
    tokio::spawn(async move {
        let _permit = permit;
        // Failure leaves the original majority-committed intent and ciphertext
        // available after the lease expires; no raw capability lives only here.
        let _ = deliver_material(&durable, &claimed, material).await;
    });
    Ok(true)
}

async fn deliver(db: &Database, keys: &EncryptionKeys, effect: &Document) -> AppResult<()> {
    // Authentication, missing-key and malformed storage failures retain their
    // exact capability and become independently actionable, never a global
    // scheduler error that prevents unrelated jobs from being claimed.
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
    deliver_material(db, effect, material).await
}

async fn deliver_material(
    db: &Database,
    effect: &Document,
    material: RevocationMaterial,
) -> AppResult<()> {
    let required_access = material.access_token.is_some();
    let required_refresh = material.refresh_token.is_some();
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
    // Current-token inequality and locally distinct authorizations do not prove
    // that a grant-wide provider operation cannot affect retained upstream
    // authorizations. Keep the capability and require external reconciliation;
    // never silently widen a token-scoped erasure request into that operation.
    if material.scope == Some(oauth_revocation::RevocationScope::Grant)
        || material.requires_grant_review()
    {
        return finish_effect(
            db,
            effect,
            false,
            Some("provider_grant_scope_requires_external_reconciliation"),
        )
        .await;
    }
    if let Some(reason) = material.skip_reason() {
        material.audit(db, effect, reason, None, None).await;
        return finish_effect(db, effect, false, Some(reason)).await;
    }
    let tokens: Vec<&str> = [
        material.access_token.as_deref(),
        material.refresh_token.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(String::as_str)
    .collect();
    if tokens.is_empty() || tokens.iter().any(|token| token.is_empty()) {
        return finish_effect(
            db,
            effect,
            false,
            Some("preserved_capability_unavailable_or_invalid"),
        )
        .await;
    }
    if super::super::token_copy_admission::results_confirmed(db, &material.authority()?, &tokens)
        .await?
    {
        let mut observed = effect.clone();
        observed.insert("attempts", attempts(effect).saturating_sub(1).max(0));
        return finish_effect(db, &observed, true, None).await;
    }
    if effect.get_str("kind").ok() == Some("oauth_disconnect")
        && effect.get_bool("direct_remote_selected").ok() != Some(true)
    {
        return wait_for_token_result(db, effect).await;
    }
    let outcome = match revoke_with_receipts(db, effect, &material).await {
        Ok(Some(outcome)) => outcome,
        Ok(None) => return wait_for_token_result(db, effect).await,
        Err(_) => {
            return finish_effect(
                db,
                effect,
                false,
                Some("credential_copy_isolation_requires_external_reconciliation"),
            )
            .await;
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
    let confirmed = |outcome: &Option<TokenOutcome>| {
        matches!(
            outcome,
            Some(TokenOutcome::Delivered | TokenOutcome::NotFound)
        )
    };
    let complete = (!required_access || confirmed(&outcome.access))
        && (!required_refresh || confirmed(&outcome.refresh));
    let missing = (required_access && outcome.access.is_none())
        || (required_refresh && outcome.refresh.is_none());
    let action = missing
        || (material.scope == Some(oauth_revocation::RevocationScope::Grant) && !complete)
        || [&outcome.access, &outcome.refresh]
            .into_iter()
            .any(|o| matches!(o, Some(TokenOutcome::Skipped(_))));
    finish_effect(
        db,
        effect,
        complete,
        action.then_some("provider_revocation_not_supported_or_credentials_missing"),
    )
    .await
}

async fn wait_for_token_result(db: &Database, effect: &Document) -> AppResult<()> {
    db.collection::<Document>(EFFECTS).update_one(doc! {
        "_id":effect.get_str("_id").map_err(|_|AppError::Internal("Invalid effect identity".into()))?,
        "status":"pending","lease_token":effect.get_str("lease_token").map_err(|_|AppError::Internal("Invalid effect lease".into()))?,
        "$expr":{"$gt":["$lease_expires_at","$$NOW"]},
    },vec![doc! {"$set":{"status":"waiting_token_result","reason":"equivalent_revocation_in_flight",
        "attempts":attempts(effect).saturating_sub(1).max(0),"lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,
        "retry_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":10}},"updated_at":"$$NOW"}}]).await?;
    Ok(())
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
    let mut filter = doc! {"status":{"$in":["pending","waiting_token_result"]},"$expr":{"$and":[
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
/// bounded reference hash, not credential text. This API cannot assert that an
/// unconfirmed supported revocation succeeded.
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
    let supported = |provider: &ProviderConfig| {
        oauth_revocation::effective_revocation(provider).is_some_and(|config| {
            matches!(config.style.as_str(), "rfc7009" | "github" | "self_bearer")
        })
    };
    let mut set = doc! {"status":"pending","reason":bson::Bson::Null,"lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,"attempts":0_i64,"retry_at":bson::Bson::Null,
    "retry_generation":next_generation,"last_reconciliation_key_hash":&key_hash,
    "last_reconciliation_request_digest":&request_digest};
    if action == "record_external_revocation_confirmed" {
        let grant_review = material.requires_grant_review()
            && effect.get_str("reason").ok()
                == Some("provider_grant_scope_requires_external_reconciliation");
        let unattributed = material.provider.is_none()
            && material.unavailable_family.as_deref() == Some("unattributed_oauth")
            && effect.get_str("reason").ok()
                == Some("oauth_provider_unknown_requires_external_reconciliation");
        let copy_review = matches!(
            effect.get_str("kind").ok(),
            Some("oauth_token_revocation" | "oauth_issuance")
        ) && effect.get_str("reason").ok()
            == Some("credential_copy_isolation_requires_external_reconciliation");
        if !grant_review && !unattributed && !copy_review {
            return Err(AppError::Conflict(
                "Effect does not await external grant reconciliation".into(),
            ));
        }
        set.extend(doc! {"status":"complete","reason":"external_grant_revocation_confirmed",
            "completion_basis":"operator_confirmed_external_grant_revocation","remote_revocation_confirmed":true});
    } else if action == "record_remote_revocation_unavailable" {
        // No transport-failure attestation. A supported immutable/current
        // configuration must be retried or repaired, never declared erased.
        if material.provider.as_ref().is_some_and(supported)
            || current.as_ref().is_some_and(supported)
        {
            return Err(AppError::Conflict(
                "Supported revocation must be confirmed; repair configuration and retry".into(),
            ));
        }
        if material.provider.is_none()
            && material.unavailable_family.as_deref() != Some("gcp_service_account")
        {
            return Err(AppError::Conflict(
                "Unknown revocation family requires repair".into(),
            ));
        }
        set.extend(doc! {"status":"complete","reason":"remote_revocation_unavailable",
            "completion_basis":"operator_recorded_unsupported_revocation_policy","remote_revocation_confirmed":false});
    } else if action == "retry_with_current_provider_config" {
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
        let changed=db.collection::<Document>(EFFECTS).update_one(predicate,doc! {"$set":set.clone()}).session(&mut *tx).await?;
        if changed.matched_count!=1 {return Err(AppError::Conflict("Revocation effect changed; reload evidence".into()));}
        if matches!(action,"record_remote_revocation_unavailable" | "record_external_revocation_confirmed") {
            db.collection::<Document>(EFFECTS).update_one(doc! {"_id":effect_id,"status":"complete"},doc! {"$unset":{"capability_encrypted":""}}).session(&mut *tx).await?;
        }
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

pub(super) const DUPLICATE_SETTLEMENT_INDEX: &str = "provider_effect_duplicate_settlement";

// Share the exact eligibility contract with the partial index. Only unresolved
// duplicates participate; retained terminal evidence cannot grow this walk.
pub(super) fn duplicate_settlement_filter() -> Document {
    doc! {"$or":[{"status":"operator_action","reason":"credential_copy_isolation_requires_external_reconciliation"},
    {"status":"waiting_token_result"}]}
}

async fn settle_confirmed_duplicate(db: &Database, keys: &EncryptionKeys) -> AppResult<()> {
    let Some(effect) = db
        .collection::<Document>(EFFECTS)
        .find_one(duplicate_settlement_filter())
        .sort(doc! {"duplicate_checked_at":1,"_id":1})
        .await?
    else {
        return Ok(());
    };
    let checked: AppResult<bool> = async {
        let plaintext = decrypt_capability(keys, &effect).await?;
        let material: RevocationMaterial = serde_json::from_slice(&plaintext)
            .map_err(|_| AppError::Internal("Invalid preserved capability".into()))?;
        let tokens: Vec<&str> = [
            material.access_token.as_deref(),
            material.refresh_token.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
        super::super::token_copy_admission::results_confirmed(db, &material.authority()?, &tokens)
            .await
    }
    .await;
    let filter = doc! {"_id":effect.get("_id").cloned().unwrap_or(bson::Bson::Null),"status":effect.get("status").cloned().unwrap_or(bson::Bson::Null),
    "capability_encrypted":effect.get("capability_encrypted").cloned().unwrap_or(bson::Bson::Null)};
    if checked.unwrap_or(false) {
        let context = (
            db,
            &filter,
            effect
                .get_str("user_id")
                .map_err(|_| AppError::Internal("Invalid effect owner".into()))?,
        );
        super::super::transaction_service::run_db_only(db,&context,|tx,context| {
            let &(db,filter,owner)=context;
            Box::pin(async move {
                let changed=db.collection::<Document>(EFFECTS).update_one(filter.clone(),doc! {
                    "$set":{"status":"complete","reason":"provider_revocation_confirmed",
                        "completion_basis":"equivalent_authority_token_results","completed_at":bson::DateTime::now()},
                    "$unset":{"capability_encrypted":""},
                }).session(&mut *tx).await?;
                if changed.matched_count==1 { super::issuance::wake_in_session(db,owner,tx).await?; }
                Ok(())
            })
        }).await?;
    } else {
        db.collection::<Document>(EFFECTS)
            .update_one(filter, doc! {"$currentDate":{"duplicate_checked_at":true}})
            .await?;
    }
    Ok(())
}

/// Independently drain authenticated requeues, including rejected reconnects
/// with no personal-erasure intent. Remote effects never require inventing one.
pub(crate) async fn drain_one(db: &Database, keys: &EncryptionKeys) -> AppResult<()> {
    let db = super::super::transaction_service::durable_database(db);
    if let Some(effect) = claim_effect(&db, None).await? {
        deliver(&db, keys, &effect).await?;
    }
    settle_confirmed_duplicate(&db, keys).await?;
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

    fn has_stage(value: &bson::Bson, stage: &str) -> bool {
        match value {
            bson::Bson::Document(doc) => {
                doc.get_str("stage").ok() == Some(stage)
                    || doc.values().any(|value| has_stage(value, stage))
            }
            bson::Bson::Array(values) => values.iter().any(|value| has_stage(value, stage)),
            _ => false,
        }
    }

    #[tokio::test]
    async fn duplicate_settlement_partial_index_bounds_mixed_retained_effects_without_sort() {
        let (db, owner) = super::super::tests::fixture().await;
        let effects = db.collection::<Document>(EFFECTS);
        let mut rows: Vec<Document> = (0..256)
            .map(|i| {
                doc! {"_id":format!("retained-{i:03}"),"user_id":&owner,
                "status":if i%2==0 {"complete"} else {"operator_action"},
                "reason":"provider_outcome_uncertain"}
            })
            .collect();
        rows.extend([
            doc! {"_id":"eligible-old-operator","status":"operator_action",
            "reason":"credential_copy_isolation_requires_external_reconciliation",
            "duplicate_checked_at":bson::DateTime::from_millis(1)},
            doc! {"_id":"eligible-waiting","status":"waiting_token_result",
            "duplicate_checked_at":bson::DateTime::from_millis(2)},
        ]);
        effects.insert_many(rows).await.unwrap();
        let first = effects
            .find_one(duplicate_settlement_filter())
            .sort(doc! {"duplicate_checked_at":1,"_id":1})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.get_str("_id"), Ok("eligible-old-operator"));
        // Explain the actual one-item filter/order using its declared index.
        // Hinting makes the assertion independent of tiny-fixture cost choices;
        // the partial index must satisfy both eligibility and order without a
        // blocking sort or a scan through unrelated retained evidence.
        let explain = db
            .run_command(doc! {"explain":{
            "find":EFFECTS,"filter":duplicate_settlement_filter(),
            "sort":{"duplicate_checked_at":1,"_id":1},"limit":1,
            "hint":DUPLICATE_SETTLEMENT_INDEX},"verbosity":"executionStats"})
            .await
            .unwrap();
        let plan = bson::Bson::Document(explain.clone());
        assert!(has_stage(&plan, "IXSCAN"));
        assert!(!has_stage(&plan, "SORT"));
        assert!(!has_stage(&plan, "COLLSCAN"));
        let stats = explain.get_document("executionStats").unwrap();
        assert_eq!(stats.get_i32("nReturned"), Ok(1));
        assert!(stats.get_i32("totalDocsExamined").unwrap() <= 2);
        db.drop().await.unwrap();
    }

    async fn activate_discovery(db: &Database, keys: &EncryptionKeys) {
        super::super::super::token_copy_admission::begin_discovery(db, true)
            .await
            .unwrap();
        while super::super::super::token_copy_admission::discover_one(db, keys)
            .await
            .unwrap()
        {}
    }
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
        let mut effect = prepare_disconnect_effect(
            keys,
            owner,
            material,
            oauth_revocation::RevocationScope::Token,
            "user_api_keys",
            "removed-source",
        )
        .await
        .unwrap();
        effect.insert("direct_remote_selected", true);
        let id = effect.get_str("_id").unwrap().to_owned();
        db.collection::<Document>(EFFECTS)
            .insert_one(effect)
            .await
            .unwrap();
        id
    }
    async fn deliver_id(db: &Database, keys: &EncryptionKeys, id: &str) {
        let effect = claim_effect_matching(db, None, Some(id))
            .await
            .unwrap()
            .unwrap();
        deliver(db, keys, &effect).await.unwrap();
    }

    #[tokio::test]
    async fn malformed_present_capability_versions_retain_valid_ciphertext_without_dispatch() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        activate_discovery(&db, &keys).await;
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
        activate_discovery(&db, &keys).await;
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
        db.collection::<Document>(EFFECTS)
            .update_one(
                doc! {"_id":&unrelated},
                doc! {"$set":{"direct_remote_selected":false}},
            )
            .await
            .unwrap();
        deliver_id(&db, &keys, &unrelated).await;
        settle_confirmed_duplicate(&db, &keys).await.unwrap();
        let held = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&unrelated})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(held.get_str("status"), Ok("waiting_token_result"));
        assert!(held.contains_key("capability_encrypted"));
        assert_eq!(a.load(Ordering::SeqCst), 1);
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
        assert_eq!(blocked.get_str("status"), Ok("waiting_token_result"));
        assert!(blocked.contains_key("capability_encrypted"));
        accept_refresh.store(true, Ordering::SeqCst);
        db.collection::<Document>(EFFECTS)
            .update_one(doc! {"_id":&first}, doc! {"$unset":{"retry_at":""}})
            .await
            .unwrap();
        deliver_id(&db, &keys, &first).await;
        for _ in 0..3 {
            settle_confirmed_duplicate(&db, &keys).await.unwrap();
        }
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
        activate_discovery(&db, &keys).await;
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
        db.collection::<Document>(EFFECTS)
            .update_one(
                doc! {"_id":&second},
                doc! {"$set":{"direct_remote_selected":false}},
            )
            .await
            .unwrap();
        deliver_id(&db, &keys, &second).await;
        let waiting = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&second})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(waiting.get_str("status"), Ok("waiting_token_result"));
        assert_eq!(attempts(&waiting), 0);
        let task_db = db.clone();
        let task_id = first.clone();
        let delivery = tokio::spawn(async move {
            let task_keys = crate::test_utils::test_encryption_keys();
            deliver_id(&task_db, &task_keys, &task_id).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
            .await
            .unwrap();
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        let pending = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&second})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.get_str("status"), Ok("waiting_token_result"));
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(10), delivery)
            .await
            .unwrap()
            .unwrap();
        settle_confirmed_duplicate(&db, &keys).await.unwrap();
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
    async fn grant_wide_revocation_requires_scoped_external_evidence_without_sending() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let sent = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = sent.clone();
        let app = Router::new().route(
            "/revoke",
            post(move || {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { StatusCode::OK }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/revoke", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for configuration in [
            doc! {"revocation":{"style":"rfc7009","url":&url,"auth":"none","request_encoding":"form","revokes_grant":true}},
            doc! {"revocation":{"style":"rfc7009","url":&url,"auth":"none","request_encoding":"form","revokes_grant":false}},
            doc! {"revocation":{"style":"rfc7009","url":&url,"auth":"none","request_encoding":"form"}},
            doc! {"revocation_url":&url},
        ] {
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
                Some("independent-access".to_owned().into()),
                None,
            );
            let (encrypted, version) =
                encrypt_capability(&keys, &id, &user, &encode_capability(&material).unwrap())
                    .await
                    .unwrap();
            db.collection::<Document>(EFFECTS).insert_one(doc! {"_id":&id,"user_id":&user,"status":"pending","context_version":version,
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
                Ok("provider_grant_scope_requires_external_reconciliation")
            );
            assert!(effect.contains_key("capability_encrypted"));
            assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 0);
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
                Ok("operator_confirmed_external_grant_revocation")
            );
            assert!(!effect.contains_key("capability_encrypted"));
            assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn failing_owner_backs_off_other_owner_drains_and_corrupt_capability_is_actionable() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        super::super::super::token_copy_admission::begin_discovery(&db, true)
            .await
            .unwrap();
        while super::super::super::token_copy_admission::discover_one(&db, &keys)
            .await
            .unwrap()
        {}
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
}
