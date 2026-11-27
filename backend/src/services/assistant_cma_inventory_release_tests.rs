use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT};
use crate::services::{audit_chain_service, audit_service, role_service};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::Engine;
use tower::ServiceExt;

const SANDBOXES: &str = "/api/v1/sandboxes";

async fn admin_token(f: &Fixture) -> (String, String) {
    let id = uuid::Uuid::new_v4();
    role_service::seed_system_roles(&f.db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&f.db).await.unwrap();
    let mut admin = crate::test_utils::test_user(&id.to_string(), UserType::Person);
    admin.role_ids.push(roles.admin);
    f.db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(admin)
        .await
        .unwrap();
    let token = crate::crypto::jwt::generate_access_token(
        &f.jwt,
        &f.config,
        &id,
        "openid profile",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    (id.to_string(), token)
}

struct ReleaseRoute {
    app: Router,
    uri: String,
    admin_id: String,
    token: String,
    body: Value,
    creation: AssistantCmaCreation,
}

impl ReleaseRoute {
    async fn new(f: &Fixture, path: &str) -> Self {
        let creation = pending(f, path).await;
        let row = parent(f, &creation).await;
        let (admin_id, token) = admin_token(f).await;
        let app = crate::routes::build_router()
            .1
            .with_state(crate::test_utils::test_app_state(f.db.clone()));
        Self {
            app,
            admin_id,
            token,
            uri: format!(
                "/api/v1/admin/cma-sandbox-reservations/{}/release-empty-inventory",
                row.id
            ),
            body: json!({"generation":row.generation,"revision":row.agent_reference_version,
                "creation_revision":creation.revision,"reason":"Service desk case 2995"}),
            creation,
        }
    }

    async fn expire(&self, f: &Fixture) {
        f.db.collection::<Document>(COLLECTION_NAME).update_one(doc!{"_id":&self.creation.id},
            doc!{"$set":{"first_dispatched_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::days(2))}}).await.unwrap();
    }

    async fn send(&self, token: &str, body: &Value) -> (StatusCode, Value) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::post(&self.uri)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let data = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        (status, serde_json::from_slice(&data).unwrap())
    }

    async fn refused(&self, f: &Fixture, reason: &str) {
        let (status, body) = self.send(&self.token, &self.body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body["message"].as_str().unwrap().contains(reason), "{body}");
        assert!(parent(f, &self.creation).await.occupies_account_slot);
        let saved = f.saved(&self.creation.id).await;
        assert_eq!(saved.revision, self.creation.revision);
        assert_eq!(saved.request_ciphertext, self.creation.request_ciphertext);
    }
}

async fn inventory(f: &Fixture, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(SANDBOXES))
        .respond_with(response)
        .mount(&f.server)
        .await;
}

#[tokio::test]
async fn empty_inventory_releases_once_and_admits_the_next_create() {
    let f = Fixture::with_management(false).await;
    let route = ReleaseRoute::new(&f, SANDBOXES).await;
    route.expire(&f).await;
    inventory(
        &f,
        ResponseTemplate::new(200)
            .set_body_json(json!({"items":[]}))
            .set_delay(Duration::from_millis(100)),
    )
    .await;
    let (other_id, other_token) = admin_token(&f).await;
    let (left, right) = tokio::join!(
        route.send(&route.token, &route.body),
        route.send(&other_token, &route.body)
    );
    let (win, lose, winner_id) = if left.0 == StatusCode::OK {
        (&left, &right, &route.admin_id)
    } else {
        (&right, &left, &other_id)
    };
    assert_eq!(win.0, StatusCode::OK, "{win:?}");
    assert_eq!(lose.0, StatusCode::CONFLICT, "{lose:?}");
    assert_eq!(win.1["reservation"]["occupies_account_slot"], false);
    let saved = f.saved(&route.creation.id).await;
    let row = parent(&f, &saved).await;
    assert_eq!(row.admission_state, SandboxAdmissionState::Retired);
    assert_eq!(saved.submission_state, "confirmed_not_created");
    assert_eq!(row.submission_state, saved.submission_state);
    assert!(
        saved.request_ciphertext.is_none()
            && saved.request_context_version.is_none()
            && row.request_ciphertext.is_none()
            && row.request_context_version.is_none()
    );
    let inspection = admin::inspect(&f.db, &f.gateway(), &row.id, &actor(&f))
        .await
        .unwrap();
    assert!(matches!(
        inspection.outcome,
        admin::InspectionOutcome::Unknown {
            reason: "released_after_inventory_check"
        }
    ));
    let audits: Vec<_> =
        f.db.collection::<AuditLog>(AUDIT)
            .find(doc! {"event_type":"admin_cma_sandbox_reservation_resolved"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(audits.len(), 1);
    let audit = &audits[0];
    assert_eq!(audit.user_id.as_ref(), Some(winner_id));
    assert!(audit.seq.is_some() && audit.entry_hash.is_some());
    assert_eq!(
        audit.event_data.as_ref().unwrap(),
        &json!({
            "action":"release_empty_inventory","result":"resolved","reason":"Service desk case 2995",
            "reservation_id":row.id,"owner_id":f.owner,"cma_service_id":f.catalog.id,
            "generation":route.body["generation"],"revision":route.body["revision"],"creation_revision":route.creation.revision,
            "inventory_checked":0,"inventory_live":0,"unknown_live":0
        })
    );
    let chain = audit_chain_service::verify_chain(
        &f.db,
        audit_service::audit_chain_hmac_key().unwrap(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(chain.status, audit_chain_service::AuditChainStatus::Ok);
    assert_gets_only(&f).await;
    let next = f
        .admit(&f.target, SANDBOXES, "new-sandbox", BODY)
        .await
        .unwrap();
    next.before_send(&f.request(SANDBOXES, "new-sandbox", BODY))
        .await
        .unwrap();
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn inventory_requires_complete_pages_and_exact_live_links() {
    let f = Fixture::with_management(false).await;
    let route = ReleaseRoute::new(&f, SANDBOXES).await;
    route.expire(&f).await;
    let owner = f.owner.clone();
    Mock::given(method("GET")).and(path(SANDBOXES)).respond_with(move |req: &wiremock::Request| {
        let assertion = req.headers["x-nyxid-identity-token"].to_str().unwrap();
        let payload = assertion.split('.').nth(1).unwrap();
        let claims: Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).unwrap()
        ).unwrap();
        assert_eq!(claims["sub"], owner);
        let page = match req.url.query_pairs().find(|(k,_)| k=="cursor") {
            None => json!({"items":[{"owner":owner,"id":"old","status":"retired"}],"next":"second"}),
            Some((_,cursor)) if cursor=="second" => json!({"items":[{"owner":owner,"id":"existing","status":"pausing"}],"next":null}),
            _ => return ResponseTemplate::new(400),
        };
        ResponseTemplate::new(200).set_body_json(page)
    }).mount(&f.server).await;
    let mut link = parent(&f, &route.creation).await;
    link.id = uuid::Uuid::new_v4().to_string();
    link.user_id = uuid::Uuid::new_v4().to_string();
    link.source = "discovered".into();
    link.idempotency_key = None;
    link.request_hash = None;
    link.request_ciphertext = None;
    link.request_context_version = None;
    link.cma_sandbox_id = Some("existing".into());
    link.admission_state = SandboxAdmissionState::Associated;
    link.occupies_account_slot = false;
    let links = f.db.collection::<AssistantCmaSandbox>(PARENTS);
    links.insert_one(&link).await.unwrap();
    route.refused(&f, "cma_inventory_unlinked_sandbox").await;
    link.user_id = f.owner.clone();
    link.cma_service_id = "another-service".into();
    links
        .replace_one(doc! {"_id":&link.id}, &link)
        .await
        .unwrap();
    route.refused(&f, "cma_inventory_unlinked_sandbox").await;
    link.cma_service_id = f.catalog.id.clone();
    links
        .replace_one(doc! {"_id":&link.id}, &link)
        .await
        .unwrap();
    let (status, body) = route.send(&route.token, &route.body).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let audit =
        f.db.collection::<AuditLog>(AUDIT)
            .find_one(doc! {"event_type":"admin_cma_sandbox_reservation_resolved"})
            .await
            .unwrap()
            .unwrap()
            .event_data
            .unwrap();
    assert_eq!(audit["inventory_checked"], 2);
    assert_eq!(audit["inventory_live"], 1);
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn inventory_errors_and_unreadable_pages_keep_the_slot() {
    let f = Fixture::with_management(false).await;
    let route = ReleaseRoute::new(&f, SANDBOXES).await;
    route.expire(&f).await;
    for response in [
        ResponseTemplate::new(503),
        ResponseTemplate::new(206).set_body_json(json!({"items":[]})),
        ResponseTemplate::new(200).set_body_string("{"),
        ResponseTemplate::new(200).set_body_json(json!([[], null])),
        ResponseTemplate::new(200).set_body_json(json!({"items":[["x",f.owner,"retired"]]})),
        ResponseTemplate::new(200).set_body_json(json!({"next":null})),
        ResponseTemplate::new(200).set_body_json(json!({"items":[],"next":"repeated"})),
        ResponseTemplate::new(200).set_body_json(json!({"items":[],"has_more":true})),
        ResponseTemplate::new(200)
            .set_body_json(json!({"items":[{"id":"x","owner":"other","status":"retired"}]})),
        ResponseTemplate::new(200).set_body_json(json!({"items":[{"id":"x","owner":f.owner}]})),
        ResponseTemplate::new(200)
            .set_body_json(json!({"items":[]}))
            .set_delay(Duration::from_secs(25)),
    ] {
        f.server.reset().await;
        inventory(&f, response).await;
        route.refused(&f, "cma_inventory_incomplete").await;
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn inventory_release_refuses_receipts_live_windows_and_stale_edits_before_reading_cma() {
    let f = Fixture::with_management(false).await;
    let route = ReleaseRoute::new(&f, SANDBOXES).await;
    route.refused(&f, "cma_replay_window_not_expired").await;
    route.expire(&f).await;
    for field in ["generation", "revision", "creation_revision"] {
        let mut stale = route.body.clone();
        stale[field] = json!(999);
        assert_eq!(
            route.send(&route.token, &stale).await.0,
            StatusCode::CONFLICT
        );
    }
    for reason in [" ".into(), "r".repeat(513)] {
        let mut invalid = route.body.clone();
        invalid["reason"] = json!(reason);
        assert!(route.send(&route.token, &invalid).await.0.is_client_error());
    }
    for (collection, id, field) in [
        (PARENTS, &route.creation.sandbox_ref_id, "operation_id"),
        (PARENTS, &route.creation.sandbox_ref_id, "cma_sandbox_id"),
        (COLLECTION_NAME, &route.creation.id, "operation_id"),
        (COLLECTION_NAME, &route.creation.id, "cma_agent_id"),
        (COLLECTION_NAME, &route.creation.id, "cma_sandbox_id"),
    ] {
        let rows = f.db.collection::<Document>(collection);
        rows.update_one(doc! {"_id":id}, doc! {"$set":{field:"receipt"}})
            .await
            .unwrap();
        route.refused(&f, "cma_inventory_release_ineligible").await;
        rows.update_one(doc! {"_id":id}, doc! {"$unset":{field:""}})
            .await
            .unwrap();
    }
    f.db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&route.creation.id},
            doc! {"$set":{"path":"/api/v1/agents"}},
        )
        .await
        .unwrap();
    route.refused(&f, "cma_inventory_release_ineligible").await;
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.db.drop().await.unwrap();
}
