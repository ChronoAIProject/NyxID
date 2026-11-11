use super::*;

#[tokio::test]
async fn optional_authority_exact_redemption_forwards_frozen_header_once() {
    let f = optional_fixture().await;
    let mut token = f.token(false).await;
    // A pending approval created before storage migration keeps its lineage.
    token.secret_context_versions.clear();
    token.access_token_encrypted = Some(
        f.state
            .encryption_keys
            .encrypt(b"owner-token")
            .await
            .unwrap(),
    );
    f.state
        .db
        .collection::<UserProviderToken>(TOKENS)
        .replace_one(doc! {"_id": &token.id}, &token)
        .await
        .unwrap();
    // Exercise the actual approval producer, including persisted additive
    // authority, read-only observation, decision, redemption and replay.
    let operation = f.operation().await;
    let approval_target = approval_target(&f.state, &f.caller, operation.service())
        .await
        .unwrap();
    f.state
        .db
        .collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: f.caller.approval_owner_user_id.clone(),
                service_id: approval_target.service_id,
                service_name: f.catalog.name.clone(),
                approval_required: true,
                approval_mode: ApprovalMode::PerRequest,
                rules: Vec::new(),
                default_effect: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    let created = Box::pin(create_request(
        &f.state,
        &f.caller,
        ExactServiceApprovalCreate {
            user_service_id: f.connection.id.clone(),
            endpoint_id: f.endpoint_id.clone(),
            catalog_digest: operation.catalog_digest,
            exact_view_digest: Some(operation.exact_view_digest),
            endpoint_contract_digest: operation.endpoint_contract_digest,
            operation_digest: operation.operation_digest,
            operation_id: f.endpoint_id.clone(),
            operation_generation: Some(operation.operation_generation),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            arguments: serde_json::json!({}),
        },
    ))
    .await
    .unwrap();
    assert_eq!(created.state, ExactServiceApprovalState::Pending);
    let request_id = created.request_id;
    let persisted = f
        .state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"_id":&request_id})
        .await
        .unwrap()
        .unwrap();
    let stored = persisted.exact_service.unwrap();
    assert!(stored.optional_provider_authority_digest.is_some());
    assert_eq!(
        stored.optional_provider_authority_digest,
        f.authority().await.optional_provider_authority_digest
    );
    let source = f
        .state
        .db
        .collection::<bson::Document>(TOKENS)
        .find_one(doc! {"_id": &token.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        records::migrate_snapshot(&f.state.db, &f.state.encryption_keys, TOKENS, &source)
            .await
            .unwrap()
    );
    let migrated = f
        .state
        .db
        .collection::<UserProviderToken>(TOKENS)
        .find_one(doc! {"_id": &token.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        migrated
            .secret_context_versions
            .get("access_token_encrypted"),
        Some(&3)
    );
    assert_eq!(migrated.state_version, token.state_version);
    assert_ne!(
        migrated.access_token_encrypted,
        token.access_token_encrypted
    );
    assert_eq!(
        stored.optional_provider_authority_digest,
        f.authority().await.optional_provider_authority_digest,
        "storage-only migration must preserve the pending approval's optional authority"
    );
    let pending_before = f
        .state
        .db
        .collection::<bson::Document>(TOKENS)
        .find_one(doc! {"_id": &token.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Box::pin(observe_request(&f.state, &f.caller, &request_id))
            .await
            .unwrap()
            .state,
        ExactServiceApprovalState::Pending
    );
    assert_eq!(
        Some(pending_before),
        f.state
            .db
            .collection::<bson::Document>(TOKENS)
            .find_one(doc! {"_id": &token.id})
            .await
            .unwrap(),
        "observe stays metadata-only after migration"
    );
    Box::pin(approval_service::process_decision(
        &f.state.db,
        &f.state.config,
        &f.state.http_client,
        None,
        None,
        &request_id,
        true,
        None,
        None,
        "web",
    ))
    .await
    .unwrap();
    let before = f
        .state
        .db
        .collection::<bson::Document>(TOKENS)
        .find_one(doc! {"user_id":&f.caller.proxy_resolution_user_id})
        .await
        .unwrap()
        .unwrap();
    let observed = Box::pin(observe_request(&f.state, &f.caller, &request_id))
        .await
        .unwrap();
    assert_eq!(observed.state, ExactServiceApprovalState::Approved);
    assert_eq!(
        before,
        f.state
            .db
            .collection::<bson::Document>(TOKENS)
            .find_one(doc! {"user_id":&f.caller.proxy_resolution_user_id})
            .await
            .unwrap()
            .unwrap()
    );
    assert!(f.effects.lock().unwrap().is_empty());
    for _ in 0..2 {
        let result = Box::pin(redeem_request(
            &f.state,
            &f.caller,
            &request_id,
            ExactServiceApprovalFence {
                catalog_digest: stored.catalog_digest.clone(),
                exact_view_digest: stored.exact_view_digest_binding.clone(),
                operation_digest: stored.operation_digest.clone(),
                operation_id: stored.operation_id.clone(),
                operation_generation: stored.operation_generation,
                idempotency_key: stored.effect_idempotency_key.clone(),
            },
            crate::services::billing::route_inventory::enforce_billing_egress_classification(
                Some(
                    crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                        crate::services::billing::route_inventory::BillingIngress::Mcp,
                    ),
                ),
                crate::services::billing::route_inventory::BillingIngress::Mcp,
            )
            .unwrap(),
        ))
        .await
        .unwrap();
        assert_eq!(
            result.state,
            ExactServiceApprovalState::Redeemed,
            "{:?}",
            result.failure_code
        );
    }
    assert_eq!(*f.effects.lock().unwrap(), vec![Some("owner-token".into())]);
    assert_eq!(f.refreshes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn optional_authority_pinned_contextual_token_rejects_cross_record_ciphertext() {
    let f = optional_fixture().await;
    let token = f.token(false).await;
    assert_eq!(
        token.secret_context_versions.get("access_token_encrypted"),
        Some(&3)
    );
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_allow_legacy_record_reads(false);
    let authority = f.authority().await;
    let snapshot = read_optional_auto_provision_snapshot(
        &f.state.db,
        &f.caller.proxy_resolution_user_id,
        &f.connection.id,
        &f.catalog.id,
        &authority.resolution.target,
    )
    .await
    .unwrap();
    let credentials = Box::pin(materialize_optional_auto_provision_snapshot(
        &f.state.db,
        &keys,
        &authority.resolution.target,
        &snapshot,
        None,
    ))
    .await
    .unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].credential.as_str(), "owner-token");
    let foreign_id = uuid::Uuid::new_v4().to_string();
    let foreign_ciphertext = records::encrypt(
        &keys,
        TOKENS,
        &foreign_id,
        &token.user_id,
        "access_token_encrypted",
        b"foreign-token",
    )
    .await
    .unwrap();
    let mut substituted = token.clone();
    substituted.access_token_encrypted = Some(foreign_ciphertext);
    f.state
        .db
        .collection::<UserProviderToken>(TOKENS)
        .replace_one(doc! {"_id": &token.id}, &substituted)
        .await
        .unwrap();
    assert!(
        crate::services::user_token_service::get_active_token_at_revision(
            &f.state.db,
            &keys,
            &token,
            None,
        )
        .await
        .is_err(),
        "the pinned reader must authenticate the persisted record identity"
    );
    assert_eq!(
        snapshot.digest,
        f.authority().await.optional_provider_authority_digest,
        "storage bytes are not approval authority"
    );
    let credentials = Box::pin(materialize_optional_auto_provision_snapshot(
        &f.state.db,
        &keys,
        &authority.resolution.target,
        &snapshot,
        None,
    ))
    .await
    .unwrap();
    assert!(
        credentials.is_empty(),
        "unavailable optional material must never inject foreign plaintext"
    );
    assert!(f.effects.lock().unwrap().is_empty());
    assert_eq!(f.refreshes.load(Ordering::SeqCst), 0);
}
