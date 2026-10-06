use crate::{
    AppState,
    models::user::UserType,
    services::{key_service, service_preference_service as preferences},
    test_utils::*,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use bson::{Document, doc};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn request(
    router: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"body": String::from_utf8_lossy(&bytes)})),
    )
}
fn access(state: &AppState, owner: &str) -> String {
    crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(owner).unwrap(),
        "openid proxy account:read",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap()
}
async fn fixture(name: &str) -> (AppState, Router, String, String, Vec<String>) {
    assert!(
        std::env::var("NYXID_TEST_DATABASE_URL").is_ok(),
        "explicit test database required"
    );
    let db = connect_transaction_test_database(name).await;
    let state = test_app_state(db.clone());
    crate::services::audit_service::init_audit_chain_hmac_key(zeroize::Zeroizing::new([2u8; 32]));
    let owner = Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for slug in ["preference-order", "second", "third"] {
        ids.push(
            crate::services::assistant_authority_tests::connected(
                &db,
                &owner,
                slug,
                "https://example.com",
            )
            .await,
        );
    }
    let token = access(&state, &owner);
    let (_, private) = crate::routes::build_router();
    let router = private.with_state(state.clone());
    (state, router, owner, token, ids)
}
const PATH: &str = "/api/v1/service-preferences";

#[tokio::test]
async fn service_preference_http_cas_noop_legacy_and_chain() {
    let (state, router, owner, token, ids) = fixture("preference_cas").await;
    let (status, absent) = request(&router, &token, "GET", PATH, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        absent,
        json!({"ordered":[], "version":0, "updated_at":null})
    );
    let body = json!({"ordered":[ids[0]], "expected_version":0});
    let (a, b) = tokio::join!(
        request(&router, &token, "PUT", PATH, Some(body.clone())),
        request(&router, &token, "PUT", PATH, Some(body))
    );
    assert_eq!(
        [a.0, b.0]
            .iter()
            .filter(|&&status| status == StatusCode::OK)
            .count(),
        1,
        "{a:?} {b:?}"
    );
    assert_eq!(
        [a.0, b.0]
            .iter()
            .filter(|&&status| status == StatusCode::CONFLICT)
            .count(),
        1
    );
    let first = request(&router, &token, "GET", PATH, None).await.1;
    assert_eq!(first["version"], 1);
    assert_eq!(if a.0 == StatusCode::OK { &a.1 } else { &b.1 }, &first);
    let noop = request(
        &router,
        &token,
        "PUT",
        PATH,
        Some(json!({"ordered":[ids[0]], "expected_version":1})),
    )
    .await;
    assert_eq!(noop, (StatusCode::OK, first.clone()));
    let (a, b) = tokio::join!(
        request(
            &router,
            &token,
            "PUT",
            PATH,
            Some(json!({"ordered":[ids[1]], "expected_version":1}))
        ),
        request(
            &router,
            &token,
            "PUT",
            PATH,
            Some(json!({"ordered":[ids[2]], "expected_version":1}))
        )
    );
    assert_eq!(
        [a.0, b.0]
            .iter()
            .filter(|&&status| status == StatusCode::OK)
            .count(),
        1
    );
    let loser = if a.0 == StatusCode::CONFLICT { a } else { b };
    assert_eq!(loser.0, StatusCode::CONFLICT);
    assert_eq!(loser.1["error_code"], 1004);
    let audits = state
        .db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"service_preference_updated"})
            .await
            .unwrap(),
        2
    );
    let audit = audits
        .find_one(doc! {"event_type":"service_preference_updated"})
        .await
        .unwrap()
        .unwrap();
    assert!(audit.get("seq").is_some());
    assert!(audit.get_str("entry_hash").is_ok());
    assert!(audit.get_str("prev_hash").is_ok());
    let event = audit.get_document("event_data").unwrap();
    assert_eq!(event.len(), 2);
    assert!(event.contains_key("count") && event.contains_key("version"));
    let verified = crate::services::audit_chain_service::verify_chain(
        &state.db,
        crate::services::audit_service::audit_chain_hmac_key().unwrap(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(verified.break_info.is_none());
    let stored = state.db.collection::<Document>("service_preferences");
    stored.delete_one(doc! {"_id": &owner}).await.unwrap();
    let now = bson::DateTime::now();
    stored
        .insert_one(doc! {"_id": &owner, "created_at":now, "updated_at":now})
        .await
        .unwrap();
    let result = request(
        &router,
        &token,
        "PUT",
        PATH,
        Some(json!({"ordered":[ids[0]], "expected_version":0})),
    )
    .await;
    assert_eq!(result.0, StatusCode::OK, "{result:?}");
    assert_eq!(result.1["version"], 1);
    assert_eq!(
        stored
            .find_one(doc! {"_id": &owner})
            .await
            .unwrap()
            .unwrap()
            .get_datetime("created_at")
            .unwrap(),
        &now
    );
}

#[tokio::test]
async fn service_preference_http_scopes_stale_slug_config_and_validation() {
    let (state, router, owner, token, ids) = fixture("preference_scope").await;
    assert_eq!(
        request(
            &router,
            &token,
            "GET",
            "/api/v1/keys/preference-order",
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let config = request(&router, &token, "GET", "/api/v1/mcp/config", None).await;
    assert_eq!(config.0, StatusCode::OK, "{config:?}");
    let saved = request(
        &router,
        &token,
        "PUT",
        PATH,
        Some(json!({"ordered":ids, "expected_version":0})),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{saved:?}");
    assert_eq!(
        request(&router, &token, "GET", "/api/v1/mcp/config", None).await,
        config
    );
    let key = key_service::create_api_key_with_scope_authorization(
        &state.db,
        &owner,
        Some(&owner),
        "scoped",
        "read",
        None,
        None,
        Some(std::slice::from_ref(&ids[2])),
        Some(&[]),
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let scoped = request(&router, &key.full_key, "GET", PATH, None).await;
    assert_eq!(scoped.0, StatusCode::OK, "{scoped:?}");
    assert_eq!(scoped.1["ordered"], json!([ids[2]]));
    assert_eq!(scoped.1["version"], 1);
    let rows = request(&router, &key.full_key, "GET", "/api/v1/keys", None)
        .await
        .1;
    assert_eq!(rows["keys"].as_array().unwrap().len(), 1);
    assert_eq!(rows["keys"][0]["preference_rank"], 1);
    let detail = request(
        &router,
        &key.full_key,
        "GET",
        &format!("/api/v1/keys/{}", ids[2]),
        None,
    )
    .await;
    assert_eq!(detail.1["preference_rank"], 1, "{detail:?}");
    assert_eq!(
        request(
            &router,
            &key.full_key,
            "PUT",
            PATH,
            Some(json!({"ordered":[], "expected_version":1}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let oauth = crate::crypto::jwt::generate_oauth_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&owner).unwrap(),
        "account:read proxy",
        None,
        None,
        None,
        None,
        None,
        "app",
    )
    .unwrap();
    assert_eq!(
        request(&router, &oauth, "GET", PATH, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &router,
            &oauth,
            "PUT",
            PATH,
            Some(json!({"ordered":[], "expected_version":1}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let delegated = crate::crypto::jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&owner).unwrap(),
        "account:read",
        "client",
        600,
        None,
    )
    .unwrap();
    assert_eq!(
        request(&router, &delegated, "GET", PATH, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &router,
            &delegated,
            "PUT",
            PATH,
            Some(json!({"ordered":[], "expected_version":1}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (sa, secret) = crate::services::service_account_service::create_service_account(
        &state.db,
        "preference-reader",
        None,
        "user-services:read",
        &[],
        None,
        &owner,
    )
    .await
    .unwrap();
    let sa_token = crate::services::service_account_service::authenticate_client_credentials(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &sa.client_id,
        &secret,
        None,
    )
    .await
    .unwrap();
    let relay = crate::crypto::jwt::generate_relay_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&owner).unwrap(),
        "read",
        None,
        &crate::crypto::jwt::RelayAgentScope {
            api_key_id: key.id.clone(),
            api_key_name: "scoped".into(),
            allowed_service_ids: vec![ids[2].clone()],
            allowed_node_ids: vec![],
            allow_all_services: false,
            allow_all_nodes: false,
        },
    )
    .unwrap();
    for rejected in [&sa_token.access_token, &relay] {
        assert_eq!(
            request(&router, rejected, "GET", PATH, None).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &router,
                rejected,
                "PUT",
                PATH,
                Some(json!({"ordered":[],"expected_version":1}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    for body in [
        json!({"ordered":(0..201).map(|_| Uuid::new_v4().to_string()).collect::<Vec<_>>(), "expected_version":1}),
        json!({"ordered":[ids[0].to_uppercase()], "expected_version":1}),
        json!({"ordered":["aaaaaaaa-aaaa-4aaa-1aaa-aaaaaaaaaaaa"], "expected_version":1}),
        json!({"ordered":[Uuid::nil().to_string()], "expected_version":1}),
        json!({"ordered":[ids[0],ids[0]], "expected_version":1}),
        json!({"ordered":[], "expected_version":-1}),
        json!({"ordered":[], "expected_version":preferences::MAX_EXPECTED_VERSION+1}),
        json!({"ordered":[], "expected_version":1,"unexpected":true}),
    ] {
        assert_eq!(
            request(&router, &token, "PUT", PATH, Some(body)).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        request(
            &router,
            &token,
            "PUT",
            PATH,
            Some(json!({"ordered":["x".repeat(17_000)], "expected_version":1}))
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let stored = state
        .db
        .collection::<Document>("service_preferences")
        .find_one(doc! {"_id": &owner})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        request(
            &router,
            &token,
            "DELETE",
            &format!("/api/v1/keys/{}", ids[0]),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let stale = request(&router, &token, "GET", PATH, None).await.1;
    assert_eq!(stale["ordered"], json!([ids[1], ids[2]]));
    assert_eq!(
        state
            .db
            .collection::<Document>("service_preferences")
            .find_one(doc! {"_id": &owner})
            .await
            .unwrap()
            .unwrap(),
        stored
    );
    let remaining = request(
        &router,
        &token,
        "GET",
        &format!("/api/v1/keys/{}", ids[2]),
        None,
    )
    .await
    .1;
    assert_eq!(remaining["preference_rank"], 2);
}

#[tokio::test]
async fn service_preference_live_org_visibility_revocation_keeps_storage_unchanged() {
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole};
    let (state, router, owner, token, ids) = fixture("preference_org_visibility").await;
    let org = Uuid::new_v4().to_string();
    state
        .db
        .collection("users")
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let shared = crate::services::assistant_authority_tests::connected(
        &state.db,
        &org,
        "shared",
        "https://example.com",
    )
    .await;
    let membership = test_membership(&org, &owner, OrgRole::Viewer, None);
    state
        .db
        .collection(MEMBERSHIPS)
        .insert_one(&membership)
        .await
        .unwrap();
    let saved = request(
        &router,
        &token,
        "PUT",
        PATH,
        Some(json!({"ordered":[shared, ids[0]], "expected_version":0})),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{saved:?}");
    let rows = request(&router, &token, "GET", "/api/v1/keys", None)
        .await
        .1;
    let org_row = rows["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == shared)
        .unwrap();
    assert_eq!(org_row["preference_rank"], 1);
    assert_eq!(org_row["credential_source"]["allowed"], false);
    let stored = state
        .db
        .collection::<Document>("service_preferences")
        .find_one(doc! {"_id": &owner})
        .await
        .unwrap()
        .unwrap();
    state
        .db
        .collection::<Document>(MEMBERSHIPS)
        .update_one(
            doc! {"_id": &membership.id},
            doc! {"$set":{"revoked_at": bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let resolved = request(&router, &token, "GET", PATH, None).await;
    assert_eq!(resolved.0, StatusCode::OK);
    assert_eq!(resolved.1["ordered"], json!([ids[0]]));
    assert_eq!(resolved.1["version"], 1);
    let detail = request(
        &router,
        &token,
        "GET",
        &format!("/api/v1/keys/{}", ids[0]),
        None,
    )
    .await;
    assert_eq!(detail.1["preference_rank"], 1, "{detail:?}");
    assert_eq!(
        state
            .db
            .collection::<Document>("service_preferences")
            .find_one(doc! {"_id": &owner})
            .await
            .unwrap()
            .unwrap(),
        stored
    );
}

#[tokio::test]
async fn service_preference_command_monitoring_bounds_detail_and_single_listing_read() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let commands = Arc::new(Mutex::new(Vec::<(String, Document)>::new()));
    let recorded = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            recorded
                .lock()
                .unwrap()
                .push((event.command_name, event.command));
        }
    });
    assert!(std::env::var("NYXID_TEST_DATABASE_URL").is_ok());
    let db = connect_transaction_test_database_with_command_handler("preference_commands", handler)
        .await;
    let state = test_app_state(db.clone());
    let owner = Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let id = crate::services::assistant_authority_tests::connected(
        &db,
        &owner,
        "ranked",
        "https://example.com",
    )
    .await;
    let token = access(&state, &owner);
    let (_, private) = crate::routes::build_router();
    let router = private.with_state(state);
    commands.lock().unwrap().clear();
    assert_eq!(
        request(&router, &token, "GET", &format!("/api/v1/keys/{id}"), None)
            .await
            .0,
        StatusCode::OK
    );
    let baseline = commands.lock().unwrap().clone();
    assert_eq!(
        baseline
            .iter()
            .filter(|(_, command)| command.get_str("find") == Ok("service_preferences"))
            .count(),
        1
    );
    assert!(!baseline.iter().any(|(_, command)| {
        command.get_str("find") == Ok("user_services")
            && command
                .get_document("filter")
                .is_ok_and(|filter| filter.contains_key("user_id") && !filter.contains_key("_id"))
    }));
    preferences::replace(
        &db,
        &owner,
        std::slice::from_ref(&id),
        0,
        &[id.clone()].into_iter().collect(),
    )
    .await
    .unwrap();
    commands.lock().unwrap().clear();
    assert_eq!(
        request(&router, &token, "GET", &format!("/api/v1/keys/{id}"), None)
            .await
            .1["preference_rank"],
        1
    );
    let ranked = commands.lock().unwrap().clone();
    let selected = ranked
        .iter()
        .find(|(_, command)| {
            command.get_str("find") == Ok("user_services")
                && command
                    .get_document("filter")
                    .is_ok_and(|filter| filter.get_document("_id").is_ok())
        })
        .expect("ranked ID query");
    assert_eq!(
        selected
            .1
            .get_document("filter")
            .unwrap()
            .get_document("_id")
            .unwrap()
            .get_array("$in")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        ranked
            .iter()
            .filter(|(_, command)| command.get_str("find") == Ok("user_api_keys"))
            .count(),
        baseline
            .iter()
            .filter(|(_, command)| command.get_str("find") == Ok("user_api_keys"))
            .count()
    );
    assert_eq!(
        ranked
            .iter()
            .filter(|(_, command)| command.get_str("find") == Ok("provider_configs"))
            .count(),
        baseline
            .iter()
            .filter(|(_, command)| command.get_str("find") == Ok("provider_configs"))
            .count(),
        "rank resolution does not reload providers"
    );
    let mut saved = vec![id.clone()];
    saved.extend((1..preferences::MAX_ORDERED_SERVICES).map(|_| Uuid::new_v4().to_string()));
    db.collection::<Document>("service_preferences")
        .update_one(doc! {"_id": &owner}, doc! {"$set":{"ordered": &saved}})
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    assert_eq!(
        request(&router, &token, "GET", &format!("/api/v1/keys/{id}"), None)
            .await
            .1["preference_rank"],
        1
    );
    let bounded = commands.lock().unwrap().clone();
    let selected = bounded
        .iter()
        .find(|(_, command)| {
            command.get_str("find") == Ok("user_services")
                && command
                    .get_document("filter")
                    .is_ok_and(|filter| filter.get_document("_id").is_ok())
        })
        .expect("selected IDs visibility query at the saved-list limit");
    assert_eq!(
        selected
            .1
            .get_document("filter")
            .unwrap()
            .get_document("_id")
            .unwrap()
            .get_array("$in")
            .unwrap()
            .len(),
        preferences::MAX_ORDERED_SERVICES
    );
    assert!(bounded.iter().any(|(_, command)| {
        command.get_str("find") == Ok("user_endpoints")
            && command.get_document("projection") == Ok(&doc! {"_id":1})
    }));
    commands.lock().unwrap().clear();
    assert_eq!(
        request(&router, &token, "GET", "/api/v1/keys", None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        commands
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, command)| command.get_str("find") == Ok("service_preferences"))
            .count(),
        1
    );
}
