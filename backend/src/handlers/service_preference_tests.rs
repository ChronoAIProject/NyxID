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
    db.collection::<Document>("user_services")
        .update_many(
            doc! {"user_id":&owner},
            doc! {"$set":{"catalog_service_id":CATALOG}},
        )
        .await
        .unwrap();
    let token = access(&state, &owner);
    let (_, private) = crate::routes::build_router();
    let router = private.with_state(state.clone());
    (state, router, owner, token, ids)
}
const CATALOG: &str = "aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa";
const GROUP: &str = "catalog:aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa";
const GROUP_PATH: &str =
    "/api/v1/service-preferences/groups/catalog:aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa";
const PATH: &str = "/api/v1/service-preferences";

#[tokio::test]
async fn service_preference_http_cas_noop_legacy_and_chain() {
    let (state, router, owner, token, ids) = fixture("preference_cas").await;
    let (status, absent) = request(&router, &token, "GET", PATH, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(absent, json!({"groups":[], "version":0, "updated_at":null}));
    let body = json!({"ordered":[ids[0]], "expected_version":0});
    let (a, b) = tokio::join!(
        request(&router, &token, "PUT", GROUP_PATH, Some(body.clone())),
        request(&router, &token, "PUT", GROUP_PATH, Some(body))
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
        GROUP_PATH,
        Some(json!({"ordered":[ids[0]], "expected_version":1})),
    )
    .await;
    assert_eq!(noop, (StatusCode::OK, first.clone()));
    let (a, b) = tokio::join!(
        request(
            &router,
            &token,
            "PUT",
            GROUP_PATH,
            Some(json!({"ordered":[ids[1]], "expected_version":1}))
        ),
        request(
            &router,
            &token,
            "PUT",
            GROUP_PATH,
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
    assert_eq!(event.len(), 3);
    assert_eq!(event.get_str("group"), Ok(GROUP));
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
        GROUP_PATH,
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
        GROUP_PATH,
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
    assert_eq!(scoped.1["groups"][0]["ordered"], json!([ids[2]]));
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
            GROUP_PATH,
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
            GROUP_PATH,
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
            GROUP_PATH,
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
    for rejected in [
        &key.full_key,
        &oauth,
        &delegated,
        &sa_token.access_token,
        &relay,
    ] {
        assert_eq!(
            request(
                &router,
                rejected,
                "DELETE",
                "/api/v1/service-preferences/hidden",
                Some(json!({"expected_version":1}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
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
                GROUP_PATH,
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
        json!({"ordered":[], "expected_version":1,"clear":true}),
    ] {
        assert_eq!(
            request(&router, &token, "PUT", GROUP_PATH, Some(body))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        request(
            &router,
            &token,
            "PUT",
            GROUP_PATH,
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
    assert_eq!(stale["groups"][0]["ordered"], json!([ids[1], ids[2]]));
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
    state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&shared},
            doc! {"$set":{"catalog_service_id":CATALOG}},
        )
        .await
        .unwrap();
    let membership = test_membership(&org, &owner, OrgRole::Viewer, None);
    state
        .db
        .collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(&membership)
        .await
        .unwrap();
    let saved = request(
        &router,
        &token,
        "PUT",
        GROUP_PATH,
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
    assert_eq!(resolved.1["groups"][0]["ordered"], json!([ids[0]]));
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
    db.collection::<Document>("user_services")
        .update_many(
            doc! {"user_id":&owner},
            doc! {"$set":{"catalog_service_id":CATALOG}},
        )
        .await
        .unwrap();
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
    let assert_no_inventory_reads = |commands: &[(String, Document)]| {
        for collection in [
            "user_services",
            "user_endpoints",
            "user_api_keys",
            "provider_configs",
            "downstream_services",
            "org_memberships",
        ] {
            assert!(
                !commands
                    .iter()
                    .any(|(_, command)| command.get_str("find") == Ok(collection)),
                "empty preference GET must not read {collection}: {commands:?}"
            );
        }
        assert_eq!(
            commands
                .iter()
                .filter(|(_, command)| command.get_str("find") == Ok("service_preferences"))
                .count(),
            1
        );
    };
    commands.lock().unwrap().clear();
    let (status, absent) =
        request(&router, &token, "GET", "/api/v1/service-preferences", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(absent, json!({"groups":[], "version":0, "updated_at":null}));
    assert_no_inventory_reads(&commands.lock().unwrap());
    let now = bson::DateTime::now();
    db.collection::<Document>("service_preferences")
        .insert_one(doc! {
            "_id": &owner, "ordered": [], "version": 3_i64,
            "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let (status, empty) =
        request(&router, &token, "GET", "/api/v1/service-preferences", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty["groups"], json!([]));
    assert_eq!(empty["version"], 3);
    assert_eq!(empty["updated_at"], now.to_chrono().to_rfc3339());
    assert_no_inventory_reads(&commands.lock().unwrap());
    db.collection::<Document>("service_preferences")
        .delete_one(doc! {"_id": &owner})
        .await
        .unwrap();
    preferences::replace_group(&db, &owner, GROUP, std::slice::from_ref(&id), 0)
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
    let (status, saved_get) =
        request(&router, &token, "GET", "/api/v1/service-preferences", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_get["groups"][0]["ordered"], json!([id]));
    assert_eq!(saved_get["version"], 1);
    let saved_reads = commands.lock().unwrap().clone();
    assert_eq!(
        saved_reads
            .iter()
            .filter(|(_, command)| command.get_str("find") == Ok("service_preferences"))
            .count(),
        1
    );
    let selected = saved_reads
        .iter()
        .find(|(_, command)| command.get_str("find") == Ok("user_services"))
        .expect("saved GET resolves bounded connection IDs");
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
    for (_, command) in &saved_reads {
        assert_ne!(command.get_str("find"), Ok("user_api_keys"));
        assert_ne!(command.get_str("find"), Ok("provider_configs"));
        if command.get_str("find") == Ok("user_endpoints") {
            assert_eq!(command.get_document("projection"), Ok(&doc! {"_id":1}));
            assert_eq!(
                command
                    .get_document("filter")
                    .unwrap()
                    .get_document("_id")
                    .unwrap()
                    .get_array("$in")
                    .unwrap()
                    .len(),
                1,
                "only the surviving selected service needs endpoint visibility"
            );
        }
        if command.get_str("find") == Ok("downstream_services") {
            assert_eq!(command.get_document("projection"), Ok(&doc! {"_id":1}));
        }
    }
    assert!(
        saved_reads
            .iter()
            .any(|(_, command)| { command.get_str("find") == Ok("user_endpoints") })
    );
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

#[tokio::test]
async fn service_preference_group_merge_capacity_and_explicit_hidden_release() {
    let (state, router, owner, token, ids) = fixture("preference_group_merge_release").await;
    let db = &state.db;
    let second_person = Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(test_user(&second_person, UserType::Person))
        .await
        .unwrap();
    let other_document = doc! {"_id":&second_person,"ordered":[Uuid::new_v4().to_string()],"version":17_i64,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()};
    db.collection::<Document>("service_preferences")
        .insert_one(&other_document)
        .await
        .unwrap();
    let audits = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
    let custom = crate::services::assistant_authority_tests::connected(
        db,
        &owner,
        "legacy-custom",
        "https://example.com",
    )
    .await;
    let org = Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let lost = crate::services::assistant_authority_tests::connected(
        db,
        &org,
        "lost-org",
        "https://example.com",
    )
    .await;
    db.collection::<Document>("user_services")
        .update_one(doc! {"_id":&ids[1]}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    let mut stored = vec![
        ids[0].clone(),
        Uuid::new_v4().to_string(),
        custom.clone(),
        ids[1].clone(),
        lost.clone(),
    ];
    stored.extend((stored.len()..200).map(|_| Uuid::new_v4().to_string()));
    let now = bson::DateTime::now();
    db.collection::<Document>("service_preferences")
        .insert_one(
            doc! {"_id":&owner,"ordered":&stored,"version":1_i64,"created_at":now,"updated_at":now},
        )
        .await
        .unwrap();
    // An omitted visible, already stored disabled member is appended to the draft.
    let noop = request(
        &router,
        &token,
        "PUT",
        GROUP_PATH,
        Some(json!({"ordered":[ids[0]],"expected_version":1})),
    )
    .await;
    assert_eq!(noop.0, StatusCode::OK, "{noop:?}");
    assert_eq!(noop.1["version"], 1);
    assert_eq!(audits.count_documents(doc! {"event_type":{"$in":["service_preference_updated","service_preference_hidden_released"]}}).await.unwrap(), 0);
    let changed = request(
        &router,
        &token,
        "PUT",
        GROUP_PATH,
        Some(json!({"ordered":[ids[1],ids[0]],"expected_version":1})),
    )
    .await;
    assert_eq!(changed.0, StatusCode::OK, "{changed:?}");
    let after = preferences::get(db, &owner).await.unwrap().unwrap();
    assert_eq!(after.ordered[0], ids[1]);
    assert_eq!(after.ordered[3], ids[0]);
    assert_eq!(
        after
            .ordered
            .iter()
            .filter(|id| !ids[..2].contains(id))
            .collect::<Vec<_>>(),
        stored
            .iter()
            .filter(|id| !ids[..2].contains(id))
            .collect::<Vec<_>>()
    );
    let rows = request(&router, &token, "GET", "/api/v1/keys", None)
        .await
        .1;
    let disabled = rows["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == ids[1])
        .unwrap();
    assert!(disabled["preference_rank"].is_null());
    assert_eq!(disabled["preference_position"], 1);
    db.collection::<Document>("user_services")
        .update_one(doc! {"_id":&ids[1]}, doc! {"$set":{"is_active":true}})
        .await
        .unwrap();
    let restored = request(&router, &token, "GET", "/api/v1/keys", None)
        .await
        .1;
    let restored = restored["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == ids[1])
        .unwrap();
    assert_eq!(restored["preference_rank"], 1);
    assert_eq!(restored["preference_position"], 1);
    assert_eq!(
        preferences::get(db, &owner).await.unwrap().unwrap().ordered,
        after.ordered
    );
    db.collection::<Document>("user_services")
        .update_one(doc! {"_id":&ids[1]}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    let capacity = request(
        &router,
        &token,
        "PUT",
        GROUP_PATH,
        Some(json!({"ordered":[ids[2]],"expected_version":2})),
    )
    .await;
    assert_eq!(capacity.0, StatusCode::BAD_REQUEST, "{capacity:?}");
    assert!(
        capacity.1["message"]
            .as_str()
            .unwrap()
            .contains("200 connections across all services")
    );
    assert!(!capacity.1.to_string().contains(&lost));
    assert_eq!(
        preferences::get(db, &owner).await.unwrap().unwrap().ordered,
        after.ordered
    );
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"service_preference_updated"})
            .await
            .unwrap(),
        1
    );
    let hidden_path = "/api/v1/service-preferences/hidden";
    for body in [
        json!({"expected_version":-1}),
        json!({"expected_version":2,"clear":true}),
        json!({"expected_version":2,"padding":"x".repeat(17000)}),
    ] {
        let result = request(&router, &token, "DELETE", hidden_path, Some(body)).await;
        assert!(
            matches!(
                result.0,
                StatusCode::BAD_REQUEST | StatusCode::PAYLOAD_TOO_LARGE
            ),
            "{result:?}"
        );
    }
    assert_eq!(
        request(
            &router,
            &token,
            "DELETE",
            hidden_path,
            Some(json!({"expected_version":1}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let released = request(
        &router,
        &token,
        "DELETE",
        hidden_path,
        Some(json!({"expected_version":2})),
    )
    .await;
    assert_eq!(released.0, StatusCode::OK, "{released:?}");
    assert_eq!(released.1["version"], 3);
    assert_eq!(released.1.as_object().unwrap().len(), 3);
    let retained = preferences::get(db, &owner).await.unwrap().unwrap();
    assert_eq!(
        retained.ordered,
        vec![ids[1].clone(), custom.clone(), ids[0].clone()]
    );
    let noop = request(
        &router,
        &token,
        "DELETE",
        hidden_path,
        Some(json!({"expected_version":3})),
    )
    .await;
    assert_eq!(noop, released);
    let audit = audits
        .find_one(doc! {"event_type":"service_preference_hidden_released"})
        .await
        .unwrap()
        .unwrap();
    let event = audit.get_document("event_data").unwrap();
    assert_eq!(event.len(), 2);
    assert_eq!(event.get_i64("released_hidden").unwrap(), 197);
    assert_eq!(event.get_i64("version").unwrap(), 3);
    assert!(audit.get("seq").is_some());
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"service_preference_hidden_released"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"service_preference_updated"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>("service_preferences")
            .find_one(doc! {"_id":&second_person})
            .await
            .unwrap()
            .unwrap(),
        other_document
    );
    // Saturated all-hidden storage allows an explicit reset/no-op and release.
    let all_hidden: Vec<_> = std::iter::once(lost.clone())
        .chain((1..200).map(|_| Uuid::new_v4().to_string()))
        .collect();
    db.collection::<Document>("service_preferences")
        .update_one(doc! {"_id":&owner}, doc! {"$set":{"ordered":&all_hidden}})
        .await
        .unwrap();
    let reset = request(
        &router,
        &token,
        "PUT",
        GROUP_PATH,
        Some(json!({"ordered":[],"expected_version":3})),
    )
    .await;
    assert_eq!(reset.0, StatusCode::OK);
    assert_eq!(reset.1["version"], 3);
    let capacity = request(
        &router,
        &token,
        "PUT",
        GROUP_PATH,
        Some(json!({"ordered":[ids[0]],"expected_version":3})),
    )
    .await;
    assert_eq!(capacity.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"service_preference_updated"})
            .await
            .unwrap(),
        1
    );
    let release = request(
        &router,
        &token,
        "DELETE",
        hidden_path,
        Some(json!({"expected_version":3})),
    )
    .await;
    assert_eq!(release.0, StatusCode::OK);
    assert_eq!(release.1["groups"], json!([]));
    let saturation_audit = audits
        .find_one(
            doc! {"event_type":"service_preference_hidden_released","event_data.version":4_i64},
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saturation_audit.get_document("event_data").unwrap(),
        &doc! {"released_hidden":200_i64,"version":4_i64}
    );
    assert_eq!(
        db.collection::<Document>("service_preferences")
            .find_one(doc! {"_id":&second_person})
            .await
            .unwrap()
            .unwrap(),
        other_document
    );
    assert_eq!(
        request(
            &router,
            &token,
            "PUT",
            GROUP_PATH,
            Some(json!({"ordered":[ids[0]],"expected_version":4}))
        )
        .await
        .0,
        StatusCode::OK
    );
    // Unknown/missing/wrong group share the same generic validation message.
    let wrong = Uuid::new_v4().to_string();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&ids[2]},
            doc! {"$set":{"catalog_service_id":wrong}},
        )
        .await
        .unwrap();
    let mut messages = Vec::new();
    for id in [&custom, &lost, &ids[2]] {
        let invalid = request(
            &router,
            &token,
            "PUT",
            GROUP_PATH,
            Some(json!({"ordered":[id],"expected_version":5})),
        )
        .await;
        assert_eq!(invalid.0, StatusCode::BAD_REQUEST);
        messages.push(invalid.1["message"].clone());
    }
    assert!(messages.iter().all(|message| message == &messages[0]));
    for group in [
        "connection:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        "catalog:invalid",
    ] {
        assert_eq!(
            request(
                &router,
                &token,
                "PUT",
                &format!("{PATH}/groups/{group}"),
                Some(json!({"ordered":[],"expected_version":5}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let audit_count = audits.count_documents(doc! {}).await.unwrap();
    let nearly_full: Vec<_> = std::iter::once(lost.clone())
        .chain((1..198).map(|_| Uuid::new_v4().to_string()))
        .collect();
    db.collection::<Document>("service_preferences")
        .update_one(doc! {"_id":&owner}, doc! {"$set":{"ordered":&nearly_full}})
        .await
        .unwrap();
    let before = db
        .collection::<Document>("service_preferences")
        .find_one(doc! {"_id":&owner})
        .await
        .unwrap()
        .unwrap();
    // Restore membership for the third submitted connection; adding three to 198 fails.
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&ids[2]},
            doc! {"$set":{"catalog_service_id":CATALOG}},
        )
        .await
        .unwrap();
    let capacity = request(
        &router,
        &token,
        "PUT",
        GROUP_PATH,
        Some(json!({"ordered":&ids,"expected_version":5})),
    )
    .await;
    assert_eq!(capacity.0, StatusCode::BAD_REQUEST, "{capacity:?}");
    assert!(
        capacity.1["message"]
            .as_str()
            .unwrap()
            .contains("200 connections across all services")
    );
    assert!(!capacity.1.to_string().contains(&lost));
    assert_eq!(
        db.collection::<Document>("service_preferences")
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(audits.count_documents(doc! {}).await.unwrap(), audit_count);
    let at_capacity: Vec<_> = nearly_full
        .iter()
        .cloned()
        .chain(ids[..2].iter().cloned())
        .collect();
    db.collection::<Document>("service_preferences")
        .update_one(doc! {"_id":&owner}, doc! {"$set":{"ordered":&at_capacity}})
        .await
        .unwrap();
    let reset = request(
        &router,
        &token,
        "PUT",
        GROUP_PATH,
        Some(json!({"ordered":[],"expected_version":5})),
    )
    .await;
    assert_eq!(reset.0, StatusCode::OK, "{reset:?}");
    assert_eq!(
        preferences::get(db, &owner).await.unwrap().unwrap().ordered,
        nearly_full
    );
    let verified = crate::services::audit_chain_service::verify_chain(
        db,
        crate::services::audit_service::audit_chain_hmac_key().unwrap(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(verified.break_info.is_none());
}

#[tokio::test]
async fn service_preference_distinct_group_cas_retry_preserves_both_orders() {
    let (state, router, owner, token, ids) = fixture("preference_distinct_cas").await;
    let other = Uuid::new_v4().to_string();
    let other_path = format!("{PATH}/groups/catalog:{other}");
    state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&ids[1]},
            doc! {"$set":{"catalog_service_id":other}},
        )
        .await
        .unwrap();
    let body_a = json!({"ordered":[ids[0]],"expected_version":0});
    let body_b = json!({"ordered":[ids[1]],"expected_version":0});
    let (a, b) = tokio::join!(
        request(&router, &token, "PUT", GROUP_PATH, Some(body_a.clone())),
        request(&router, &token, "PUT", &other_path, Some(body_b.clone()))
    );
    assert_eq!(
        [a.0, b.0]
            .into_iter()
            .filter(|s| *s == StatusCode::OK)
            .count(),
        1,
        "{a:?} {b:?}"
    );
    assert_eq!(
        [a.0, b.0]
            .into_iter()
            .filter(|s| *s == StatusCode::CONFLICT)
            .count(),
        1
    );
    let (path, mut body) = if a.0 == StatusCode::CONFLICT {
        (GROUP_PATH, body_a)
    } else {
        (other_path.as_str(), body_b)
    };
    body["expected_version"] = json!(1);
    assert_eq!(
        request(&router, &token, "PUT", path, Some(body)).await.0,
        StatusCode::OK
    );
    let stored = preferences::get(&state.db, &owner).await.unwrap().unwrap();
    assert_eq!(stored.ordered.len(), 2);
    assert!(stored.ordered.contains(&ids[0]) && stored.ordered.contains(&ids[1]));
    let before = request(&router, &token, "GET", PATH, None).await;
    let a = json!({"ordered":[ids[2],ids[0]],"expected_version":2});
    let b = json!({"ordered":[],"expected_version":2});
    let (a, b) = tokio::join!(
        request(&router, &token, "PUT", GROUP_PATH, Some(a)),
        request(&router, &token, "PUT", &other_path, Some(b))
    );
    assert_eq!(
        [a.0, b.0]
            .into_iter()
            .filter(|s| *s == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        [a.0, b.0]
            .into_iter()
            .filter(|s| *s == StatusCode::CONFLICT)
            .count(),
        1
    );
    assert_ne!(request(&router, &token, "GET", PATH, None).await, before);
    let latest = request(&router, &token, "GET", PATH, None).await.1;
    let (path, ordered) = if a.0 == StatusCode::CONFLICT {
        (GROUP_PATH, json!([ids[2], ids[0]]))
    } else {
        (other_path.as_str(), json!([]))
    };
    let retry = request(
        &router,
        &token,
        "PUT",
        path,
        Some(json!({"ordered":ordered,"expected_version":latest["version"]})),
    )
    .await;
    assert_eq!(retry.0, StatusCode::OK, "{retry:?}");
    let stored = preferences::get(&state.db, &owner).await.unwrap().unwrap();
    assert_eq!(stored.ordered, vec![ids[2].clone(), ids[0].clone()]);
    assert_eq!(stored.version, 4);
}

#[tokio::test]
async fn service_preference_slug_proxy_and_implicit_llm_execution_unchanged() {
    use futures::TryStreamExt;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let (state, router, owner, token, ids) = fixture("preference_execution_unchanged").await;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    for (upstream, target) in [(&a, "alpha"), (&b, "beta")] {
        Mock::given(method("GET"))
            .and(path("/v1/ok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"target":target})))
            .mount(upstream)
            .await;
        Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"fixed","object":"chat.completion","model":"gpt-4o","target":target,"choices":[{"index":0,"message":{"role":"assistant","content":target},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))).mount(upstream).await;
    }
    let provider_id = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    state.db.collection::<Document>("provider_configs").insert_one(doc! {"_id":&provider_id,"slug":"openai","name":"Local test provider","provider_type":"api_key","is_active":true,"created_by":&owner,"created_at":now,"updated_at":now}).await.unwrap();
    let mut catalog = test_auto_connected_catalog_service();
    catalog.id = CATALOG.into();
    catalog.slug = "llm-openai".into();
    catalog.service_category = "llm".into();
    catalog.provider_config_id = Some(provider_id);
    catalog.requires_user_credential = true;
    catalog.base_url = format!("{}/v1", a.uri());
    state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&catalog)
        .await
        .unwrap();
    for (id, upstream) in [(&ids[0], &a), (&ids[1], &b)] {
        let row = state
            .db
            .collection::<Document>("user_services")
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        state
            .db
            .collection::<Document>("user_endpoints")
            .update_one(
                doc! {"_id":row.get_str("endpoint_id").unwrap()},
                doc! {"$set":{"url":format!("{}/v1", upstream.uri())}},
            )
            .await
            .unwrap();
    }
    state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&ids[2]}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    // Establish the unchanged database-order selector instead of assuming a Mongo sort.
    let chosen = crate::services::user_service_service::find_by_catalog_service_id(
        &state.db, &owner, CATALOG,
    )
    .await
    .unwrap()
    .unwrap()
    .id;
    let preferred = if chosen == ids[0] { &ids[1] } else { &ids[0] };
    let payload = json!({"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]});
    let paths = [
        "/api/v1/proxy/s/preference-order/ok",
        "/api/v1/llm/openai/v1/chat/completions",
        "/api/v1/llm/gateway/v1/chat/completions",
    ];
    let mut baseline = Vec::new();
    for url in paths {
        let result = request(
            &router,
            &token,
            if url.ends_with("/ok") { "GET" } else { "POST" },
            url,
            if url.ends_with("/ok") {
                None
            } else {
                Some(payload.clone())
            },
        )
        .await;
        assert_eq!(result.0, StatusCode::OK, "{url}: {result:?}");
        baseline.push(result);
    }
    let before_config = request(&router, &token, "GET", "/api/v1/mcp/config", None).await;
    assert_eq!(before_config.0, StatusCode::OK);
    let saved = request(
        &router,
        &token,
        "PUT",
        GROUP_PATH,
        Some(json!({"ordered":[preferred, &chosen],"expected_version":0})),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{saved:?}");
    for (index, url) in paths.into_iter().enumerate() {
        let result = request(
            &router,
            &token,
            if url.ends_with("/ok") { "GET" } else { "POST" },
            url,
            if url.ends_with("/ok") {
                None
            } else {
                Some(payload.clone())
            },
        )
        .await;
        assert_eq!(
            result, baseline[index],
            "preference must not change execution: {url}"
        );
    }
    assert_eq!(
        request(&router, &token, "GET", "/api/v1/mcp/config", None).await,
        before_config
    );
    let a_hits = a.received_requests().await.unwrap();
    let b_hits = b.received_requests().await.unwrap();
    assert_eq!(
        a_hits.len() + b_hits.len(),
        6,
        "exactly one provider effect per call"
    );
    let chosen_hits = if chosen == ids[0] { &a_hits } else { &b_hits };
    assert_eq!(
        chosen_hits
            .iter()
            .filter(|request| request.method.as_str() == "POST")
            .count(),
        4
    );
    assert_eq!(
        a_hits
            .iter()
            .filter(|request| request.method.as_str() == "GET")
            .count(),
        2,
        "named slug always chooses alpha"
    );
    let audits = state.db.collection::<crate::models::audit_log::AuditLog>(
        crate::models::audit_log::COLLECTION_NAME,
    );
    for event_type in [
        "proxy_request",
        "llm_proxy_request",
        "llm_gateway_request",
        "llm_gateway_routed_via_personal",
    ] {
        let events = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let events: Vec<_> = audits
                    .find(doc! {"event_type":event_type})
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
        .expect("paired asynchronous execution audits arrive");
        assert_eq!(events.len(), 2, "no retry/extra {event_type} audit");
        assert_eq!(
            events[0].event_data, events[1].event_data,
            "same target for {event_type}"
        );
        assert!(
            events
                .iter()
                .all(|event| event.user_id.as_deref() == Some(owner.as_str())
                    && event.api_key_id.is_none())
        );
        if event_type == "llm_gateway_routed_via_personal" {
            assert_eq!(
                events[0].event_data.as_ref().unwrap()["user_service_id"],
                chosen
            );
        }
    }
}
