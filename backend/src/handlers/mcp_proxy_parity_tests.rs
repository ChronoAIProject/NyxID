//! REST/MCP credential parity on real HTTP requests and node protocol frames.
use super::*;
use crate::services::node_ws_manager::{NodeOutboundMessage, NodeProxyResponse};

async fn legacy_fixture() -> Fixture {
    let f = Box::pin(Fixture::new()).await;
    f.state
        .db
        .collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
        .delete_one(doc! {"_id": &f.service})
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.key_id},
            doc! {"$set": {"allow_all_services": true}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("service_endpoints")
        .update_one(
            doc! {"service_id": &f.catalog},
            doc! {"$set": {"request_body_schema": {"type": "object"}}},
        )
        .await
        .unwrap();
    f
}

/// `fallback` mirrors production: only sessions minted from unrestricted
/// first-party access tokens may authenticate without a live bearer, so
/// owner sessions pass `true` and service-account sessions pass `false`.
async fn session_headers(
    f: &Fixture,
    actor: &str,
    fallback: bool,
    mut headers: HeaderMap,
) -> HeaderMap {
    let sid = f
        .state
        .mcp_sessions
        .create_with_proxy_access(actor, true, fallback)
        .await
        .unwrap()
        .unwrap();
    f.state
        .mcp_sessions
        .activate_services(&sid, std::slice::from_ref(&f.catalog))
        .await
        .unwrap();
    headers.insert("mcp-session-id", sid.parse().unwrap());
    headers
}

fn bearer(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    headers
}

async fn service_account(f: &Fixture, owner: &str) -> (String, HeaderMap) {
    use crate::services::service_account_service as accounts;
    let (sa, secret) = accounts::create_service_account(
        &f.state.db,
        "Parity caller",
        None,
        "proxy",
        &[],
        None,
        owner,
    )
    .await
    .unwrap();
    let token = accounts::authenticate_client_credentials(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &sa.client_id,
        &secret,
        Some("proxy"),
    )
    .await
    .unwrap()
    .access_token;
    let headers = session_headers(f, &sa.id, false, bearer(&token)).await;
    (sa.id, headers)
}

pub(super) async fn node_responder(f: &Fixture, calls: usize) -> tokio::task::JoinHandle<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    register_test_node_connection(&f.state, &f.node, tx).await;
    let captured = f.headers.clone();
    let paths = f.paths.clone();
    let manager = f.state.node_ws_manager.clone();
    let node = f.node.clone();
    tokio::spawn(async move {
        for _ in 0..calls {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let NodeOutboundMessage::Text(text) = frame else {
                panic!("expected proxy frame")
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            let mut headers = HeaderMap::new();
            for (name, value) in value["headers"].as_object().unwrap() {
                headers.insert(
                    axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    value.as_str().unwrap().parse().unwrap(),
                );
            }
            captured.lock().unwrap().push(headers);
            let mut path = value["path"].as_str().unwrap().to_string();
            if let Some(query) = value["query"].as_str().filter(|value| !value.is_empty()) {
                path.push('?');
                path.push_str(query);
            }
            paths.lock().unwrap().push(path);
            manager.deliver_proxy_response(
                &node,
                NodeProxyResponse {
                    request_id: value["request_id"].as_str().unwrap().into(),
                    status: 200,
                    headers: vec![],
                    body: br#"{"ok":true}"#.to_vec(),
                },
            );
        }
    })
}

async fn bind_node(f: &Fixture, actor: &str) {
    f.state
        .db
        .collection::<bson::Document>("node_service_bindings")
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "user_id": actor,
            "node_id": &f.node, "service_id": &f.catalog, "is_active": true,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn mcp_proxy_caller_bearer_matches_rest_for_each_auth_type_direct_and_node() {
    let f = Box::pin(legacy_fixture()).await;
    let subject = uuid::Uuid::parse_str(&f.owner).unwrap();
    let access = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &subject,
        "proxy",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let delegated = jwt::generate_delegated_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &subject,
        "proxy",
        "downstream",
        300,
        None,
    )
    .unwrap();
    let relay = jwt::generate_relay_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &subject,
        "proxy",
        None,
        &jwt::RelayAgentScope {
            api_key_id: f.key_id.clone(),
            api_key_name: "parity".into(),
            allowed_service_ids: vec![],
            allowed_node_ids: vec![f.node.clone()],
            allow_all_services: true,
            allow_all_nodes: false,
        },
    )
    .unwrap();
    let (sa_id, sa_headers) = Box::pin(service_account(&f, &f.owner)).await;
    // Cookie and MCP sessions authenticate the same person without a bearer.
    let raw_session = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    f.state
        .db
        .collection::<crate::models::session::Session>("sessions")
        .insert_one(crate::models::session::Session {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: f.owner.clone(),
            token_hash: crate::crypto::token::hash_token(&raw_session),
            ip_address: None,
            user_agent: None,
            revoked: false,
            expires_at: now + chrono::Duration::hours(1),
            created_at: now,
            last_active_at: now,
        })
        .await
        .unwrap();
    let mut cookie = HeaderMap::new();
    cookie.insert(
        "cookie",
        format!("nyx_session={raw_session}").parse().unwrap(),
    );
    let mut key_bearer = f.key_headers();
    key_bearer.extend(bearer(&f.key));
    let mut callers = Vec::new();
    for (name, headers) in [
        ("api-key", f.key_headers()),
        ("api-key-bearer", key_bearer),
        ("access", bearer(&access)),
        ("delegated", bearer(&delegated)),
        ("relay", bearer(&relay)),
        ("session", cookie),
    ] {
        callers.push((name, session_headers(&f, &f.owner, true, headers).await));
    }
    callers.push(("service-account", sa_headers));
    for node in [false, true] {
        let responder = if node {
            bind_node(&f, &f.owner).await;
            bind_node(&f, &sa_id).await;
            Some(node_responder(&f, callers.len() * 6).await)
        } else {
            None
        };
        for enabled in [true, false] {
            f.state
                .db
                .collection::<bson::Document>("downstream_services")
                .update_one(
                    doc! {"_id": &f.catalog},
                    doc! {"$set": {"forward_access_token": enabled}},
                )
                .await
                .unwrap();
            for (name, headers) in &callers {
                let expected = headers.get("authorization").filter(|_| enabled);
                for (mcp, universal) in [(false, false), (true, false), (true, true)] {
                    Box::pin(f.call_service(
                        "chrono-sandbox",
                        mcp,
                        universal,
                        headers,
                        json!({"body": {}}),
                    ))
                    .await;
                    let sent = f.headers.lock().unwrap().pop().unwrap();
                    // Never print credential values, even on assertion failure.
                    assert!(
                        sent.get("authorization") == expected,
                        "Authorization parity: {name}, node={node}, flag={enabled}, mcp={mcp}"
                    );
                }
            }
        }
        if let Some(responder) = responder {
            responder.await.unwrap();
        }
    }
    f.state.db.drop().await.unwrap();
}

async fn provider_requirement(f: &Fixture, owner: &str, method: &str, key: &str) -> String {
    let provider = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<bson::Document>("provider_configs")
        .insert_one(doc! {
            "_id": &provider, "slug": format!("parity-{method}"), "name": "Parity provider",
            "provider_type": "api_key", "is_active": true, "created_by": &f.owner,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("service_provider_requirements")
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "service_id": &f.catalog,
            "provider_config_id": &provider, "required": true,
            "injection_method": method, "injection_key": key,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let credential = uuid::Uuid::new_v4().to_string();
    crate::services::user_token_service::store_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        owner,
        &provider,
        &credential,
        None,
        None,
    )
    .await
    .unwrap();
    credential
}

#[tokio::test]
async fn mcp_proxy_node_provider_path_query_and_headers_match_rest() {
    let f = Box::pin(legacy_fixture()).await;
    let path = Box::pin(provider_requirement(&f, &f.owner, "path", "prefix-")).await;
    let query = Box::pin(provider_requirement(&f, &f.owner, "query", "api_key")).await;
    let header = Box::pin(provider_requirement(
        &f,
        &f.owner,
        "header",
        "X-Provider-Key",
    ))
    .await;
    let bearer = Box::pin(provider_requirement(
        &f,
        &f.owner,
        "bearer",
        "Authorization",
    ))
    .await;
    f.state
        .db
        .collection::<bson::Document>("downstream_services")
        .update_one(
            doc! {"_id": &f.catalog},
            doc! {"$set": {"default_request_headers": [
                {"name": "X-Provider-Key", "value": "overridden-default", "overridable": false},
            ]}},
        )
        .await
        .unwrap();
    bind_node(&f, &f.owner).await;
    let responder = node_responder(&f, 3).await;
    let expected_path = format!("/prefix-{path}/execute?api_key={query}");
    let expected_bearer = format!("Bearer {bearer}");
    let mut rest = None;
    for (mcp, universal) in [(false, false), (true, false), (true, true)] {
        Box::pin(f.call_service(
            "chrono-sandbox",
            mcp,
            universal,
            &f.key_headers(),
            json!({"body": {}}),
        ))
        .await;
        let mut sent = f.headers.lock().unwrap().pop().unwrap();
        let sent_path = f.paths.lock().unwrap().pop().unwrap();
        assert!(
            sent_path == expected_path,
            "delegated path/query substitution"
        );
        assert!(
            sent.get("x-provider-key").unwrap() == header.as_str(),
            "delegated header"
        );
        assert!(
            sent.get("authorization").unwrap() == expected_bearer.as_str(),
            "delegated bearer"
        );
        // REST creates its own ingress correlation ID; it is not credential state.
        sent.remove("x-request-id");
        if let Some((path, headers)) = &rest {
            assert!(
                &sent_path == path && &sent == headers,
                "complete REST/MCP node parity"
            );
        } else {
            rest = Some((sent_path, sent));
        }
    }
    responder.await.unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mcp_proxy_session_fallback_never_replays_an_invalid_bearer() {
    let f = Box::pin(legacy_fixture()).await;
    let invalid = uuid::Uuid::new_v4().to_string();
    let headers = session_headers(&f, &f.owner, true, bearer(&invalid)).await;
    let auth = authenticate_mcp(&f.state, &headers, true)
        .await
        .ok()
        .unwrap();
    assert_eq!(auth.auth_method, AuthMethod::Session);
    assert!(mcp_exec_context(&auth).caller_token.is_none());
    let mut valid = f.key_headers();
    valid.extend(bearer(&f.key));
    let auth = authenticate_mcp(&f.state, &valid, false)
        .await
        .ok()
        .unwrap();
    assert!(
        !format!("{auth:?}").contains(&f.key),
        "MCP Debug must redact bearer material"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mcp_proxy_legacy_provider_effective_owner_matches_rest() {
    let f = Box::pin(legacy_fixture()).await;
    let (sa_id, headers) = Box::pin(service_account(&f, &f.owner)).await;
    // A decoy credential on the SA catches accidental lookup by subject.
    Box::pin(provider_requirement(&f, &sa_id, "header", "X-Provider-Key")).await;
    let requirement = f
        .state
        .db
        .collection::<bson::Document>("service_provider_requirements")
        .find_one(doc! {"service_id": &f.catalog})
        .await
        .unwrap()
        .unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let personal_credential = uuid::Uuid::new_v4().to_string();
    let org_credential = uuid::Uuid::new_v4().to_string();
    for (owner, credential) in [(&f.owner, &personal_credential), (&org, &org_credential)] {
        crate::services::user_token_service::store_api_key(
            &f.state.db,
            &f.state.encryption_keys,
            owner,
            requirement.get_str("provider_config_id").unwrap(),
            credential,
            None,
            None,
        )
        .await
        .unwrap();
    }
    f.state
        .db
        .collection::<bson::Document>("downstream_services")
        .update_one(
            doc! {"_id": &f.catalog},
            doc! {"$set": {"inject_delegation_token": true, "delegation_token_scope": "proxy"}},
        )
        .await
        .unwrap();
    for (owner, expected) in [
        (Some(f.owner.as_str()), &personal_credential),
        (Some(org.as_str()), &org_credential),
        (None, &personal_credential), // pre-owner rows fall back to created_by
    ] {
        f.state
            .db
            .collection::<bson::Document>("service_accounts")
            .update_one(
                doc! {"_id": &sa_id},
                doc! {"$set": {"owner_user_id": owner}},
            )
            .await
            .unwrap();
        for (mcp, universal) in [(false, false), (true, false), (true, true)] {
            Box::pin(f.call_service(
                "chrono-sandbox",
                mcp,
                universal,
                &headers,
                json!({"body": {}}),
            ))
            .await;
            let sent = f.headers.lock().unwrap().pop().unwrap();
            assert!(
                sent.get("x-provider-key").unwrap() == expected.as_str(),
                "effective-owner credential selection, mcp={mcp}"
            );
            let claims = jwt::verify_token(
                &f.state.jwt_keys,
                &f.state.config,
                sent.get("x-nyxid-delegation-token")
                    .unwrap()
                    .to_str()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                claims.sub, sa_id,
                "the credential owner never replaces the actor"
            );
        }
    }
    // Ordinary callers keep their personal provider connection.
    for mcp in [false, true] {
        Box::pin(f.call_service(
            "chrono-sandbox",
            mcp,
            false,
            &f.key_headers(),
            json!({"body": {}}),
        ))
        .await;
        let sent = f.headers.lock().unwrap().pop().unwrap();
        assert!(sent.get("x-provider-key").unwrap() == personal_credential.as_str());
    }
    f.state.db.drop().await.unwrap();
}
