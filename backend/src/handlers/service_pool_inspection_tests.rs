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
