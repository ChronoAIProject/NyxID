//! Setup facts bind their exact row and real owner. Anonymous native drafts
//! additionally use their server-keyed sender scope; adoption must re-encrypt
//! under the new person in the same exact-source ownership transaction.
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::platform_setup::{COLLECTION_NAME, PlatformSetup, SetupFacts, SetupState},
};
use bson::{Document, doc};
use zeroize::Zeroizing;

const FIELD: &str = "facts_ciphertext";
fn invalid() -> AppError {
    AppError::Internal("Setup facts require authenticated repair".into())
}
fn identity<'a>(id: &str, owner: Option<&'a str>, scope: &str) -> AppResult<&'a str> {
    if uuid::Uuid::parse_str(id).is_err() || scope.is_empty() {
        return Err(invalid());
    }
    match owner {
        Some(owner) if !owner.is_empty() => Ok(owner),
        None if scope.strip_prefix("native:").is_some_and(|hash| {
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
        }) =>
        {
            Ok("anonymous")
        }
        _ => Err(invalid()),
    }
}
pub(crate) fn version(keys: &EncryptionKeys) -> Option<u8> {
    keys.contextual_record_writes().then_some(3)
}
pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    id: &str,
    owner: Option<&str>,
    scope: &str,
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    let owner = identity(id, owner, scope)?;
    // Validation by template remains with the service. This checks the bounded
    // serialization/encoding contract without inventing another input policy.
    if bytes.len() > super::platform_setup_service::MAX_FACT_BYTES {
        return Err(invalid());
    }
    let facts: SetupFacts = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    drop(facts);
    super::record_field_service::encrypt_context(
        keys,
        &RecordContext::new(COLLECTION_NAME, id, owner, &[FIELD, scope])?,
        bytes,
    )
    .await
}
pub(super) async fn bytes(
    keys: &EncryptionKeys,
    row: &PlatformSetup,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let owner = identity(
        &row.id,
        row.owner_user_id.as_deref(),
        &row.idempotency_scope,
    )?;
    if row.facts_ciphertext.is_empty() {
        return Err(invalid());
    }
    let bytes = super::record_field_service::decrypt_context(
        keys,
        &RecordContext::new(
            COLLECTION_NAME,
            &row.id,
            owner,
            &[FIELD, &row.idempotency_scope],
        )?,
        row.facts_context_version,
        &row.facts_ciphertext,
    )
    .await?;
    if bytes.len() > super::platform_setup_service::MAX_FACT_BYTES {
        return Err(invalid());
    }
    let facts = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    super::platform_setup_service::validate_facts(row.template, &facts, false)
        .map_err(|_| invalid())?;
    Ok(bytes)
}
pub(crate) async fn decrypt(keys: &EncryptionKeys, row: &PlatformSetup) -> AppResult<SetupFacts> {
    serde_json::from_slice(&bytes(keys, row).await?).map_err(|_| invalid())
}
fn row(source: &Document) -> AppResult<PlatformSetup> {
    let row: PlatformSetup = bson::from_document(source.clone()).map_err(|_| invalid())?;
    identity(
        &row.id,
        row.owner_user_id.as_deref(),
        &row.idempotency_scope,
    )?;
    if row
        .facts_context_version
        .is_some_and(|version| version != 3)
        || (row.facts_ciphertext.is_empty()
            && (row.facts_context_version.is_some()
                || row.state != SetupState::Cancelled
                || row.published.is_some()
                || source.get_str("owner_retirement") != Ok("deleted")))
    {
        return Err(invalid());
    }
    Ok(row)
}
pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<()> {
    let row = row(source)?;
    if row.facts_ciphertext.is_empty() {
        return Ok(());
    }
    if row.facts_context_version != Some(3) {
        return Err(invalid());
    }
    let owner = identity(
        &row.id,
        row.owner_user_id.as_deref(),
        &row.idempotency_scope,
    )?;
    keys.authenticate_current_record(
        &row.facts_ciphertext,
        &RecordContext::new(
            COLLECTION_NAME,
            &row.id,
            owner,
            &[FIELD, &row.idempotency_scope],
        )?,
    )
    .await?;
    decrypt(keys, &row).await?;
    Ok(())
}
pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    let row = row(source)?;
    if row.facts_ciphertext.is_empty() {
        return Ok(false);
    }
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    if verify_current_snapshot(keys, source).await.is_ok() {
        return Ok(false);
    }
    let bytes = bytes(keys, &row).await?;
    let ciphertext = encrypt(
        keys,
        &row.id,
        row.owner_user_id.as_deref(),
        &row.idempotency_scope,
        &bytes,
    )
    .await?;
    let set = doc! {FIELD:bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext},"facts_context_version":3_i32};
    let mut candidate = source.clone();
    candidate.extend(set.clone());
    verify_current_snapshot(keys, &candidate).await?;
    Ok(super::transaction_service::durable_database(db)
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            doc! {"$set":set},
        )
        .await?
        .matched_count
        == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        platform_inference_service::{GuideAuthority, PublicPurpose},
        platform_setup_service as service,
    };
    use std::{sync::Arc, time::Duration};

    #[tokio::test]
    async fn actual_anonymous_adoption_migration_and_retirement_preserve_bound_facts() {
        let mut f = crate::services::platform_onboarding_test_support::distinct_fixture().await;
        let mut staging = crate::test_utils::test_encryption_keys();
        staging.set_contextual_record_writes(false);
        f.state.encryption_keys = Arc::new(staging);
        let authority = GuideAuthority {
            profile: f.profile.clone(),
            binding: None,
            sender: "700".into(),
            update_id: 9901,
            purpose: PublicPurpose::Guide,
            receipt: None,
        };
        let facts = SetupFacts {
            name: "Adoption cafe".into(),
            timezone: "UTC".into(),
            ..Default::default()
        };
        let draft = service::guide_draft(
            &f.state,
            &authority,
            crate::models::platform_setup::SetupTemplate::RestaurantStarter,
            &facts,
        )
        .await
        .unwrap();
        assert!(draft.facts_context_version.is_none());
        assert!(
            service::decrypt_facts(&f.state.encryption_keys, &draft)
                .await
                .unwrap()
                == facts
        );
        let collection = f.state.db.collection::<Document>(COLLECTION_NAME);
        let source = collection
            .find_one(doc! {"_id":&draft.id})
            .await
            .unwrap()
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        collection
            .update_one(
                doc! {"_id":&draft.id},
                doc! {"$set":{"concurrent_extra":true}},
            )
            .await
            .unwrap();
        assert!(!migrate_snapshot(&f.state.db, &keys, &source).await.unwrap());
        let source = collection
            .find_one(doc! {"_id":&draft.id})
            .await
            .unwrap()
            .unwrap();
        assert!(migrate_snapshot(&f.state.db, &keys, &source).await.unwrap());
        let source = collection
            .find_one(doc! {"_id":&draft.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(source.get_bool("concurrent_extra"), Ok(true));
        assert!(!migrate_snapshot(&f.state.db, &keys, &source).await.unwrap());
        let mut strict = crate::test_utils::test_encryption_keys();
        strict.set_allow_legacy_record_reads(false);
        assert!(service::decrypt_facts(&strict, &draft).await.is_err());
        let current: PlatformSetup = bson::from_document(source.clone()).unwrap();
        assert!(service::decrypt_facts(&strict, &current).await.unwrap() == facts);
        for changed in ["_id", "owner_user_id", "idempotency_scope"] {
            let mut swapped = source.clone();
            swapped.insert(
                changed,
                if changed == "idempotency_scope" {
                    format!("native:{}", "f".repeat(64))
                } else {
                    uuid::Uuid::new_v4().to_string()
                },
            );
            assert!(verify_current_snapshot(&strict, &swapped).await.is_err());
        }
        for version in [
            bson::Bson::Boolean(true),
            bson::Bson::String("3".into()),
            bson::Bson::Int32(2),
        ] {
            let mut malformed = source.clone();
            malformed.insert("facts_context_version", version);
            assert!(verify_current_snapshot(&strict, &malformed).await.is_err());
            assert!(
                migrate_snapshot(&f.state.db, &keys, &malformed)
                    .await
                    .is_err()
            );
        }
        f.state.encryption_keys = Arc::new(strict);
        // Hold the actual ownership path after its authenticated source read.
        // An unrelated added field must invalidate its entire captured source.
        let pause = Arc::new(service::AdoptionPause::default());
        let adoption = service::ADOPTION_PAUSE.scope(
            pause.clone(),
            service::adopt_anonymous(
                &f.state,
                &f.owner,
                &draft.id,
                draft.revision,
                &f.binding.id,
                f.binding.generation,
            ),
        );
        let mutate = async {
            tokio::time::timeout(Duration::from_secs(10), pause.entered.notified())
                .await
                .unwrap();
            collection
                .update_one(doc! {"_id":&draft.id}, doc! {"$set":{"after_capture":true}})
                .await
                .unwrap();
            pause.release.notify_one();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(adoption, mutate)
        })
        .await
        .unwrap();
        assert!(matches!(result, Err(AppError::Conflict(_))));
        let adopted = service::adopt_anonymous(
            &f.state,
            &f.owner,
            &draft.id,
            draft.revision,
            &f.binding.id,
            f.binding.generation,
        )
        .await
        .unwrap();
        assert!(
            service::decrypt_facts(&f.state.encryption_keys, &adopted)
                .await
                .unwrap()
                == facts
        );
        let mut former_context = adopted.clone();
        former_context.owner_user_id = None;
        assert!(
            service::decrypt_facts(&f.state.encryption_keys, &former_context)
                .await
                .is_err()
        );
        let replay = service::adopt_anonymous(
            &f.state,
            &f.owner,
            &draft.id,
            draft.revision,
            &f.binding.id,
            f.binding.generation,
        )
        .await
        .unwrap();
        assert_eq!(replay.revision, adopted.revision);
        f.state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .delete_one(doc! {"_id":&f.owner})
            .await
            .unwrap();
        service::retire_owner(&f.state.db, &f.owner, true)
            .await
            .unwrap();
        let cleared = collection
            .find_one(doc! {"_id":&draft.id})
            .await
            .unwrap()
            .unwrap();
        verify_current_snapshot(&f.state.encryption_keys, &cleared)
            .await
            .unwrap();
        let cleared_row: PlatformSetup = bson::from_document(cleared.clone()).unwrap();
        assert!(
            service::decrypt_facts(&f.state.encryption_keys, &cleared_row)
                .await
                .is_err()
        );
        let mut dangling = cleared;
        dangling.insert("facts_context_version", 3_i32);
        assert!(
            verify_current_snapshot(&f.state.encryption_keys, &dangling)
                .await
                .is_err()
        );
    }
}
