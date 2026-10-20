//! Durable provider issuance journal. A request may outlive both its HTTP
//! caller and its target account. These rows are independently discoverable;
//! creating one never creates a personal-deletion intent.
use super::*;
use crate::{
    crypto::aes::EncryptionKeys, models::provider_config::ProviderConfig,
    services::user_credentials_service::ResolvedOAuthCredentials,
};
use provider::{EFFECTS, RevocationMaterial};
use zeroize::Zeroizing;

// Only secret-free routine receipts expire. Operator evidence, uncertain
// dispatches and revocation receipts retain their independent lifecycle.
const ROUTINE_RECEIPT_DAYS: i64 = 7;
pub(super) async fn ensure_indexes(db: &Database) -> AppResult<()> {
    let db = super::super::transaction_service::durable_database(db);
    for keys in [
        doc! {"user_id":1,"status":1,"_id":1},
        doc! {"status":1,"retry_at":1,"updated_at":1,"_id":1},
        doc! {"kind":1,"status":1,"updated_at":1,"_id":1},
    ] {
        db.collection::<Document>(EFFECTS)
            .create_index(mongodb::IndexModel::builder().keys(keys).build())
            .await?;
    }
    db.collection::<Document>(EFFECTS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"routine_expires_at":1})
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

/// Persist before sending any request that can issue credentials. Closing
/// personal admission and starting issuance write the same durable owner fence.
pub(crate) async fn begin(
    db: &Database,
    owner: &str,
    target: &str,
    state: &str,
    provider: &str,
) -> AppResult<String> {
    begin_family(db, owner, target, state, Some(provider), "oauth", None).await
}
pub(crate) async fn begin_gcp(
    db: &Database,
    actor: &str,
    target: &str,
    state: &str,
) -> AppResult<(String, String)> {
    let canonical = super::super::erasure_admission::credential_owner(db, target).await?;
    let id = begin_family(
        db,
        &canonical,
        target,
        state,
        None,
        "gcp_service_account",
        Some(actor),
    )
    .await?;
    Ok((id, canonical))
}

async fn begin_family(
    db: &Database,
    owner: &str,
    target: &str,
    state: &str,
    provider: Option<&str>,
    family: &str,
    initiating_actor: Option<&str>,
) -> AppResult<String> {
    let db = super::super::transaction_service::durable_database(db);
    let id = uuid::Uuid::new_v4().to_string();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<String> = async {
        super::super::erasure_admission::fence_credential_target_in_session(
            &db, target, owner, &mut tx,
        )
        .await?;
        if let Some(actor)=initiating_actor {
            fence_initiator(&db,actor,owner,&mut tx).await?;
        }
        db.collection::<Document>(EFFECTS)
            .insert_one(doc! {"_id":&id,"user_id":owner,
            "target_owner_id":target,"source_id":state,"provider_config_id":provider,"credential_family":family,"initiating_actor_id":initiating_actor,
            "kind":"oauth_issuance","status":"in_flight","reason":"provider_outcome_unconfirmed",
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()})
            .session(&mut tx)
            .await?;
        Ok(id.clone())
    }
    .await;
    super::super::org_membership_authority::finish(tx, result).await
}

async fn fence_initiator(
    db: &Database,
    actor: &str,
    owner: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    super::super::erasure_admission::fence_in_session(db, actor, &mut *tx).await?;
    if actor != owner
        && db
            .collection::<Document>(USERS)
            .find_one(doc! {"_id":owner,"user_type":"org"})
            .session(&mut *tx)
            .await?
            .is_some()
    {
        super::super::org_membership_authority::fence_org(db, owner, &mut *tx).await?;
        super::super::org_membership_authority::fence_live_member(db, actor, &mut *tx).await?;
        if db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME).update_one(
            doc! {"org_user_id":owner,"member_user_id":actor,"role":"admin","revoked_at":bson::Bson::Null},
            doc! {"$inc":{"credential_issuance_fence":1_i64}}
        ).session(&mut *tx).await?.matched_count!=1 {
            return Err(AppError::Conflict("Credential initiator no longer administers this organization".into()));
        }
    }
    Ok(())
}

/// Invalidate any active scheduler lease as part of receiving a new obligation.
/// A stale worker therefore cannot publish completion over this transition.
pub(super) async fn wake_in_session(
    db: &Database,
    owner: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(JOBS).update_one(doc! {"user_id":owner}, vec![doc! {"$set":{
        "domains":{"$map":{"input":"$domains","as":"domain","in":{"$cond":[
            {"$eq":["$$domain.domain","provider_credentials"]},
            {"$mergeObjects":["$$domain",{"status":"pending","reason":"provider_issuance_unsettled","completed_at":bson::Bson::Null,"cursor":bson::Bson::Null}]},"$$domain"]}}},
        "status":"pending","completed_at":bson::Bson::Null,"retry_at":"$$NOW","updated_at":"$$NOW",
        "lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null}}]).session(tx).await?;
    Ok(())
}

/// Retain returned material BEFORE any fallible local storage/materialization.
/// Repeating the same attempt is idempotent; a different response cannot replace
/// its capability. No automatic revocation: a rejected reconnect may share a
/// provider grant with a live connection and needs authenticated reconciliation.
#[derive(Clone, Copy)]
pub(crate) struct IssuedTokens<'a> {
    pub access: Option<&'a str>,
    pub refresh: Option<&'a str>,
}
crate::errors::redacted_debug!(IssuedTokens<'_>);

pub(crate) async fn received(
    db: &Database,
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    provider: &ProviderConfig,
    credentials: &ResolvedOAuthCredentials,
    tokens: IssuedTokens<'_>,
) -> AppResult<()> {
    let IssuedTokens { access, refresh } = tokens;
    if access.is_none() && refresh.is_none() {
        // A malformed success is an unknown issuance outcome, not a retained
        // revocation capability and not evidence that nothing was issued.
        return Ok(());
    }
    let material = RevocationMaterial::new(
        provider.clone(),
        Some(ResolvedOAuthCredentials {
            app_source: credentials.app_source,
            client_id: credentials.client_id.clone(),
            client_secret: credentials.client_secret.clone(),
            credential_user_id: credentials.credential_user_id.clone(),
        }),
        access.map(|v| Zeroizing::new(v.to_owned())),
        refresh.map(|v| Zeroizing::new(v.to_owned())),
    );
    received_material(db, keys, id, owner, &material, access, refresh).await
}
pub(crate) async fn received_gcp(
    db: &Database,
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    access: &str,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> AppResult<()> {
    let material =
        RevocationMaterial::gcp_service_account(Zeroizing::new(access.to_owned()), expires_at);
    received_material(db, keys, id, owner, &material, Some(access), None).await
}
async fn received_material(
    db: &Database,
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    material: &RevocationMaterial,
    access: Option<&str>,
    refresh: Option<&str>,
) -> AppResult<()> {
    let encoded = provider::encode_capability(material)?;
    let fingerprint = super::super::audit_service::keyed_fingerprint(
        super::super::audit_service::audit_chain_hmac_key(),
        b"provider-issuance-response-v1",
        &encoded,
    )
    .ok_or_else(|| AppError::Internal("Issuance evidence key is unavailable".into()))?;
    let (capability, version) = provider::encrypt_capability(keys, id, owner, &encoded).await?;
    let db = super::super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<()> = async {
        let changed = db.collection::<Document>(EFFECTS).update_one(doc! {"_id":id,"user_id":owner,"kind":"oauth_issuance","capability_encrypted":{"$exists":false},"reason":{"$ne":"credential_installed"}},
            doc! {"$set":{"status":"received","reason":"provider_install_pending",
                "response_fingerprint":&fingerprint,"installation_tokens":installation_tokens(access, refresh)?,"capability_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:capability},"context_version":version},"$unset":{"routine_expires_at":"","completed_at":""},"$currentDate":{"updated_at":true}}).session(&mut tx).await?;
        if changed.matched_count != 1 {
            if db.collection::<Document>(EFFECTS).find_one(doc! {"_id":id,"user_id":owner,"response_fingerprint":&fingerprint}).session(&mut tx).await?.is_some() {return Ok(());}
            return Err(AppError::Conflict("Provider attempt already holds a different response; reload its evidence".into()));
        }
        wake_in_session(&db,owner,&mut tx).await
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

/// Observe every readable response, including error envelopes. A partial
/// error containing usable returned material remains a revocation obligation;
/// only an explicit protocol rejection without token fields settles dispatch.
pub(crate) async fn observe_body(
    db: &Database,
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    provider: &ProviderConfig,
    credentials: &ResolvedOAuthCredentials,
    body: &str,
) -> AppResult<()> {
    let Ok(value) = serde_json::from_str(body).map(crate::crypto::secret_json::SecretJson) else {
        return Ok(());
    };
    observe_value(db, keys, id, owner, provider, credentials, &value.0).await
}
pub(crate) async fn observe_value(
    db: &Database,
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    provider: &ProviderConfig,
    credentials: &ResolvedOAuthCredentials,
    value: &serde_json::Value,
) -> AppResult<()> {
    let nested = value.get("data");
    let token = |field: &str| {
        value
            .get(field)
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                nested
                    .and_then(|data| data.get(field))
                    .and_then(serde_json::Value::as_str)
            })
    };
    let access = token("access_token");
    let refresh = token("refresh_token");
    if access.is_some() || refresh.is_some() {
        return received(
            db,
            keys,
            id,
            owner,
            provider,
            credentials,
            crate::services::personal_erasure_service::issuance::IssuedTokens { access, refresh },
        )
        .await;
    }
    let malformed_tokens = [Some(value), nested].into_iter().flatten().any(|object| {
        ["access_token", "refresh_token"]
            .iter()
            .any(|field| object.get(field).is_some_and(|field| !field.is_null()))
    });
    let rejected = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|code| !code.is_empty())
        || value
            .get("code")
            .and_then(serde_json::Value::as_i64)
            .is_some_and(|code| code != 0);
    if rejected && !malformed_tokens {
        settled(db, id, false).await?;
    }
    Ok(())
}

/// Only call after confirmed local installation, or a definitive protocol
/// response proving no issuance (e.g. authorization_pending). Unknown HTTP or
/// commit outcomes remain discoverable and block personal completion.
pub(crate) async fn settled(db: &Database, id: &str, installed: bool) -> AppResult<()> {
    if installed {
        let durable = super::super::transaction_service::durable_database(db);
        if durable.collection::<Document>(EFFECTS).find_one(doc! {"_id":id,"kind":"oauth_issuance","status":"complete","reason":"credential_installed"}).await?.is_some() { return Ok(()); }
        return Err(AppError::Conflict(
            "Credential installation has no atomic receipt".into(),
        ));
    }
    let db = super::super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<()> = async {
        let row=db.collection::<Document>(EFFECTS).find_one(doc! {"_id":id,"kind":"oauth_issuance"}).session(&mut tx).await?
            .ok_or_else(||AppError::Conflict("Provider attempt is unavailable".into()))?;
        let owner=row.get_str("user_id").map_err(|_|AppError::Internal("Invalid provider attempt owner".into()))?;
        let reason="protocol_no_issuance";
        if row.get_str("status")==Ok("complete") && row.get_str("reason")==Ok(reason) {return Ok(());}
        let filter=doc! {"_id":id,"kind":"oauth_issuance",
            "capability_encrypted":{"$exists":false},
            "$or":[{"status":"in_flight"},{"status":"operator_action","reason":"provider_outcome_uncertain"}]};
        let changed=db.collection::<Document>(EFFECTS).update_one(filter,vec![
            doc! {"$set":{"status":"complete","reason":reason,"completed_at":"$$NOW","updated_at":"$$NOW",
                "routine_expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"day","amount":ROUTINE_RECEIPT_DAYS}}}},
            doc! {"$unset":["capability_encrypted","context_version"]}]).session(&mut tx).await?;
        if changed.matched_count!=1 {return Err(AppError::Conflict("Provider response no longer matches the unresolved dispatch".into()));}
        wake_in_session(&db,owner,&mut tx).await
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

/// Metadata only. Authorization is supplied by an authenticated admin/operator
/// route; an attempt ID itself conveys no authority.
#[derive(serde::Deserialize)]
pub(crate) struct EffectMetadata {
    #[serde(rename = "_id")]
    pub id: String,
    pub kind: Option<String>,
    pub source_id: Option<String>,
    pub target_owner_id: Option<String>,
    pub provider_config_id: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    #[serde(default)]
    pub retry_generation: i64,
    pub last_reconciliation_request_digest: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}
#[cfg(test)]
pub(crate) async fn unresolved(
    db: &Database,
    owner: &str,
    after: Option<&str>,
) -> AppResult<Vec<EffectMetadata>> {
    effect_page(db, owner, after, true).await
}
pub(crate) async fn outcomes(
    db: &Database,
    owner: &str,
    after: Option<&str>,
) -> AppResult<Vec<EffectMetadata>> {
    effect_page(db, owner, after, false).await
}
async fn effect_page(
    db: &Database,
    owner: &str,
    after: Option<&str>,
    only_unresolved: bool,
) -> AppResult<Vec<EffectMetadata>> {
    use futures::TryStreamExt;
    let mut filter = doc! {"user_id":owner};
    if only_unresolved {
        filter.insert(
            "$or",
            bson::bson!([{"status":{"$ne":"complete"}}, {"capability_encrypted":{"$exists":true}}]),
        );
    }
    if let Some(after) = after {
        filter.insert("_id", doc! {"$gt":after});
    }
    Ok(super::super::transaction_service::durable_database(db).collection::<EffectMetadata>(EFFECTS)
        .find(filter)
        .projection(doc! {"_id":1,"kind":1,"source_id":1,"target_owner_id":1,"provider_config_id":1,"status":1,"reason":1,"retry_generation":1,"last_reconciliation_request_digest":1,"created_at":1,"updated_at":1})
        .sort(doc! {"_id":1}).limit(64).await?.try_collect().await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::IntoFuture;
    pub(super) fn app() -> (ProviderConfig, ResolvedOAuthCredentials) {
        let provider = bson::from_document(doc! {"_id":"provider","slug":"provider","name":"Provider","provider_type":"oauth2","is_active":true,"created_by":"admin","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
        let credentials = ResolvedOAuthCredentials {
            app_source: crate::services::oauth_app_source::OAuthAppSource::Platform,
            client_id: "test-app".to_owned().into(),
            client_secret: Some("distinctive-app-secret".to_owned().into()),
            credential_user_id: None,
        };
        (provider, credentials)
    }
    async fn pending_key(
        db: &Database,
        keys: &EncryptionKeys,
        owner: &str,
        connection: &str,
    ) -> crate::models::user_api_key::UserApiKey {
        use crate::services::user_api_key_service::{CreateApiKeyParams, create_api_key};
        create_api_key(
            db,
            keys,
            owner,
            CreateApiKeyParams {
                label: "Install fixture",
                credential_type: "oauth2",
                credential: "",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: Some("provider"),
                connection_id: Some(connection),
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "pending_auth",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap()
    }
    #[derive(Debug)]
    struct HeldWrap {
        inner: crate::crypto::local_key_provider::LocalKeyProvider,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl crate::crypto::key_provider::KeyProvider for HeldWrap {
        async fn wrap_dek(&self, key: &[u8]) -> AppResult<crate::crypto::key_provider::WrappedKey> {
            self.entered.notify_one();
            self.release.notified().await;
            self.inner.wrap_dek(key).await
        }
        async fn unwrap_dek(
            &self,
            key: &crate::crypto::key_provider::WrappedKey,
        ) -> AppResult<Zeroizing<Vec<u8>>> {
            self.inner.unwrap_dek(key).await
        }
        fn current_key_id(&self) -> u8 {
            self.inner.current_key_id()
        }
        fn has_key_id(&self, id: u8) -> bool {
            self.inner.has_key_id(id)
        }
        fn has_previous_key(&self) -> bool {
            false
        }
    }
    #[tokio::test]
    async fn stale_timeout_scan_cannot_classify_fresh_receipt_or_wake_installed_parent() {
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let connection = uuid::Uuid::new_v4().to_string();
        pending_key(&db, &keys, &owner, &connection).await;
        let id = begin(&db, &owner, &owner, "classifier-install-race", "provider")
            .await
            .unwrap();
        let (app, credentials) = app();
        received(
            &db,
            &keys,
            &id,
            &owner,
            &app,
            &credentials,
            crate::services::personal_erasure_service::issuance::IssuedTokens {
                access: Some("classifier-access"),
                refresh: None,
            },
        )
        .await
        .unwrap();
        db.collection::<Document>(EFFECTS).update_one(doc! {"_id":&id}, vec![doc! {"$set":{"updated_at":{"$dateSubtract":{"startDate":"$$NOW","unit":"second","amount":121}}}}]).await.unwrap();
        let selected = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        // A response refresh retains its status but invalidates the selected generation.
        db.collection::<Document>(EFFECTS)
            .update_one(doc! {"_id":&id}, doc! {"$currentDate":{"updated_at":true}})
            .await
            .unwrap();
        let fresh = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        classify_uncertain_snapshot(&db, &selected).await.unwrap();
        assert_eq!(
            db.collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap(),
            fresh
        );
        installing(
            &id,
            crate::services::user_api_key_service::write_oauth_tokens_to_key(
                &db,
                &keys,
                &connection,
                "classifier-access",
                None,
                None,
                None,
                None,
            ),
        )
        .await
        .unwrap();
        let installed = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(installed.get_str("reason"), Ok("credential_installed"));
        // A checkpoint sentinel makes an erroneous wake observable without
        // closing actual writer admission before the production installation.
        let parent = doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"status":"complete","completed_at":bson::DateTime::now(),"domains":[{"domain":"provider_credentials","status":"complete"}]};
        db.collection::<Document>(JOBS)
            .insert_one(&parent)
            .await
            .unwrap();
        classify_uncertain_snapshot(&db, &selected).await.unwrap();
        assert_eq!(
            db.collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap(),
            installed
        );
        assert_eq!(
            db.collection::<Document>(JOBS)
                .find_one(doc! {"user_id":&owner})
                .await
                .unwrap()
                .unwrap(),
            parent
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn held_production_install_loses_to_reconciled_revocation_without_installing_material() {
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let connection = uuid::Uuid::new_v4().to_string();
        let key = pending_key(&db, &keys, &owner, &connection).await;
        let id = begin(&db, &owner, &owner, "held-install", "provider")
            .await
            .unwrap();
        let (app, credentials) = app();
        received(
            &db,
            &keys,
            &id,
            &owner,
            &app,
            &credentials,
            crate::services::personal_erasure_service::issuance::IssuedTokens {
                access: Some("held-response-access"),
                refresh: None,
            },
        )
        .await
        .unwrap();
        assert!(
            provider::reconcile_effect(
                &db,
                &keys,
                &owner,
                &owner,
                &crate::services::personal_erasure_service::provider::EffectReconciliationRequest {
                    effect_id: id.to_string(),
                    action: ("retry").to_string(),
                    evidence_reference: ("premature").to_string(),
                    expected_retry_generation: 0,
                    idempotency_key: uuid::Uuid::new_v4().to_string()
                }
            )
            .await
            .is_err()
        );
        let held = std::sync::Arc::new(HeldWrap {
            inner: crate::crypto::local_key_provider::LocalKeyProvider::new([117; 32], None),
            entered: Default::default(),
            release: Default::default(),
        });
        let write_keys = EncryptionKeys::with_provider(held.clone());
        let write_db = db.clone();
        let write_id = id.clone();
        let writer = tokio::spawn(async move {
            installing(
                &write_id,
                crate::services::user_api_key_service::write_oauth_tokens_to_key(
                    &write_db,
                    &write_keys,
                    &connection,
                    "held-response-access",
                    None,
                    None,
                    None,
                    None,
                ),
            )
            .await
        });
        held.entered.notified().await;
        db.collection::<Document>(EFFECTS).update_one(doc! {"_id":&id},vec![doc! {"$set":{"updated_at":{"$dateSubtract":{"startDate":"$$NOW","unit":"second","amount":121}}}}]).await.unwrap();
        expose_uncertain(&db).await.unwrap();
        provider::reconcile_effect(
            &db,
            &keys,
            &owner,
            &owner,
            &crate::services::personal_erasure_service::provider::EffectReconciliationRequest {
                effect_id: id.to_string(),
                action: ("retry").to_string(),
                evidence_reference: ("fixture-evidence").to_string(),
                expected_retry_generation: 0,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        provider::drain_one(&db, &keys).await.unwrap();
        held.release.notify_one();
        assert!(writer.await.unwrap().is_err());
        let actual = db
            .collection::<crate::models::user_api_key::UserApiKey>(
                crate::models::user_api_key::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&key.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(actual.status, "pending_auth");
        assert!(actual.access_token_encrypted.is_none());
        assert!(settled(&db, &id, true).await.is_err());
        assert!(super::super::status(&db, &owner).await.unwrap().is_none());
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn held_issuance_blocks_erasure_and_late_response_invalidates_completion_and_lease() {
        let (db, owner) = super::super::tests::fixture().await;
        let id = begin(&db, &owner, &owner, "durable-attempt", "provider")
            .await
            .unwrap();
        let job = super::super::accept(&db, &owner, &owner, None)
            .await
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        assert!(matches!(
            provider::pass(&db, &keys, &job).await.unwrap(),
            provider::Pass::Pending
        ));
        // Simulate a stale scheduler owning the local-cleanup lease. Receiving
        // the held provider response must atomically invalidate this claim.
        db.collection::<Document>(JOBS).update_one(doc! {"_id":&job.id},doc! {"$set":{"status":"complete","completed_at":bson::DateTime::now(),"lease_token":"stale-worker"}}).await.unwrap();
        let (provider, credentials) = app();
        received(
            &db,
            &keys,
            &id,
            &owner,
            &provider,
            &credentials,
            crate::services::personal_erasure_service::issuance::IssuedTokens {
                access: Some("distinctive-held-access"),
                refresh: Some("distinctive-held-refresh"),
            },
        )
        .await
        .unwrap();
        let parent = db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id":&job.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(parent.get_str("status").unwrap(), "pending");
        assert_eq!(parent.get("completed_at"), Some(&bson::Bson::Null));
        assert_eq!(parent.get("lease_token"), Some(&bson::Bson::Null));
        assert_eq!(
            db.collection::<Document>(JOBS)
                .update_one(
                    doc! {"_id":&job.id,"lease_token":"stale-worker"},
                    doc! {"$set":{"status":"complete"}}
                )
                .await
                .unwrap()
                .matched_count,
            0
        );
        assert!(
            begin(&db, &owner, &owner, "late-attempt", "provider")
                .await
                .is_err()
        );
        assert!(
            received(
                &db,
                &keys,
                &id,
                &owner,
                &provider,
                &credentials,
                crate::services::personal_erasure_service::issuance::IssuedTokens {
                    access: Some("replacement"),
                    refresh: None
                }
            )
            .await
            .is_err()
        );
        assert_eq!(unresolved(&db, &owner, None).await.unwrap().len(), 1);
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn rejected_non_erasure_reconnect_remains_independently_discoverable_without_phantom_job()
    {
        let (db, owner) = super::super::tests::fixture().await;
        let id = begin(&db, &owner, &owner, "concurrent-reconnect", "provider")
            .await
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let (provider, credentials) = app();
        received(
            &db,
            &keys,
            &id,
            &owner,
            &provider,
            &credentials,
            crate::services::personal_erasure_service::issuance::IssuedTokens {
                access: Some("uninstalled-access"),
                refresh: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(unresolved(&db, &owner, None).await.unwrap().len(), 1);
        assert_eq!(
            db.collection::<Document>(JOBS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let connection = uuid::Uuid::new_v4().to_string();
        let key = pending_key(&db, &keys, &owner, &connection).await;
        // A finalizer is confirmation only; it cannot attest installation.
        assert!(settled(&db, &id, true).await.is_err());
        installing(&id, crate::services::service_history::collection::<crate::models::user_api_key::UserApiKey>(
            &db, crate::models::user_api_key::COLLECTION_NAME).update_one(doc! {"_id":&key.id},
            doc! {"$set":{"label":"metadata-only"}}).into_future()).await.unwrap();
        assert!(settled(&db, &id, true).await.is_err());
        assert!(
            installing(
                &id,
                crate::services::user_api_key_service::write_oauth_tokens_to_key(
                    &db,
                    &keys,
                    &connection,
                    "different-response",
                    None,
                    None,
                    None,
                    None,
                )
            )
            .await
            .is_err()
        );
        assert!(
            installing(
                &id,
                crate::services::user_api_key_service::write_oauth_tokens_to_key(
                    &db,
                    &keys,
                    "lost-cas-connection",
                    "uninstalled-access",
                    None,
                    None,
                    None,
                    None,
                )
            )
            .await
            .is_err()
        );
        assert!(settled(&db, &id, true).await.is_err());
        installing(
            &id,
            crate::services::user_api_key_service::write_oauth_tokens_to_key(
                &db,
                &keys,
                &connection,
                "uninstalled-access",
                None,
                None,
                None,
                None,
            ),
        )
        .await
        .unwrap();
        // Simulate crash after commit: no finalizer has run, yet the response
        // is no longer independently revocable and the exact destination is recorded.
        let receipt = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.get_str("status"), Ok("complete"));
        assert_eq!(receipt.get_str("installed_record_id"), Ok(key.id.as_str()));
        assert!(
            provider::reconcile_effect(
                &db,
                &keys,
                &owner,
                &owner,
                &crate::services::personal_erasure_service::provider::EffectReconciliationRequest {
                    effect_id: id.to_string(),
                    action: ("retry").to_string(),
                    evidence_reference: ("fixture").to_string(),
                    expected_retry_generation: 0,
                    idempotency_key: uuid::Uuid::new_v4().to_string()
                }
            )
            .await
            .is_err()
        );
        settled(&db, &id, true).await.unwrap();
        assert!(unresolved(&db, &owner, None).await.unwrap().is_empty());
        assert!(
            !db.collection::<Document>(EFFECTS)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap()
                .contains_key("capability_encrypted")
        );
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[tokio::test]
    async fn late_definitive_response_resolves_timeout_classification_but_never_erases_partial_tokens()
     {
        let (db, owner) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let (provider, credentials) = super::tests::app();
        let id = begin(&db, &owner, &owner, "held-response", &provider.id)
            .await
            .unwrap();
        db.collection::<Document>(EFFECTS).update_one(doc! {"_id":&id},vec![doc! {"$set":{"updated_at":{"$dateSubtract":{"startDate":"$$NOW","unit":"second","amount":121}}}}]).await.unwrap();
        expose_uncertain(&db).await.unwrap();
        assert_eq!(
            unresolved(&db, &owner, None).await.unwrap()[0]
                .reason
                .as_deref(),
            Some("provider_outcome_uncertain")
        );
        observe_body(
            &db,
            &keys,
            &id,
            &owner,
            &provider,
            &credentials,
            r#"{"error":"authorization_pending"}"#,
        )
        .await
        .unwrap();
        settled(&db, &id, false).await.unwrap(); // same-attempt definite replay
        let receipt = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.get_str("status"), Ok("complete"));
        assert!(receipt.get_datetime("completed_at").is_ok());
        assert!(receipt.get_datetime("routine_expires_at").is_ok());
        assert!(!receipt.contains_key("capability_encrypted"));
        // A contradictory returned capability must survive even a prior
        // no-issuance receipt; never let its old TTL expire the obligation.
        observe_body(
            &db,
            &keys,
            &id,
            &owner,
            &provider,
            &credentials,
            r#"{"error":"invalid_grant","refresh_token":"distinctive-late-refresh"}"#,
        )
        .await
        .unwrap();
        let retained = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.get_str("status"), Ok("received"));
        assert!(retained.contains_key("capability_encrypted"));
        assert!(!retained.contains_key("routine_expires_at"));
        assert!(settled(&db, &id, false).await.is_err());
        observe_body(
            &db,
            &keys,
            &id,
            &owner,
            &provider,
            &credentials,
            r#"{"error":"invalid_grant","refresh_token":"distinctive-late-refresh"}"#,
        )
        .await
        .unwrap(); // uncertain commit replay
        let after = db
            .collection::<Document>(EFFECTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after, retained);
        assert!(
            observe_body(
                &db,
                &keys,
                &id,
                &owner,
                &provider,
                &credentials,
                r#"{"refresh_token":"different-response"}"#
            )
            .await
            .is_err()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn unresolved_effect_pages_are_owner_scoped_and_routine_cleanup_preserves_exception_evidence()
     {
        let (db, owner) = super::super::tests::fixture().await;
        let rows:Vec<Document>=(0..131).map(|i|doc! {"_id":format!("effect-{i:03}"),"user_id":&owner,
            "kind":"oauth_issuance","status":"operator_action","reason":"provider_outcome_uncertain",
            "capability_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![19,29]},"created_at":bson::DateTime::now()}).collect();
        db.collection::<Document>(EFFECTS)
            .insert_many(rows)
            .await
            .unwrap();
        db.collection::<Document>(EFFECTS)
            .insert_one(
                doc! {"_id":"effect-000-other","user_id":"other-owner","status":"operator_action"},
            )
            .await
            .unwrap();
        let mut ids = Vec::new();
        let mut after = None;
        loop {
            let page = unresolved(&db, &owner, after.as_deref()).await.unwrap();
            if page.is_empty() {
                break;
            }
            after = page.last().map(|row| row.id.clone());
            ids.extend(page.into_iter().map(|row| row.id));
        }
        assert_eq!(ids.len(), 131);
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 131);
        assert!(super::super::status(&db, &owner).await.unwrap().is_none());
        // Cleanup eligibility is explicit: no TTL on unknown outcomes or
        // operator attestations, including rows with no remaining capability.
        let indexes: Vec<_> = {
            use futures::TryStreamExt;
            db.collection::<Document>(EFFECTS)
                .list_indexes()
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap()
        };
        assert!(indexes.iter().any(|index| {
            index.keys == doc! {"routine_expires_at":1}
                && index
                    .options
                    .as_ref()
                    .and_then(|options| options.expire_after)
                    == Some(std::time::Duration::ZERO)
        }));
        assert!(
            db.collection::<Document>(EFFECTS)
                .find_one(doc! {"user_id":&owner,"routine_expires_at":{"$exists":true}})
                .await
                .unwrap()
                .is_none()
        );
        db.drop().await.unwrap();
    }
}

/// A timeout/crash leaves an obligation, not an automatic no-issuance claim.
/// Make old dispatches visible as operator action using server time.
pub(super) async fn expose_uncertain(db: &Database) -> AppResult<()> {
    use futures::TryStreamExt;
    let db = super::super::transaction_service::durable_database(db);
    let ids:Vec<Document>=db.collection::<Document>(EFFECTS).find(doc! {"kind":"oauth_issuance","status":{"$in":["in_flight","received"]},"$expr":uncertain_age_predicate()}).projection(doc! {"_id":1,"user_id":1,"status":1,"updated_at":1}).sort(doc! {"updated_at":1,"_id":1}).limit(32).await?.try_collect().await?;
    for row in ids {
        classify_uncertain_snapshot(&db, &row).await?;
    }
    Ok(())
}

fn uncertain_age_predicate() -> Document {
    doc! {"$lt":["$updated_at",{"$dateSubtract":{"startDate":"$$NOW","unit":"second","amount":120}}]}
}

/// Classification owns only the exact eligible generation selected by the
/// scanner. A completed install or a fresh response makes that scan obsolete.
async fn classify_uncertain_snapshot(db: &Database, row: &Document) -> AppResult<()> {
    let owner = row
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid issuance owner".into()))?;
    let id = row
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid issuance identity".into()))?;
    let status = row
        .get_str("status")
        .map_err(|_| AppError::Internal("Invalid issuance status".into()))?;
    let updated_at = row
        .get_datetime("updated_at")
        .map_err(|_| AppError::Internal("Invalid issuance timestamp".into()))?;
    if !matches!(status, "in_flight" | "received") {
        return Ok(());
    }
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        let changed = db.collection::<Document>(EFFECTS).update_one(doc! {
            "_id":id,"user_id":owner,"kind":"oauth_issuance","status":status,
            "updated_at":updated_at,"$expr":uncertain_age_predicate()
        }, doc! {"$set":{"status":"operator_action","reason":if status=="received" {"provider_issuance_not_installed"} else {"provider_outcome_uncertain"}},"$currentDate":{"updated_at":true}}).session(&mut tx).await?;
        if changed.matched_count == 1 {
            wake_in_session(db, owner, &mut tx).await?;
        }
        Ok(())
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

/// Explicit operator evidence is bound to one still-unresolved dispatch. A late
/// real response invalidates this disposition and reopens its parent atomically.
pub(crate) async fn attest_no_issuance(
    db: &Database,
    actor: &str,
    owner: &str,
    id: &str,
    evidence: &str,
) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    if evidence.trim().is_empty() || evidence.len() > 1024 || evidence.chars().any(char::is_control)
    {
        return Err(AppError::ValidationError(
            "A bounded external evidence reference is required".into(),
        ));
    }
    let db = super::super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<()>=async {
        let changed=db.collection::<Document>(EFFECTS).update_one(doc! {"_id":id,"user_id":owner,"kind":"oauth_issuance","status":"operator_action","capability_encrypted":{"$exists":false}},doc! {"$set":{"status":"complete","reason":"operator_attested_no_issuance","completion_basis":"external_no_issuance_evidence"},"$currentDate":{"updated_at":true}}).session(&mut tx).await?;
        if changed.matched_count!=1 {return Err(AppError::Conflict("Dispatch is not awaiting no-issuance evidence".into()));}
        let key=super::super::audit_service::audit_chain_hmac_key().ok_or_else(||AppError::Internal("Audit chain is unavailable".into()))?;
        super::super::audit_service::log_actor_event_in_session(&db,&mut tx,key,&super::super::audit_service::AuditActor {user_id:actor.to_owned(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},"provider.issuance.no_issuance_attested",serde_json::json!({"owner_id":owner,"attempt_id":id,"evidence_sha256":hex::encode(Sha256::digest(evidence.as_bytes()))})).await?;
        wake_in_session(&db,owner,&mut tx).await
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

tokio::task_local! {
    static INSTALL_ATTEMPT: String;
    static INSTALL_WRITE: InstallWrite;
}
struct InstallWrite {
    attempt: String,
    destination: Document,
    ciphertext: Document,
    tokens: Document,
}
pub(crate) fn installation_tokens(
    access: Option<&str>,
    refresh: Option<&str>,
) -> AppResult<Document> {
    let mut tokens = Document::new();
    for (field, value) in [("access", access), ("refresh", refresh)] {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            let fingerprint = super::super::audit_service::keyed_fingerprint(
                super::super::audit_service::audit_chain_hmac_key(),
                b"provider-install-token-v1",
                value.as_bytes(),
            )
            .ok_or_else(|| AppError::Internal("Issuance evidence key is unavailable".into()))?;
            tokens.insert(field, fingerprint);
        }
    }
    Ok(tokens)
}
/// Scope only the material mutation. Metadata writes in the broader callback
/// cannot consume a response, and a fan-out cannot choose another destination.
pub(crate) async fn key_mutation<F: std::future::Future + Send>(
    destination: Document,
    set: &Document,
    access: &str,
    refresh: Option<&str>,
    write: F,
) -> AppResult<F::Output>
where
    F::Output: Send,
{
    let Ok(attempt) = INSTALL_ATTEMPT.try_with(Clone::clone) else {
        return Ok(write.await);
    };
    let ciphertext = ["access_token_encrypted", "refresh_token_encrypted"]
        .into_iter()
        .filter_map(|field| {
            set.get(field)
                .map(|value| (field.to_owned(), value.clone()))
        })
        .collect();
    let binding = InstallWrite {
        attempt,
        destination,
        ciphertext,
        tokens: installation_tokens(Some(access), refresh)?,
    };
    Ok(INSTALL_WRITE.scope(binding, write).await)
}

pub(crate) async fn installing<F: std::future::Future + Send>(id: &str, write: F) -> F::Output
where
    F::Output: Send,
{
    INSTALL_ATTEMPT.scope(id.to_owned(), write).await
}

/// The installation and removal of its redundant response capability commit
/// together. A requeued/revoking effect cannot race a held credential writer.
pub(crate) struct InstallationTarget<'a> {
    pub owner: &'a str,
    pub provider: &'a str,
    pub collection: &'a str,
    pub record_id: &'a str,
}
crate::errors::redacted_debug!(InstallationTarget<'_>);

pub(crate) async fn installed_in_session(
    db: &Database,
    id: &str,
    destination: InstallationTarget<'_>,
    tokens: Document,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let InstallationTarget {
        owner: target,
        provider,
        collection,
        record_id,
    } = destination;
    let mut filter = doc! {
        "_id":id,"kind":"oauth_issuance","target_owner_id":target,
        "status":"received","installation_tokens":tokens,"capability_encrypted":{"$exists":true},"lease_token":bson::Bson::Null
    };
    if provider == "gcp_service_account" {
        filter.insert("credential_family", "gcp_service_account");
        filter.insert("provider_config_id", bson::Bson::Null);
    } else {
        filter.insert("provider_config_id", provider);
    }
    let receipt = db
        .collection::<Document>(EFFECTS)
        .find_one(filter.clone())
        .session(&mut *tx)
        .await?
        .ok_or_else(|| {
            AppError::Conflict("Provider response disposition changed before installation".into())
        })?;
    let actor = receipt
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid provider response owner".into()))?;
    super::super::erasure_admission::fence_credential_target_in_session(
        db, target, actor, &mut *tx,
    )
    .await?;
    if let Ok(initiator) = receipt.get_str("initiating_actor_id") {
        fence_initiator(db, initiator, actor, &mut *tx).await?;
    }
    let changed = db.collection::<Document>(EFFECTS).update_one(filter,vec![doc! {"$set":{
        "status":"complete","reason":"credential_installed","completed_at":"$$NOW","updated_at":"$$NOW",
        "installed_collection":{"$literal":collection},"installed_record_id":{"$literal":record_id},
        "routine_expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"day","amount":ROUTINE_RECEIPT_DAYS}}
    }},doc! {"$unset":["capability_encrypted","context_version"]}]).session(tx).await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict(
            "Provider response disposition changed before installation".into(),
        ));
    }
    Ok(())
}
pub(crate) async fn install_current_key_in_session(
    db: &Database,
    rows: &[Document],
    tx: &mut ClientSession,
) -> AppResult<()> {
    let Ok((attempt, destination, ciphertext, tokens)) = INSTALL_WRITE.try_with(|binding| {
        (
            binding.attempt.clone(),
            binding.destination.clone(),
            binding.ciphertext.clone(),
            binding.tokens.clone(),
        )
    }) else {
        return Ok(());
    };
    if rows.len() != 1 {
        return Err(AppError::Conflict(
            "Expected exactly one credential installation".into(),
        ));
    }
    let row = &rows[0];
    if destination.is_empty()
        || ciphertext.is_empty()
        || !destination
            .iter()
            .all(|(field, value)| row.get(field) == Some(value))
        || !ciphertext
            .iter()
            .all(|(field, value)| row.get(field) == Some(value))
    {
        return Err(AppError::Conflict(
            "Credential installation did not match the response destination".into(),
        ));
    }
    if db
        .collection::<Document>(EFFECTS)
        .find_one(doc! {
            "_id":&attempt,"kind":"oauth_issuance","installation_tokens":&tokens,
            "status":"received"
        })
        .session(&mut *tx)
        .await?
        .is_none()
    {
        return Err(AppError::Conflict(
            "Credential material does not match the received response".into(),
        ));
    }
    installed_in_session(
        db,
        &attempt,
        crate::services::personal_erasure_service::issuance::InstallationTarget {
            owner: row.get_str("user_id").map_err(|_| {
                AppError::Internal("Credential installation owner is unavailable".into())
            })?,
            provider: if row.get_str("credential_type") == Ok("gcp_service_account") {
                "gcp_service_account"
            } else {
                row.get_str("provider_config_id").map_err(|_| {
                    AppError::Internal("Credential installation provider is unavailable".into())
                })?
            },
            collection: crate::models::user_api_key::COLLECTION_NAME,
            record_id: row.get_str("_id").map_err(|_| {
                AppError::Internal("Credential installation identity is unavailable".into())
            })?,
        },
        tokens,
        tx,
    )
    .await
}
