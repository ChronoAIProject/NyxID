use crate::services::record_field_service;

async fn assert_retained_body(
    f: &Fixture,
    row: &AssistantCmaCreation,
    body: &[u8],
    contextual: bool,
) {
    assert_eq!(row.request_context_version, contextual.then_some(3));
    let context =
        RecordContext::new(COLLECTION_NAME, &row.id, &f.owner, &["request_ciphertext"]).unwrap();
    let ciphertext = row.request_ciphertext.as_deref().unwrap();
    assert!(!ciphertext.windows(body.len()).any(|window| window == body));
    assert_eq!(
        record_field_service::decrypt_context(
            &f.keys,
            &context,
            contextual.then_some(3),
            ciphertext
        )
        .await
        .unwrap()
        .as_slice(),
        body
    );
    let parent = sandboxes::get_owned(&f.db, &f.owner, &row.sandbox_ref_id)
        .await
        .unwrap();
    assert_eq!(parent.request_context_version, contextual.then_some(3));
    assert_eq!(
        sandboxes::recover_creation_request(
            &f.db,
            &f.keys,
            &f.owner,
            &parent.id,
            parent.generation
        )
        .await
        .unwrap()
        .as_slice(),
        body
    );
}

#[tokio::test]
async fn assistant_cma_creation_cutover_modes_approval_retry_and_permit() {
    use crate::models::{
        approval_request::{ApprovalRequest, COLLECTION_NAME as REQUESTS},
        service_approval_config::{
            ApprovalMode, COLLECTION_NAME as CONFIGS, ServiceApprovalConfig,
        },
    };
    use crate::services::approval_service;
    for contextual in [false, true] {
        for route in ["/api/v1/sandboxes", "/api/v1/agents"] {
            let mut f = Fixture::new().await;
            f.keys.set_contextual_record_writes(contextual);
            f.observations().await;
            let body = br#"{ "name": "cutover request", "machine_included": false }"#;
            let key = "writer-cutover-approval-key";
            let initial = f.admit(&f.target, route, key, body).await.unwrap();
            let row = initial.creation.as_ref().unwrap();
            assert_retained_body(&f, row, body, contextual).await;
            // Rolling readers accept existing rows regardless of their write setting.
            f.keys.set_contextual_record_writes(!contextual);
            let retry = f.admit(&f.target, route, key, body).await.unwrap();
            let retried = retry.creation.as_ref().unwrap();
            assert_eq!(
                bson::to_document(retried).unwrap(),
                bson::to_document(row).unwrap()
            );
            assert_retained_body(&f, retried, body, contextual).await;
            f.keys.set_contextual_record_writes(contextual);
            f.assert_single_intent().await;
            assert_eq!(f.post_count().await, 0);

            crate::test_utils::set_test_approval_preference(&f.db, &f.owner, true).await;
            f.db.collection::<ServiceApprovalConfig>(CONFIGS)
                .insert_one(ServiceApprovalConfig {
                    id: uuid::Uuid::new_v4().to_string(),
                    user_id: f.owner.clone(),
                    service_id: f.catalog.id.clone(),
                    service_name: "CMA".into(),
                    approval_required: true,
                    approval_mode: ApprovalMode::PerRequest,
                    rules: vec![],
                    default_effect: None,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                })
                .await
                .unwrap();
            let is_agent = route.ends_with("/agents");
            Mock::given(method("POST"))
                .and(path(route))
                .respond_with(
                    ResponseTemplate::new(if is_agent { 202 } else { 201 }).set_body_json(
                        if is_agent {
                            serde_json::json!({"id":"op_1","status":"ready","agent_id":"agent_1"})
                        } else {
                            serde_json::json!({"id":"sandbox_1"})
                        },
                    ),
                )
                .expect(1)
                .mount(&f.server)
                .await;
            let mut state = crate::test_utils::test_app_state(f.db.clone());
            let mut proxy_keys = crate::test_utils::test_encryption_keys();
            proxy_keys.set_contextual_record_writes(contextual);
            state.encryption_keys = std::sync::Arc::new(proxy_keys);
            let mut auth = crate::test_utils::test_auth_user(&f.owner);
            auth.auth_method = crate::mw::auth::AuthMethod::AccessToken;
            auth.scope = "proxy".into();
            let service = f.catalog.id.clone();
            let proxy_state = state.clone();
            let mut proxy = tokio::spawn(async move {
                let request = axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/proxy/{service}{route}"))
                    .header("Content-Type", "application/json")
                    .header("Idempotency-Key", key)
                    .extension(
                        crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                            crate::services::billing::BillingIngress::Proxy,
                        ),
                    )
                    .body(axum::body::Body::from(body.as_slice()))
                    .unwrap();
                Box::pin(crate::handlers::proxy::proxy_request_inner(
                    &proxy_state,
                    &auth,
                    &service,
                    route,
                    request,
                    &mut String::new(),
                ))
                .await
            });
            let pending = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    if let Some(request) =
                        f.db.collection::<ApprovalRequest>(REQUESTS)
                            .find_one(doc! {"user_id":&f.owner,"status":"pending"})
                            .await
                            .unwrap()
                    {
                        break request;
                    }
                    if proxy.is_finished() {
                        panic!(
                            "proxy returned before reaching approval: {:?}",
                            (&mut proxy).await.unwrap().err()
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("CMA creation must reach an approval request");
            assert_eq!(
                f.post_count().await,
                0,
                "no provider effect before approval"
            );
            assert_eq!(pending.service_id, f.catalog.id);
            approval_service::process_decision(
                &f.db,
                &state.config,
                &state.http_client,
                None,
                None,
                &pending.id,
                true,
                None,
                None,
                "web",
            )
            .await
            .unwrap();
            let response = proxy.await.unwrap().unwrap();
            assert_eq!(response.status().as_u16(), if is_agent { 202 } else { 201 });
            let _ = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            assert_eq!(f.post_count().await, 1);
            let confirmed = f.saved(&row.id).await;
            assert_eq!(confirmed.submission_state, "confirmed");
            assert_eq!(confirmed.idempotency_key, key);
            assert!(
                confirmed.request_ciphertext.is_none()
                    && confirmed.request_context_version.is_none()
            );
            assert!(f.admit(&f.target, route, key, body).await.is_err());
            assert_eq!(f.post_count().await, 1);
            f.assert_single_intent().await;
            f.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn assistant_cma_cutover_lost_receipt_replays_both_creation_kinds() {
    for contextual in [false, true] {
        for route in ["/api/v1/sandboxes", "/api/v1/agents"] {
            let mut f = Fixture::new().await;
            f.keys.set_contextual_record_writes(contextual);
            f.observations().await;
            let body = br#"{ "name": "lost receipt bytes", "machine_included": false }"#;
            let key = "cutover-lost-receipt-key";
            let is_agent = route.ends_with("/agents");
            Mock::given(method("POST"))
                .and(path(route))
                .respond_with(
                    ResponseTemplate::new(if is_agent { 202 } else { 201 }).set_body_json(
                        if is_agent {
                            serde_json::json!({"id":"op_1","status":"ready","agent_id":"agent_1"})
                        } else {
                            serde_json::json!({"id":"sandbox_1"})
                        },
                    ),
                )
                .expect(2)
                .mount(&f.server)
                .await;
            let permit = f.admit(&f.target, route, key, body).await.unwrap();
            let row = permit.creation.as_ref().unwrap();
            let request = f.request(route, key, body);
            permit.before_send(&request).await.unwrap();
            drop(reqwest::Client::new().execute(request).await.unwrap());
            // Retained read-only reconciliation cannot guess the missing receipt or clear bytes.
            let before = bson::to_document(&f.saved(&row.id).await).unwrap();
            recover_owner(&f.db, &f.gateway(), &f.owner).await.unwrap();
            assert_eq!(bson::to_document(&f.saved(&row.id).await).unwrap(), before);
            f.keys.set_contextual_record_writes(!contextual);
            let result = recover_creation(
                &f.db,
                &f.keys,
                &f.gateway(),
                &f.owner,
                &row.sandbox_ref_id,
                row.sandbox_generation,
            )
            .await
            .unwrap();
            assert!(!result.pending);
            let again = recover_creation(
                &f.db,
                &f.keys,
                &f.gateway(),
                &f.owner,
                &row.sandbox_ref_id,
                row.sandbox_generation,
            )
            .await
            .unwrap();
            assert!(!again.pending);
            assert_eq!(f.post_count().await, 2);
            for request in f
                .server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method == "POST")
            {
                assert_eq!(request.body, body);
                assert_eq!(request.headers["idempotency-key"], key);
            }
            assert!(f.saved(&row.id).await.request_ciphertext.is_none());
            f.assert_single_intent().await;
            f.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn assistant_cma_cutover_cross_format_and_old_reader_fail_without_mutation() {
    for contextual in [false, true] {
        let mut f = Fixture::new().await;
        f.keys.set_contextual_record_writes(contextual);
        let body = br#"{"name":"retained after read failure","machine_included":false}"#;
        let permit = f
            .admit(&f.target, "/api/v1/sandboxes", "cross-format-key", body)
            .await
            .unwrap();
        let row = permit.creation.as_ref().unwrap();
        permit
            .before_send(&f.request("/api/v1/sandboxes", "cross-format-key", body))
            .await
            .unwrap();
        let collection = f
            .db
            .collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME);
        let parent = collection
            .find_one(doc! {"_id":&row.sandbox_ref_id})
            .await
            .unwrap()
            .unwrap();
        let creation = bson::to_document(&f.saved(&row.id).await).unwrap();
        let decoded: AssistantCmaSandbox = bson::from_document(parent.clone()).unwrap();
        if !contextual {
            // Old reader refuses the absent marker before decoding. Its raw decoder also refuses.
            assert_ne!(decoded.request_context_version, Some(3));
            let context = RecordContext::new(
                crate::models::assistant_cma_sandbox::COLLECTION_NAME,
                &decoded.id,
                &f.owner,
                &["request_ciphertext"],
            )
            .unwrap();
            assert!(
                f.keys
                    .decrypt_record(decoded.request_ciphertext.as_deref().unwrap(), &context)
                    .await
                    .is_err()
            );
            f.keys.set_allow_legacy_record_reads(false);
            assert!(
                sandboxes::recover_creation_request(
                    &f.db,
                    &f.keys,
                    &f.owner,
                    &decoded.id,
                    decoded.generation
                )
                .await
                .is_err()
            );
            f.keys.set_allow_legacy_record_reads(true);
        }
        for marker in [
            if contextual {
                bson::Bson::Null
            } else {
                bson::Bson::Int32(3)
            },
            bson::Bson::Int32(-1),
            bson::Bson::Int32(259),
        ] {
            collection
                .update_one(
                    doc! {"_id":&decoded.id},
                    doc! {"$set":{"request_context_version":marker}},
                )
                .await
                .unwrap();
            let before = collection
                .find_one(doc! {"_id":&decoded.id})
                .await
                .unwrap()
                .unwrap();
            assert!(
                recover_creation(
                    &f.db,
                    &f.keys,
                    &f.gateway(),
                    &f.owner,
                    &decoded.id,
                    decoded.generation
                )
                .await
                .is_err()
            );
            assert_eq!(
                collection
                    .find_one(doc! {"_id":&decoded.id})
                    .await
                    .unwrap()
                    .unwrap(),
                before
            );
            assert_eq!(
                bson::to_document(&f.saved(&row.id).await).unwrap(),
                creation
            );
            assert_eq!(f.post_count().await, 0);
        }
        collection
            .replace_one(doc! {"_id":&decoded.id}, &parent)
            .await
            .unwrap();
        if !contextual {
            collection
                .update_one(
                    doc! {"_id":&decoded.id},
                    doc! {"$unset":{"request_context_version":""}},
                )
                .await
                .unwrap();
        }
        assert_retained_body(&f, row, body, contextual).await;
        assert_eq!(
            bson::to_document(&f.saved(&row.id).await).unwrap(),
            creation
        );
        f.observations().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/sandboxes"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(serde_json::json!({"id":"sandbox_1"})),
            )
            .expect(1)
            .mount(&f.server)
            .await;
        let recovered = recover_creation(
            &f.db,
            &f.keys,
            &f.gateway(),
            &f.owner,
            &decoded.id,
            decoded.generation,
        )
        .await
        .unwrap();
        assert!(
            !recovered.pending,
            "an upgraded reader can still recover after every refusal"
        );
        assert_eq!(f.post_count().await, 1);
        assert_eq!(f.saved(&row.id).await.submission_state, "confirmed");
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn assistant_cma_cutover_proven_absence_retry_replaces_ciphertext_and_marker_together() {
    for contextual in [false, true] {
        let mut f = Fixture::new().await;
        f.keys.set_contextual_record_writes(contextual);
        let body = br#"{"name":"retry original intent","machine_included":false}"#;
        let key = "cutover-proven-absent-retry";
        let first = f
            .admit(&f.target, "/api/v1/agents", key, body)
            .await
            .unwrap();
        let old = first.creation.as_ref().unwrap();
        let absent = change_creation(&f.db, old, CreationChange::NoResource)
            .await
            .unwrap();
        assert!(absent.request_ciphertext.is_none() && absent.request_context_version.is_none());
        f.keys.set_contextual_record_writes(!contextual);
        let retry = f
            .admit(&f.target, "/api/v1/agents", key, body)
            .await
            .unwrap();
        let current = retry.creation.as_ref().unwrap();
        assert_eq!(current.id, old.id);
        assert_eq!(current.sandbox_ref_id, old.sandbox_ref_id);
        assert_eq!(current.idempotency_key, old.idempotency_key);
        assert!(current.sandbox_generation > old.sandbox_generation);
        assert!(current.submission_generation > old.submission_generation);
        assert_retained_body(&f, current, body, !contextual).await;
        // A retained receipt can reconcile without decrypting or rewriting the body.
        f.observations().await;
        change_creation(&f.db, current, CreationChange::Dispatch)
            .await
            .unwrap();
        change_creation(
            &f.db,
            current,
            CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
        )
        .await
        .unwrap();
        recover_owner(&f.db, &f.gateway(), &f.owner).await.unwrap();
        assert_eq!(f.saved(&old.id).await.submission_state, "confirmed");
        assert!(f.saved(&old.id).await.request_ciphertext.is_none());
        assert_eq!(f.post_count().await, 0);
        f.assert_single_intent().await;
        f.db.drop().await.unwrap();
    }
}
