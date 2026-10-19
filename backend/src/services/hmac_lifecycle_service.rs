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
                != Some(audit::compute_entry_hash(&row, keys.0[2].as_slice())?.as_str()))
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
        && (row.seq < 1 || row.entry_hash != ledger::compute_entry_hash(&row, keys.0[4].as_slice()))
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
        for domain in 0..6 {
            let mut changed =
                HmacKeyset(std::array::from_fn(|i| zeroize::Zeroizing::new(*keys.0[i])));
            changed.0[domain][0] ^= 1;
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
