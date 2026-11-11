use super::*;
use crate::{
    models::{
        credential_verification::CredentialVerificationCheck,
        downstream_service::{DownstreamService, test_helpers::dummy_service},
        service_endpoint::EndpointRisk,
        user::UserType,
        user_service::UserService,
    },
    services::user_api_key_service::{self, CreateApiKeyParams},
    test_utils::*,
};
use futures::TryStreamExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

struct Fixture {
    state: AppState,
    auth: AuthUser,
    id: String,
    server: MockServer,
    catalog: DownstreamService,
}
impl Fixture {
    async fn new() -> Self {
        init_test_audit_chain_hmac_key();
        let db = connect_test_database("credential_verification")
            .await
            .expect("review Mongo required");
        let state = test_app_state(db);
        let owner = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection("users")
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let server = MockServer::start().await;
        let mut catalog = dummy_service();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.slug = "verification-fixture".into();
        catalog.base_url = server.uri();
        catalog.auth_method = "bearer".into();
        catalog.auth_key_name = "Authorization".into();
        catalog.requires_user_credential = true;
        catalog.credential_verification = Some(CredentialVerificationCheck {
            operation: "models_list".into(),
            query: Default::default(),
        });
        state
            .db
            .collection("downstream_services")
            .insert_one(&catalog)
            .await
            .unwrap();
        let mut op = test_service_endpoint(&catalog.id, "models_list", "GET", "/models");
        op.risk = Some(EndpointRisk::Read);
        state
            .db
            .collection("service_endpoints")
            .insert_one(op)
            .await
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let key = user_api_key_service::build_api_key(
            &state.encryption_keys,
            &owner,
            CreateApiKeyParams {
                label: "fixture",
                credential_type: "api_key",
                credential: "fixture-secret-not-for-output",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();
        state
            .db
            .collection("user_api_keys")
            .insert_one(&key)
            .await
            .unwrap();
        let endpoint = test_user_endpoint(
            &endpoint_id,
            &owner,
            "fixture",
            &server.uri(),
            None,
            Some(&catalog.id),
        );
        state
            .db
            .collection("user_endpoints")
            .insert_one(endpoint)
            .await
            .unwrap();
        let mut service = test_user_service(
            &id,
            &owner,
            &catalog.slug,
            &endpoint_id,
            Some(&catalog.id),
            None,
        );
        service.api_key_id = Some(key.id);
        service.auth_method = "bearer".into();
        service.auth_key_name = "Authorization".into();
        state
            .db
            .collection("user_services")
            .insert_one(service)
            .await
            .unwrap();
        Self {
            state,
            auth: test_auth_user(&owner),
            id,
            server,
            catalog,
        }
    }
    async fn check(&self) -> VerificationView {
        check(
            State(self.state.clone()),
            self.auth.clone(),
            Path(self.id.clone()),
        )
        .await
        .unwrap()
        .0
    }
    async fn snapshot(&self) -> verification::Snapshot {
        verification::snapshot(&self.state.db, &self.id)
            .await
            .unwrap()
            .unwrap()
    }
    async fn mock(&self, status: u16) {
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header(
                "authorization",
                "Bearer fixture-secret-not-for-output",
            ))
            .respond_with(ResponseTemplate::new(status).set_body_string("private-provider-body"))
            .expect(1)
            .mount(&self.server)
            .await;
    }
}

#[tokio::test]
async fn provider_statuses_are_evidence_without_changing_lifecycle_or_retaining_body() {
    for (status, expected, reason) in [
        (200, "verified", "provider_accepted"),
        (401, "rejected", "provider_unauthorized"),
        (403, "rejected", "provider_forbidden"),
        (503, "not_verified", "provider_unavailable"),
    ] {
        let f = Fixture::new().await;
        f.mock(status).await;
        let view = f.check().await;
        assert_eq!(view.state, expected, "status {status}: {view:?}");
        assert_eq!(view.reason_code, reason);
        let snapshot = f.snapshot().await;
        assert_eq!(snapshot.key.status, "active");
        assert!(snapshot.service.is_active);
        assert_eq!(snapshot.view().unwrap().state, expected);
        let raw = bson::to_document(&snapshot.service).unwrap().to_string();
        assert!(!raw.contains("private-provider-body"));
        assert!(!raw.contains("fixture-secret"));
        assert!(!format!("{view:?}").contains("fixture-secret"));
        f.server.verify().await;
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn refresh_rechecks_and_overlapping_check_is_rate_limited() {
    let f = Fixture::new().await;
    f.mock(401).await;
    assert_eq!(f.check().await.state, "rejected");
    assert_eq!(f.check().await.reason_code, "check_rate_limited");
    f.server.reset().await;
    f.state.db.collection::<bson::Document>("user_services").update_one(doc!{"_id":&f.id},doc!{"$set":{"credential_verification.started_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(11))}}).await.unwrap();
    f.mock(200).await;
    assert_eq!(f.check().await.state, "verified");
    f.server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn undeclared_unpublished_and_unsafe_operations_never_send() {
    let f = Fixture::new().await;
    for update in [
        doc! {"method":"POST"},
        doc! {"method":"GET","risk":"write"},
        doc! {"risk":"read","publication":"draft"},
        doc! {"publication":"published","path":"//foreign.invalid"},
    ] {
        f.state
            .db
            .collection::<bson::Document>("service_endpoints")
            .update_one(doc! {"service_id":&f.catalog.id}, doc! {"$set":update})
            .await
            .unwrap();
        let view = f.check().await;
        assert_eq!(view.state, "not_verified");
        assert!(!view.can_check);
    }
    f.state
        .db
        .collection::<bson::Document>("downstream_services")
        .update_one(
            doc! {"_id":&f.catalog.id},
            doc! {"$unset":{"credential_verification":""}},
        )
        .await
        .unwrap();
    assert_eq!(f.check().await.reason_code, "check_not_declared");
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn replacement_destination_and_operation_edits_invalidate_evidence() {
    let f = Fixture::new().await;
    f.mock(200).await;
    assert_eq!(f.check().await.state, "verified");
    let baseline = f.snapshot().await;
    let mut changed = f.snapshot().await;
    changed.key.credential_epoch += 1;
    assert_eq!(changed.view().unwrap().state, "not_verified");
    changed = f.snapshot().await;
    changed.endpoint.url.push_str("/new");
    assert_eq!(changed.view().unwrap().state, "not_verified");
    changed = f.snapshot().await;
    changed.operation.as_mut().unwrap().operation_generation += 1;
    assert_eq!(changed.view().unwrap().state, "not_verified");
    changed = f.snapshot().await;
    changed.key.last_used_at = Some(Utc::now());
    assert_eq!(
        changed.binding(),
        baseline.binding(),
        "last-used is not proof or an epoch"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn late_success_cannot_verify_replaced_credential() {
    let f = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(150)))
        .mount(&f.server)
        .await;
    let snap = f.snapshot().await;
    let key_id = snap.key.id.clone();
    let work = run(&f.state, &f.auth, snap);
    let replacement = async {
        for _ in 0..50 {
            if !f.server.received_requests().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        f.state
            .db
            .collection::<bson::Document>("user_api_keys")
            .update_one(
                doc! {"_id":key_id},
                vec![doc! {"$set":{"credential_epoch":{"$add":["$credential_epoch",1]}}}],
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(work, replacement);
    assert_eq!(result.unwrap().reason_code, "binding_changed");
    assert_eq!(f.snapshot().await.view().unwrap().state, "not_verified");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn redirects_and_oversized_bodies_never_verify_or_follow() {
    for oversized in [false, true] {
        let f = Fixture::new().await;
        let response = if oversized {
            ResponseTemplate::new(200)
                .set_body_bytes(vec![b'x'; verification::MAX_RESPONSE_BYTES + 1])
        } else {
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/redirected", f.server.uri()))
        };
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(response)
            .mount(&f.server)
            .await;
        let view = f.check().await;
        assert_eq!(view.state, "not_verified");
        assert_eq!(
            view.reason_code,
            if oversized {
                "response_too_large"
            } else {
                "redirect_refused"
            }
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn timeout_keeps_saved_connection_and_reports_not_verified() {
    let f = Fixture::new().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(12)))
        .mount(&f.server)
        .await;
    let view = after_save(&f.state, &f.auth, &f.id).await.unwrap();
    assert_eq!(view.state, "not_verified");
    assert_eq!(view.reason_code, "check_timeout");
    assert!(f.snapshot().await.service.is_active);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn exclusions_legacy_rows_and_ordinary_requests_do_not_create_evidence() {
    let f = Fixture::new().await;
    let mut s = f.snapshot().await;
    assert_eq!(s.view().unwrap().state, "not_verified");
    s.key.last_used_at = Some(Utc::now());
    assert_eq!(s.view().unwrap().state, "not_verified");
    for kind in ["oauth2", "none", "node_managed"] {
        s.key.credential_type = kind.into();
        assert!(s.view().is_none());
    }
    s.key.credential_type = "api_key".into();
    s.service.node_id = Some("node".into());
    assert!(s.view().is_none());
    s.service.node_id = None;
    s.service.credential_binding = Some("platform".into());
    assert!(s.view().is_none());
    let raw = bson::to_document(&f.snapshot().await.service).unwrap();
    assert!(!raw.contains_key("credential_verification"));
    let restored: UserService = bson::from_document(raw).unwrap();
    assert!(restored.credential_verification.is_none());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_admin_exact_connection_and_nonowner_authority() {
    use crate::models::org_membership::OrgRole;
    let f = Fixture::new().await;
    let org = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection("users")
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let owner = f.auth.user_id.to_string();
    let s = f.snapshot().await;
    for (collection, id) in [
        ("user_services", &f.id),
        ("user_api_keys", &s.key.id),
        ("user_endpoints", &s.endpoint.id),
    ] {
        f.state
            .db
            .collection::<bson::Document>(collection)
            .update_one(doc! {"_id":id}, doc! {"$set":{"user_id":&org}})
            .await
            .unwrap();
    }
    assert!(
        check(State(f.state.clone()), f.auth.clone(), Path(f.id.clone()))
            .await
            .is_err()
    );
    let membership = test_membership(&org, &owner, OrgRole::Admin, None);
    f.state
        .db
        .collection("org_memberships")
        .insert_one(&membership)
        .await
        .unwrap();
    f.mock(200).await;
    assert_eq!(f.check().await.state, "verified");
    f.state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(doc! {"_id":&membership.id}, doc! {"$set":{"role":"member"}})
        .await
        .unwrap();
    assert!(
        check(State(f.state.clone()), f.auth.clone(), Path(f.id.clone()))
            .await
            .is_err()
    );
    let mut external = f.auth.clone();
    external.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    assert!(
        check(State(f.state.clone()), external, Path(f.id.clone()))
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn trusted_defaults_only_and_admin_clear_is_preserved() {
    let f = Fixture::new().await;
    for slug in [
        "llm-anthropic",
        "llm-openai",
        "llm-deepseek",
        "llm-mistral",
        "llm-google-ai",
    ] {
        let mut catalog = f.catalog.clone();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.slug = slug.into();
        catalog.credential_verification = None;
        f.state
            .db
            .collection("downstream_services")
            .insert_one(&catalog)
            .await
            .unwrap();
        verification::seed_declaration(&f.state.db, &catalog)
            .await
            .unwrap();
        let seeded = f
            .state
            .db
            .collection::<DownstreamService>("downstream_services")
            .find_one(doc! {"_id":&catalog.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            seeded.credential_verification.unwrap().operation,
            "models_list"
        );
        f.state.db.collection::<bson::Document>("downstream_services").update_one(doc!{"_id":&catalog.id},doc!{"$set":{"credential_verification":bson::Bson::Null,"credential_verification_admin_modified":true}}).await.unwrap();
        verification::seed_declaration(&f.state.db, &catalog)
            .await
            .unwrap();
        assert!(
            f.state
                .db
                .collection::<DownstreamService>("downstream_services")
                .find_one(doc! {"_id":&catalog.id})
                .await
                .unwrap()
                .unwrap()
                .credential_verification
                .is_none()
        );
    }
    let mut custom = f.catalog.clone();
    custom.id = uuid::Uuid::new_v4().to_string();
    custom.credential_verification = None;
    custom.openapi_spec_url = Some(format!("{}/untrusted-spec", f.server.uri()));
    f.state
        .db
        .collection("downstream_services")
        .insert_one(&custom)
        .await
        .unwrap();
    verification::seed_declaration(&f.state.db, &custom)
        .await
        .unwrap();
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        f.state
            .db
            .collection::<DownstreamService>("downstream_services")
            .find_one(doc! {"_id":&custom.id})
            .await
            .unwrap()
            .unwrap()
            .credential_verification
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn successful_ordinary_request_is_not_verification_and_unavailable_save_is_successful() {
    let f = Fixture::new().await;
    f.mock(200).await;
    let request = Request::builder()
        .uri(format!(
            "/api/v1/proxy/{}/models?_nyxid_via={}",
            f.catalog.id, f.id
        ))
        .extension(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        )
        .body(Body::empty())
        .unwrap();
    let result = super::super::proxy::proxy_request(
        State(f.state.clone()),
        f.auth.clone(),
        TelemetryContext::default(),
        Path((f.catalog.id.clone(), "models".into())),
        request,
    )
    .await
    .unwrap();
    assert!(result.status().is_success());
    assert_eq!(f.snapshot().await.view().unwrap().state, "not_verified");
    assert!(f.snapshot().await.service.credential_verification.is_none());
    f.server.reset().await;
    f.mock(503).await;
    let view = after_save(&f.state, &f.auth, &f.id).await.unwrap();
    assert_eq!(view.reason_code, "provider_unavailable");
    assert_eq!(f.snapshot().await.key.status, "active");
    assert!(f.snapshot().await.service.is_active);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn key_create_returns_committed_connection_when_provider_is_unavailable() {
    let f = Fixture::new().await;
    f.mock(503).await;
    let body = serde_json::from_value(serde_json::json!({
        "service_slug": f.catalog.slug, "label": "Saved despite provider outage",
        "credential": "fixture-secret-not-for-output",
    }))
    .unwrap();
    let response = crate::handlers::keys::create_key(
        State(f.state.clone()),
        f.auth.clone(),
        TelemetryContext::default(),
        Json(body),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(response.status, "active");
    assert!(response.is_active);
    let evidence = response.credential_verification.unwrap();
    assert_eq!(evidence.state, "not_verified");
    assert_eq!(evidence.reason_code, "provider_unavailable");
    let saved = verification::snapshot(&f.state.db, &response.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.key.status, "active");
    assert!(saved.service.is_active);
    f.server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn local_approval_policy_does_not_create_provider_rejection_or_an_approval() {
    let f = Fixture::new().await;
    set_test_approval_preference(&f.state.db, &f.auth.user_id.to_string(), true).await;
    let result = f.check().await;
    assert_eq!(result.state, "not_verified");
    assert_eq!(result.reason_code, "check_unavailable");
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn query_auth_uses_only_the_exact_saved_credential() {
    let f = Fixture::new().await;
    for (collection, id) in [
        ("downstream_services", &f.catalog.id),
        ("user_services", &f.id),
    ] {
        f.state
            .db
            .collection::<bson::Document>(collection)
            .update_one(
                doc! {"_id":id},
                doc! {"$set":{"auth_method":"query","auth_key_name":"key"}},
            )
            .await
            .unwrap();
    }
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(wiremock::matchers::query_param(
            "key",
            "fixture-secret-not-for-output",
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&f.server)
        .await;
    assert_eq!(f.check().await.state, "verified");
    let stored = bson::to_document(&f.snapshot().await.service).unwrap();
    assert!(!stored.to_string().contains("fixture-secret"));
    f.server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_mcp_save_keeps_compatibility_and_reports_unverified_without_guessing_a_connection()
{
    let f = Fixture::new().await;
    let result = crate::services::mcp_service::connect_service(
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.node_ws_manager,
        &f.auth.user_id.to_string(),
        &f.catalog.id,
        Some("legacy-fixture-secret"),
        None,
        "https://nyxid.example",
        None,
        &[],
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "connected");
    assert_eq!(result["credential_verification"]["state"], "not_verified");
    assert_eq!(
        result["credential_verification"]["reason_code"],
        "legacy_connection_check_unavailable"
    );
    assert_eq!(result["credential_verification"]["can_check"], false);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(!result.to_string().contains("legacy-fixture-secret"));
    f.state.db.drop().await.unwrap();
}

#[test]
fn declaration_accepts_only_fixed_bounded_nonsecret_arguments() {
    let good = CredentialVerificationCheck {
        operation: "models_list".into(),
        query: [("limit".into(), "10".into())].into(),
    };
    assert!(verification::validate_declaration(&good).is_ok());
    for value in ["secret", "https://foreign.invalid", "1001", "-1"] {
        let bad = CredentialVerificationCheck {
            query: [("limit".into(), value.into())].into(),
            ..good.clone()
        };
        assert!(verification::validate_declaration(&bad).is_err());
    }
    assert!(
        verification::validate_declaration(&CredentialVerificationCheck {
            operation: "../models".into(),
            ..good
        })
        .is_err()
    );
}

#[tokio::test]
async fn ambiguous_operation_is_unavailable_in_inventory_and_checks() {
    let f = Fixture::new().await;
    let snapshot = f.snapshot().await;
    let operation = snapshot.operation.as_ref().unwrap().clone();
    let mut duplicate = operation.clone();
    duplicate.id = uuid::Uuid::new_v4().to_string();
    assert!(
        verification::select_operation(
            &snapshot.service,
            &f.catalog,
            &[operation, duplicate.clone()],
        )
        .is_none()
    );
    f.state
        .db
        .collection("service_endpoints")
        .insert_one(duplicate)
        .await
        .unwrap();
    let result = f.check().await;
    assert_eq!(result.reason_code, "check_unavailable");
    assert!(!result.can_check);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn verification_audit_is_only_identifiers_and_stable_outcome_metadata() {
    let f = Fixture::new().await;
    f.mock(401).await;
    assert_eq!(f.check().await.state, "rejected");
    let audit = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Some(row) = f
                .state
                .db
                .collection::<crate::models::audit_log::AuditLog>("audit_log")
                .find_one(doc! {"event_type":"credential_verification_checked"})
                .await
                .unwrap()
            {
                break row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        audit.event_data.unwrap(),
        serde_json::json!({"user_service_id":f.id,"state":"rejected","reason_code":"provider_unauthorized"})
    );
    let rows: Vec<bson::Document> = f
        .state
        .db
        .collection("audit_log")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let dump = format!("{rows:?}");
    assert!(!dump.contains("fixture-secret"));
    assert!(!dump.contains("private-provider-body"));
    f.state.db.drop().await.unwrap();
}
