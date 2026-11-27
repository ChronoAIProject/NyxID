//! Run between two batches of the PR 7b saved-login workload on its disposable database.
use super::*;
use crate::services::encrypted_record_census::{Census, Scope as CensusScope};

#[tokio::test]
#[ignore = "requires a PR 7b process and its live pre-enrollment generation"]
async fn enroll_while_the_previous_binary_owns_a_generation() {
    let name = std::env::var("NYXID_MIXED_CENSUS_DATABASE").unwrap();
    assert!(name.ends_with("_census_lo"));
    let client = mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let db = client.database(&name);
    crate::test_utils::init_test_audit_chain_hmac_key();
    let old = db
        .collection::<Document>("encrypted_record_census_log")
        .find_one(doc! {"protocol":1})
        .await
        .unwrap()
        .unwrap();
    assert!(!old.contains_key("scope_revision"));
    let scope: CensusScope = field(&old, "scope").unwrap();
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(true);
    let old_handle = Census::resume(&db, &keys, old.get_str("_id").unwrap(), scope.clone())
        .await
        .unwrap();
    assert!(old_handle.request_cut().await.is_ok());
    let cfg = test_app_config();
    let admin = actor();
    let enrolled = enroll(&db, &cfg, &admin, &enroll_request()).await.unwrap();
    append(&db, &cfg, &admin, &input(&enrolled, Change::VerifyRing))
        .await
        .unwrap();
    assert!(Census::start(&db, &keys, scope.clone()).await.is_err());
    let enrolled_scope = CensusScope {
        restore_epoch: enrolled.scope.restore_epoch,
        deployment_revision: enrolled.scope.deployment_revision,
        ..scope
    };
    let (new_handle, _) = Census::start(&db, &keys, enrolled_scope).await.unwrap();
    assert_ne!(new_handle.generation(), old_handle.generation());
    assert!(new_handle.request_cut().await.is_ok());
}
