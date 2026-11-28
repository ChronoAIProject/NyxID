use super::tests_support::{hmac_key, setup_pending_row, sign_poll};
use super::*;
use crate::crypto::token::hash_token;
use crate::models::{device_code, security_capability_activation as activation};
use crate::test_utils::test_encryption_keys;
use mongodb::bson::{Document, doc};
use uuid::Uuid;

#[tokio::test]
async fn activation_hashes_new_generations_and_preserves_pending_legacy_approval() {
    let (db, legacy, signer) = setup_pending_row("iot_code_hash_legacy").await.unwrap();
    db.collection::<activation::SecurityCapabilityActivation>(activation::COLLECTION_NAME)
        .insert_one(activation::SecurityCapabilityActivation {
            id: Uuid::new_v4().to_string(),
            record_kind: activation::RECORD_KIND.into(),
            capability: USER_CODE_READINESS.into(),
            active_since: Utc::now(),
        })
        .await
        .unwrap();
    let key = hmac_key();
    let encryption = test_encryption_keys();
    let hashed = initiate(
        &db,
        &key,
        DeviceCodeInitiateInput {
            device_pubkey: signer.verifying_key().to_bytes(),
            hw_id: "fixture".into(),
            suggested_label: None,
            frontend_url: "https://example.test".into(),
        },
    )
    .await
    .unwrap();
    let rows = db.collection::<Document>(device_code::COLLECTION_NAME);
    let filter = doc! {"device_code_hash": hash_token(&hashed.device_code)};
    let lookup = super::rotation::lookup_hmac(&key, &hashed.user_code);
    assert!(
        rows.find_one(doc! {"user_code_history.0.code": &lookup})
            .await
            .unwrap()
            .is_some()
    );
    let row = rows.find_one(filter.clone()).await.unwrap().unwrap();
    assert!(!row.to_string().contains(&hashed.user_code));
    let actor = Uuid::new_v4().to_string();
    let approve_code = |code| {
        approve(
            &db,
            &key,
            &encryption,
            &actor,
            DeviceCodeApproveInput {
                user_code: code,
                org_id: None,
                label: None,
                default_services: None,
                allow_auto_connected_services: false,
            },
        )
    };
    assert!(matches!(
        approve_code(lookup).await,
        Err(AppError::DeviceUserCodeInvalid)
    ));
    let timestamp = Utc::now().timestamp();
    let signed_poll = |timestamp| {
        poll(
            &db,
            &key,
            &encryption,
            DeviceCodePollInput {
                device_code: hashed.device_code.clone(),
                timestamp,
                signature: sign_poll(&hashed.device_code, timestamp, &signer),
            },
        )
    };
    let DeviceCodePoll::Pending {
        current_user_code, ..
    } = signed_poll(timestamp).await.unwrap()
    else {
        panic!("expected pending")
    };
    assert_eq!(current_user_code, hashed.user_code);
    rows.update_one(filter.clone(), doc!{"$set":{"last_rotated_at": mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(31))}}).await.unwrap();
    let DeviceCodePoll::Pending {
        current_user_code, ..
    } = signed_poll(timestamp + 1).await.unwrap()
    else {
        panic!("expected pending")
    };
    assert_ne!(current_user_code, hashed.user_code);
    let row = rows.find_one(filter).await.unwrap().unwrap().to_string();
    assert!(!row.contains(&current_user_code) && !row.contains(&hashed.user_code));
    assert!(matches!(
        approve_code(hashed.user_code).await,
        Err(AppError::DeviceUserCodeInvalid)
    ));
    approve_code(current_user_code).await.unwrap();
    approve_code(legacy.user_code).await.unwrap();
}
