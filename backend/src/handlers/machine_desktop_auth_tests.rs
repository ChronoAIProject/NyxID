//! Real production-router handshakes: signed JWTs, live sessions, no AuthUser injection.
use super::*;
use crate::{
    crypto::{jwt, token::hash_token},
    models::session::{Session, SessionPurpose},
    test_utils::*,
};
use bson::{Document, doc};
use serde_json::{Value, json};
use std::future::IntoFuture;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Error, client::IntoClientRequest},
};
use uuid::Uuid;

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub(super) async fn session(state: &AppState, owner: &str) -> (String, String) {
    let id = Uuid::new_v4().to_string();
    let cookie = Uuid::new_v4().to_string();
    state
        .db
        .collection::<Session>("sessions")
        .insert_one(Session {
            id: id.clone(),
            user_id: owner.into(),
            token_hash: hash_token(&cookie),
            purpose: Some(SessionPurpose::Authenticated),
            mfa_method: None,
            mfa_assured: Some(true),
            mfa_exemption: None,
            ip_address: None,
            user_agent: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            revoked: false,
            created_at: Utc::now(),
            last_active_at: Utc::now(),
        })
        .await
        .unwrap();
    (id, cookie)
}

fn bearer(state: &AppState, owner: &str, sid: &str, patch: Value) -> String {
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(owner).unwrap(),
        "openid account:read",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let mut claims =
        serde_json::to_value(jwt::verify_token(&state.jwt_keys, &state.config, &token).unwrap())
            .unwrap();
    claims["sid"] = json!(sid);
    claims
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    let token = jsonwebtoken::encode(
        &jsonwebtoken::decode_header(&token).unwrap(),
        &claims,
        &state.jwt_keys.encoding,
    )
    .unwrap();
    format!("Bearer {token}")
}

async fn handshake(
    base: &str,
    node: &str,
    query: &str,
    headers: &[(&str, &str)],
) -> (u16, Option<Socket>) {
    let mut req = format!("{base}/api/v1/assistant/nyxagent/machines/{node}/desktop{query}")
        .into_client_request()
        .unwrap();
    for (key, value) in headers {
        req.headers_mut().append(
            axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    match connect_async(req).await {
        Ok((socket, response)) => (response.status().as_u16(), Some(socket)),
        Err(Error::Http(response)) => (response.status().as_u16(), None),
        Err(error) => panic!("unexpected handshake error: {error}"),
    }
}

#[tokio::test]
async fn machine_desktop_real_router_admission_matrix() {
    let f =
        crate::services::assistant_authority_tests::orchestrator_fixture("desktop_real_auth").await;
    let node = crate::services::machine_integration_tests::node(&f, &f.owner).await;
    let (sid, cookie) = session(&f.state, &f.owner).await;
    let cookie = format!("nyx_session={cookie}");
    let valid = bearer(&f.state, &f.owner, &sid, json!({}));
    let mixed_scheme = valid.replacen("Bearer", "bEaReR", 1);
    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("ws://{}", listener.local_addr().unwrap());
    let server =
        tokio::spawn(axum::serve(listener, private.with_state(f.state.clone())).into_future());
    let frontend = url::Url::parse(&f.state.config.frontend_url)
        .unwrap()
        .origin()
        .ascii_serialization();
    let api = url::Url::parse(&f.state.config.base_url)
        .unwrap()
        .origin()
        .ascii_serialization();
    let (node_peer, requested) = peer(&f.state, &node, Some(1)).await;
    let (status, socket) = handshake(
        &base,
        &node.id,
        "",
        &[
            ("authorization", "Bearer \u{0080}"),
            ("cookie", &cookie),
            ("origin", &frontend),
        ],
    )
    .await;
    assert_eq!(
        status, 401,
        "non-ASCII authorization cannot fall back to the valid owner cookie"
    );
    assert!(
        socket.is_none(),
        "malformed authorization must never upgrade"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), requested.notified())
            .await
            .is_err(),
        "malformed authorization must send no desktop command to the node"
    );
    node_peer.abort();
    f.state.node_ws_manager.unregister_connection(&node.id);
    for (name, headers, expected) in [
        (
            "native absent",
            vec![("authorization", valid.as_str())],
            101,
        ),
        (
            "native frontend",
            vec![("authorization", &valid), ("origin", &frontend)],
            101,
        ),
        (
            "native Android API",
            vec![("authorization", &valid), ("origin", &api)],
            101,
        ),
        (
            "cookie frontend",
            vec![("cookie", &cookie), ("origin", &frontend)],
            101,
        ),
        (
            "mixed-case header names",
            vec![("AuThOrIzAtIoN", &valid), ("OrIgIn", &frontend)],
            101,
        ),
        (
            "mixed-case scheme cannot fall back to cookie",
            vec![
                ("authorization", &mixed_scheme),
                ("cookie", &cookie),
                ("origin", &frontend),
            ],
            403,
        ),
        (
            "non-ASCII origin",
            vec![
                ("authorization", &valid),
                ("origin", "https://\u{0080}.example"),
            ],
            403,
        ),
        (
            "duplicate cookie origin",
            vec![
                ("cookie", &cookie),
                ("origin", &frontend),
                ("OrIgIn", &frontend),
            ],
            403,
        ),
        ("MCP session alone", vec![("Mcp-Session-Id", &sid)], 401),
        (
            "token in subprotocol alone",
            vec![(
                "Sec-WebSocket-Protocol",
                valid.strip_prefix("Bearer ").unwrap(),
            )],
            401,
        ),
        ("cookie absent", vec![("cookie", &cookie)], 403),
        (
            "empty explicit credential",
            vec![
                ("cookie", &cookie),
                ("authorization", ""),
                ("origin", &frontend),
            ],
            403,
        ),
        (
            "cookie API",
            vec![("cookie", &cookie), ("origin", &api)],
            403,
        ),
        ("anonymous", vec![("origin", &frontend)], 401),
        (
            "bad bearer cookie",
            vec![
                ("authorization", "Bearer invalid"),
                ("cookie", &cookie),
                ("origin", &frontend),
            ],
            401,
        ),
        (
            "malformed auth cookie",
            vec![
                ("authorization", "Basic invalid"),
                ("cookie", &cookie),
                ("origin", &frontend),
            ],
            403,
        ),
        (
            "empty bearer cookie",
            vec![
                ("authorization", "Bearer "),
                ("cookie", &cookie),
                ("origin", &frontend),
            ],
            403,
        ),
        (
            "duplicate auth",
            vec![
                ("authorization", &valid),
                ("authorization", &valid),
                ("origin", &frontend),
            ],
            403,
        ),
    ] {
        let (status, socket) = handshake(&base, &node.id, "", &headers).await;
        drop(socket);
        assert_eq!(status, expected, "{name}");
    }
    let query_token = format!("?access_token={}", valid.strip_prefix("Bearer ").unwrap());
    assert_eq!(handshake(&base, &node.id, &query_token, &[]).await.0, 401);
    let conversation_key = crate::services::assistant_agent_credential_service::load_or_provision(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap();
    let conversation_bearer = format!("Bearer {}", conversation_key.raw_key.as_str());
    for credential in [
        ("authorization", conversation_bearer.as_str()),
        ("x-api-key", conversation_key.raw_key.as_str()),
    ] {
        assert_eq!(
            handshake(
                &base,
                &node.id,
                "",
                &[credential, ("cookie", &cookie), ("origin", &frontend)]
            )
            .await
            .0,
            403,
            "a provisioned assistant conversation key is never human desktop authority"
        );
    }
    for origin in [
        "",
        "null",
        "https://unrelated.example",
        "http://localhost:3000/",
        "http://localhost:3000 http://localhost:3001",
        "http://localhost:3000, http://localhost:3001",
    ] {
        for credential in [
            ("authorization", valid.as_str()),
            ("cookie", cookie.as_str()),
        ] {
            assert_eq!(
                handshake(&base, &node.id, "", &[credential, ("origin", origin)])
                    .await
                    .0,
                403,
                "origin {origin:?}"
            );
        }
    }
    assert_eq!(
        handshake(
            &base,
            &node.id,
            "",
            &[
                ("authorization", &valid),
                ("origin", &frontend),
                ("origin", &frontend)
            ]
        )
        .await
        .0,
        403
    );
    for patch in [
        json!({"sid":null}),
        json!({"sid":Uuid::new_v4().to_string()}),
        json!({"sid":"invalid"}),
        json!({"client_id":"developer-client"}),
        json!({"act":{"sub":"acting-client"}}),
        json!({"delegated":true,"act":{"sub":Uuid::new_v4().to_string()}}),
        json!({"relay":true}),
        json!({"sa":true}),
        json!({"token_type":"refresh"}),
        json!({"allow_all_services":false}),
        json!({"allow_all_nodes":false}),
        json!({"allowed_service_ids":["service"]}),
        json!({"allowed_node_ids":[node.id]}),
        json!({"resources":[]}),
        json!({"resources":["https://resource.example"]}),
        json!({"exp":Utc::now().timestamp()-300}),
    ] {
        let token = bearer(&f.state, &f.owner, &sid, patch.clone());
        let status = handshake(
            &base,
            &node.id,
            "",
            &[
                ("authorization", &token),
                ("origin", &frontend),
                ("cookie", &cookie),
            ],
        )
        .await
        .0;
        assert!(
            [401, 403].contains(&status),
            "claim patch {patch}: {status}"
        );
    }
    for update in [
        doc! {"revoked":true},
        doc! {"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::hours(1))},
        doc! {"user_id":Uuid::new_v4().to_string()},
        doc! {"purpose":"mfa_pending"},
    ] {
        f.state
            .db
            .collection::<Document>("sessions")
            .update_one(doc! {"_id":&sid}, doc! {"$set":&update})
            .await
            .unwrap();
        assert_eq!(
            handshake(&base, &node.id, "", &[("authorization", &valid)])
                .await
                .0,
            401
        );
        f.state.db.collection::<Document>("sessions").update_one(doc!{"_id":&sid},doc!{"$set":{"revoked":false,"expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1)),"user_id":&f.owner,"purpose":"authenticated"}}).await.unwrap();
    }
    for update in [
        doc! {"is_active":false},
        doc! {"user_type":"org"},
        doc! {"mfa_enabled":true},
    ] {
        f.state
            .db
            .collection::<Document>("users")
            .update_one(doc! {"_id":&f.owner}, doc! {"$set":update})
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("sessions")
            .update_one(doc! {"_id":&sid}, doc! {"$set":{"mfa_assured":false}})
            .await
            .unwrap();
        let status = handshake(&base, &node.id, "", &[("authorization", &valid)])
            .await
            .0;
        assert!([401, 403].contains(&status));
        f.state
            .db
            .collection::<Document>("users")
            .update_one(
                doc! {"_id":&f.owner},
                doc! {"$set":{"is_active":true,"user_type":"person","mfa_enabled":false}},
            )
            .await
            .unwrap();
    }
    f.state
        .db
        .collection::<Document>("sessions")
        .update_one(
            doc! {"_id":&sid},
            doc! {"$set":{"revoked":false,"mfa_assured":true}},
        )
        .await
        .unwrap();
    let other = Uuid::new_v4().to_string();
    f.state
        .db
        .collection("users")
        .insert_one(test_user(&other, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let (other_sid, other_cookie) = session(&f.state, &other).await;
    let other_token = bearer(&f.state, &other, &other_sid, json!({}));
    let other_cookie = format!("nyx_session={other_cookie}");
    assert_eq!(
        handshake(
            &base,
            &node.id,
            "",
            &[
                ("authorization", &other_token),
                ("cookie", &cookie),
                ("origin", &frontend)
            ]
        )
        .await
        .0,
        403,
        "bearer outsider wins over owner cookie"
    );
    assert_eq!(
        handshake(
            &base,
            &node.id,
            "",
            &[("authorization", &valid), ("cookie", &other_cookie)]
        )
        .await
        .0,
        101,
        "bearer owner wins over outsider cookie"
    );
    let created = crate::services::key_service::create_api_key(
        &f.state.db,
        &f.owner,
        "desktop-denied",
        "proxy",
        None,
        None,
        None,
        None,
        Some(true),
        None,
        Some(true),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let api_key = format!("Bearer {}", created.full_key);
    for header in [
        ("authorization", api_key.as_str()),
        ("x-api-key", created.full_key.as_str()),
    ] {
        assert_eq!(
            handshake(&base, &node.id, "", &[header, ("origin", &frontend)])
                .await
                .0,
            403
        );
    }
    for (query, expected) in [
        ("?context_id=missing", 409),
        ("?conversation_id=missing", 404),
    ] {
        let status = handshake(&base, &node.id, query, &[("authorization", &valid)])
            .await
            .0;
        assert_eq!(status, expected, "selector {query}");
    }
    // Live organization write ACL and local capability ceiling remain authoritative.
    use crate::models::{org_membership::OrgRole, user::UserType};
    let org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection("users")
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    // Register under the real owner: signing-secret encryption binds that owner.
    let org_node = crate::services::machine_integration_tests::ungranted_node(&f, &org).await;
    for (role, expected) in [
        (OrgRole::Admin, 101),
        (OrgRole::Member, 403),
        (OrgRole::Viewer, 403),
    ] {
        f.state
            .db
            .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
            .delete_many(doc! {"org_user_id":&org})
            .await
            .unwrap();
        f.state
            .db
            .collection(crate::models::org_membership::COLLECTION_NAME)
            .insert_one(test_membership(&org, &f.owner, role, None))
            .await
            .unwrap();
        assert_eq!(
            handshake(&base, &org_node.id, "", &[("authorization", &valid)])
                .await
                .0,
            expected
        );
    }
    f.state
        .db
        .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .delete_many(doc! {"org_user_id":&org})
        .await
        .unwrap();
    assert_eq!(
        handshake(&base, &org_node.id, "", &[("authorization", &valid)])
            .await
            .0,
        403
    );
    f.state
        .db
        .collection::<Document>("nodes")
        .update_one(
            doc! {"_id":&node.id},
            doc! {"$set":{"machine.computer":false}},
        )
        .await
        .unwrap();
    assert_ne!(
        handshake(&base, &node.id, "", &[("authorization", &valid)])
            .await
            .0,
        101
    );
    f.state
        .db
        .collection::<Document>("nodes")
        .delete_one(doc! {"_id":&node.id})
        .await
        .unwrap();
    assert_eq!(
        handshake(&base, &node.id, "", &[("authorization", &valid)])
            .await
            .0,
        404
    );
    server.abort();
    f.state.db.drop().await.unwrap();
}

async fn peer(
    state: &AppState,
    node: &Node,
    stall_at: Option<usize>,
) -> (
    tokio::task::JoinHandle<()>,
    std::sync::Arc<tokio::sync::Notify>,
) {
    use crate::services::node_ws_manager::{NodeCapabilitiesMsg, NodeOutboundMessage};
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    register_test_node_connection(state, &node.id, tx).await;
    state.node_ws_manager.record_capabilities(
        &node.id,
        &NodeCapabilitiesMsg {
            machine: node.machine.clone(),
            ..Default::default()
        },
    );
    let manager = state.node_ws_manager.clone();
    let node_id = node.id.clone();
    let stalled = std::sync::Arc::new(tokio::sync::Notify::new());
    let observed = stalled.clone();
    let task = tokio::spawn(async move {
        let mut requests = 0;
        while let Some(message) = rx.recv().await {
            if let NodeOutboundMessage::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "machine_request" {
                    requests += 1;
                    if stall_at.is_some_and(|threshold| requests >= threshold) {
                        observed.notify_one();
                        continue;
                    }
                    manager.deliver_machine_result(
                        &node_id,
                        nyxid_machine::Response {
                            request_id: value["request_id"].as_str().unwrap().into(),
                            result: json!({"ok":true}),
                        },
                    );
                }
            }
        }
    });
    (task, stalled)
}

#[tokio::test]
async fn machine_desktop_real_router_revocation_closes_idle_viewer() {
    revoked_viewer(None, Revocation::Session, false).await;
}

#[tokio::test]
async fn machine_desktop_real_router_revocation_interrupts_stalled_node_command() {
    revoked_viewer(Some(2), Revocation::Session, false).await;
}

#[tokio::test]
async fn machine_desktop_real_router_revocation_interrupts_initial_node_open() {
    revoked_viewer(Some(1), Revocation::Session, false).await;
}

#[tokio::test]
async fn machine_desktop_real_router_cookie_stream_revocation() {
    revoked_viewer(None, Revocation::Session, true).await;
}

#[tokio::test]
async fn machine_desktop_real_router_org_removal_interrupts_stalled_command() {
    revoked_viewer(Some(2), Revocation::OrgRemoval, false).await;
}

#[tokio::test]
async fn machine_desktop_real_router_org_demotion_interrupts_stalled_command() {
    revoked_viewer(Some(2), Revocation::OrgDemotion, false).await;
}

#[tokio::test]
async fn machine_desktop_real_router_node_deletion_interrupts_stalled_command() {
    revoked_viewer(Some(2), Revocation::NodeDeletion, false).await;
}

#[derive(Clone, Copy)]
enum Revocation {
    Session,
    OrgRemoval,
    OrgDemotion,
    NodeDeletion,
}

async fn revoked_viewer(stall_at: Option<usize>, revocation: Revocation, cookie_auth: bool) {
    let f = crate::services::assistant_authority_tests::orchestrator_fixture("desktop_live_revoke")
        .await;
    let org = Uuid::new_v4().to_string();
    let node = if matches!(revocation, Revocation::OrgRemoval | Revocation::OrgDemotion) {
        f.state
            .db
            .collection("users")
            .insert_one(test_user(&org, crate::models::user::UserType::Org))
            .await
            .unwrap();
        f.state
            .db
            .collection(crate::models::org_membership::COLLECTION_NAME)
            .insert_one(test_membership(
                &org,
                &f.owner,
                crate::models::org_membership::OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        crate::services::machine_integration_tests::ungranted_node(&f, &org).await
    } else {
        crate::services::machine_integration_tests::node(&f, &f.owner).await
    };
    let (peer, stalled) = peer(&f.state, &node, stall_at).await;
    let (sid, cookie) = session(&f.state, &f.owner).await;
    let cookie = format!("nyx_session={cookie}");
    let token = bearer(&f.state, &f.owner, &sid, json!({}));
    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("ws://{}", listener.local_addr().unwrap());
    let server =
        tokio::spawn(axum::serve(listener, private.with_state(f.state.clone())).into_future());
    let frontend = url::Url::parse(&f.state.config.frontend_url)
        .unwrap()
        .origin()
        .ascii_serialization();
    let headers = if cookie_auth {
        vec![("cookie", cookie.as_str()), ("origin", frontend.as_str())]
    } else {
        vec![("authorization", token.as_str())]
    };
    let (status, socket) = handshake(&base, &node.id, "", &headers).await;
    assert_eq!(status, 101);
    let mut socket = socket.unwrap();
    if stall_at != Some(1) {
        let connected = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(connected.to_text().unwrap().contains("connected"));
    }
    if stall_at.is_some() {
        tokio::time::timeout(Duration::from_secs(3), stalled.notified())
            .await
            .unwrap();
    }
    match revocation {
        Revocation::Session => {
            f.state
                .db
                .collection::<Document>("sessions")
                .update_one(doc! {"_id":&sid}, doc! {"$set":{"revoked":true}})
                .await
                .unwrap();
        }
        Revocation::OrgRemoval => {
            f.state
                .db
                .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
                .delete_many(doc! {"org_user_id":&org, "member_user_id":&f.owner})
                .await
                .unwrap();
        }
        Revocation::OrgDemotion => {
            f.state
                .db
                .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
                .update_one(
                    doc! {"org_user_id":&org, "member_user_id":&f.owner},
                    doc! {"$set":{"role":"member"}},
                )
                .await
                .unwrap();
        }
        Revocation::NodeDeletion => {
            f.state
                .db
                .collection::<Document>("nodes")
                .delete_one(doc! {"_id":&node.id})
                .await
                .unwrap();
        }
    }
    if !matches!(revocation, Revocation::Session) {
        assert!(
            live_session(&f.state, &f.owner, Uuid::parse_str(&sid).unwrap())
                .await
                .unwrap(),
            "resource revocation must be detected while the person's login session remains live"
        );
    }
    if stall_at.is_some() {
        let row = desktop::get(&f.state.db, &node.id).await.unwrap().unwrap();
        let frame = Frame {
            kind: Kind::Desktop,
            end: false,
            id: Uuid::parse_str(&row.session_id).unwrap(),
            sequence: 1,
            bytes: b"buffered desktop pixels",
        };
        f.state
            .node_ws_manager
            .deliver_desktop_frame(&node.id, &frame, &frame.encode().unwrap());
    }
    let closed = tokio::time::timeout(Duration::from_secs(14), async {
        while let Some(message) = socket.next().await {
            let message = message.unwrap();
            assert!(
                !message.is_binary(),
                "revoked authority must never receive desktop pixels"
            );
            if let tokio_tungstenite::tungstenite::Message::Close(Some(close)) = message {
                return u16::from(close.code);
            }
        }
        0
    })
    .await
    .unwrap();
    assert_eq!(closed, 1008);
    server.abort();
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[derive(Debug)]
struct PausedKeyProvider {
    inner: crate::crypto::local_key_provider::LocalKeyProvider,
    armed: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl crate::crypto::key_provider::KeyProvider for PausedKeyProvider {
    async fn wrap_dek(&self, dek: &[u8]) -> AppResult<crate::crypto::key_provider::WrappedKey> {
        self.inner.wrap_dek(dek).await
    }
    async fn unwrap_dek(
        &self,
        wrapped: &crate::crypto::key_provider::WrappedKey,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        if self.armed.swap(false, std::sync::atomic::Ordering::Relaxed) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.unwrap_dek(wrapped).await
    }
    fn current_key_id(&self) -> u8 {
        self.inner.current_key_id()
    }
    fn has_key_id(&self, id: u8) -> bool {
        self.inner.has_key_id(id)
    }
    fn has_previous_key(&self) -> bool {
        self.inner.has_previous_key()
    }
}

#[tokio::test]
async fn machine_desktop_real_router_revocation_during_decryption_refuses_upgrade() {
    let mut f = crate::services::assistant_authority_tests::orchestrator_fixture(
        "desktop_decryption_revoke",
    )
    .await;
    let provider = std::sync::Arc::new(PausedKeyProvider {
        inner: crate::crypto::local_key_provider::LocalKeyProvider::from_config(&f.state.config),
        armed: std::sync::atomic::AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    f.state.encryption_keys = std::sync::Arc::new(
        crate::crypto::aes::EncryptionKeys::with_provider(provider.clone()),
    );
    let node = crate::services::machine_integration_tests::node(&f, &f.owner).await;
    let (peer, requested) = peer(&f.state, &node, Some(1)).await;
    let (sid, _) = session(&f.state, &f.owner).await;
    let token = bearer(&f.state, &f.owner, &sid, json!({}));
    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("ws://{}", listener.local_addr().unwrap());
    let server =
        tokio::spawn(axum::serve(listener, private.with_state(f.state.clone())).into_future());
    provider
        .armed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let pending =
        tokio::spawn(
            async move { handshake(&base, &node.id, "", &[("authorization", &token)]).await },
        );
    tokio::time::timeout(Duration::from_secs(5), provider.entered.notified())
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("sessions")
        .update_one(doc! {"_id":&sid}, doc! {"$set":{"revoked":true}})
        .await
        .unwrap();
    provider.release.notify_one();
    let (status, socket) = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, 401);
    assert!(
        socket.is_none(),
        "revoked pending upgrade must admit no pixel stream"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), requested.notified())
            .await
            .is_err(),
        "revoked pending upgrade must send no DesktopOpen"
    );
    server.abort();
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_desktop_real_router_non_person_cookie_has_no_node_effect() {
    let f = crate::services::assistant_authority_tests::orchestrator_fixture("desktop_org_cookie")
        .await;
    let node = crate::services::machine_integration_tests::node(&f, &f.owner).await;
    let (peer, requested) = peer(&f.state, &node, Some(1)).await;
    let (_, cookie) = session(&f.state, &f.owner).await;
    f.state
        .db
        .collection::<Document>("users")
        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"user_type":"org"}})
        .await
        .unwrap();
    let cookie = format!("nyx_session={cookie}");
    let frontend = url::Url::parse(&f.state.config.frontend_url)
        .unwrap()
        .origin()
        .ascii_serialization();
    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("ws://{}", listener.local_addr().unwrap());
    let server =
        tokio::spawn(axum::serve(listener, private.with_state(f.state.clone())).into_future());
    let (status, socket) = handshake(
        &base,
        &node.id,
        "",
        &[("cookie", &cookie), ("origin", &frontend)],
    )
    .await;
    assert_eq!(status, 403);
    assert!(socket.is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(200), requested.notified())
            .await
            .is_err()
    );
    server.abort();
    peer.abort();
    f.state.db.drop().await.unwrap();
}
