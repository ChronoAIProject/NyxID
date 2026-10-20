//! First adoption pins existing domain material; later source/key changes must
//! match. No chain row, canonical bytes or delayed-operation fingerprint moves.
use crate::{
    crypto::hmac_lifecycle::HmacKeyset,
    errors::{AppError, AppResult},
    models::hmac_keyset::{COLLECTION_NAME, HmacKeysetPin},
};
use mongodb::{
    bson::doc,
    options::{CollectionOptions, ReadConcern, ReadPreference, SelectionCriteria, WriteConcern},
};

fn options() -> CollectionOptions {
    CollectionOptions::builder()
        .write_concern(WriteConcern::majority())
        .read_concern(ReadConcern::linearizable())
        .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
        .build()
}
pub(crate) async fn is_prepared(db: &mongodb::Database) -> AppResult<bool> {
    Ok(db
        .collection_with_options::<HmacKeysetPin>(COLLECTION_NAME, options())
        .find_one(doc! {"_id":"legacy-v1"})
        .max_time(std::time::Duration::from_secs(2))
        .await?
        .is_some())
}
pub(crate) async fn ensure_consistent(
    db: &mongodb::Database,
    keys: &HmacKeyset,
    legacy_for_adoption: Option<&HmacKeyset>,
) -> AppResult<()> {
    let collection = db.collection_with_options::<HmacKeysetPin>(COLLECTION_NAME, options());
    let commitments = keys.commitments();
    if collection
        .find_one(doc! {"_id":"legacy-v1"})
        .max_time(std::time::Duration::from_secs(2))
        .await?
        .is_none()
    {
        // Recent MAC evidence checks the two historical signing domains. Other
        // persisted authority/retry fingerprints have no generic verifier:
        // require exact legacy-source preparation for ALL six domains too.
        let legacy = legacy_for_adoption.ok_or_else(|| {
            AppError::Internal(
                "HMAC keyset adoption requires preparation with unchanged legacy sources".into(),
            )
        })?;
        if legacy.commitments() != commitments {
            return Err(AppError::Internal("HMAC adoption differs from legacy domain keys; prepare before rotating source material".into()));
        }
        verify_recent_heads_before_first_pin(db, keys).await?;
    }
    collection
        .update_one(
            doc! {"_id":"legacy-v1"},
            doc! {"$setOnInsert":{
                "version":1, "commitments":&commitments, "created_at":bson::DateTime::now()
            }},
        )
        .upsert(true)
        .await?;
    let pin = collection
        .find_one(doc! {"_id":"legacy-v1"})
        .max_time(std::time::Duration::from_secs(2))
        .await?
        .ok_or_else(|| {
            AppError::Internal("HMAC lifecycle consistency record unavailable".into())
        })?;
    if pin.version != 1 || pin.commitments != commitments {
        return Err(AppError::Internal("HMAC domain keys changed; restore the pinned keyset before starting writers. Historical chains and operation fingerprints must not be re-signed".into()));
    }
    Ok(())
}

/// Bounded adoption evidence, NOT a historical chain audit. The exact unchanged
/// legacy-domain comparison above preserves every historical/retry key byte;
/// verify one recent MAC per chain to reject accidentally changed source keys.
/// Existing Integrity verification continues walking the full retained chains.
async fn verify_recent_heads_before_first_pin(
    db: &mongodb::Database,
    keys: &HmacKeyset,
) -> AppResult<()> {
    use super::{audit_chain_service as audit, billing::ledger};
    use crate::models::{audit_log::AuditLog, billing_ledger::BillingLedgerEntry};
    let invalid = || {
        AppError::Internal(
            "Recent HMAC chain evidence did not verify; no adoption pin was written".into(),
        )
    };
    if let Some(row) = db
        .collection_with_options::<AuditLog>(crate::models::audit_log::COLLECTION_NAME, options())
        .find_one(doc! {"seq":{"$exists":true}})
        .sort(doc! {"seq":-1})
        .max_time(std::time::Duration::from_secs(2))
        .await?
        && (row.seq.is_none_or(|seq| seq < 1)
            || row.entry_hash.as_deref()
                != Some(audit::compute_entry_hash(&row, keys.audit_chain().as_slice())?.as_str()))
    {
        return Err(invalid());
    }
    if let Some(row) = db
        .collection_with_options::<BillingLedgerEntry>(
            crate::models::billing_ledger::COLLECTION_NAME,
            options(),
        )
        .find_one(doc! {"seq":{"$exists":true}})
        .sort(doc! {"seq":-1})
        .max_time(std::time::Duration::from_secs(2))
        .await?
        && (row.seq < 1
            || row.entry_hash != ledger::compute_entry_hash(&row, keys.billing_ledger().as_slice()))
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn independent_replicas_reject_changed_seed_without_mutating_pinned_evidence() {
        let db = crate::test_utils::connect_test_database("hmac_lifecycle_pinning")
            .await
            .expect("isolated MongoDB required");
        let uri = std::env::var("NYXID_TEST_DATABASE_URL").expect("isolated test URI required");
        let other = mongodb::Client::with_uri_str(uri)
            .await
            .unwrap()
            .database(db.name());
        let keys = HmacKeyset(std::array::from_fn(|i| {
            zeroize::Zeroizing::new([i as u8 + 23; 32])
        }));
        ensure_consistent(&db, &keys, Some(&keys)).await.unwrap();
        ensure_consistent(&other, &keys, None).await.unwrap();
        let before = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        for domain in crate::crypto::hmac_lifecycle::DOMAINS {
            let changed = keys.with_changed_domain(domain);
            assert!(matches!(
                ensure_consistent(&other, &changed, None).await,
                Err(AppError::Internal(_))
            ));
        }
        assert_eq!(
            before,
            db.collection::<bson::Document>(COLLECTION_NAME)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
        );
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use crate::{
        crypto::jwt,
        models::user::UserType,
        services::{auth_device_service as device, cli_pairing_service as pairing, internal_auth},
        test_utils,
    };

    #[tokio::test]
    async fn prepared_seeds_preserve_pending_protocols_and_tokens_after_jwt_rotation() {
        let db = test_utils::connect_transaction_test_database("hmac_pending_protocols").await;
        let user = uuid::Uuid::new_v4();
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(test_utils::test_user(&user.to_string(), UserType::Person))
            .await
            .unwrap();
        let mut config = test_utils::test_app_config();
        config.encryption_key = Some("17".repeat(32));
        config.cli_pairing_hmac_key = None;
        config.audit_chain_hmac_key = None;
        config.billing_ledger_hmac_key = None;
        config.internal_dispatch_hmac_key = None;
        let old = crate::legacy_hmac_keyset(&config, b"original-jwt-source");
        let (old_signer, rotated_signer) = jwt::test_rotation_keys();
        let pairing = pairing::create(
            &db,
            old.cli_pairing().as_slice(),
            pairing::CreateParams {
                user_id: &user.to_string(),
                kind: "api-key-create",
                prefill: serde_json::json!({}),
                ttl_secs: None,
            },
        )
        .await
        .unwrap();
        let device = device::initiate(&db, old.auth_device().as_slice(), Default::default())
            .await
            .unwrap();
        let mut pending_rows = Vec::new();
        for collection in [
            crate::models::cli_pairing::COLLECTION_NAME,
            crate::models::auth_device_code::COLLECTION_NAME,
        ] {
            let row = db
                .collection::<bson::Document>(collection)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            pending_rows.push((collection, row));
        }
        let old_dispatch = internal_auth::InternalAuth::new(
            db.clone(),
            old.internal_dispatch().clone(),
            std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(300),
        );
        let headers =
            old_dispatch.signed_headers("POST", "/internal/pending", b"exact pending body");
        let reply = jwt::generate_relay_reply_token(
            &old_signer,
            &config,
            "pending-agent",
            "conversation",
            "message",
            "lark",
        )
        .unwrap();
        let refresh = jwt::generate_refresh_token(&old_signer, &config, &user)
            .unwrap()
            .0;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("independent.json");
        old.export_new(&path).unwrap();
        let prepared = HmacKeyset::load(&path).unwrap();
        ensure_consistent(&db, &prepared, Some(&old)).await.unwrap();
        let pin_before = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        config.encryption_key = Some("28".repeat(32));
        config.jwt_private_key_path = dir
            .path()
            .join("retired-private.pem")
            .to_str()
            .unwrap()
            .into();
        config.hmac_keyset_path = Some(path.to_str().unwrap().into());
        let independent = crate::load_prepared_hmac_keyset(&db, &config).await;
        assert_eq!(independent.commitments(), old.commitments());
        assert_ne!(
            independent.commitments(),
            crate::legacy_hmac_keyset(&config, b"rotated-jwt-source").commitments()
        );
        for (collection, before) in pending_rows {
            assert_eq!(
                before,
                db.collection::<bson::Document>(collection)
                    .find_one(doc! {"_id":before.get("_id").unwrap()})
                    .await
                    .unwrap()
                    .unwrap()
            );
        }
        assert_eq!(
            pairing::claim(
                &db,
                independent.cli_pairing().as_slice(),
                &pairing.code,
                &user.to_string(),
                None
            )
            .await
            .unwrap()
            .id,
            pairing.id
        );
        device::preview(
            &db,
            independent.auth_device().as_slice(),
            &device.user_code,
            None,
            Default::default(),
        )
        .await
        .unwrap();
        // Device polling uses the same persisted HMACs as preview; no record is re-signed.
        assert_eq!(
            device::poll_and_claim(
                &db,
                independent.auth_device().as_slice(),
                &device.device_code
            )
            .await
            .unwrap(),
            device::PollClaim::Pending
        );
        let new_dispatch = internal_auth::InternalAuth::new(
            db.clone(),
            independent.internal_dispatch().clone(),
            std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(300),
        );
        assert!(
            new_dispatch
                .authenticate(&headers, "POST", "/internal/pending", b"exact pending body")
                .await
        );
        assert!(
            !new_dispatch
                .authenticate(&headers, "POST", "/internal/pending", b"exact pending body")
                .await
        );
        jwt::validate_relay_reply_token(&rotated_signer, &config, &reply).unwrap();
        jwt::verify_token(&rotated_signer, &config, &refresh).unwrap();
        assert_eq!(
            pin_before,
            db.collection::<bson::Document>(COLLECTION_NAME)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
        );
        db.drop().await.unwrap();
    }
}
