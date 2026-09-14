use super::*;
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};
use std::sync::Arc;
const KEY: &[u8] = b"public-code-compatibility-test";

async fn database(name: &str) -> Database {
    let db = connect_transaction_test_database(name).await;
    crate::db::ensure_indexes(&db).await.unwrap();
    db
}

#[tokio::test]
async fn concurrent_legacy_and_v2_creation_reserves_before_either_request_insert() {
    let db = database("device_concurrent_public_code").await;
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let generator = || {
        let mut first = true;
        move || {
            if first {
                first = false;
                "ABCDEFGH".into()
            } else {
                generate_user_code()
            }
        }
    };
    let (legacy, v2) = tokio::join!(
        initiate_reserved(
            &db,
            KEY,
            InitiateInput::default(),
            false,
            generator(),
            Some(barrier.clone())
        ),
        initiate_reserved(
            &db,
            KEY,
            InitiateInput::default(),
            true,
            generator(),
            Some(barrier)
        ),
    );
    let (legacy, v2) = (legacy.unwrap(), v2.unwrap());
    assert_ne!(legacy.user_code, v2.user_code);
    assert_eq!(
        db.collection::<bson::Document>(RESERVATION_COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    assert_eq!(collection(&db).count_documents(doc! {}).await.unwrap(), 1);
    assert_eq!(
        collection_for_protocol(&db, true)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn duplicate_codes_in_either_collection_fail_closed_without_approval_or_denial() {
    for same_collection in [false, true] {
        let db = database("device_ambiguous_public_code").await;
        let request = initiate_v2(&db, KEY, InitiateInput::default(), true)
            .await
            .unwrap();
        let code = normalize_user_code(&request.user_code).unwrap();
        let (_, row) = find_by_user_code(&db, &hmac_hex(KEY, code.as_bytes()))
            .await
            .unwrap();
        let mut duplicate = row.clone();
        duplicate.id = Uuid::new_v4().to_string();
        duplicate.device_code_hmac = Uuid::new_v4().to_string();
        duplicate.user_code_reservation_hmac = None;
        duplicate.status = AuthDeviceCodeStatus::Delivered;
        collection_for_protocol(&db, same_collection)
            .insert_one(&duplicate)
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let actor = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor, UserType::Person))
            .await
            .unwrap();
        assert!(matches!(
            find_by_user_code(&db, &row.user_code_hmac).await,
            Err(AppError::AuthDeviceUserCodeInvalid)
        ));
        assert!(matches!(
            approve(
                &db,
                &state.config,
                &state.jwt_keys,
                &state.encryption_keys,
                KEY,
                ApproveInput {
                    user_id: actor.clone(),
                    user_code: code.clone(),
                    approver_ip: None,
                    approver_user_agent: None
                }
            )
            .await,
            Err(AppError::AuthDeviceUserCodeInvalid)
        ));
        assert!(matches!(
            deny(
                &db,
                KEY,
                DenyInput {
                    user_id: actor,
                    user_code: code,
                    denier_ip: None,
                    denier_user_agent: None
                }
            )
            .await,
            Err(AppError::AuthDeviceUserCodeInvalid)
        ));
        let stored = collection_for_protocol(&db, true)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, AuthDeviceCodeStatus::Pending);
        assert!(stored.approved_user_id.is_none());
    }
}

#[tokio::test]
async fn lookup_returns_the_queried_collection_even_with_a_legacy_row_capability_flag() {
    let db = database("device_actual_collection").await;
    let request = initiate(&db, KEY, InitiateInput::default()).await.unwrap();
    collection(&db)
        .update_one(doc! {}, doc! {"$set": {"supports_grant_choice": true}})
        .await
        .unwrap();
    let code = normalize_user_code(&request.user_code).unwrap();
    let (queried, _) = find_by_user_code(&db, &hmac_hex(KEY, code.as_bytes()))
        .await
        .unwrap();
    assert_eq!(queried.name(), AUTH_DEVICE_CODES);
}

#[tokio::test]
async fn failed_request_insertion_keeps_its_reservation_until_ttl() {
    let db = database("device_failed_insert_reservation").await;
    db.run_command(
        doc! {"collMod": V2_COLLECTION_NAME, "validator": {"never_present": {"$exists": true}}},
    )
    .await
    .unwrap();
    assert!(
        initiate_with_user_code_generator_for_protocol(
            &db,
            KEY,
            InitiateInput::default(),
            true,
            || "ABCDEFGH".into()
        )
        .await
        .is_err()
    );
    let reserved = db
        .collection::<bson::Document>(RESERVATION_COLLECTION_NAME)
        .find_one(doc! {"user_code_hmac": hmac_hex(KEY, b"ABCDEFGH")})
        .await
        .unwrap()
        .unwrap();
    assert!(reserved.get_datetime("expires_at").unwrap().to_chrono() > Utc::now());
    assert!(
        initiate_with_user_code_generator_for_protocol(
            &db,
            KEY,
            InitiateInput::default(),
            false,
            || "ABCDEFGH".into()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn compatibility_gate_separates_old_writer_namespace_and_keeps_old_rows_readable() {
    let db = database("device_staged_issuance_gate").await;
    let compatibility = initiate_v2(&db, KEY, InitiateInput::default(), false)
        .await
        .unwrap();
    let code = normalize_user_code(&compatibility.user_code).unwrap();
    assert_eq!(code.len(), 9);
    assert!(code.starts_with('2'));
    let (_, marker_row) = find_by_user_code(&db, &hmac_hex(KEY, code.as_bytes()))
        .await
        .unwrap();
    // An old writer reserves only eight characters in its own pending index.
    // It cannot collide with the compatibility v2 marker namespace.
    let mut old_row = marker_row.clone();
    old_row.id = Uuid::new_v4().to_string();
    old_row.supports_grant_choice = false;
    old_row.device_code_hmac = "old-writer-private".into();
    old_row.user_code_hmac = hmac_hex(KEY, &code.as_bytes()[1..]);
    old_row.user_code_reservation_hmac = None;
    collection(&db).insert_one(&old_row).await.unwrap();
    let (old_reader, actual) = find_by_user_code(&db, &old_row.user_code_hmac)
        .await
        .unwrap();
    assert_eq!(old_reader.name(), AUTH_DEVICE_CODES);
    assert_eq!(actual.id, old_row.id);
    let enabled = initiate_v2(&db, KEY, InitiateInput::default(), true)
        .await
        .unwrap();
    assert_eq!(normalize_user_code(&enabled.user_code).unwrap().len(), 8);
    assert!(collection(&db).find_one(doc! {"user_code_hmac": hmac_hex(KEY, normalize_user_code(&enabled.user_code).unwrap().as_bytes())}).await.unwrap().is_none(), "old readers cannot serve eight-character v2 issuance; fleet drain is required");
    assert_eq!(
        find_by_user_code(&db, &marker_row.user_code_hmac)
            .await
            .unwrap()
            .1
            .id,
        marker_row.id
    );
}

#[tokio::test]
async fn retained_code_lookup_uses_non_partial_index() {
    let db = database("device_lookup_query_plan").await;
    for name in [AUTH_DEVICE_CODES, V2_COLLECTION_NAME] {
        db.collection::<bson::Document>(name)
            .insert_many((0..100).map(|i| {
                doc! {
                    "_id": format!("retained-{i}"), "user_code_hmac": format!("retained-{i}"),
                    "device_code_hmac": format!("private-{i}"), "status": "delivered",
                }
            }))
            .await
            .unwrap();
        let plan = db.run_command(doc! {"explain": {"find": name, "filter": {"user_code_hmac": "example"}, "limit": 2}, "verbosity": "queryPlanner"}).await.unwrap();
        let plan = format!("{plan:?}");
        assert!(plan.contains("IXSCAN"), "{plan}");
        assert!(!plan.contains("COLLSCAN"), "{plan}");
    }
}
