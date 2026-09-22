use super::*;
use crate::services::destination_routing::tests::{Echo, connect, seed};
use crate::test_utils::*;

async fn setup_approval(
    state: &AppState,
    owner: &str,
    user_service: &crate::models::user_service::UserService,
    operation_name: &str,
) -> (ExactServiceApprovalCaller, ExactServiceApprovalResult) {
    let caller = ExactServiceApprovalCaller {
        actor_user_id: owner.into(),
        proxy_resolution_user_id: owner.into(),
        approval_owner_user_id: owner.into(),
        requester_type: "delegated".into(),
        requester_id: "test-client".into(),
        requester_label: None,
        api_key_id: None,
        has_catalog_read: true,
        allow_all_services: true,
        allowed_service_ids: vec![],
        allow_all_nodes: true,
        allowed_node_ids: vec![],
    };
    let endpoint = state.db.collection::<crate::models::service_endpoint::ServiceEndpoint>(crate::models::service_endpoint::COLLECTION_NAME).find_one(doc! {"service_id":user_service.catalog_service_id.as_ref().unwrap(),"name":operation_name}).await.unwrap().unwrap();
    let args = if operation_name == "docs_batch_update_document" {
        serde_json::json!({"documentId":"approval-doc","body":{"requests":[]}})
    } else {
        serde_json::json!({})
    };
    let resolution = resolve_exact_catalog(state, &caller, &user_service.id, &endpoint.id, &args)
        .await
        .unwrap();
    let target = approval_target(state, &caller, resolution.service())
        .await
        .unwrap();
    approval_service::set_service_approval_config(
        &state.db,
        owner,
        &target.service_id,
        "Workspace",
        Some(true),
        Some(&ApprovalMode::PerRequest),
        None,
        None,
    )
    .await
    .unwrap();
    let channel = notification_service::get_or_create_channel(&state.db, owner)
        .await
        .unwrap();
    state
        .db
        .collection::<mongodb::bson::Document>(crate::models::notification_channel::COLLECTION_NAME)
        .update_one(
            doc! {"_id":channel.id},
            doc! {"$set":{"approval_timeout_secs":300}},
        )
        .await
        .unwrap();
    let input = ExactServiceApprovalCreate {
        user_service_id: user_service.id.clone(),
        endpoint_id: endpoint.id.clone(),
        catalog_digest: resolution.catalog_digest,
        exact_view_digest: Some(resolution.exact_view_digest),
        endpoint_contract_digest: resolution.endpoint_contract_digest,
        operation_digest: resolution.operation_digest,
        operation_id: endpoint.id,
        operation_generation: Some(endpoint.operation_generation),
        idempotency_key: uuid::Uuid::new_v4().to_string(),
        arguments: args,
    };
    let future = create_request(state, &caller, input);
    eprintln!(
        "workspace exact create future bytes: {}",
        std::mem::size_of_val(&future)
    );
    let result = future.await.unwrap();
    assert_eq!(result.state, ExactServiceApprovalState::Pending);
    (caller, result)
}

fn fence(result: &ExactServiceApprovalResult) -> ExactServiceApprovalFence {
    ExactServiceApprovalFence {
        catalog_digest: result.catalog_digest.clone(),
        exact_view_digest: result.exact_view_digest.clone(),
        operation_digest: result.operation_digest.clone(),
        operation_id: result.operation_id.clone(),
        operation_generation: result.operation_generation,
        idempotency_key: result.idempotency_key.clone(),
    }
}

async fn approve(state: &AppState, result: &ExactServiceApprovalResult) {
    approval_service::process_decision(
        &state.db,
        &state.config,
        &state.http_client,
        None,
        None,
        &result.request_id,
        true,
        None,
        None,
        "web",
    )
    .await
    .unwrap();
}

async fn redeem(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    result: &ExactServiceApprovalResult,
) -> ExactServiceApprovalResult {
    use crate::services::billing::route_inventory::*;
    let future = redeem_request(
        state,
        caller,
        &result.request_id,
        fence(result),
        enforce_billing_egress_classification(
            Some(BillingRoutePolicy::Metered(BillingIngress::Mcp)),
            BillingIngress::Mcp,
        )
        .unwrap(),
    );
    eprintln!(
        "workspace exact redeem future bytes: {}",
        std::mem::size_of_val(&future)
    );
    future.await.unwrap()
}

#[tokio::test]
async fn workspace_pending_approval_preserves_catalog_and_execution_drift_fences() {
    let db = connect_test_database("workspace_pending_approval")
        .await
        .unwrap();
    seed(&db, false).await;
    let owner = uuid::Uuid::new_v4().to_string();
    let service = connect(&db, &owner, "api-google-workspace").await;
    let state = test_app_state(db.clone());
    let (caller, pending) = setup_approval(&state, &owner, &service, "drive_list_files").await;
    let before = load_bound_request(&state, &caller, &pending.request_id)
        .await
        .unwrap();
    assert_eq!((before.expires_at - before.created_at).num_seconds(), 300);
    assert_eq!(
        observe_request(&state, &caller, &pending.request_id)
            .await
            .unwrap()
            .state,
        ExactServiceApprovalState::Pending
    );
    approve(&state, &pending).await;
    // The policy writer can run before the additive endpoint sync finishes.
    crate::services::provider_service::seed_default_services_with_destinations(
        &db,
        &state.encryption_keys,
        true,
    )
    .await
    .unwrap();
    let observed = observe_request(&state, &caller, &pending.request_id)
        .await
        .unwrap();
    assert_eq!(
        observed.failure_code.as_deref(),
        Some("execution_authority_drift")
    );
    crate::services::catalog_spec_sync::sync_seeded_service_endpoints_with_destinations(&db, true)
        .await
        .unwrap();
    let observed = observe_request(&state, &caller, &pending.request_id)
        .await
        .unwrap();
    assert_eq!(observed.failure_code.as_deref(), Some("catalog_drift"));
    assert_eq!(
        load_bound_request(&state, &caller, &pending.request_id)
            .await
            .unwrap()
            .status,
        "approved"
    );
    let result = redeem(&state, &caller, &pending).await;
    assert_eq!(result.state, ExactServiceApprovalState::Drifted);
    assert_eq!(result.failure_code.as_deref(), Some("catalog_drift"));
    assert_eq!(
        load_bound_request(&state, &caller, &pending.request_id)
            .await
            .unwrap()
            .exact_service
            .unwrap()
            .redemption
            .unwrap()
            .failure_code
            .as_deref(),
        Some("catalog_drift")
    );
}

#[tokio::test]
async fn workspace_old_shape_drive_approval_and_new_docs_approval_redeem() {
    let db = connect_test_database("workspace_approval_echo")
        .await
        .unwrap();
    seed(&db, true).await;
    let owner = uuid::Uuid::new_v4().to_string();
    let service = connect(&db, &owner, "api-google-workspace").await;
    let echo = Echo::start().await;
    let mut state = test_app_state(db.clone());
    state.http_client = echo.client.clone();
    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER.scope(echo.client_builder.clone(),async {
        for operation in ["drive_list_files","docs_batch_update_document"] {
            let (caller,pending) = setup_approval(&state,&owner,&service,operation).await;
            if operation == "drive_list_files" {
                // Real pre-digest row shape: these fields did not exist. All
                // catalog, operation, policy, and producer fences remain live.
                db.collection::<ApprovalRequest>(REQUESTS).update_one(doc! {"_id":&pending.request_id},doc! {"$unset":{"exact_service.execution_authority_digest":"","exact_service.execution_authority_binding":""}}).await.unwrap();
            } else {
                let bound = load_bound_request(&state,&caller,&pending.request_id).await.unwrap().exact_service.unwrap();
                let resolution = resolve_exact_catalog(&state,&caller,&service.id,&pending.endpoint_id,&bound.arguments).await.unwrap();
                let future = resolve_execution_authority(&state,&caller,&service.id,&service.slug,ExecutionResolutionMode::ReadOnlySnapshot,&resolution,&bound.arguments);
                eprintln!("workspace resolve_execution_authority future bytes: {}", std::mem::size_of_val(&future));
                let live = future.await.unwrap();
                assert_eq!(live.resolution.target.base_url,"https://docs.googleapis.com");
                assert_eq!(bound.execution_authority_binding.unwrap().digest,live.digest);
            }
            approve(&state,&pending).await;
            let result = redeem(&state,&caller,&pending).await;
            assert_eq!(result.state,ExactServiceApprovalState::Redeemed,"{result:?}");
        }
    }).await;
    let calls = echo.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["host"], "www.googleapis.com");
    assert_eq!(calls[1]["host"], "docs.googleapis.com");
}
