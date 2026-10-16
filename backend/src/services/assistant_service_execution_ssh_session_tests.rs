use super::*;
use crate::models::ssh_auth_mode::SshAuthMode;
use crate::{
    handlers::{ssh_tunnel, ssh_web_terminal},
    services::{billing::route_inventory::BillingRoutePolicy, ssh_service},
};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Extension, Path, State, ws::WebSocketUpgrade},
    http::HeaderMap,
    routing::get,
};
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{Error as WsError, client::IntoClientRequest};

async fn fixture(
    f: &Fixture,
    owner: &str,
    mode: SshAuthMode,
    with_row: bool,
    port: u16,
) -> (String, Option<String>) {
    let catalog = catalog(f).await;
    let config = ssh_service::build_ssh_config(
        &f.state.encryption_keys,
        &catalog,
        None,
        ssh_service::SshConfigInput {
            host: "127.0.0.1",
            port,
            certificate_auth_enabled: mode == SshAuthMode::Cert,
            ssh_auth_mode: Some(mode),
            certificate_ttl_minutes: 30,
            allowed_principals: &["ubuntu".to_owned()],
        },
    )
    .await
    .unwrap();
    f.state.db.collection::<Document>("downstream_services").update_one(doc!{"_id":&catalog},doc!{"$set":{
        "created_by":owner,"owner_user_id":owner,"service_type":"ssh","ssh_config":bson::to_bson(&config).unwrap(),"inference":bson::Bson::Null
    }}).await.unwrap();
    let row = if with_row {
        let id = connection(f, owner, "session-exact", Some(&catalog)).await;
        f.state
            .db
            .collection::<Document>("user_services")
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{"service_type":"ssh","ssh_auth_mode":bson::to_bson(&mode).unwrap()}},
            )
            .await
            .unwrap();
        Some(id)
    } else {
        None
    };
    (catalog, row)
}
fn auth(f: &Fixture) -> crate::mw::auth::AuthUser {
    let mut auth = f.auth.clone();
    auth.assistant_chat = Some(std::sync::Arc::new(f.chat.clone()));
    auth
}
fn public_key() -> String {
    ssh_key::PrivateKey::random(&mut rand::rngs::OsRng, ssh_key::Algorithm::Ed25519)
        .unwrap()
        .public_key()
        .to_openssh()
        .unwrap()
}
async fn certificate(
    f: &Fixture,
    reference: &str,
    key: &str,
    ack: Option<&str>,
) -> AppResult<axum::response::Response> {
    let mut headers = HeaderMap::new();
    if let Some(ack) = ack {
        headers.insert(ACK_HEADER, ack.parse().unwrap());
    }
    Box::pin(ssh_tunnel::issue_ssh_certificate(
        State(f.state.clone()),
        auth(f),
        Default::default(),
        Path(reference.into()),
        headers,
        Json(ssh_tunnel::IssueSshCertificateRequest {
            public_key: key.into(),
            principal: "ubuntu".into(),
        }),
    ))
    .await
}
async fn card(response: axum::response::Response) -> Value {
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap()
}
#[tokio::test]
async fn actual_certificate_personal_connection_and_legacy_catalog_org_exact_connection_modes() {
    for legacy in [false, true] {
        let f = orchestrator_fixture("ssh_certificate_baseline").await;
        mode(&f, AccessMode::Autopilot).await;
        let (c, id) = fixture(&f, &f.owner, SshAuthMode::Cert, !legacy, 22).await;
        assert_eq!(
            certificate(&f, id.as_deref().unwrap_or(&c), &public_key(), None)
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::OK
        );
    }
    for access in [
        AccessMode::Nyxmode,
        AccessMode::AskEveryTime,
        AccessMode::Autopilot,
    ] {
        let f = orchestrator_fixture("ssh_certificate_org").await;
        mode(&f, access).await;
        let owner = org(&f).await;
        let (catalog, id) = fixture(&f, &owner, SshAuthMode::Cert, true, 22).await;
        if access == AccessMode::Autopilot {
            super::receipts::require_approval(&f, &catalog).await;
        }
        let id = id.unwrap();
        let key = public_key();
        let response = certificate(&f, &id, &key, None).await.unwrap();
        if access == AccessMode::Autopilot {
            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let rows = f
                .state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .find(doc! {"conversation_id":&f.row.id,"decided_by":"access_mode"})
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get_str("tool_name"), Ok("service_approval"));
            assert_eq!(
                rows[0]
                    .get_document("confirmation")
                    .unwrap()
                    .get_str("reason"),
                Ok("approval")
            );
            continue;
        }
        let ack = allow(&f, card(response).await).await;
        let changed = certificate(&f, &id, &public_key(), Some(&ack))
            .await
            .unwrap();
        let refused = card(changed).await;
        assert_eq!(refused["error"], "acknowledgement_invalid");
        // Invalid presentation never consumes the valid exact action.
        assert_eq!(
            certificate(&f, &id, &key, Some(&ack))
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::OK
        );
    }
}
#[tokio::test]
async fn legacy_catalog_session_rejects_deleted_selection_and_held_config_drift() {
    let f = orchestrator_fixture("ssh_catalog_selection_drift").await;
    mode(&f, AccessMode::Autopilot).await;
    let (c, id) = fixture(&f, &f.owner, SshAuthMode::Cert, true, 22).await;
    let id = id.unwrap();
    choose(&f, &id).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .delete_one(doc! {"_id":id})
        .await
        .unwrap();
    assert!(certificate(&f, &c, &public_key(), None).await.is_err());
    f.state
        .db
        .collection::<Document>(COLLECTION)
        .delete_many(doc! {"kind":"selection"})
        .await
        .unwrap();
    let config = ssh_service::get_ssh_service(&f.state.db, &c).await.unwrap();
    let admission = crate::services::assistant_ssh_session::admit(
        &f.state,
        Some(&f.chat),
        None,
        &config,
        &c,
        "SSH certificate",
        json!({"public_key":"bound"}),
        None,
        60,
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(doc! {"_id":&c}, doc! {"$set":{"ssh_config.port":2222}})
        .await
        .unwrap();
    assert!(admission.revalidate(&f.state, None).await.is_err());
}
async fn websocket_server(
    f: &Fixture,
    reference: &str,
    terminal: bool,
) -> (String, tokio::task::JoinHandle<()>) {
    let state = f.state.clone();
    let auth = auth(f);
    let reference = reference.to_owned();
    let app = Router::new().route(
        "/session",
        get(move |ws: WebSocketUpgrade, headers: HeaderMap| {
            let state = state.clone();
            let auth = auth.clone();
            let reference = reference.clone();
            async move {
                if terminal {
                    ssh_web_terminal::ssh_web_terminal(
                        State(state),
                        Extension(BillingRoutePolicy::Metered(
                            crate::services::billing::BillingIngress::SshWebTerminal,
                        )),
                        auth,
                        Path(reference),
                        axum::extract::Query(ssh_web_terminal::WebTerminalQuery {
                            principal: "ubuntu".into(),
                            cols: 80,
                            rows: 24,
                        }),
                        ConnectInfo("127.0.0.1:12345".parse().unwrap()),
                        headers,
                        ws,
                    )
                    .await
                } else {
                    ssh_tunnel::ssh_tunnel_ws(
                        State(state),
                        Extension(BillingRoutePolicy::Metered(
                            crate::services::billing::BillingIngress::SshTunnel,
                        )),
                        auth,
                        Path(reference),
                        ConnectInfo("127.0.0.1:12345".parse().unwrap()),
                        headers,
                        ws,
                    )
                    .await
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/session", listener.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }),
    )
}
async fn connect(
    url: &str,
    ack: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Value,
> {
    let mut request = url.into_client_request().unwrap();
    if let Some(ack) = ack {
        request
            .headers_mut()
            .insert(ACK_HEADER, ack.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(WsError::Http(response)) => {
            assert_eq!(response.status().as_u16(), 409);
            Err(serde_json::from_slice(response.body().as_deref().unwrap()).unwrap())
        }
        Err(error) => panic!("SSH route: {error:?}"),
    }
}
#[tokio::test]
async fn actual_direct_tunnel_personal_legacy_and_org_session_consent_preserve_binary_transport() {
    for (legacy, is_org, access) in [
        (false, false, AccessMode::Autopilot),
        (true, false, AccessMode::Autopilot),
        (false, true, AccessMode::Nyxmode),
        (false, true, AccessMode::AskEveryTime),
        (false, true, AccessMode::Autopilot),
    ] {
        let f = orchestrator_fixture("ssh_direct_session").await;
        mode(&f, access).await;
        let owner = if is_org {
            org(&f).await
        } else {
            f.owner.clone()
        };
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = target.local_addr().unwrap().port();
        let (c, id) = fixture(&f, &owner, SshAuthMode::ProxyOnly, !legacy, port).await;
        let (target_started, mut starts) = tokio::sync::mpsc::channel(1);
        let peer = tokio::spawn(async move {
            let (mut socket, _) = target.accept().await.unwrap();
            target_started.send(()).await.unwrap();
            socket.write_all(b"SSH-2.0-test\r\n").await.unwrap();
            let mut b = [0; 32];
            let _ = socket.read(&mut b).await;
        });
        let (url, server) = websocket_server(&f, id.as_deref().unwrap_or(&c), false).await;
        let initial = connect(&url, None).await;
        let mut socket = if is_org && access != AccessMode::Autopilot {
            let card = initial.expect_err("organization Auto/Ask must require consent");
            assert!(starts.try_recv().is_err());
            let ack = allow(&f, card).await;
            connect(&url, Some(&ack)).await.unwrap()
        } else {
            initial.expect("Full access must open without a consent card")
        };
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(frame.into_data().as_ref(), b"SSH-2.0-test\r\n");
        socket.close(None).await.unwrap();
        server.abort();
        peer.abort();
    }
}
#[tokio::test]
async fn actual_terminal_org_auto_card_then_exact_node_session_and_bypass() {
    for access in [
        AccessMode::Nyxmode,
        AccessMode::AskEveryTime,
        AccessMode::Autopilot,
    ] {
        let f = orchestrator_fixture("ssh_terminal_session").await;
        mode(&f, access).await;
        let owner = org(&f).await;
        let (_, id) = fixture(&f, &owner, SshAuthMode::NodeKey, true, 22).await;
        let id = id.unwrap();
        let node = crate::services::machine_integration_tests::node(&f, &owner).await;
        f.state
            .db
            .collection::<Document>("user_services")
            .update_one(doc! {"_id":&id}, doc! {"$set":{"node_id":&node.id}})
            .await
            .unwrap();
        let (sender, mut outgoing) = tokio::sync::mpsc::channel(8);
        test_utils::register_test_node_connection(&f.state, &node.id, sender).await;
        let manager = f.state.node_ws_manager.clone();
        let node_id = node.id.clone();
        let (seen, mut observed) = tokio::sync::mpsc::channel(1);
        let peer = tokio::spawn(async move {
            while let Some(crate::services::node_ws_manager::NodeOutboundMessage::Text(text)) =
                outgoing.recv().await
            {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "web_terminal_open" {
                    let session = value["session_id"].as_str().unwrap();
                    manager.deliver_web_terminal_started(&node_id, session);
                    seen.send(value).await.unwrap();
                }
            }
        });
        let (url, server) = websocket_server(&f, &id, true).await;
        let initial = connect(&url, None).await;
        let mut socket = if access != AccessMode::Autopilot {
            let card = initial.expect_err("organization Auto/Ask must require consent");
            assert!(observed.try_recv().is_err());
            let ack = allow(&f, card).await;
            connect(&url, Some(&ack)).await.unwrap()
        } else {
            initial.expect("Full access must open without a consent card")
        };
        let value = tokio::time::timeout(std::time::Duration::from_secs(10), observed.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(value["service_slug"], "session-exact");
        assert_eq!(value["principal"], "ubuntu");
        socket.close(None).await.unwrap();
        peer.abort();
        server.abort();
    }
}
