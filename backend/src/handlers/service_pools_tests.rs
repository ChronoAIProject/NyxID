use axum::{
    Json,
    extract::{Path, State},
};
use mongodb::bson::doc;
use serde_json::json;
use uuid::Uuid;

use super::service_pools_handler as handler;
use crate::errors::AppError;
use crate::models::service_pool::{COLLECTION_NAME, ServicePool};
use crate::services::service_pool_service::{self, UpdatePoolInput};
use crate::test_utils::{
    connect_transaction_test_database, test_app_state, test_auth_user, test_user_service,
};

#[tokio::test]
async fn service_pool_http_slug_identifiers_resolve_within_the_pool_owner() {
    let db = connect_transaction_test_database("pool_http_identifiers").await;
    let owner = Uuid::new_v4().to_string();
    let stranger = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    let foreign_member = Uuid::new_v4().to_string();
    for (id, service_owner) in [(&member, &owner), (&foreign_member, &stranger)] {
        db.collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(test_user_service(
                id,
                service_owner,
                "shared-member-slug",
                &Uuid::new_v4().to_string(),
                None,
                None,
            ))
            .await
            .unwrap();
    }
    let state = test_app_state(db.clone());
    let mut created = Vec::new();
    for pool_owner in [&owner, &stranger] {
        let (_, Json(pool)) = handler::create_pool(
            State(state.clone()),
            test_auth_user(pool_owner),
            Json(
                serde_json::from_value(json!({
                    "slug": "shared-pool-slug", "name": "Identifier review", "strategy": "weighted",
                    "members": [{ "user_service_id": "shared-member-slug", "weight": 7 }]
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        created.push(pool);
    }
    assert_eq!(created[0].members[0].user_service_id, member);
    assert_eq!(created[1].members[0].user_service_id, foreign_member);
    let Json(shown) = handler::get_pool(
        State(state.clone()),
        test_auth_user(&owner),
        Path("shared-pool-slug".into()),
    )
    .await
    .unwrap();
    assert_eq!(shown.id, created[0].id);
    let Json(updated) = handler::add_member(
        State(state.clone()),
        test_auth_user(&owner),
        Path("shared-pool-slug".into()),
        Json(
            serde_json::from_value(
                json!({ "user_service_id": "shared-member-slug", "enabled": false }),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        updated.members.len(),
        1,
        "a slug update must resolve the existing member rather than append an alias"
    );
    assert_eq!(updated.members[0].user_service_id, member);
    assert_eq!(
        updated.members[0].weight, 7,
        "omitted settings survive slug resolution"
    );
    assert!(!updated.members[0].enabled);
    let foreign = handler::add_member(
        State(state.clone()),
        test_auth_user(&owner),
        Path(created[0].id.clone()),
        Json(serde_json::from_value(json!({ "user_service_id": foreign_member })).unwrap()),
    )
    .await;
    assert!(matches!(
        foreign,
        Err(AppError::ServicePoolMemberInvalid(_))
            | Err(AppError::ValidationError(_))
            | Err(AppError::NotFound(_))
    ));
    let Json(untouched) = handler::get_pool(
        State(state),
        test_auth_user(&stranger),
        Path(created[1].id.clone()),
    )
    .await
    .unwrap();
    assert_eq!(untouched.members[0].weight, 7);
    assert!(untouched.members[0].enabled);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_http_update_omission_preserves_and_null_clears_saved_settings() {
    let db = connect_transaction_test_database("pool_http_nullable").await;
    let owner = Uuid::new_v4().to_string();
    let state = test_app_state(db.clone());
    let (_, Json(created)) = handler::create_pool(
        State(state.clone()),
        test_auth_user(&owner),
        Json(
            serde_json::from_value(json!({
                "slug": "draft-failover",
                "name": "Draft failover",
                "description": "Keep this description",
                "strategy": "priority",
                "failover": { "max_attempts": 2 },
                "members": []
            }))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let Json(_) = handler::update_pool(
        State(state.clone()),
        test_auth_user(&owner),
        Path(created.id.clone()),
        Json(serde_json::from_value(json!({ "name": "Renamed" })).unwrap()),
    )
    .await
    .unwrap();
    let saved = service_pool_service::get_pool(&db, &owner, &created.id)
        .await
        .unwrap();
    assert_eq!(saved.description.as_deref(), Some("Keep this description"));
    assert_eq!(saved.failover.as_ref().unwrap().max_attempts, 2);
    let Json(_) = handler::update_pool(
        State(state),
        test_auth_user(&owner),
        Path(created.id.clone()),
        Json(serde_json::from_value(json!({ "description": null, "failover": null })).unwrap()),
    )
    .await
    .unwrap();
    let saved = service_pool_service::get_pool(&db, &owner, &created.id)
        .await
        .unwrap();
    assert_eq!(saved.description, None);
    assert_eq!(saved.failover, None);
    let stored = db
        .collection::<mongodb::bson::Document>(COLLECTION_NAME)
        .find_one(doc! { "_id": &created.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.get("description"), Some(&mongodb::bson::Bson::Null));
    assert_eq!(stored.get("failover"), Some(&mongodb::bson::Bson::Null));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_http_member_updates_preserve_omitted_settings() {
    let db = connect_transaction_test_database("pool_http_member_patch").await;
    let owner = Uuid::new_v4().to_string();
    let member_id = Uuid::new_v4().to_string();
    let service = test_user_service(
        &member_id,
        &owner,
        "member",
        &Uuid::new_v4().to_string(),
        None,
        None,
    );
    db.collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(&service)
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let (_, Json(created)) = handler::create_pool(
        State(state.clone()),
        test_auth_user(&owner),
        Json(
            serde_json::from_value(json!({
                "slug": "member-patch", "name": "Member patch", "strategy": "weighted",
                "members": [{ "user_service_id": member_id, "weight": 7, "enabled": false }]
            }))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let Json(after_add) = handler::add_member(
        State(state.clone()),
        test_auth_user(&owner),
        Path(created.id.clone()),
        Json(serde_json::from_value(json!({ "user_service_id": member_id })).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(after_add.members[0].weight, 7);
    assert!(!after_add.members[0].enabled);
    let Json(after_set) = handler::set_members(
        State(state.clone()),
        test_auth_user(&owner),
        Path(created.id.clone()),
        Json(
            serde_json::from_value(
                json!({ "members": [{ "user_service_id": member_id, "enabled": true }] }),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(after_set.members[0].weight, 7);
    assert!(after_set.members[0].enabled);
    let Json(_) = handler::update_pool(
        State(state),
        test_auth_user(&owner),
        Path(created.id.clone()),
        Json(
            serde_json::from_value(
                json!({ "members": [{ "user_service_id": member_id, "weight": 4 }] }),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let saved = service_pool_service::get_pool(&db, &owner, &created.id)
        .await
        .unwrap();
    assert_eq!(saved.members[0].weight, 4);
    assert!(saved.members[0].enabled);
    db.drop().await.unwrap();
}

#[test]
fn service_pool_http_member_model_distinguishes_null_from_omission() {
    let omitted: handler::PoolMemberRequest =
        serde_json::from_value(json!({ "user_service_id": "member" })).unwrap();
    let cleared: handler::PoolMemberRequest =
        serde_json::from_value(json!({ "user_service_id": "member", "model": null })).unwrap();
    let supplied: handler::PoolMemberRequest =
        serde_json::from_value(json!({ "user_service_id": "member", "model": "target-model" }))
            .unwrap();
    assert_eq!(omitted.model, None);
    assert_eq!(cleared.model, Some(None));
    assert_eq!(supplied.model, Some(Some("target-model".into())));
}

#[tokio::test]
async fn service_pool_configuration_revision_rejects_a_stale_mutation() {
    let db = connect_transaction_test_database("pool_config_revision").await;
    let owner = Uuid::new_v4().to_string();
    let state = test_app_state(db.clone());
    let (_, Json(created)) = handler::create_pool(
        State(state),
        test_auth_user(&owner),
        Json(serde_json::from_value(json!({ "slug": "revision", "name": "Original" })).unwrap()),
    )
    .await
    .unwrap();
    let initial = service_pool_service::get_pool(&db, &owner, &created.id)
        .await
        .unwrap();
    let updated = service_pool_service::update_pool(
        &db,
        &owner,
        &created.id,
        UpdatePoolInput {
            name: Some("Concurrent edit".into()),
            expected_revision: Some(initial.config_revision),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let stale = service_pool_service::update_pool(
        &db,
        &owner,
        &created.id,
        UpdatePoolInput {
            name: Some("Stale overwrite".into()),
            expected_revision: Some(initial.config_revision),
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(stale, Err(AppError::Conflict(_))));
    let stored: ServicePool = db
        .collection(COLLECTION_NAME)
        .find_one(doc! { "_id": &created.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.name, "Concurrent edit");
    assert_eq!(stored.config_revision, updated.config_revision);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_same_api_contract_is_enforced_by_writes_and_live_planning() {
    use crate::models::downstream_service::{
        DownstreamService, InferenceWireProtocol, ServiceInference,
    };
    use crate::models::user::UserType;
    use crate::test_utils::{test_auto_connected_catalog_service, test_user, test_user_endpoint};

    let db = connect_transaction_test_database("pool_contract_authority").await;
    let owner = Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let mut ids = Vec::new();
    let mut catalogs = Vec::new();
    for index in 0..3 {
        let mut catalog = test_auto_connected_catalog_service();
        catalog.slug = format!("contract-api-{index}");
        catalog.inference = Some(ServiceInference {
            wire_protocol: if index == 2 {
                InferenceWireProtocol::AnthropicMessages
            } else {
                InferenceWireProtocol::OpenaiCompletions
            },
            model_list: false,
            realtime: false,
        });
        db.collection::<DownstreamService>("downstream_services")
            .insert_one(&catalog)
            .await
            .unwrap();
        let member = Uuid::new_v4().to_string();
        let endpoint = Uuid::new_v4().to_string();
        let slug = format!("contract-member-{index}");
        db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
            .insert_one(test_user_endpoint(
                &endpoint,
                &owner,
                &slug,
                "https://contract-review.invalid/v1",
                None,
                Some(&catalog.id),
            ))
            .await
            .unwrap();
        db.collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(test_user_service(
                &member,
                &owner,
                &slug,
                &endpoint,
                Some(&catalog.id),
                None,
            ))
            .await
            .unwrap();
        ids.push(member);
        catalogs.push(catalog.id);
    }
    let state = test_app_state(db.clone());
    let input = json!({
        "slug":"contract-pool", "name":"Contract review", "strategy":"priority",
        "members":[{"user_service_id":ids[0]}, {"user_service_id":ids[1]}]
    });
    let rejected = handler::create_pool(
        State(state.clone()),
        test_auth_user(&owner),
        Json(serde_json::from_value(input.clone()).unwrap()),
    )
    .await;
    assert!(matches!(
        rejected,
        Err(AppError::ServicePoolMemberInvalid(_))
    ));
    assert_eq!(
        db.collection::<ServicePool>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );

    let mut declared = input;
    for member in declared["members"].as_array_mut().unwrap() {
        member["same_api_compatible"] = json!(true);
    }
    let (_, Json(created)) = handler::create_pool(
        State(state.clone()),
        test_auth_user(&owner),
        Json(serde_json::from_value(declared).unwrap()),
    )
    .await
    .unwrap();
    let rejected = handler::add_member(
        State(state.clone()),
        test_auth_user(&owner),
        Path(created.id.clone()),
        Json(
            serde_json::from_value(json!({
                "user_service_id":ids[2], "same_api_compatible":true
            }))
            .unwrap(),
        ),
    )
    .await;
    assert!(
        matches!(rejected, Err(AppError::ServicePoolMemberInvalid(_))),
        "a declaration must not override a known protocol mismatch"
    );

    let rejected = handler::update_pool(
        State(state.clone()),
        test_auth_user(&owner),
        Path(created.id.clone()),
        Json(
            serde_json::from_value(json!({
                "name":"Must not save", "members":[
                    {"user_service_id":ids[0], "same_api_compatible":null},
                    {"user_service_id":ids[1]}
                ]
            }))
            .unwrap(),
        ),
    )
    .await;
    assert!(matches!(
        rejected,
        Err(AppError::ServicePoolMemberInvalid(_))
    ));
    let saved = service_pool_service::get_pool(&db, &owner, &created.id)
        .await
        .unwrap();
    assert_eq!(saved.name, "Contract review");
    assert_eq!(saved.members.len(), 2);
    assert_eq!(saved.config_revision, created.config_revision);
    assert!(
        saved
            .members
            .iter()
            .all(|member| member.same_api_compatible)
    );

    let plan = service_pool_service::plan_candidates(
        &db,
        &state.encryption_keys,
        &owner,
        None,
        &owner,
        "contract-pool",
        &http::Method::POST,
        0,
    )
    .await
    .unwrap();
    assert_eq!(plan.candidates.len(), 2);
    db.collection::<mongodb::bson::Document>("downstream_services")
        .update_one(
            doc! {"_id":&catalogs[1]},
            doc! {"$set":{"inference.wire_protocol":"anthropic_messages"}},
        )
        .await
        .unwrap();
    let drift = service_pool_service::plan_candidates(
        &db,
        &state.encryption_keys,
        &owner,
        None,
        &owner,
        "contract-pool",
        &http::Method::POST,
        0,
    )
    .await;
    assert!(
        matches!(drift, Err(AppError::ServicePoolMemberInvalid(_))),
        "live catalog drift must be caught before dispatch"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_weight_limit_applies_to_create_and_atomic_mutations() {
    let db = connect_transaction_test_database("pool_weight_bounds").await;
    let owner = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    db.collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(test_user_service(
            &member,
            &owner,
            "weight-member",
            &Uuid::new_v4().to_string(),
            None,
            None,
        ))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let input = json!({"slug":"weight-pool","name":"Weight","strategy":"weighted","members":[{"user_service_id":member,"weight":1001}]});
    assert!(
        handler::create_pool(
            State(state.clone()),
            test_auth_user(&owner),
            Json(serde_json::from_value(input.clone()).unwrap())
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.collection::<ServicePool>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let mut valid = input;
    valid["members"][0]["weight"] = 0.into();
    let (_, Json(pool)) = handler::create_pool(
        State(state.clone()),
        test_auth_user(&owner),
        Json(serde_json::from_value(valid).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(pool.members[0].weight, 1);
    let before = db
        .collection::<mongodb::bson::Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":&pool.id})
        .await
        .unwrap()
        .unwrap();
    let update = json!({"name":"Must not persist","expected_revision":pool.config_revision,"members":[{"user_service_id":member,"weight":1001}]});
    assert!(
        handler::update_pool(
            State(state.clone()),
            test_auth_user(&owner),
            Path(pool.id.clone()),
            Json(serde_json::from_value(update).unwrap())
        )
        .await
        .is_err()
    );
    assert!(
        handler::add_member(
            State(state),
            test_auth_user(&owner),
            Path(pool.id.clone()),
            Json(serde_json::from_value(json!({"user_service_id":member,"weight":1001})).unwrap())
        )
        .await
        .is_err()
    );
    assert_eq!(
        before,
        db.collection::<mongodb::bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&pool.id})
            .await
            .unwrap()
            .unwrap()
    );
    db.drop().await.unwrap();
}
