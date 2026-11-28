use crate::models::{downstream_service, provider_config, service_provider_requirement};
use crate::services::{node_service, user_token_service};
use nyxid_node_proxy_test::{CredentialStore, config::NodeConfig, secret_backend::SecretBackend};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DOWNLOAD: &str = "https://files.example.invalid/archive?sig=abc%2f%FF&part=1&part=2";
const TOKEN: &str = "fixture:location-token";

async fn add_query_requirement(state: &AppState, service_id: &str, owner: &str) {
    let provider_id = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    state.db.collection::<bson::Document>(provider_config::COLLECTION_NAME)
        .insert_one(doc! {"_id": &provider_id, "slug":"location-query", "name":"Location",
            "provider_type":"api_key", "is_active":true, "created_by":owner, "created_at":now, "updated_at":now})
        .await.unwrap();
    state
        .db
        .collection::<bson::Document>(service_provider_requirement::COLLECTION_NAME)
        .insert_one(
            doc! {"_id": Uuid::new_v4().to_string(), "service_id":service_id,
            "provider_config_id":&provider_id, "required":true, "injection_method":"query",
            "injection_key":"delegated", "created_at":now, "updated_at":now},
        )
        .await
        .unwrap();
    user_token_service::store_api_key(
        &state.db,
        &state.encryption_keys,
        owner,
        &provider_id,
        TOKEN,
        None,
        None,
    )
    .await
    .unwrap();
}

async fn check_locations(node_routed: bool) {
    let db = crate::test_utils::connect_transaction_test_database(if node_routed {
        "node_response_locations"
    } else {
        "direct_response_locations"
    })
    .await;
    let mut state = test_app_state(db.clone());
    state.platform_user_rate_limit = crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled();
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    crate::test_utils::set_test_approval_preference(&db, &owner, false).await;
    let auth = access_token_auth(&owner);
    let upstream = MockServer::start().await;
    let origin = upstream.uri();
    let cases = [
        ("relative", 201, "next".to_owned(), Some("jobs/next")),
        (
            "fragment",
            200,
            "#result".to_owned(),
            Some("jobs/fragment?view=own#result"),
        ),
        (
            "absolute",
            202,
            format!("{origin}/api/jobs/complete?view=ready"),
            Some("jobs/complete?view=ready"),
        ),
        ("redirect", 300, "next".to_owned(), Some("jobs/next")),
        (
            "followed",
            200,
            "next".to_owned(),
            (!node_routed).then_some("jobs/moved/next"),
        ),
        ("external", 300, DOWNLOAD.to_owned(), Some(DOWNLOAD)),
        (
            "external-path",
            300,
            format!("https://files.example.invalid/bot{TOKEN}/archive"),
            None,
        ),
        (
            "external-encoded",
            300,
            "https://files.example.invalid/archive?sig=fixture%3alocation-token".to_owned(),
            None,
        ),
        (
            "external-userinfo",
            300,
            "https://user@files.example.invalid/archive".to_owned(),
            None,
        ),
        ("external-created", 201, DOWNLOAD.to_owned(), None),
        (
            "userinfo",
            202,
            origin.replacen("://", "://user:password@", 1) + "/api/jobs",
            None,
        ),
        ("echo", 202, format!("{origin}/api/bot{TOKEN}/jobs"), None),
        (
            "encoded",
            201,
            "?key=fixture%3Alocation-token".to_owned(),
            None,
        ),
    ];
    for (case, status, value, _) in &cases {
        let mut reply = ResponseTemplate::new(*status);
        if *case == "fragment" {
            reply = reply.insert_header("content-type", "application/octet-stream");
        }
        for name in nyxid_service_adapters::response_location::HEADERS {
            reply = reply.insert_header(name, value.as_str());
        }
        if *case == "followed" {
            Mock::given(wiremock::matchers::path_regex("/jobs/moved/start$"))
                .respond_with(reply)
                .mount(&upstream)
                .await;
            reply = ResponseTemplate::new(302).insert_header("location", "moved/start");
        }
        Mock::given(wiremock::matchers::path_regex(format!("/jobs/{case}$")))
            .respond_with(reply)
            .mount(&upstream)
            .await;
    }

    for method in ["path", "query", "delegated", "node-local"] {
        if method == "node-local" && !node_routed {
            continue;
        }
        let start = upstream.received_requests().await.unwrap().len();
        let mut service = downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = format!("location-{method}");
        let service_base = format!("{origin}/api");
        service.base_url = if method == "node-local" {
            String::new()
        } else {
            service_base.clone()
        };
        if !node_routed && method != "delegated" {
            service.service_category = "internal".into();
            service.auth_method = method.into();
            service.auth_key_name = if method == "path" { "bot" } else { "key" }.into();
            set_catalog_credential(&state.encryption_keys, &mut service, TOKEN.as_bytes()).await;
        }
        db.collection::<downstream_service::DownstreamService>(downstream_service::COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();
        if method == "delegated" {
            add_query_requirement(&state, &service.id, &owner).await;
        }
        let mut node_tasks = None;
        if node_routed {
            let node = insert_online_node(&state, &owner, &service.slug).await;
            node_service::create_binding(&db, &owner, &node.id, &service.id)
                .await
                .unwrap();
            let dir = tempfile::tempdir().unwrap();
            let backend = SecretBackend::new("file", &node.id, dir.path()).unwrap();
            let mut config = NodeConfig::new(String::new(), node.id.clone(), "file".into());
            let target_url = Some(service_base.as_str());
            match method {
                "query" | "node-local" => config.add_query_param_credential_via(
                    &service.slug,
                    "key",
                    TOKEN,
                    target_url,
                    &backend,
                ),
                "path" => config.add_path_prefix_credential_via(
                    &service.slug,
                    "bot",
                    TOKEN,
                    target_url,
                    &backend,
                ),
                _ => config.set_no_auth_via(&service.slug, target_url, &backend),
            }
            .unwrap();
            let store = CredentialStore::from_config_with_backend(&config, &backend).unwrap();
            node_tasks = Some(
                start_node_executor_with_credentials(state.clone(), &node, store, cases.len())
                    .await,
            );
        }
        for (case, status, _, expected) in &cases {
            let path = format!("jobs/{case}");
            let prefix = format!("/api/v1/proxy/{}/", service.id);
            let response = proxy_request_inner(
                &state,
                &auth,
                &service.id,
                &path,
                proxy_request(&format!("{prefix}{path}?view=own")),
                &mut String::new(),
            )
            .await
            .unwrap();
            assert_eq!(response.status().as_u16(), *status, "{method} {case}");
            assert!(
                response
                    .headers()
                    .values()
                    .all(|v| !v.to_str().unwrap().contains("location-token"))
            );
            for name in nyxid_service_adapters::response_location::HEADERS {
                let expected = match expected {
                    Some(tail) if *tail == DOWNLOAD => {
                        (name == "location").then(|| DOWNLOAD.to_owned())
                    }
                    _ if method == "node-local" => None,
                    _ => expected.map(|tail| format!("{prefix}{tail}")),
                };
                assert_eq!(
                    response
                        .headers()
                        .get(name)
                        .map(|v| v.to_str().unwrap().to_owned()),
                    expected,
                    "{method} {case} {name}"
                );
            }
            to_bytes(response.into_body(), 4096).await.unwrap();
        }
        if let Some((executor, server)) = node_tasks {
            executor.await.unwrap();
            server.abort();
        }
        let received = upstream.received_requests().await.unwrap();
        let mut requests = received[start..]
            .iter()
            .filter(|r| !r.url.path().ends_with("/moved/start"));
        assert_eq!(requests.clone().count(), cases.len());
        assert!(requests.all(|r| match method {
            "path" => r.url.path().starts_with(&format!("/api/bot{TOKEN}/")),
            _ => r.url.query_pairs().any(|(k, v)| k
                == (if method != "delegated" {
                    "key"
                } else {
                    "delegated"
                })
                && v == TOKEN),
        }));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn direct_response_locations_use_caller_coordinates() {
    Box::pin(check_locations(false)).await;
}

#[tokio::test]
async fn node_response_locations_use_caller_coordinates() {
    Box::pin(check_locations(true)).await;
}

#[tokio::test]
async fn older_node_responses_forward_ordinary_headers_and_check_delegated_urls() {
    use tokio_tungstenite::tungstenite::Message;

    let db = crate::test_utils::connect_transaction_test_database("legacy_node_locations").await;
    let state = test_app_state(db.clone());
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    crate::test_utils::set_test_approval_preference(&db, &owner, false).await;
    let mut service = downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.base_url = "http://127.0.0.1/api".into();
    db.collection::<downstream_service::DownstreamService>(downstream_service::COLLECTION_NAME)
        .insert_one(&service)
        .await
        .unwrap();
    add_query_requirement(&state, &service.id, &owner).await;
    let node = insert_online_node(&state, &owner, &service.slug).await;
    node_service::create_binding(&db, &owner, &node.id, &service.id)
        .await
        .unwrap();
    let (mut socket, server) = connect_test_node_socket(state.clone(), &node).await;
    let agent = tokio::spawn(async move {
        for streamed in [false, true] {
            let request = loop {
                let message = socket.next().await.unwrap().unwrap();
                if let Message::Text(text) = message {
                    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] == "proxy_request" {
                        break frame;
                    }
                }
            };
            let query = request["query"].as_str().unwrap();
            assert!(
                url::form_urlencoded::parse(query.as_bytes())
                    .any(|(key, value)| key == "delegated" && value == TOKEN)
            );
            // These are the buffered and streaming frames sent by older nodes.
            let location = if streamed {
                "https://files.example/export?key=fixture%3Alocation-token"
            } else {
                DOWNLOAD
            };
            let content_location = if streamed {
                format!("?key={TOKEN}")
            } else {
                "#result".to_owned()
            };
            let frame = serde_json::json!({
                "type": if streamed { "proxy_response_start" } else { "proxy_response" },
                "request_id": request["request_id"], "status": 302,
                "headers": {
                    "Location": location, "Operation-Location": "poll",
                    "Content-Location": content_location, "etag": "version"
                }
            });
            socket
                .send(Message::Text(frame.to_string().into()))
                .await
                .unwrap();
            if streamed {
                let end = serde_json::json!({"type":"proxy_response_end", "request_id":request["request_id"]});
                socket
                    .send(Message::Text(end.to_string().into()))
                    .await
                    .unwrap();
            }
        }
    });
    let auth = access_token_auth(&owner);
    let prefix = format!("/api/v1/proxy/{}/jobs/", service.id);
    for streamed in [false, true] {
        let response = proxy_request_inner(
            &state,
            &auth,
            &service.id,
            "jobs/start",
            proxy_request(&format!("{prefix}start?view=own")),
            &mut String::new(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let headers = response.headers();
        assert_eq!(headers["etag"], "version");
        assert_eq!(headers["operation-location"], format!("{prefix}poll"));
        if streamed {
            assert!(!headers.contains_key("location"));
            assert!(!headers.contains_key("content-location"));
        } else {
            assert_eq!(headers["location"], DOWNLOAD);
            assert_eq!(
                headers["content-location"],
                format!("{prefix}start?view=own#result")
            );
        }
        to_bytes(response.into_body(), 1024).await.unwrap();
    }
    agent.await.unwrap();
    server.abort();
    db.drop().await.unwrap();
}
