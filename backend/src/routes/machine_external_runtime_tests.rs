//! Public HTTP authentication and MCP dispatch into the actual CLI Runtime.
//! The loopback bridge is transport only; grants, leases, signatures, source
//! revocation and shell/files execution all use production implementations.
mod peer;

use crate::{
    AppState,
    crypto::jwt,
    models::{
        api_key::ApiKey,
        machine_job::{MachineJob, MachineJobBinding},
        node::Node,
        user::UserType,
    },
    services::{
        api_key_credential_service as credentials, auth_agent_key_login_service as login,
        key_service,
    },
    test_utils,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::Engine;
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};
use peer::Peer;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
use uuid::Uuid;
use zeroize::Zeroizing;

struct DatabaseCommandFailure {
    command: String,
    server_code: Option<i32>,
    transient_transaction: bool,
    unknown_commit: bool,
}

impl std::fmt::Debug for DatabaseCommandFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseCommandFailure")
            .field("command", &self.command)
            .field("server_code", &self.server_code)
            .field("transient_transaction", &self.transient_transaction)
            .field("unknown_commit", &self.unknown_commit)
            .finish()
    }
}

struct Fixture {
    state: AppState,
    app: Router,
    owner: String,
    owner_token: Zeroizing<String>,
    key: ApiKey,
    key_secret: Zeroizing<String>,
    node: Node,
    peer: Peer,
    grant: Value,
    sessions: Mutex<HashMap<String, String>>,
    database_failures: Arc<Mutex<Vec<DatabaseCommandFailure>>>,
    call_ordinal: AtomicUsize,
}

async fn http(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        app.clone().oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                    [127, 0, 0, 1],
                    0,
                ))))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        ),
    )
    .await
    .expect("bounded public request")
    .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn rpc(f: &Fixture, token: &str, method: &str, params: Value) -> (StatusCode, Value) {
    let session = f
        .sessions
        .lock()
        .unwrap()
        .get(&crate::crypto::token::hash_token(token))
        .cloned();
    let mut request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json");
    // MCP's documented Agent Key transport is x-api-key; its Bearer branch
    // intentionally verifies OAuth JWTs rather than accepting Key secrets.
    request =
        if token.starts_with(crate::crypto::token::AGENT_KEY_PREFIX) || token.starts_with("nyx_") {
            request.header("x-api-key", token)
        } else {
            request.header("authorization", format!("Bearer {token}"))
        };
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    let response=tokio::time::timeout(std::time::Duration::from_secs(30),f.app.clone().oneshot(request.body(Body::from(json!({"jsonrpc":"2.0","id":Uuid::new_v4().to_string(),"method":method,"params":params}).to_string())).unwrap())).await.unwrap().unwrap();
    if method == "initialize"
        && let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|s| s.to_str().ok())
    {
        f.sessions
            .lock()
            .unwrap()
            .insert(crate::crypto::token::hash_token(token), session.to_string());
    }
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn call(f: &Fixture, token: &str, name: &str, args: Value) -> Value {
    let ordinal = f.call_ordinal.fetch_add(1, Ordering::Relaxed) + 1;
    let source_class = if token == f.key_secret.as_str() {
        "fixture_parent_key"
    } else if token.starts_with(crate::crypto::token::AGENT_KEY_PREFIX) || token.starts_with("nyx_")
    {
        "other_key"
    } else {
        "bearer"
    };
    f.database_failures.lock().unwrap().clear();
    let (status, response) = rpc(
        f,
        token,
        "tools/call",
        json!({"name":name,"arguments":args}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "public MCP HTTP status for call #{ordinal} {name} ({source_class})"
    );
    assert!(
        response.get("error").is_none(),
        "public MCP protocol error for call #{ordinal} {name} ({source_class}): {response}; observed database command failures during request: {:?}",
        f.database_failures.lock().unwrap()
    );
    assert_ne!(
        response["result"]["isError"],
        true,
        "public MCP tool error for call #{ordinal} {name} ({source_class}): {response}; observed database command failures during request: {:?}",
        f.database_failures.lock().unwrap()
    );
    serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("MCP text result"),
    )
    .expect("Machine result JSON")
}
async fn denied(f: &Fixture, token: &str, name: &str, args: Value) {
    let (status, response) = rpc(
        f,
        token,
        "tools/call",
        json!({"name":name,"arguments":args}),
    )
    .await;
    assert!(
        status.is_client_error()
            || response.get("error").is_some()
            || response["result"]["isError"] == true,
        "expected denial: {response}"
    );
}
async fn fixture(label: &str) -> Fixture {
    use mongodb::event::{EventHandler, command::CommandEvent};

    let database_failures = Arc::new(Mutex::new(Vec::new()));
    let observed_failures = database_failures.clone();
    let db = test_utils::connect_transaction_test_database_with_command_handler(
        label,
        EventHandler::callback(move |event| {
            let failures: Vec<_> = match event {
                CommandEvent::Failed(event) => vec![DatabaseCommandFailure {
                    command: event.command_name,
                    server_code: match event.failure.kind.as_ref() {
                        mongodb::error::ErrorKind::Command(error) => Some(error.code),
                        _ => None,
                    },
                    transient_transaction: event
                        .failure
                        .contains_label("TransientTransactionError"),
                    unknown_commit: event
                        .failure
                        .contains_label("UnknownTransactionCommitResult"),
                }],
                // Mongo can return write errors inside an otherwise successful
                // command response. Retain their codes through the same filter.
                CommandEvent::Succeeded(event) => event
                    .reply
                    .get_array("writeErrors")
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(bson::Bson::as_document)
                    .chain(event.reply.get_document("writeConcernError").ok())
                    .map(|error| {
                        let has_label = |label: &str| {
                            event.reply.get_array("errorLabels").is_ok_and(|labels| {
                                labels.iter().any(|value| value.as_str() == Some(label))
                            })
                        };
                        DatabaseCommandFailure {
                            command: event.command_name.clone(),
                            server_code: error.get_i32("code").ok(),
                            transient_transaction: has_label("TransientTransactionError"),
                            unknown_commit: has_label("UnknownTransactionCommitResult"),
                        }
                    })
                    .collect(),
                _ => return,
            };
            // Maintenance shares this client. These are bounded command
            // diagnostics, not proof that a failure belongs to the MCP call.
            // Never retain command/reply bodies, messages or source IDs.
            let mut observed = observed_failures.lock().unwrap();
            for failure in failures {
                if observed.len() == 16 {
                    observed.remove(0);
                }
                observed.push(failure);
            }
        }),
    )
    .await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_utils::test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_utils::test_app_state(db.clone());
    let owner_token = Zeroizing::new(
        jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(&owner).unwrap(),
            "read write",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap(),
    );
    let (peer, node) = Peer::start(&state, &owner).await;
    let created = key_service::create_api_key(
        &db,
        &owner,
        "Public local journey",
        "proxy",
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(true),
        Some(false),
        Some(true),
        None,
        None,
        Some("codex"),
        None,
    )
    .await
    .unwrap();
    let key: ApiKey = db
        .collection(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":&created.id})
        .await
        .unwrap()
        .unwrap();
    let (public, private) = super::build_router_with_state(state.clone());
    let app = public.merge(private).with_state(state.clone());
    let grant_path = format!("/api/v1/api-keys/{}/machine-grant", key.id);
    let mut approval = json!({
        "expected_state_version": key.state_version,
        "selection": {
            "assignments": {
                (node.id.clone()): {
                    "capabilities": {"shell": true, "files": true},
                    "file_roots": [peer.root.path().to_str().unwrap()],
                    "service_ids": []
                }
            }
        }
    });
    let (status, refused) = http(&app, "PUT", &grant_path, &owner_token, approval.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["error"], "validation_error");
    assert_eq!(refused["error_code"], 1008);
    let message = refused["message"]
        .as_str()
        .expect("selection refusal reason");
    assert!(
        message.starts_with("Validation error: machine_selection_required:"),
        "{refused}"
    );
    assert!(message.contains(nyxid_machine::context::SHARED_LEGACY_WARNING));
    assert_eq!(
        db.collection::<Document>(crate::models::machine_external_grant::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0,
        "omitting consent must not create Machine authority"
    );
    let unchanged: ApiKey = db
        .collection(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id": &key.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.state_version, key.state_version);
    approval["selection"]["separation"] = json!("shared_legacy");
    approval["selection"]["legacy_warning_acknowledged"] = json!(true);
    let (status, grant) = http(&app, "PUT", &grant_path, &owner_token, approval).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "grant approval via owner endpoint: {grant}"
    );
    Fixture {
        state,
        app,
        owner,
        owner_token,
        key,
        key_secret: Zeroizing::new(created.full_key),
        node,
        peer,
        grant,
        sessions: Mutex::new(HashMap::new()),
        database_failures,
        call_ordinal: AtomicUsize::new(0),
    }
}
async fn child(f: &Fixture) -> (String, Zeroizing<String>) {
    let id = Uuid::new_v4().to_string();
    let secret = credentials::generate_secret();
    let current_parent: ApiKey = f
        .state
        .db
        .collection(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.key.id})
        .await
        .unwrap()
        .unwrap();
    let summary = login::key_summary(&f.state.db, &current_parent, false)
        .await
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    login::issue_selected(
        &f.state.db,
        &f.owner,
        &Uuid::new_v4().to_string(),
        &Default::default(),
        None,
        &login::Selection::Existing {
            api_key_id: f.key.id.clone(),
            permission_snapshot: Some(summary.permission_snapshot),
        },
        Some(Utc::now() + Duration::hours(1)),
        Utc::now(),
        &f.key.id,
        &id,
        &secret,
        &mut session,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    let (status, grant) = http(
        &f.app,
        "GET",
        &format!("/api/v1/api-keys/{}/machine-grant", f.key.id),
        &f.owner_token,
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "owner reads grant after child issuance"
    );
    assert_eq!(
        grant["source_valid"], true,
        "child issuance retains its parent source"
    );
    assert_eq!(
        grant["grant_valid"], true,
        "reviewed child issuance must not revoke the unchanged parent Machine grant"
    );
    assert_eq!(grant["grant"]["id"], f.grant["id"]);
    assert_eq!(grant["grant"]["revision"], f.grant["revision"]);
    (id, secret)
}
async fn job(f: &Fixture, id: &str) -> MachineJob {
    f.state
        .db
        .collection(crate::models::machine_job::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}
async fn background(f: &Fixture, secret: &str) -> String {
    // Flush the streaming secret redactor at stdout EOF while the actual job
    // stays alive for later cancellation and source-revocation assertions.
    call(f,secret,"nyx__machine_exec",json!({"machine":f.node.id,"command":"printf public-runtime; exec 1>&-; sleep 90","background":true,"timeout_secs":120})).await["job_id"].as_str().unwrap().to_string()
}
async fn assert_background_output(f: &Fixture, secret: &str, job_id: &str) {
    let mut output = Value::Null;
    let observed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            output = call(
                f,
                secret,
                "nyx__machine_job",
                json!({"machine":f.node.id,"job_id":job_id,"wait_secs":1}),
            )
            .await;
            if output.to_string().contains("public-runtime") {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        observed.is_ok(),
        "fixture marker missing after bounded public queries: status={}, output_available={}, stdout_bytes={}",
        output["status"],
        output["output_available"],
        output["stdout"].as_str().map_or(0, str::len)
    );
}
async fn finish(f: Fixture) {
    f.peer.runtime.shutdown().await;
    let db = f.state.db.clone();
    drop(f);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn local_journey_external_http_node_and_oauth_refresh() {
    let mut f = fixture("local_journey_external_http").await;
    // The external credential journey keeps the owner generic preference explicit.
    crate::test_utils::set_test_approval_preference(&f.state.db, &f.owner, false).await;

    let (child_id, secret) = child(&f).await;
    let (status, listed) = rpc(&f, &secret, "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::OK, "Agent Key MCP discovery transport");
    for name in [
        "nyx__machine_list",
        "nyx__machine_exec",
        "nyx__machine_job",
        "nyx__machine_job_cancel",
        "nyx__machine_list_files",
        "nyx__machine_read_file",
        "nyx__machine_write_file",
        "nyx__machine_edit_file",
    ] {
        assert!(
            listed["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["name"] == name),
            "missing public tool {name}"
        );
    }
    let machines = call(&f, &secret, "nyx__machine_list", json!({})).await;
    assert!(machines.to_string().contains(&f.node.id));
    let path = f.peer.root.path().join("journey.txt").display().to_string();
    call(
        &f,
        &secret,
        "nyx__machine_write_file",
        json!({"machine":f.node.id,"path":path,"content":"before","mode":"create"}),
    )
    .await;
    call(
        &f,
        &secret,
        "nyx__machine_edit_file",
        json!({"machine":f.node.id,"path":path,"old_string":"before","new_string":"after"}),
    )
    .await;
    let read = call(
        &f,
        &secret,
        "nyx__machine_read_file",
        json!({"machine":f.node.id,"path":path}),
    )
    .await;
    assert!(read.to_string().contains("after"));
    let files = call(
        &f,
        &secret,
        "nyx__machine_list_files",
        json!({"machine":f.node.id,"path":f.peer.root.path().display().to_string()}),
    )
    .await;
    assert!(files.to_string().contains("journey.txt"));
    let id = background(&f, &secret).await;
    let persisted = job(&f, &id).await;
    assert!(
        persisted.conversation_id.is_empty()
            && persisted.api_key_id.is_empty()
            && persisted.agent_id.is_empty()
    );
    assert!(
        matches!(persisted.binding,Some(MachineJobBinding::External{principal:crate::models::machine_external_grant::ExternalPrincipalRef::ApiKey{api_key_credential_id:Some(ref actual),..},..}) if actual==&child_id)
    );
    let history = call(
        &f,
        &f.key_secret,
        "nyx__machine_job",
        json!({"machine":f.node.id}),
    )
    .await;
    assert!(history.to_string().contains(&id));
    assert_background_output(&f, &secret, &id).await;
    call(
        &f,
        &secret,
        "nyx__machine_job_cancel",
        json!({"machine":f.node.id,"job_id":id}),
    )
    .await;
    assert_eq!(std::fs::read_to_string(path).unwrap(), "after");
    // The real mounted human route remains unavailable to execution credentials.
    let (status, _) = http(
        &f.app,
        "GET",
        "/api/v1/remote-machines",
        &secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    gateway(&mut f).await;
    let oauth = oauth_login(&f, None).await;
    let oauth_job = background(&f, &oauth.access).await;
    let original = job(&f, &oauth_job).await;
    let refreshed = oauth_refresh(&f, &oauth).await;
    call(
        &f,
        &refreshed,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":oauth_job}),
    )
    .await;
    let renewed = job(&f, &oauth_job).await;
    match (original.binding, renewed.binding) {
        (
            Some(MachineJobBinding::External {
                principal: a,
                grant_id: ag,
                grant_revision: ar,
                authentication_expires_at: ae,
                ..
            }),
            Some(MachineJobBinding::External {
                principal: b,
                grant_id: bg,
                grant_revision: br,
                authentication_expires_at: be,
                ..
            }),
        ) => {
            assert_eq!(a, b);
            assert_eq!(ag, bg);
            assert_eq!(ar, br);
            assert!(be >= ae);
        }
        _ => panic!("OAuth work must retain actual external binding"),
    }
    call(
        &f,
        &refreshed,
        "nyx__machine_job_cancel",
        json!({"machine":f.node.id,"job_id":oauth_job}),
    )
    .await;
    let dispatched = f.peer.dispatches.load(Ordering::SeqCst);
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.node.id},
            doc! {"$set":{"machine.authority_versions":[2_i32]}},
        )
        .await
        .unwrap();
    denied(
        &f,
        &refreshed,
        "nyx__machine_exec",
        json!({"machine":f.node.id,"command":"printf never-downgrade"}),
    )
    .await;
    assert_eq!(
        f.peer.dispatches.load(Ordering::SeqCst),
        dispatched,
        "old Node never receives downgraded external work"
    );
    finish(f).await;
}

#[tokio::test]
async fn local_journey_source_mutations_stop_only_original_work() {
    let f = fixture("local_journey_source_mutations").await;
    let oauth = oauth_login(&f, None).await;
    let independent_job = background(&f, &oauth.access).await;
    let (a_id, a) = child(&f).await;
    let (_, b) = child(&f).await;
    let a_job = background(&f, &a).await;
    let b_job = background(&f, &b).await;
    let before = job(&f, &a_job).await;
    call(
        &f,
        &b,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":a_job}),
    )
    .await;
    let after = job(&f, &a_job).await;
    assert_eq!(
        before.binding, after.binding,
        "sibling cannot rebind or renew original child authority"
    );
    let (status, response) = http(
        &f.app,
        "DELETE",
        &format!("/api/v1/api-keys/{}/credentials/{a_id}", f.key.id),
        &f.owner_token,
        Value::Null,
    )
    .await;
    assert!(
        status.is_success(),
        "owner child revoke HTTP {status}: {response}"
    );
    denied(
        &f,
        &a,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":a_job}),
    )
    .await;
    let rows: Vec<Document> = f
        .state
        .db
        .collection(crate::models::machine_access::OUTBOX)
        .find(doc! {"kind":"external","pending":true})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        !rows.is_empty(),
        "real source mutation must create a durable receipt"
    );
    for row in rows {
        crate::services::machine_external_service::revoke_outbox(&f.state, row.clone())
            .await
            .unwrap();
        crate::services::machine_external_service::revoke_outbox(&f.state, row)
            .await
            .unwrap();
    }
    assert_eq!(job(&f, &a_job).await.state, "revoked");
    let grant: Document = f
        .state
        .db
        .collection(crate::models::machine_external_grant::COLLECTION_NAME)
        .find_one(doc! {"_id":f.grant["id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        grant.get_i64("revision").unwrap(),
        f.grant["revision"].as_i64().unwrap()
    );
    assert_background_output(&f, &b, &b_job).await;
    let (status, rotated) = http(
        &f.app,
        "POST",
        &format!("/api/v1/api-keys/{}/rotate", f.key.id),
        &f.owner_token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "real parent rotation");
    let successor = Zeroizing::new(rotated["full_key"].as_str().unwrap().to_string());
    denied(
        &f,
        &b,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":b_job}),
    )
    .await;
    let (_, tools) = rpc(&f, &successor, "tools/list", json!({})).await;
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| !tool["name"]
                .as_str()
                .unwrap_or("")
                .starts_with("nyx__machine_")),
        "new parent has no inherited grant"
    );
    drain_source_receipts(&f).await;
    assert_eq!(job(&f, &b_job).await.state, "revoked");
    call(
        &f,
        &oauth.access,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":independent_job}),
    )
    .await;
    let (status, _) = http(
        &f.app,
        "DELETE",
        &format!("/api/v1/users/me/consents/{}", oauth.client),
        &f.owner_token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    denied(
        &f,
        &oauth.access,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":independent_job}),
    )
    .await;
    drain_source_receipts(&f).await;
    assert_eq!(job(&f, &independent_job).await.state, "revoked");
    let successor_oauth = oauth_login(&f, Some(&oauth.client)).await;
    denied(
        &f,
        &successor_oauth.access,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":independent_job}),
    )
    .await;
    let result = call(
        &f,
        &successor_oauth.access,
        "nyx__machine_exec",
        json!({"machine":f.node.id,"command":"printf new-consent"}),
    )
    .await;
    assert_eq!(result["stdout"], "new-consent");
    finish(f).await;
}

#[tokio::test]
async fn local_journey_confirmation_and_runtime_boundaries() {
    let f = fixture("local_journey_confirmations").await;
    let (_, a) = child(&f).await;
    let (_, b) = child(&f).await;
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.node.id},
            doc! {"$set":{"machine_confirm":"all"}},
        )
        .await
        .unwrap();
    let args = json!({"machine":f.node.id,"command":"printf once >> confirmed.txt","cwd":f.peer.root.path().display().to_string()});
    let pending_result = call(&f, &a, "nyx__machine_exec", args.clone()).await;
    assert_eq!(pending_result["status"], "confirmation_pending");
    assert!(!f.peer.root.path().join("confirmed.txt").exists());
    let (status, listing) = http(
        &f.app,
        "GET",
        "/api/v1/machine-confirmations",
        &f.owner_token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let pending = listing["confirmations"]
        .as_array()
        .unwrap()
        .first()
        .unwrap();
    let id = pending["id"].as_str().unwrap();
    let (status, decision) = http(
        &f.app,
        "POST",
        &format!("/api/v1/machine-confirmations/{id}/decision"),
        &f.owner_token,
        json!({"approve":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "owner approval: {decision}");
    let mut approved = args.clone();
    approved["acknowledgement_id"] = json!(id);
    denied(&f, &b, "nyx__machine_exec", approved.clone()).await;
    let executed = call(&f, &a, "nyx__machine_exec", approved.clone()).await;
    let executed_job = executed["job_id"].as_str().unwrap().to_string();
    let replay = call(&f, &a, "nyx__machine_exec", approved).await;
    assert_eq!(replay["status"], "already_consumed");
    assert_eq!(
        std::fs::read_to_string(f.peer.root.path().join("confirmed.txt")).unwrap(),
        "once"
    );
    let fresh_args = json!({"machine":f.node.id,"command":"printf forbidden >> confirmed.txt","cwd":f.peer.root.path().display().to_string()});
    let pending = call(&f, &a, "nyx__machine_exec", fresh_args.clone()).await;
    let pending_id = pending["confirmation_id"].as_str().unwrap();
    let row: Document = f
        .state
        .db
        .collection(crate::models::machine_external_execution::CONFIRMATIONS)
        .find_one(doc! {"_id":pending_id})
        .await
        .unwrap()
        .unwrap();
    let age = row.get_datetime("expires_at").unwrap().timestamp_millis()
        - row.get_datetime("created_at").unwrap().timestamp_millis();
    assert_eq!(age, 15 * 60 * 1000, "pending contract is fifteen minutes");
    f.state.db.collection::<Document>(crate::models::machine_external_execution::CONFIRMATIONS).update_one(doc!{"_id":pending_id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    let dispatched = f.peer.dispatches.load(Ordering::SeqCst);
    let (status, _) = http(
        &f.app,
        "POST",
        &format!("/api/v1/machine-confirmations/{pending_id}/decision"),
        &f.owner_token,
        json!({"approve":true}),
    )
    .await;
    assert!(!status.is_success());
    assert_eq!(f.peer.dispatches.load(Ordering::SeqCst), dispatched);
    let pending = call(&f, &a, "nyx__machine_exec", fresh_args.clone()).await;
    let approved_id = pending["confirmation_id"].as_str().unwrap();
    let (status, decision) = http(
        &f.app,
        "POST",
        &format!("/api/v1/machine-confirmations/{approved_id}/decision"),
        &f.owner_token,
        json!({"approve":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let approved_at =
        chrono::DateTime::parse_from_rfc3339(decision["decided_at"].as_str().unwrap()).unwrap();
    let until =
        chrono::DateTime::parse_from_rfc3339(decision["expires_at"].as_str().unwrap()).unwrap();
    assert_eq!(
        (until - approved_at).num_seconds(),
        600,
        "approved contract is ten minutes"
    );
    f.state.db.collection::<Document>(crate::models::machine_external_execution::CONFIRMATIONS).update_one(doc!{"_id":approved_id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    let mut expired = fresh_args;
    expired["acknowledgement_id"] = json!(approved_id);
    denied(&f, &a, "nyx__machine_exec", expired).await;
    assert_eq!(f.peer.dispatches.load(Ordering::SeqCst), dispatched);
    // A replacement profile cannot inherit the old runtime's output index.
    f.state.db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc!{"_id":&f.node.id},doc!{"$set":{"machine.runtime_id":Uuid::new_v4().to_string(),"machine_confirm":"none"}}).await.unwrap();
    let old = call(
        &f,
        &a,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":executed_job}),
    )
    .await;
    assert_eq!(old["output_available"], false);
    assert_eq!(old["output_unavailable_reason"], "runtime_replaced");
    assert_eq!(
        f.peer.dispatches.load(Ordering::SeqCst),
        dispatched,
        "no old output request reaches replacement runtime"
    );
    assert_eq!(
        std::fs::read_to_string(f.peer.root.path().join("confirmed.txt")).unwrap(),
        "once"
    );
    finish(f).await;
}

async fn drain_source_receipts(f: &Fixture) {
    for _ in 0..4 {
        let rows: Vec<Document> = f
            .state
            .db
            .collection(crate::models::machine_access::OUTBOX)
            .find(doc! {"kind":{"$in":["external","external_lease"]},"pending":true})
            .limit(100)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        if rows.is_empty() {
            break;
        }
        for row in rows {
            crate::services::machine_external_service::revoke_outbox(&f.state, row)
                .await
                .unwrap();
        }
    }
}

struct OAuthLogin {
    client: String,
    access: Zeroizing<String>,
    refresh: Zeroizing<String>,
}
async fn oauth_request(
    f: &Fixture,
    method: &str,
    path: &str,
    body: String,
    human: bool,
) -> (StatusCode, Value, Option<String>) {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded");
    if human {
        r = r.header(
            "authorization",
            format!("Bearer {}", f.owner_token.as_str()),
        );
    }
    let response = f
        .app
        .clone()
        .oneshot(r.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let redirect = response
        .headers()
        .get("location")
        .and_then(|x| x.to_str().ok())
        .map(str::to_string);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        redirect,
    )
}
async fn oauth_login(f: &Fixture, existing_client: Option<&str>) -> OAuthLogin {
    oauth_login_with_services(f, existing_client, &[]).await
}
async fn oauth_login_with_services(
    f: &Fixture,
    existing_client: Option<&str>,
    service_ids: &[String],
) -> OAuthLogin {
    let client = if let Some(id) = existing_client {
        id.to_string()
    } else {
        crate::services::oauth_client_service::create_client(
            &f.state.db,
            "Public local OAuth journey",
            &["https://local-client.example/callback".into()],
            "public",
            &f.owner,
            "",
            "openid proxy machine offline_access",
            crate::models::oauth_client::ScopeProvenance::Explicit,
            false,
            None,
            None,
            &[],
        )
        .await
        .unwrap()
        .0
        .id
    };
    let verifier = Zeroizing::new(format!(
        "{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    ));
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("response_type", "code"),
        ("client_id", client.as_str()),
        ("redirect_uri", "https://local-client.example/callback"),
        ("scope", "openid proxy machine offline_access"),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("prompt", "consent"),
    ]);
    let (status, _, redirect) = oauth_request(
        f,
        "GET",
        &format!("/oauth/authorize?{}", query.finish()),
        String::new(),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::FOUND, "public authorize");
    let redirect = url::Url::parse(&redirect.expect("consent redirect")).unwrap();
    let signed = Zeroizing::new(
        redirect
            .query_pairs()
            .find(|(k, _)| k == "consent_request")
            .expect("public authorize signed consent request")
            .1
            .into_owned(),
    );
    let (status,prepared)=http(&f.app,"POST","/api/v1/users/me/oauth-machine-consent-requests",&f.owner_token,json!({"consent_request":signed.as_str(),"selection":{"separation":"shared_legacy","legacy_warning_acknowledged":true,"assignments":{(f.node.id.clone()):{"capabilities":{"shell":true,"files":true},"file_roots":[f.peer.root.path().display().to_string()],"service_ids":service_ids}}}})).await;
    assert_eq!(status, StatusCode::OK, "protected Machine selection");
    let selected = prepared["token"].as_str().unwrap();
    let mut decision = url::form_urlencoded::Serializer::new(String::new());
    decision.extend_pairs([
        ("response_type", "code"),
        ("client_id", client.as_str()),
        ("redirect_uri", "https://local-client.example/callback"),
        ("scope", "openid proxy machine offline_access"),
        ("decision", "allow"),
        ("allow_all_services", "false"),
        ("consent_request", selected),
    ]);
    for service_id in service_ids {
        decision.append_pair("allowed_service_ids", service_id);
    }
    let (status, _, redirect) = oauth_request(
        f,
        "POST",
        "/oauth/authorize/decision",
        decision.finish(),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::FOUND, "public consent decision");
    let callback = url::Url::parse(&redirect.expect("code redirect")).unwrap();
    assert_eq!(callback.host_str(), Some("local-client.example"));
    let code = Zeroizing::new(
        callback
            .query_pairs()
            .find(|(k, _)| k == "code")
            .expect("public consent returns code")
            .1
            .into_owned(),
    );
    let mut exchange = url::form_urlencoded::Serializer::new(String::new());
    exchange.extend_pairs([
        ("grant_type", "authorization_code"),
        ("client_id", client.as_str()),
        ("redirect_uri", "https://local-client.example/callback"),
        ("code", code.as_str()),
        ("code_verifier", verifier.as_str()),
    ]);
    let (status, tokens, _) =
        oauth_request(f, "POST", "/oauth/token", exchange.finish(), false).await;
    assert_eq!(status, StatusCode::OK, "public PKCE exchange");
    let access = Zeroizing::new(
        tokens["access_token"]
            .as_str()
            .expect("access token")
            .to_string(),
    );
    let refresh = Zeroizing::new(
        tokens["refresh_token"]
            .as_str()
            .expect("refresh token")
            .to_string(),
    );
    let (status,initialized)=rpc(f,&access,"initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"untrusted-client-label","version":"local"}})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(initialized.get("error").is_none());
    OAuthLogin {
        client,
        access,
        refresh,
    }
}
async fn oauth_refresh(f: &Fixture, login: &OAuthLogin) -> Zeroizing<String> {
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    form.extend_pairs([
        ("grant_type", "refresh_token"),
        ("client_id", login.client.as_str()),
        ("refresh_token", login.refresh.as_str()),
    ]);
    let (status, tokens, _) = oauth_request(f, "POST", "/oauth/token", form.finish(), false).await;
    assert_eq!(status, StatusCode::OK, "public refresh");
    let access = Zeroizing::new(tokens["access_token"].as_str().unwrap().to_string());
    rpc(f,&access,"initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"another-label","version":"local"}})).await;
    access
}

async fn fresh_request_context(f: &mut Fixture) {
    let mut fresh =
        test_utils::test_app_state_with_config(f.state.db.clone(), f.state.config.clone());
    assert!(!Arc::ptr_eq(&fresh.mcp_sessions, &f.state.mcp_sessions));
    assert!(!Arc::ptr_eq(&fresh.jwks_cache, &f.state.jwks_cache));
    // This is a new authentication/session/cache context over the same durable
    // database. The existing Node socket's owner and dispatcher deliberately
    // remain shared; no synthetic verified caller or authority is injected.
    fresh.jwt_keys = f.state.jwt_keys.clone();
    fresh.encryption_keys = f.state.encryption_keys.clone();
    fresh.node_ws_manager = f.state.node_ws_manager.clone();
    fresh.node_dispatch = f.state.node_dispatch.clone();
    fresh.replica_identity = f.state.replica_identity.clone();
    fresh.billing = f.state.billing.clone();
    f.peer.replace_request_context(fresh.clone()).await;
    let (public, private) = super::build_router_with_state(fresh.clone());
    f.app = public.merge(private).with_state(fresh.clone());
    f.state = fresh;
    f.sessions.lock().unwrap().clear();
}

async fn gateway(f: &mut Fixture) {
    let count = Arc::new(AtomicUsize::new(0));
    let reached = count.clone();
    let credential = Zeroizing::new(format!("local-provider-{}", Uuid::new_v4()));
    let expected = credential.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            Router::new().fallback(move |request: Request<Body>| {
                let expected = expected.clone();
                let reached = reached.clone();
                async move {
                    assert!(
                        request
                            .headers()
                            .get("authorization")
                            .unwrap()
                            .to_str()
                            .unwrap()
                            == format!("Bearer {}", expected.as_str()),
                        "local provider authentication mismatch"
                    );
                    reached.fetch_add(1, Ordering::SeqCst);
                    "public gateway reached local upstream"
                }
            }),
        )
        .await;
    });
    let provider = crate::services::user_api_key_service::create_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        crate::services::user_api_key_service::CreateApiKeyParams {
            label: "Public gateway fixture",
            credential_type: "bearer",
            credential: &credential,
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
    let endpoint = Uuid::new_v4().to_string();
    let id = Uuid::new_v4().to_string();
    let slug = "public-local-api";
    f.state
        .db
        .collection(crate::models::user_endpoint::COLLECTION_NAME)
        .insert_one(test_utils::test_user_endpoint(
            &endpoint, &f.owner, slug, &url, None, None,
        ))
        .await
        .unwrap();
    let mut service = test_utils::test_user_service(&id, &f.owner, slug, &endpoint, None, None);
    service.api_key_id = Some(provider.id);
    service.auth_method = "bearer".into();
    f.state
        .db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(service)
        .await
        .unwrap();
    let command = "curl -sS -H \"Authorization: Bearer $NYXID_GATEWAY_TOKEN\" \"$NYXID_GATEWAY_URL/s/public-local-api/hello\"";
    denied(
        f,
        &f.key_secret,
        "nyx__machine_exec",
        json!({"machine":f.node.id,"command":command,"services":[slug]}),
    )
    .await;
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "grant ceiling rejects before upstream"
    );
    let (status, reviewed_key) = http(
        &f.app,
        "GET",
        &format!("/api/v1/api-keys/{}", f.key.id),
        &f.owner_token,
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "owner reviews current Key mutation version"
    );
    let (status,grant)=http(&f.app,"PUT",&format!("/api/v1/api-keys/{}/machine-grant",f.key.id),&f.owner_token,json!({"expected_state_version":reviewed_key["state_version"],"expected_revision":f.grant["revision"],"expected_grant_id":f.grant["id"],"selection":{"separation":"shared_legacy","legacy_warning_acknowledged":true,"assignments":{(f.node.id.clone()):{"capabilities":{"shell":true,"files":true},"file_roots":[f.peer.root.path().display().to_string()],"service_ids":[id]}}}})).await;
    assert_eq!(status, StatusCode::OK);
    f.grant = grant;
    let undeclared = call(
        f,
        &f.key_secret,
        "nyx__machine_exec",
        json!({"machine":f.node.id,"command":command}),
    )
    .await;
    assert!(!undeclared.to_string().contains("public gateway reached"));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let allowed = call(
        f,
        &f.key_secret,
        "nyx__machine_exec",
        json!({"machine":f.node.id,"command":command,"services":[slug]}),
    )
    .await;
    assert_eq!(allowed["stdout"], "public gateway reached local upstream");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let job_id = allowed["job_id"].as_str().unwrap();
    let persisted = job(f, job_id).await;
    assert_eq!(persisted.services[0].id, id);
    assert!(
        !serde_json::to_string(&persisted)
            .unwrap()
            .contains(credential.as_str())
    );
    // A long-lived OAuth job reaches the gateway only after its original
    // backend request context has been replaced. The gateway must reconstruct
    // the exact consent, grant and service ceiling from the persisted binding.
    let oauth = oauth_login_with_services(f, None, std::slice::from_ref(&id)).await;
    let marker = f.peer.root.path().join("fresh-context-gateway-ready");
    let delayed =
        format!("while [ ! -e fresh-context-gateway-ready ]; do sleep 0.05; done; {command}");
    let started = call(
        f,
        &oauth.access,
        "nyx__machine_exec",
        json!({"machine":f.node.id,"command":delayed,"cwd":f.peer.root.path().display().to_string(),"services":[slug],"background":true,"timeout_secs":120}),
    )
    .await;
    let original_id = started["job_id"].as_str().unwrap().to_owned();
    let original = job(f, &original_id).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    fresh_request_context(f).await;
    let (status, initialized) = rpc(
        f,
        &oauth.access,
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fresh-replica-context","version":"local"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(initialized.get("error").is_none());
    call(
        f,
        &oauth.access,
        "nyx__machine_write_file",
        json!({"machine":f.node.id,"path":marker.display().to_string(),"content":"release","mode":"create"}),
    )
    .await;
    let recovered = call(
        f,
        &oauth.access,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":original_id,"wait_secs":5}),
    )
    .await;
    assert_eq!(recovered["stdout"], "public gateway reached local upstream");
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let recovered_binding = job(f, &original_id).await.binding;
    assert!(
        original.binding == recovered_binding,
        "fresh gateway context retains the exact original OAuth binding"
    );
    let refreshed = oauth_refresh(f, &oauth).await;
    call(
        f,
        &refreshed,
        "nyx__machine_job",
        json!({"machine":f.node.id,"job_id":original_id}),
    )
    .await;
    let refreshed_job = job(f, &original_id).await;
    match (&original.binding, &refreshed_job.binding) {
        (
            Some(MachineJobBinding::External {
                principal: a,
                grant_id: ag,
                grant_revision: ar,
                protected_consent: ac,
                ..
            }),
            Some(MachineJobBinding::External {
                principal: b,
                grant_id: bg,
                grant_revision: br,
                protected_consent: bc,
                ..
            }),
        ) => {
            assert!(
                a == b && ag == bg && ar == br && ac == bc,
                "public refresh on fresh context cannot rebind original work"
            );
        }
        _ => panic!("recovered OAuth job retains persisted external source"),
    }
    f.state
        .db
        .collection::<Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(doc! {"_id":id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    denied(
        f,
        &f.key_secret,
        "nyx__machine_exec",
        json!({"machine":f.node.id,"command":command,"services":[slug]}),
    )
    .await;
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "live ACL loss does not reach upstream"
    );
    server.abort();
}
