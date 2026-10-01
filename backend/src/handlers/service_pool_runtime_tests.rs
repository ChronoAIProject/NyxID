use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn pool_proxy_configuration_change_during_body_read_stops_before_dispatch() {
    for replace in [false, true] {
        let fixture = fixture(
            "pool_ingress_config_drift",
            StatusCode::OK,
            "priority",
            false,
        )
        .await;
        let db = fixture.state.db.clone();
        let pool_id = fixture.pool_id.clone();
        let mutation_body = async_stream::stream! {
            let pools = db.collection::<Document>("service_pools");
            if replace {
                let mut replacement = pools.find_one(doc! {"_id":&pool_id}).await.unwrap().unwrap();
                replacement.insert("_id", Uuid::new_v4().to_string());
                pools.delete_one(doc! {"_id":&pool_id}).await.unwrap();
                pools.insert_one(replacement).await.unwrap();
            } else {
                pools.update_one(doc! {"_id":&pool_id}, doc! {
                    "$inc":{"config_revision":1_i64},
                    "$set":{"failover.retry_ambiguous_dispatch":true},
                }).await.unwrap();
            }
            yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"{}"));
        };
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/proxy/s/review-route/perform")
            .header("content-type", "application/json")
            .body(Body::from_stream(mutation_body))
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
        let result = super::super::proxy::proxy_request_by_slug(
            State(fixture.state.clone()),
            fixture.auth.clone(),
            crate::telemetry::TelemetryContext::default(),
            Path(("review-route".into(), "perform".into())),
            request,
        )
        .await;
        assert!(
            matches!(result, Err(crate::errors::AppError::Conflict(_))),
            "ingress policy must remain bound to the same pool ID and revision during body read (replace={replace})"
        );
        assert!(fixture.first.requests.lock().await.is_empty());
        assert!(fixture.second.requests.lock().await.is_empty());
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_proxy_transport_exhaustion_explains_each_attempt_without_destination_details() {
    use futures::TryStreamExt;
    let fixture = fixture(
        "pool_transport_exhaustion",
        StatusCode::OK,
        "priority",
        false,
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_many(doc! {}, doc! { "$set": { "url": &unavailable } })
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": {
                "failover.retry_on": ["connect_error"],
            } },
        )
        .await
        .unwrap();

    let response = match try_call(&fixture, "{}").await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"], "service_pool_no_viable_member");
    let attempts = body["details"]["attempts"]
        .as_array()
        .expect("transport exhaustion must explain the attempted failures");
    assert_eq!(attempts.len(), 2);
    for (index, attempt) in attempts.iter().enumerate() {
        assert_eq!(attempt["attempt"], index + 1);
        assert_eq!(attempt["priority"], index);
        assert_eq!(attempt["reason"], "connect_error");
    }
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!text.contains(&unavailable));
    assert!(!text.contains("127.0.0.1"));
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let events: Vec<crate::models::audit_log::AuditLog> = fixture
                .state
                .db
                .collection("audit_log")
                .find(doc! { "event_type": "service_pool_attempt" })
                .sort(doc! { "event_data.attempt": 1 })
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            if events.len() >= 2 {
                break events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("transport failures must produce an audit entry for each attempt");
    assert_eq!(events.len(), 2);
    for (index, event) in events.iter().enumerate() {
        let data = event.event_data.as_ref().unwrap();
        assert_eq!(data["pool_id"], fixture.pool_id);
        assert_eq!(data["attempt"], index + 1);
        assert!(
            data["user_service_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
        assert_eq!(data["reason"], "connect_error");
        assert!(data["upstream_status"].is_null());
        assert!(!data.to_string().contains("127.0.0.1"));
        assert!(event.seq.is_some() && event.entry_hash.is_some());
    }
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_skips_an_ineligible_primary_org_pool_for_an_authorized_org() {
    use crate::models::org_membership::OrgRole;

    let mut fixture = fixture("pool_org_cascade", StatusCode::OK, "priority", false).await;
    bind_first_credential(&fixture).await;
    let first_org = fixture.auth.user_id.to_string();
    use_org_member(&mut fixture, OrgRole::Member, Some(Vec::new())).await;
    let actor = fixture.auth.user_id.to_string();
    let second_org = Uuid::new_v4().to_string();
    let db = &fixture.state.db;
    db.collection::<crate::models::user::User>("users")
        .insert_one(test_user(&second_org, UserType::Org))
        .await
        .unwrap();
    db.collection::<crate::models::org_membership::OrgMembership>("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &second_org,
            &actor,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    db.collection::<Document>("users")
        .update_one(
            doc! { "_id": &actor },
            doc! { "$set": { "primary_org_id": &first_org } },
        )
        .await
        .unwrap();
    let second = db
        .collection::<Document>("user_services")
        .find_one(doc! { "_id": &fixture.second_member_id })
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! { "_id": &fixture.second_member_id },
            doc! { "$set": { "user_id": &second_org } },
        )
        .await
        .unwrap();
    db.collection::<Document>("user_endpoints")
        .update_one(
            doc! { "_id": second.get_str("endpoint_id").unwrap() },
            doc! { "$set": { "user_id": &second_org } },
        )
        .await
        .unwrap();
    let mut backup_pool = db
        .collection::<Document>("service_pools")
        .find_one(doc! { "_id": &fixture.pool_id })
        .await
        .unwrap()
        .unwrap();
    backup_pool.insert("_id", Uuid::new_v4().to_string());
    backup_pool.insert("user_id", &second_org);
    backup_pool.insert("members", vec![doc! {
        "user_service_id": &fixture.second_member_id, "enabled": true, "weight": 1, "priority": 0,
    }]);
    db.collection::<Document>("service_pools")
        .insert_one(backup_pool)
        .await
        .unwrap();
    db.collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$pull": {
                "members": { "user_service_id": &fixture.second_member_id }
            } },
        )
        .await
        .unwrap();
    let before = fixture.state.encryption_keys.decrypt_stats();

    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
    to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(fixture.first.requests.lock().await.is_empty());
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    assert_eq!(fixture.state.encryption_keys.decrypt_stats(), before);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_async_location_stays_on_the_member_that_created_the_job() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    let fixture = fixture("pool_async_headers", StatusCode::OK, "priority", false).await;
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(202)
                .insert_header("location", "/tasks/result?view=full")
                .insert_header("operation-location", "/operations/123")
                .insert_header("retry-after", "2")
                .set_body_json(serde_json::json!({ "status": "pending" })),
        )
        .mount(&upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/result"))
        .respond_with(ResponseTemplate::new(200).set_body_string("job-from-first"))
        .mount(&upstream)
        .await;
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "label": "review-first" },
            doc! { "$set": { "url": upstream.uri() } },
        )
        .await
        .unwrap();
    let member = fixture
        .state
        .db
        .collection::<Document>("user_services")
        .find_one(doc! { "slug": "review-first" })
        .await
        .unwrap()
        .unwrap();
    let member_id = member.get_str("_id").unwrap();

    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let location = response
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        location,
        format!("/api/v1/proxy/s/review-first/tasks/result?view=full&_nyxid_via={member_id}")
    );
    assert_eq!(
        response
            .headers()
            .get("operation-location")
            .unwrap()
            .to_str()
            .unwrap(),
        format!("/api/v1/proxy/s/review-first/operations/123?_nyxid_via={member_id}")
    );
    assert_eq!(response.headers()["retry-after"], "2");
    to_bytes(response.into_body(), 4096).await.unwrap();

    // Pool membership can change while the provider's asynchronous job runs.
    // The returned location must retain its exact creator, with ordinary ACLs.
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": {
                "members.0.enabled": false,
            } },
        )
        .await
        .unwrap();
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(&location)
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
    let response = super::super::proxy::proxy_request_by_slug(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        crate::telemetry::TelemetryContext::default(),
        Path(("review-first".into(), "tasks/result".into())),
        request,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap(),
        "job-from-first"
    );
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].url.query(), Some("view=full"));
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_known_429_with_hung_first_body_keeps_rejection_evidence() {
    let mut fixture = fixture("pool_hung_429_body", StatusCode::OK, "priority", false).await;
    let _release =
        replace_first_with_stalled_response(&mut fixture, StallAt::RejectionFirstBody).await;
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": {
                "failover.per_attempt_timeout_ms": 1000,
                "failover.overall_deadline_ms": 5000,
                "failover.retry_on": ["http_429"],
            } },
        )
        .await
        .unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(6), call(&fixture, "{}"))
        .await
        .expect("known rejection must retain a bounded fallback path");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
    to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    let health = fixture
        .state
        .db
        .collection::<Document>(crate::models::service_pool_member_health::COLLECTION_NAME)
        .find_one(doc! { "pool_id": &fixture.pool_id, "last_status": 429 })
        .await
        .unwrap()
        .expect("rejection records member health");
    assert!(
        health.get_datetime("cooldown_until").unwrap().to_chrono()
            > chrono::Utc::now() + chrono::Duration::seconds(110),
        "a stalled error body must not discard the received Retry-After header",
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_destination_change_during_approval_is_rejected_before_decryption() {
    let mut fixture = fixture("pool_approval_drift", StatusCode::OK, "priority", true).await;
    bind_first_credential(&fixture).await;
    fixture.auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    fixture.auth.scope = crate::mw::auth::PROXY_SCOPE.into();
    fixture.auth.api_key_id = Some(Uuid::new_v4().to_string());
    let catalog = fixture
        .state
        .db
        .collection::<Document>("downstream_services")
        .find_one(doc! { "slug": "pool-review-api" })
        .await
        .unwrap()
        .unwrap();
    fixture.state.db.collection::<Document>("service_approval_configs").insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "user_id": fixture.auth.user_id.to_string(),
        "service_id": catalog.get_str("_id").unwrap(), "service_name": "Approval drift review",
        "approval_required": true, "approval_mode": "per_request", "rules": [],
        "created_at": mongodb::bson::DateTime::now(), "updated_at": mongodb::bson::DateTime::now(),
    }).await.unwrap();
    let before = fixture.state.encryption_keys.decrypt_stats();
    let change_during_approval = async {
        let approval = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(approval) = fixture
                    .state
                    .db
                    .collection::<Document>("approval_requests")
                    .find_one(doc! { "status": "pending" })
                    .await
                    .unwrap()
                {
                    break approval;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("selected member reaches the approval boundary");
        assert_eq!(fixture.state.encryption_keys.decrypt_stats(), before);
        fixture
            .state
            .db
            .collection::<Document>("user_endpoints")
            .update_one(
                doc! { "label": "review-first" },
                doc! { "$set": { "url": &fixture.second.url } },
            )
            .await
            .unwrap();
        fixture
            .state
            .db
            .collection::<Document>("approval_requests")
            .update_one(
                doc! { "_id": approval.get_str("_id").unwrap(), "status": "pending" },
                doc! { "$set": { "status": "approved" } },
            )
            .await
            .unwrap();
    };
    let (response, ()) = tokio::join!(try_call(&fixture, "{}"), change_during_approval);
    let response = match response {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(fixture.state.encryption_keys.decrypt_stats(), before);
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_priority_without_explicit_policy_uses_failover_defaults() {
    let fixture = fixture(
        "pool_priority_default_policy",
        StatusCode::TOO_MANY_REQUESTS,
        "priority",
        false,
    )
    .await;
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$unset": { "failover": "" } },
        )
        .await
        .unwrap();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
    to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_agent_limit_counts_ingress_once_across_fallback() {
    let mut fixture = fixture(
        "pool_agent_ingress_limit",
        StatusCode::TOO_MANY_REQUESTS,
        "priority",
        false,
    )
    .await;
    fixture.auth.api_key_id = Some(Uuid::new_v4().to_string());
    // The local limiter seam permits a non-refilling bucket; the MongoDB
    // limiter deliberately rejects a zero refill rate before admission.
    fixture.state.per_agent_limiter = Arc::new(crate::mw::rate_limit::PerAgentRateLimiter::new());
    // One admission without refill makes the assertion independent of test speed.
    fixture.auth.rate_limit_per_second = Some(0);
    fixture.auth.rate_limit_burst = Some(1);
    let first = call(&fixture, "{}").await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["x-nyxid-pool-attempts"], "2");
    to_bytes(first.into_body(), 4096).await.unwrap();
    let second = match try_call(&fixture, "{}").await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_redirect_connection_failure_cannot_be_replayed_as_unsent() {
    let fixture = fixture("pool_redirect_replay", StatusCode::OK, "priority", false).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_address = listener.local_addr().unwrap();
    drop(listener);
    let redirect = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(307).insert_header(
            "location",
            format!("http://{closed_address}/already-dispatched"),
        ))
        .mount(&redirect)
        .await;
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "label": "review-first" },
            doc! { "$set": { "url": redirect.uri() } },
        )
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": { "failover.retry_on": ["connect_error"] } },
        )
        .await
        .unwrap();
    let response = match try_call(&fixture, "{}").await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert!(!response.status().is_success());
    assert_eq!(redirect.received_requests().await.unwrap().len(), 1);
    assert!(
        fixture.second.requests.lock().await.is_empty(),
        "a connection failure after a redirected POST is not proof of an unsent request"
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_forbidden_operation_never_decrypts_a_member_credential() {
    let fixture = fixture(
        "pool_operation_decrypt_fence",
        StatusCode::OK,
        "priority",
        false,
    )
    .await;
    bind_first_credential(&fixture).await;
    fixture.state.db.collection::<Document>("downstream_services").update_one(
        doc! { "slug": "pool-review-api" },
        doc! { "$set": { "proxy_operation_policy": { "rules": [{ "method": "GET", "path_template": "/status" }] } } },
    ).await.unwrap();
    let before = fixture.state.encryption_keys.decrypt_stats();
    let response = match try_call(&fixture, "{}").await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert!(!response.status().is_success());
    assert_eq!(
        fixture.state.encryption_keys.decrypt_stats(),
        before,
        "native operation authorization must precede credential materialization"
    );
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_excluded_node_member_is_skipped_before_credential_materialization() {
    let mut fixture = fixture("pool_node_decrypt_fence", StatusCode::OK, "priority", false).await;
    bind_first_credential(&fixture).await;
    fixture.auth.allow_all_nodes = false;
    fixture.auth.allowed_node_ids.clear();
    fixture
        .state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "slug": "review-first" },
            doc! { "$set": { "node_id": Uuid::new_v4().to_string() } },
        )
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": { "failover.max_attempts": 1 } },
        )
        .await
        .unwrap();
    let before = fixture.state.encryption_keys.decrypt_stats();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
    to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        fixture.state.encryption_keys.decrypt_stats(),
        before,
        "an excluded node must be filtered before materializing its server credential"
    );
    assert!(fixture.first.requests.lock().await.is_empty());
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_backup_tier_rotates_only_when_visited() {
    let fixture = fixture("pool_tier_visits", StatusCode::OK, "priority", false).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let intermittent = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(move |_: &wiremock::Request| {
            let index = counted.fetch_add(1, Ordering::SeqCst);
            wiremock::ResponseTemplate::new(if index.is_multiple_of(2) { 200 } else { 429 })
                .set_body_string("intermittent primary")
        })
        .mount(&intermittent)
        .await;
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "label": "review-first" },
            doc! { "$set": { "url": intermittent.uri() } },
        )
        .await
        .unwrap();
    let third = upstream(StatusCode::OK, "third").await;
    let endpoint_id = Uuid::new_v4().to_string();
    let third_id = Uuid::new_v4().to_string();
    let owner = fixture.auth.user_id.to_string();
    let catalog = fixture
        .state
        .db
        .collection::<Document>("downstream_services")
        .find_one(doc! { "slug": "pool-review-api" })
        .await
        .unwrap()
        .unwrap();
    let catalog_id = catalog.get_str("_id").unwrap();
    fixture
        .state
        .db
        .collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(test_user_endpoint(
            &endpoint_id,
            &owner,
            "review-third",
            &third.url,
            None,
            Some(catalog_id),
        ))
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(test_user_service(
            &third_id,
            &owner,
            "review-third",
            &endpoint_id,
            Some(catalog_id),
            None,
        ))
        .await
        .unwrap();
    fixture.state.db.collection::<Document>("service_pools").update_one(
        doc! { "_id": &fixture.pool_id }, doc! {
            "$push": { "members": { "user_service_id": third_id, "enabled": true, "weight": 1, "priority": 1 } },
            "$set": { "failover.cooldown.failures_to_open": 10, "tier_balance": "round_robin" },
        }
    ).await.unwrap();
    for _ in 0..4 {
        let response = call(&fixture, "{}").await;
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 4096).await.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(
        fixture.second.requests.lock().await.len(),
        1,
        "backup B gets one of the two actual visits"
    );
    assert_eq!(
        third.requests.lock().await.len(),
        1,
        "backup C gets the other visit"
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_caller_disconnect_cancels_first_body_without_backup() {
    use crate::downstream_disconnect::{DisconnectAwareListener, DisconnectAwareMakeService};
    let mut fixture = fixture("pool_caller_cancel", StatusCode::OK, "priority", true).await;
    let mut release = replace_first_with_stalled_response(&mut fixture, StallAt::FirstBody).await;
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": {
                "failover.per_attempt_timeout_ms": 5_000, "failover.overall_deadline_ms": 10_000,
                "failover.retry_on": ["timeout", "transport_error"],
            } },
        )
        .await
        .unwrap();
    let state = fixture.state.clone();
    let auth = fixture.auth.clone();
    let app = Router::new().route(
        "/api/v1/proxy/s/review-route/perform",
        any(move |mut request: Request<Body>| {
            let state = state.clone();
            let auth = auth.clone();
            async move {
                request
                    .extensions_mut()
                    .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
                super::super::proxy::proxy_request_by_slug(
                    State(state),
                    auth,
                    crate::telemetry::TelemetryContext::default(),
                    Path(("review-route".into(), "perform".into())),
                    request,
                )
                .await
                .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/api/v1/proxy/s/review-route/perform",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(
            DisconnectAwareListener::new(listener),
            DisconnectAwareMakeService::new(app),
        )
        .await
        .unwrap();
    });
    let client =
        tokio::spawn(async move { reqwest::Client::new().post(url).body("{}").send().await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while fixture.first.requests.lock().await.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("primary dispatched before cancelling caller");
    client.abort();
    let _ = client.await;
    tokio::time::timeout(std::time::Duration::from_secs(2), release.closed()).await
        .expect("caller disconnect must cancel actual upstream body before the five-second attempt timeout");
    assert!(fixture.second.requests.lock().await.is_empty());
    server.abort();
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_health_403_requires_explicit_trigger_on_repeated_requests() {
    for opt_in in [false, true] {
        let fixture = fixture(
            "pool_health_403_policy",
            StatusCode::FORBIDDEN,
            "priority",
            false,
        )
        .await;
        fixture.state.db.collection::<Document>("service_pools").update_one(doc! {"_id":&fixture.pool_id},
            doc! {"$set":{"failover.retry_on": if opt_in {vec!["http_403"]} else {vec!["http_429"]}}}).await.unwrap();
        for _ in 0..2 {
            let response = call(&fixture, "{}").await;
            assert_eq!(
                response.status(),
                if opt_in {
                    StatusCode::OK
                } else {
                    StatusCode::FORBIDDEN
                }
            );
            to_bytes(response.into_body(), 4096).await.unwrap();
        }
        assert_eq!(
            fixture.first.requests.lock().await.len(),
            if opt_in { 1 } else { 2 }
        );
        assert_eq!(
            fixture.second.requests.lock().await.len(),
            if opt_in { 2 } else { 0 }
        );
        assert_eq!(
            fixture
                .state
                .db
                .collection::<Document>("service_pool_member_health")
                .count_documents(doc! {"cooldown_until":{"$gt":mongodb::bson::DateTime::now()}})
                .await
                .unwrap(),
            u64::from(opt_in)
        );
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_hung_403_body_keeps_configured_health_and_retry_policy() {
    for opt_in in [false, true] {
        let mut fixture = fixture("pool_hung_403", StatusCode::OK, "priority", true).await;
        let _release =
            replace_first_with_stalled_response(&mut fixture, StallAt::ForbiddenFirstBody).await;
        fixture.state.db.collection::<Document>("service_pools").update_one(
            doc! {"_id":&fixture.pool_id}, doc! {"$set":{
                "failover.per_attempt_timeout_ms":1000, "failover.overall_deadline_ms":5000,
                "failover.retry_on":if opt_in {vec!["timeout","http_403"]} else {vec!["timeout"]}
            }}).await.unwrap();
        let response = match try_call(&fixture, "{}").await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        assert_eq!(
            response.status(),
            if opt_in {
                StatusCode::OK
            } else {
                StatusCode::GATEWAY_TIMEOUT
            }
        );
        to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(fixture.first.requests.lock().await.len(), 1);
        assert_eq!(
            fixture.second.requests.lock().await.len(),
            usize::from(opt_in)
        );
        assert_eq!(
            fixture
                .state
                .db
                .collection::<Document>("service_pool_member_health")
                .count_documents(doc! {"cooldown_until":{"$gt":mongodb::bson::DateTime::now()}})
                .await
                .unwrap(),
            u64::from(opt_in)
        );
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_529_default_post_cools_without_replay_and_opt_in_retries() {
    for opt_in in [false, true] {
        let fixture = fixture(
            "pool_529_safety",
            StatusCode::from_u16(529).unwrap(),
            "priority",
            opt_in,
        )
        .await;
        fixture
            .state
            .db
            .collection::<Document>("service_pools")
            .update_one(
                doc! {"_id":&fixture.pool_id},
                doc! {"$unset":{"failover.retry_on":""}},
            )
            .await
            .unwrap();
        let response = call(&fixture, "{}").await;
        assert_eq!(response.status().as_u16(), if opt_in { 200 } else { 529 });
        to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            fixture.second.requests.lock().await.len(),
            usize::from(opt_in)
        );
        assert_eq!(
            fixture
                .state
                .db
                .collection::<Document>("service_pool_member_health")
                .count_documents(
                    doc! {"last_status":529,"cooldown_until":{"$gt":mongodb::bson::DateTime::now()}}
                )
                .await
                .unwrap(),
            1
        );
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_encoding_identity_is_final_and_compressed_rejection_still_retries() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method},
    };
    for status in [200, 429] {
        let fixture = fixture("pool_encoding", StatusCode::OK, "priority", false).await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("accept-encoding", "identity"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("content-encoding", "gzip")
                    .set_body_bytes(vec![
                        31, 139, 8, 0, 0, 0, 0, 0, 0, 3, 171, 174, 5, 0, 67, 191, 166, 163, 2, 0,
                        0, 0,
                    ]),
            )
            .expect(1)
            .mount(&server)
            .await;
        fixture
            .state
            .db
            .collection::<Document>("user_endpoints")
            .update_one(
                doc! {"label":"review-first"},
                doc! {"$set":{"url":server.uri()}},
            )
            .await
            .unwrap();
        fixture.state.db.collection::<Document>("downstream_services").update_one(doc! {"slug":"pool-review-api"},doc! {"$set":{"default_request_headers":[{"name":"accept-encoding","value":"gzip","overridable":false,"sensitive":false}]}}).await.unwrap();
        let response = try_call(&fixture, "{}").await;
        if status == 200 {
            assert!(matches!(
                response,
                Err(crate::errors::AppError::ServicePoolUpstreamEncodingUnsupported)
            ));
            assert!(fixture.second.requests.lock().await.is_empty());
        } else {
            let response = response.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
            to_bytes(response.into_body(), 4096).await.unwrap();
        }
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_committed_caller_drop_does_not_cool_but_upstream_failure_does() {
    for upstream_failure in [false, true] {
        let fixture = fixture("pool_committed_cause", StatusCode::OK, "priority", true).await;
        fixture
            .state
            .db
            .collection::<Document>("service_pools")
            .update_one(
                doc! {"_id":&fixture.pool_id},
                doc! {"$set":{"failover.retry_on":["transport_error"]}},
            )
            .await
            .unwrap();
        let (release, pending) = tokio::sync::oneshot::channel::<()>();
        let pending = Arc::new(Mutex::new(Some(pending)));
        let app = Router::new().route(
            "/perform",
            any(move || {
                let pending = pending.clone();
                async move {
                    let pending = pending.lock().await.take().unwrap();
                    let stream = async_stream::stream! {
                        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"partial-output"));
                        let _ = pending.await;
                        yield Err(std::io::Error::other("upstream failed"));
                    };
                    Response::new(Body::from_stream(stream))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        fixture
            .state
            .db
            .collection::<Document>("user_endpoints")
            .update_one(doc! {"label":"review-first"}, doc! {"$set":{"url":url}})
            .await
            .unwrap();
        let response = call(&fixture, "{}").await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.into_body().into_data_stream();
        assert_eq!(stream.next().await.unwrap().unwrap(), "partial-output");
        let reason = if upstream_failure {
            release.send(()).unwrap();
            assert!(stream.next().await.unwrap().is_err());
            "transport_error"
        } else {
            drop(stream);
            let mut release = release;
            tokio::time::timeout(std::time::Duration::from_secs(3), release.closed())
                .await
                .expect("actual upstream cancelled");
            "caller_cancelled"
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if fixture
                    .state
                    .db
                    .collection::<Document>("audit_log")
                    .count_documents(
                        doc! {"event_type":"service_pool_attempt","event_data.reason":reason},
                    )
                    .await
                    .unwrap()
                    == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            fixture
                .state
                .db
                .collection::<Document>("service_pool_member_health")
                .count_documents(doc! {"cooldown_until":{"$gt":mongodb::bson::DateTime::now()}})
                .await
                .unwrap(),
            u64::from(upstream_failure)
        );
        assert!(fixture.second.requests.lock().await.is_empty());
        server.abort();
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_proxy_denied_earlier_org_service_cannot_hide_later_priority_pool() {
    use crate::models::org_membership::OrgRole;

    for denied in ["viewer", "admin_only", "scope"] {
        let mut fixture = fixture("pool_org_cascade", StatusCode::OK, "priority", false).await;
        bind_first_credential(&fixture).await;
        let first_org = fixture.auth.user_id.to_string();
        use_org_member(
            &mut fixture,
            if denied == "viewer" {
                OrgRole::Viewer
            } else {
                OrgRole::Member
            },
            if denied == "scope" {
                Some(Vec::new())
            } else {
                None
            },
        )
        .await;
        fixture
            .state
            .db
            .collection::<Document>("user_services")
            .update_one(
                doc! {"slug":"review-first"},
                doc! {"$set":{"slug":"review-route","admin_only":denied == "admin_only"}},
            )
            .await
            .unwrap();
        let actor = fixture.auth.user_id.to_string();
        let second_org = Uuid::new_v4().to_string();
        let db = &fixture.state.db;
        db.collection::<crate::models::user::User>("users")
            .insert_one(test_user(&second_org, UserType::Org))
            .await
            .unwrap();
        db.collection::<crate::models::org_membership::OrgMembership>("org_memberships")
            .insert_one(crate::test_utils::test_membership(
                &second_org,
                &actor,
                OrgRole::Member,
                None,
            ))
            .await
            .unwrap();
        db.collection::<Document>("users")
            .update_one(
                doc! { "_id": &actor },
                doc! { "$set": { "primary_org_id": &first_org } },
            )
            .await
            .unwrap();
        let second = db
            .collection::<Document>("user_services")
            .find_one(doc! { "_id": &fixture.second_member_id })
            .await
            .unwrap()
            .unwrap();
        db.collection::<Document>("user_services")
            .update_one(
                doc! { "_id": &fixture.second_member_id },
                doc! { "$set": { "user_id": &second_org } },
            )
            .await
            .unwrap();
        db.collection::<Document>("user_endpoints")
            .update_one(
                doc! { "_id": second.get_str("endpoint_id").unwrap() },
                doc! { "$set": { "user_id": &second_org } },
            )
            .await
            .unwrap();
        let mut backup_pool = db
            .collection::<Document>("service_pools")
            .find_one(doc! { "_id": &fixture.pool_id })
            .await
            .unwrap()
            .unwrap();
        backup_pool.insert("_id", Uuid::new_v4().to_string());
        backup_pool.insert("user_id", &second_org);
        backup_pool.insert("members", vec![doc! {
        "user_service_id": &fixture.second_member_id, "enabled": true, "weight": 1, "priority": 0,
    }]);
        db.collection::<Document>("service_pools")
            .insert_one(backup_pool)
            .await
            .unwrap();
        db.collection::<Document>("service_pools")
            .update_one(
                doc! { "_id": &fixture.pool_id },
                doc! { "$pull": {
                    "members": { "user_service_id": &fixture.second_member_id }
                } },
            )
            .await
            .unwrap();
        let before = fixture.state.encryption_keys.decrypt_stats();

        let response = call(&fixture, "{}").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
        assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
        to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(fixture.first.requests.lock().await.is_empty());
        assert_eq!(fixture.second.requests.lock().await.len(), 1);
        assert_eq!(fixture.state.encryption_keys.decrypt_stats(), before);
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_error_body_failure_returns_sanitized_attributed_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = fixture("pool_error_headers", StatusCode::OK, "priority", false).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut byte = [0; 1];
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 999\r\nContent-Encoding: gzip\r\nRetry-After: 120\r\nConnection: close\r\n\r\n").await.unwrap();
    });
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(doc! {"label":"review-first"}, doc! {"$set":{"url":url}})
        .await
        .unwrap();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.headers()["retry-after"], "120");
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
    assert_eq!(response.headers()["x-nyxid-pool-member"], "review-first");
    assert!(!response.headers().contains_key("content-length"));
    assert!(!response.headers().contains_key("content-encoding"));
    assert!(
        to_bytes(response.into_body(), 4096)
            .await
            .unwrap()
            .is_empty()
    );
    server.await.unwrap();
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_all_cooled_uses_standard_error_and_retry_after() {
    let fixture = fixture(
        "pool_all_cooled_error",
        StatusCode::TOO_MANY_REQUESTS,
        "priority",
        false,
    )
    .await;
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_many(doc! {}, doc! {"$set":{"url":&fixture.first.url}})
        .await
        .unwrap();
    let first = call(&fixture, "{}").await;
    assert_eq!(first.status(), StatusCode::TOO_MANY_REQUESTS);
    to_bytes(first.into_body(), 4096).await.unwrap();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        response.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            >= 119
    );
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["error"], "service_pool_cooling_down");
    assert!(body["error_code"].as_u64().unwrap() > 0);
    assert_eq!(fixture.first.requests.lock().await.len(), 2);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_live_node_capability_loss_is_unsent_and_can_use_backup() {
    let fixture = fixture(
        "pool_node_capability_loss",
        StatusCode::OK,
        "priority",
        false,
    )
    .await;
    bind_first_credential(&fixture).await;
    let db = &fixture.state.db;
    let node = Uuid::new_v4().to_string();
    let now = mongodb::bson::DateTime::now();
    db.collection::<Document>("nodes").insert_one(doc! {"_id":&node,"user_id":fixture.auth.user_id.to_string(),"name":"Node","status":"online","is_active":true,"auth_token_hash":"test","created_at":now,"updated_at":now,"connection_owner":{"instance_name":&fixture.state.replica_identity.instance_name,"generation_id":&fixture.state.replica_identity.generation_id,"connection_id":"socket","internal_base_url":"http://127.0.0.1","claimed_at":now,"renewed_at":now,"expires_at":mongodb::bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(5)),"http_cancellation":true}}).await.unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"slug":"review-first"},
            doc! {"$set":{"node_id":&node}},
        )
        .await
        .unwrap();
    db.collection::<Document>("service_pools")
        .update_one(
            doc! {"_id":&fixture.pool_id},
            doc! {"$set":{"failover.retry_on":["node_offline"]}},
        )
        .await
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    fixture
        .state
        .node_ws_manager
        .register_connection_with_id(&node, "socket".into(), tx);
    // Durable planning metadata was capable; exact live session has lost support.
    let before = fixture.state.encryption_keys.decrypt_stats();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
    assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
    to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(rx.try_recv().is_err());
    assert!(fixture.first.requests.lock().await.is_empty());
    assert_eq!(fixture.state.encryption_keys.decrypt_stats(), before);
    db.drop().await.unwrap();
}
