use super::*;
use crate::handlers::service_pools_handler as handler;
use axum::{Json, extract::Query};
use serde_json::{Value, json};

async fn inspect(fixture: &Fixture, query: Value, health: bool) -> handler::PoolCandidatesResponse {
    let args = (
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(serde_json::from_value(query).unwrap()),
    );
    let Json(result) = if health {
        handler::health(args.0, args.1, args.2, args.3).await
    } else {
        handler::pool_candidates(args.0, args.1, args.2, args.3).await
    }
    .unwrap();
    result
}

#[tokio::test]
async fn pool_inspection_inventory_does_not_require_guessing_an_allowed_operation() {
    let fixture = fixture(
        "pool_inventory_operations",
        StatusCode::OK,
        "priority",
        false,
    )
    .await;
    fixture.state.db.collection::<Document>("downstream_services").update_many(doc!{}, doc!{"$set":{"proxy_operation_policy":{"rules":[{"method":"GET","path_template":"/items"}]}}}).await.unwrap();
    let decrypts = fixture.state.encryption_keys.decrypt_stats();
    let inventory = inspect(&fixture, json!({"check_operation":false}), false).await;
    assert_eq!(inventory.candidates.len(), 2);
    let legacy_default = inspect(&fixture, json!({}), false).await;
    assert!(legacy_default.operation_checked);
    assert_eq!(legacy_default.method.as_deref(), Some("POST"));
    assert_eq!(legacy_default.path.as_deref(), Some("/"));
    assert!(
        legacy_default
            .candidates
            .iter()
            .all(|row| row.reason.as_deref() == Some("operation_unsupported"))
    );
    assert!(
        inventory
            .candidates
            .iter()
            .all(|row| row.eligible && !row.name.is_empty())
    );
    let serialized = serde_json::to_value(inventory).unwrap();
    assert_eq!(serialized["operation_checked"], false);
    assert!(serialized["method"].is_null() && serialized["path"].is_null());
    let denied = inspect(&fixture, json!({"method":"POST","path":"/"}), false).await;
    assert!(denied.operation_checked);
    assert!(
        denied
            .candidates
            .iter()
            .all(|row| row.reason.as_deref() == Some("operation_unsupported"))
    );
    let allowed = inspect(&fixture, json!({"method":"GET","path":"/items"}), false).await;
    assert_eq!(allowed.method.as_deref(), Some("GET"));
    assert_eq!(allowed.path.as_deref(), Some("/items"));
    assert!(allowed.candidates.iter().all(|row| row.eligible));
    for path in [
        "test/",
        "//items",
        "/../items",
        "/items?key=value",
        "/%2fitems",
    ] {
        let rejected = handler::pool_candidates(
            State(fixture.state.clone()),
            fixture.auth.clone(),
            Path(fixture.pool_id.clone()),
            Query(
                serde_json::from_value(
                    json!({"method":"GET","path":path,"search":"no-matching-connection"}),
                )
                .unwrap(),
            ),
        )
        .await;
        assert!(
            matches!(rejected, Err(crate::errors::AppError::BadRequest(_))),
            "invalid paths are rejected even with empty inventory"
        );
    }
    assert_eq!(decrypts, fixture.state.encryption_keys.decrypt_stats());
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_inspection_saved_operation_health_is_separate_from_drafts_and_legacy() {
    let fixture = fixture(
        "pool_inspection_health",
        StatusCode::TOO_MANY_REQUESTS,
        "priority",
        false,
    )
    .await;
    to_bytes(call(&fixture, "{}").await.into_body(), 1024)
        .await
        .unwrap();
    let checked = inspect(&fixture, json!({"method":"POST","path":"perform"}), false).await;
    // The fixture request uses the same operation that cooldown is scoped to.
    let first = checked
        .candidates
        .iter()
        .find(|row| row.slug == "review-first")
        .unwrap();
    assert_eq!(first.reason.as_deref(), Some("cooldown"));
    let first_id = first.user_service_id.clone();
    let draft = inspect(&fixture, json!({"method":"POST","path":"perform","peer_ids":format!("{first_id},{}",fixture.second_member_id)}), false).await;
    assert!(
        draft
            .candidates
            .iter()
            .all(|row| row.eligible && row.cooldown_until.is_none())
    );
    let inventory = inspect(&fixture, json!({"check_operation":false}), false).await;
    assert!(
        inventory
            .candidates
            .iter()
            .all(|row| row.eligible && row.last_status.is_none())
    );
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! {"_id":&fixture.pool_id},
            doc! {"$set":{"members.0.enabled":false}},
        )
        .await
        .unwrap();
    assert!(
        inspect(&fixture, json!({"check_operation":false}), false)
            .await
            .candidates
            .iter()
            .all(|row| row.eligible)
    );
    let saved = inspect(&fixture, json!({"check_operation":false}), true).await;
    assert!(
        saved
            .candidates
            .iter()
            .any(|row| row.reason.as_deref() == Some("disabled"))
    );
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! {"_id":&fixture.pool_id},
            doc! {"$set":{"members.0.enabled":true,"strategy":"weighted"}},
        )
        .await
        .unwrap();
    let legacy = inspect(&fixture, json!({"method":"POST","path":"perform"}), true).await;
    assert!(
        legacy.candidates.iter().all(|row| row.eligible
            && row.cooldown_until.is_none()
            && row.consecutive_failures == 0)
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_inspection_draft_selection_is_independent_of_search_paging_and_saved_peers() {
    let fixture = fixture("pool_inspection_draft", StatusCode::OK, "priority", false).await;
    // Make one connection custom: both it and a catalog peer require declarations.
    fixture
        .state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&fixture.second_member_id},
            doc! {"$unset":{"catalog_service_id":""}},
        )
        .await
        .unwrap();
    let all = inspect(&fixture, json!({"peer_ids":""}), false).await;
    let first_id = all
        .candidates
        .iter()
        .find(|row| row.slug == "review-first")
        .unwrap()
        .user_service_id
        .clone();
    let peers = format!("{first_id},{}", fixture.second_member_id);
    let selected = inspect(
        &fixture,
        json!({"selected_only":true,"peer_ids":peers,"search":"no-match","limit":1,"after":"999"}),
        false,
    )
    .await;
    assert_eq!(selected.candidates.len(), 2);
    assert!(!selected.has_more);
    assert!(selected.next_cursor.is_none());
    assert!(
        selected
            .candidates
            .iter()
            .all(|row| row.requires_compatibility_declaration
                && row.reason.as_deref() == Some("compatibility_declaration_required"))
    );
    let declared = inspect(
        &fixture,
        json!({"selected_only":true,"peer_ids":peers,"declared_peer_ids":peers}),
        false,
    )
    .await;
    assert!(declared.candidates.iter().all(|row| row.eligible));
    let removed_custom = inspect(
        &fixture,
        json!({"selected_only":true,"peer_ids":first_id,"declared_peer_ids":""}),
        false,
    )
    .await;
    assert_eq!(removed_custom.candidates.len(), 1);
    assert!(removed_custom.candidates[0].eligible);
    assert!(!removed_custom.candidates[0].requires_compatibility_declaration);
    let page = inspect(&fixture, json!({"limit":1,"peer_ids":""}), false).await;
    assert_eq!(page.candidates.len(), 1);
    assert!(page.has_more);
    let next = inspect(
        &fixture,
        json!({"limit":1,"after":page.next_cursor,"peer_ids":""}),
        false,
    )
    .await;
    assert_eq!(next.candidates.len(), 1);
    assert!(!next.has_more);
    assert_ne!(
        next.candidates[0].user_service_id,
        page.candidates[0].user_service_id
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_inspection_unavailable_credentials_do_not_poison_healthy_peers() {
    let fixture = fixture(
        "pool_unavailable_credentials",
        StatusCode::OK,
        "priority",
        false,
    )
    .await;
    bind_first_credential(&fixture).await;
    let db = &fixture.state.db;
    let first = db
        .collection::<Document>("user_services")
        .find_one(doc! {"slug":"review-first"})
        .await
        .unwrap()
        .unwrap();
    let first_id = first.get_str("_id").unwrap();
    let key_id = first.get_str("api_key_id").unwrap();
    let peers = format!("{first_id},{}", fixture.second_member_id);
    let decrypts = fixture.state.encryption_keys.decrypt_stats();
    for status in [
        "failed",
        "inactive",
        "expired",
        "revoked",
        "refresh_failed",
        "pending_auth",
    ] {
        db.collection::<Document>("user_api_keys")
            .update_one(doc! {"_id":key_id}, doc! {"$set":{"status":status}})
            .await
            .unwrap();
        for (query, health) in [
            (json!({"check_operation":false}), false),
            (json!({"method":"POST","path":"perform"}), false),
            (
                json!({"check_operation":false,"selected_only":true,"peer_ids":peers}),
                false,
            ),
            (json!({"check_operation":false}), true),
            (json!({"method":"POST","path":"perform"}), true),
        ] {
            let result = inspect(&fixture, query, health).await;
            assert_eq!(result.candidates.len(), 2, "{status}");
            let unavailable = result
                .candidates
                .iter()
                .find(|r| r.user_service_id == first_id)
                .unwrap();
            assert!(!unavailable.eligible, "{status}");
            assert_eq!(
                unavailable.reason.as_deref(),
                Some("credential_unavailable")
            );
            assert!(
                result
                    .candidates
                    .iter()
                    .find(|r| r.user_service_id == fixture.second_member_id)
                    .unwrap()
                    .eligible
            );
        }
        assert_eq!(decrypts, fixture.state.encryption_keys.decrypt_stats());
        let response = call(&fixture, "{}").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
        assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
        to_bytes(response.into_body(), 1024).await.unwrap();
    }
    // Active metadata without material is unavailable too, including incomplete OAuth.
    for credential_type in ["bearer", "oauth2", "gcp_service_account", "node_managed"] {
        db.collection::<Document>("user_api_keys").update_one(
            doc! {"_id":key_id},
            doc! {"$set":{"status":"active","credential_type":credential_type},"$unset":{"credential_encrypted":""}},
        ).await.unwrap();
        let result = inspect(&fixture, json!({"check_operation":false}), false).await;
        assert_eq!(
            result
                .candidates
                .iter()
                .find(|r| r.user_service_id == first_id)
                .unwrap()
                .reason
                .as_deref(),
            Some("credential_unavailable")
        );
        assert!(
            result
                .candidates
                .iter()
                .find(|r| r.user_service_id == fixture.second_member_id)
                .unwrap()
                .eligible
        );
    }
    assert_eq!(decrypts, fixture.state.encryption_keys.decrypt_stats());
    assert!(fixture.first.requests.lock().await.is_empty());
    assert_eq!(fixture.second.requests.lock().await.len(), 6);
    let key = db
        .collection::<Document>("user_api_keys")
        .find_one(doc! {"_id":key_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!key.contains_key("last_used_at"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_inspection_credential_integrity_errors_still_fail() {
    let fixture = fixture(
        "pool_credential_integrity",
        StatusCode::OK,
        "priority",
        false,
    )
    .await;
    bind_first_credential(&fixture).await;
    let db = &fixture.state.db;
    let first = db
        .collection::<Document>("user_services")
        .find_one(doc! {"slug":"review-first"})
        .await
        .unwrap()
        .unwrap();
    let key_id = first.get_str("api_key_id").unwrap();
    let inspect_result = || {
        handler::pool_candidates(
            State(fixture.state.clone()),
            fixture.auth.clone(),
            Path(fixture.pool_id.clone()),
            Query(serde_json::from_value(json!({"check_operation":false})).unwrap()),
        )
    };
    // Deserialization failures from actual database reads must propagate.
    db.collection::<Document>("user_api_keys")
        .update_one(doc! {"_id":key_id}, doc! {"$set":{"status":17}})
        .await
        .unwrap();
    assert!(matches!(
        inspect_result().await,
        Err(crate::errors::AppError::DatabaseError(_))
    ));
    db.collection::<Document>("user_api_keys")
        .delete_one(doc! {"_id":key_id})
        .await
        .unwrap();
    assert!(matches!(
        inspect_result().await,
        Err(crate::errors::AppError::Internal(_))
    ));
    assert!(matches!(
        try_call(&fixture, "{}").await,
        Err(crate::errors::AppError::Internal(_))
    ));
    db.collection::<Document>("user_endpoints")
        .delete_one(doc! {"_id":first.get_str("endpoint_id").unwrap()})
        .await
        .unwrap();
    assert!(matches!(
        inspect_result().await,
        Err(crate::errors::AppError::Internal(_))
    ));
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_inspection_failed_agent_override_skips_only_its_member() {
    let mut fixture = fixture("pool_failed_override", StatusCode::OK, "priority", false).await;
    bind_first_credential(&fixture).await;
    let db = &fixture.state.db;
    let first = db
        .collection::<Document>("user_services")
        .find_one(doc! {"slug":"review-first"})
        .await
        .unwrap()
        .unwrap();
    let first_id = first.get_str("_id").unwrap();
    let mut override_key = db
        .collection::<Document>("user_api_keys")
        .find_one(doc! {"_id":first.get_str("api_key_id").unwrap()})
        .await
        .unwrap()
        .unwrap();
    let override_id = Uuid::new_v4().to_string();
    override_key.insert("_id", &override_id);
    override_key.insert("status", "failed");
    db.collection::<Document>("user_api_keys")
        .insert_one(override_key)
        .await
        .unwrap();
    let agent_id = Uuid::new_v4().to_string();
    fixture.auth.api_key_id = Some(agent_id.clone());
    db.collection::<Document>("agent_service_bindings").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"api_key_id":&agent_id,"user_id":fixture.auth.user_id.to_string(),
        "user_service_id":first_id,"user_api_key_id":&override_id,
        "created_at":mongodb::bson::DateTime::now(),"updated_at":mongodb::bson::DateTime::now(),
    }).await.unwrap();
    let decrypts = fixture.state.encryption_keys.decrypt_stats();
    for (query, health) in [
        (json!({"check_operation":false}), false),
        (json!({"method":"POST","path":"perform"}), false),
        (
            json!({"check_operation":false,"selected_only":true,"peer_ids":format!("{first_id},{}",fixture.second_member_id)}),
            false,
        ),
        (json!({"check_operation":false}), true),
        (json!({"method":"POST","path":"perform"}), true),
    ] {
        let result = inspect(&fixture, query, health).await;
        assert_eq!(
            result
                .candidates
                .iter()
                .find(|r| r.user_service_id == first_id)
                .unwrap()
                .reason
                .as_deref(),
            Some("credential_unavailable")
        );
        assert!(
            result
                .candidates
                .iter()
                .find(|r| r.user_service_id == fixture.second_member_id)
                .unwrap()
                .eligible
        );
    }
    let owner = fixture.auth.user_id.to_string();
    let plan = crate::services::service_pool_service::plan_candidates_with_allowlist(
        db,
        &fixture.state.encryption_keys,
        &owner,
        Some(&agent_id),
        &owner,
        "review-route",
        &Method::POST,
        Some("perform"),
        2,
        Some(b"{}"),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(plan.candidates.len(), 1);
    assert_eq!(plan.candidates[0].service.id, fixture.second_member_id);
    assert_eq!(decrypts, fixture.state.encryption_keys.decrypt_stats());
    db.collection::<Document>("user_api_keys")
        .delete_one(doc! {"_id":&override_id})
        .await
        .unwrap();
    let result = handler::health(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(serde_json::from_value(json!({"method":"POST","path":"perform"})).unwrap()),
    )
    .await;
    assert!(matches!(result, Err(crate::errors::AppError::Internal(_))));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_inspection_node_platform_and_noauth_do_not_require_server_user_credentials() {
    let mut fixture = fixture(
        "pool_credential_bindings",
        StatusCode::OK,
        "priority",
        false,
    )
    .await;
    bind_first_credential(&fixture).await;
    let db = &fixture.state.db;
    let first = db
        .collection::<Document>("user_services")
        .find_one(doc! {"slug":"review-first"})
        .await
        .unwrap()
        .unwrap();
    let first_id = first.get_str("_id").unwrap();
    let key_id = first.get_str("api_key_id").unwrap();
    db.collection::<Document>("user_api_keys").update_one(doc! {"_id":key_id},doc! {"$set":{"status":"failed","credential_type":"node_managed"},"$unset":{"credential_encrypted":""}}).await.unwrap();
    let decrypts = fixture.state.encryption_keys.decrypt_stats();
    let node_id = Uuid::new_v4().to_string();
    db.collection::<Document>("user_services")
        .update_one(doc! {"_id":first_id}, doc! {"$set":{"node_id":&node_id}})
        .await
        .unwrap();
    let now = chrono::Utc::now();
    db.collection::<Document>("nodes").insert_one(doc! {
        "_id":&node_id,"user_id":fixture.auth.user_id.to_string(),"name":"pool test node","status":"online","auth_token_hash":"test-only","is_active":true,
        "created_at":mongodb::bson::DateTime::from_chrono(now),"updated_at":mongodb::bson::DateTime::from_chrono(now),
        "connection_owner":{"instance_name":"test","generation_id":"test","connection_id":"test","internal_base_url":"http://127.0.0.1:1",
        "claimed_at":mongodb::bson::DateTime::from_chrono(now),"renewed_at":mongodb::bson::DateTime::from_chrono(now),"expires_at":mongodb::bson::DateTime::from_chrono(now+chrono::Duration::hours(1)),"http_cancellation":true},
    }).await.unwrap();
    let node_result = inspect(&fixture, json!({"check_operation":false}), false).await;
    assert!(node_result.candidates.iter().all(|r| r.eligible));
    // Node-owned credentials remain excluded by the effective route ACL.
    fixture.auth.allow_all_nodes = false;
    fixture.auth.allowed_node_ids = vec![];

    for health in [false, true] {
        let result = inspect(&fixture, json!({"check_operation":false}), health).await;
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(
            result.candidates[0].user_service_id,
            fixture.second_member_id
        );
    }
    fixture.auth.allow_all_nodes = true;
    fixture.auth.allow_all_services = false;
    fixture.auth.allowed_service_ids = vec![fixture.second_member_id.clone()];
    assert_eq!(
        inspect(&fixture, json!({"check_operation":false}), false)
            .await
            .candidates
            .len(),
        1
    );
    fixture.auth.allow_all_services = true;
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":first_id},
            doc! {"$set":{"auth_method":"none"},"$unset":{"node_id":""}},
        )
        .await
        .unwrap();
    assert!(
        inspect(&fixture, json!({"check_operation":false}), false)
            .await
            .candidates
            .iter()
            .all(|r| r.eligible)
    );
    db.collection::<Document>("downstream_services").update_one(doc! {"_id":first.get_str("catalog_service_id").unwrap()},doc! {"$set":{"auth_method":"bearer","service_category":"internal","requires_user_credential":false,"visibility":"public","credential_encrypted":mongodb::bson::Binary{subtype:mongodb::bson::spec::BinarySubtype::Generic,bytes:vec![1]}}}).await.unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":first_id},
            doc! {"$set":{"auth_method":"bearer","credential_binding":"platform"}},
        )
        .await
        .unwrap();
    db.collection::<Document>("node_service_bindings").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"node_id":&node_id,"user_id":fixture.auth.user_id.to_string(),
        "service_id":first.get_str("catalog_service_id").unwrap(),"is_active":true,"priority":0,
        "created_at":mongodb::bson::DateTime::from_chrono(now),"updated_at":mongodb::bson::DateTime::from_chrono(now),
    }).await.unwrap();
    fixture.auth.allow_all_nodes = false;
    let platform = inspect(&fixture, json!({"check_operation":false}), false).await;
    let row = platform
        .candidates
        .iter()
        .find(|r| r.user_service_id == first_id)
        .unwrap();
    assert!(row.eligible);
    assert_eq!(row.credential_binding, "platform");
    assert_eq!(decrypts, fixture.state.encryption_keys.decrypt_stats());
    assert!(fixture.first.requests.lock().await.is_empty());
    db.drop().await.unwrap();
}

#[test]
fn pool_credential_unavailability_is_narrowly_classified() {
    use crate::errors::AppError;
    use crate::services::service_pool_service::member_unavailable;
    assert!(member_unavailable(&AppError::CredentialUnavailable(
        "failed".into()
    )));
    assert!(!member_unavailable(&AppError::BadRequest(
        "API key is failed".into()
    )));
    assert!(!member_unavailable(&AppError::BadRequest(
        "Invalid proxy path".into()
    )));
    assert!(!member_unavailable(&AppError::Internal(
        "missing key row".into()
    )));
    assert!(!member_unavailable(&AppError::ValidationError(
        "invalid platform route".into()
    )));
}

#[tokio::test]
async fn pool_inspection_name_search_precedes_paging_and_preserves_scope() {
    let mut fixture = fixture("pool_name_search", StatusCode::OK, "priority", false).await;
    let db = &fixture.state.db;
    let services: Vec<Document> = db
        .collection::<Document>("user_services")
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let first_id = services[0].get_str("_id").unwrap();
    let last = &services[1];
    let last_id = last.get_str("_id").unwrap();
    let original_catalog_id = last.get_str("catalog_service_id").unwrap().to_owned();
    let label = "Preferred member [West]";
    db.collection::<Document>("user_endpoints")
        .update_one(
            doc! {"_id":last.get_str("endpoint_id").unwrap()},
            doc! {"$set":{"label":label}},
        )
        .await
        .unwrap();
    let decrypts = fixture.state.encryption_keys.decrypt_stats();
    let page = inspect(&fixture, json!({"check_operation":false,"limit":1}), false).await;
    assert_eq!(page.candidates[0].user_service_id, first_id);
    for search in ["preferred MEMBER [west]", last.get_str("slug").unwrap()] {
        let found = inspect(
            &fixture,
            json!({"check_operation":false,"search":search,"limit":1}),
            false,
        )
        .await;
        assert_eq!(found.candidates.len(), 1);
        assert_eq!(found.candidates[0].user_service_id, last_id);
        assert_eq!(found.candidates[0].name, label);
        assert_ne!(found.candidates[0].group_name, "Custom connections");
        assert!(found.candidates[0].group_slug.is_some());
        assert!(!found.has_more);
    }
    // Only the later inventory row references this catalog. Matching its literal
    // original name or slug must happen before limit/skip, not on a loaded page.
    let catalog_id = Uuid::new_v4().to_string();
    let catalog_name = "Original [West].* (Pool)+";
    let catalog_slug = "original-west-pool";
    let mut catalog = db
        .collection::<Document>("downstream_services")
        .find_one(doc! {"_id":last.get_str("catalog_service_id").unwrap()})
        .await
        .unwrap()
        .unwrap();
    catalog.insert("_id", &catalog_id);
    catalog.insert("name", catalog_name);
    catalog.insert("slug", catalog_slug);
    db.collection::<Document>("downstream_services")
        .insert_one(catalog)
        .await
        .unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":last_id},
            doc! {"$set":{"catalog_service_id":&catalog_id}},
        )
        .await
        .unwrap();
    for search in ["original [west].* (pool)+", "ORIGINAL-WEST-POOL", ".*"] {
        let found = inspect(
            &fixture,
            json!({"check_operation":false,"search":search,"limit":1}),
            false,
        )
        .await;
        assert_eq!(found.candidates.len(), 1, "{search}");
        assert_eq!(found.candidates[0].user_service_id, last_id, "{search}");
        assert_eq!(found.candidates[0].group_name, catalog_name);
        assert_eq!(
            found.candidates[0].group_slug.as_deref(),
            Some(catalog_slug)
        );
        assert_eq!(
            found.candidates[0].catalog_service_id.as_deref(),
            Some(catalog_id.as_str())
        );
        assert!(!found.has_more);
        assert!(
            inspect(
                &fixture,
                json!({"check_operation":false,"search":search,"limit":1,"after":"1"}),
                false
            )
            .await
            .candidates
            .is_empty()
        );
    }
    assert!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":"[not-present]"}),
            false
        )
        .await
        .candidates
        .is_empty()
    );
    // Matching foreign-owned instances must never enter the candidate result.
    let mut foreign = last.clone();
    foreign.insert("_id", Uuid::new_v4().to_string());
    foreign.insert("user_id", Uuid::new_v4().to_string());
    foreign.insert("catalog_service_id", &catalog_id);
    db.collection::<Document>("user_services")
        .insert_one(foreign)
        .await
        .unwrap();
    assert_eq!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":label}),
            false
        )
        .await
        .candidates
        .len(),
        1
    );
    for search in [catalog_name, catalog_slug] {
        let found = inspect(
            &fixture,
            json!({"check_operation":false,"search":search,"limit":1}),
            false,
        )
        .await;
        assert_eq!(found.candidates.len(), 1);
        assert_eq!(found.candidates[0].user_service_id, last_id);
        assert!(
            !found.has_more,
            "foreign owned catalog match must not affect paging"
        );
    }
    fixture.auth.allow_all_services = false;
    fixture.auth.allowed_service_ids = vec![first_id.to_owned()];
    assert!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":label}),
            false
        )
        .await
        .candidates
        .is_empty()
    );
    for search in [catalog_name, catalog_slug] {
        assert!(
            inspect(
                &fixture,
                json!({"check_operation":false,"search":search,"limit":1}),
                false
            )
            .await
            .candidates
            .is_empty()
        );
    }
    let selected = inspect(&fixture, json!({"check_operation":false,"search":"no match","selected_only":true,"peer_ids":format!("{first_id},{last_id}"),"limit":1,"after":"999"}), false).await;
    assert_eq!(selected.candidates.len(), 1);
    assert_eq!(selected.candidates[0].user_service_id, first_id);
    fixture.auth.allow_all_services = true;
    // Platform resolution also displays the endpoint label, with no credential materialization.
    db.collection::<Document>("downstream_services").update_one(
        doc! {"_id":&original_catalog_id},
        doc! {"$set":{"auth_method":"bearer","service_category":"internal","requires_user_credential":false,"visibility":"public","credential_encrypted":mongodb::bson::Binary{subtype:mongodb::bson::spec::BinarySubtype::Generic,bytes:vec![1]}}},
    ).await.unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":last_id},
            doc! {"$set":{"catalog_service_id":&original_catalog_id,"credential_binding":"platform","auth_method":"bearer"}},
        )
        .await
        .unwrap();
    let platform = inspect(
        &fixture,
        json!({"check_operation":false,"search":label}),
        false,
    )
    .await;
    assert_eq!(platform.candidates.len(), 1);
    assert_eq!(platform.candidates[0].name, label);
    assert_eq!(platform.candidates[0].credential_binding, "platform");
    assert!(platform.candidates[0].eligible);
    // Matching-name pages retain stable pagination, without consuming unrelated rows.
    db.collection::<Document>("user_endpoints")
        .update_one(
            doc! {"_id":services[0].get_str("endpoint_id").unwrap()},
            doc! {"$set":{"label":"Another member [West]"}},
        )
        .await
        .unwrap();
    let page = inspect(
        &fixture,
        json!({"check_operation":false,"search":"member [West]","limit":1}),
        false,
    )
    .await;
    assert_eq!(page.candidates.len(), 1);
    assert!(page.has_more);
    let next = inspect(&fixture, json!({"check_operation":false,"search":"member [West]","limit":1,"after":page.next_cursor}), false).await;
    assert_eq!(next.candidates.len(), 1);
    assert!(!next.has_more);
    assert_ne!(
        page.candidates[0].user_service_id,
        next.candidates[0].user_service_id
    );
    assert_eq!(decrypts, fixture.state.encryption_keys.decrypt_stats());
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    db.drop().await.unwrap();
}
