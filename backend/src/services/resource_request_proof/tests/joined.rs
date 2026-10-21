//! Opt-in companion for CMA's real public-handler acceptance test. Never starts
//! production workers; all writes belong to a fresh, dedicated loopback Mongo.

use super::*;
use crate::crypto::token::{constant_time_eq, generate_random_token, hash_token};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{Mutex, Notify};
use zeroize::{Zeroize, Zeroizing};

struct Joined {
    fixture: Fixture,
    ordinary: String,
    role: String,
    control_hash: String,
    target: Mutex<Option<String>>,
    stopped: Notify,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    action: String,
    enabled: Option<bool>,
    target: Option<String>,
    path: Option<String>,
    transport: Option<String>,
}

fn invalid() -> AppError {
    AppError::ValidationError("Invalid isolated fixture control".into())
}

async fn envelope(response: axum::response::Response) -> Json<Value> {
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    Json(
        json!({"status":status,"body":serde_json::from_slice::<Value>(&bytes)
        .unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)))}),
    )
}

async fn control(
    State(state): State<Arc<Joined>>,
    headers: HeaderMap,
    Json(input): Json<Control>,
) -> Result<Json<Value>, StatusCode> {
    let supplied = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !constant_time_eq(
        hash_token(supplied).as_bytes(),
        state.control_hash.as_bytes(),
    ) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let result = apply(&state, input).await;
    Ok(match result {
        Ok(response) => response,
        Err(error) => envelope(error.into_response()).await,
    })
}

async fn apply(state: &Joined, input: Control) -> AppResult<Json<Value>> {
    let f = &state.fixture;
    let db = &f.state.db;
    match input.action.as_str() {
        "forward" => {
            let target = input.target.as_deref().ok_or_else(invalid)?;
            let url = reqwest::Url::parse(target).map_err(|_| invalid())?;
            if url.scheme() != "http"
                || url.host_str() != Some("127.0.0.1")
                || url.port().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(invalid());
            }
            // Bind this fixture to one local CMA listener for its entire life.
            let mut selected = state.target.lock().await;
            if selected.as_deref().is_some_and(|value| value != target) {
                return Err(invalid());
            }
            if selected.is_none() {
                db.collection::<Document>(SERVICES)
                    .update_one(doc! {"_id":&f.catalog}, doc! {"$set":{"base_url":target}})
                    .await?;
                db.collection::<Document>(ENDPOINTS)
                    .update_one(doc! {"user_id":&f.subject}, doc! {"$set":{"url":target}})
                    .await?;
                *selected = Some(target.into());
            }
            drop(selected);
            let path = input.path.as_deref().ok_or_else(invalid)?;
            let pathname = path.split('?').next().unwrap_or("");
            if !(pathname == "/api/v1/agents" || pathname.starts_with("/api/v1/agents/"))
                || path.contains(['#', '\\', '%'])
                || path.contains("..")
                || path.len() > 1024
            {
                return Err(invalid());
            }
            match input.transport.as_deref() {
                Some("mcp") if matches!(path, "/api/v1/agents" | "/api/v1/agents?limit=1") => {
                    let response = egress::call(f, &f.state, true).await?;
                    Ok(Json(json!({"status":response.status,"body":
                        serde_json::from_str::<Value>(&response.text).unwrap_or_else(|_| json!(response.text))})))
                }
                Some("rest") => Ok(envelope(egress::rest_path(f, path).await?).await),
                _ => Err(invalid()),
            }
        }
        "permissions" => {
            let enabled = input.enabled.ok_or_else(invalid)?;
            let permissions: Vec<&str> = if enabled {
                vec!["cma:sandbox:use"]
            } else {
                vec![]
            };
            db.collection::<Document>(crate::models::role::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&state.role},
                    doc! {"$set":{"permissions":permissions}},
                )
                .await?;
            Ok(Json(json!({"ok":true})))
        }
        "service_grant" => {
            let enabled = input.enabled.ok_or_else(invalid)?;
            let grants: Vec<&str> = if enabled { vec![&f.connection] } else { vec![] };
            db.collection::<Document>(KEYS)
                .update_many(
                    doc! {"_id":{"$in":[&state.ordinary,&f.key.id]}},
                    doc! {"$set":{"allowed_service_ids":grants}},
                )
                .await?;
            Ok(Json(json!({"ok":true})))
        }
        "revoke_ordinary" | "revoke_proof" | "proof_key_active" => {
            let key = if input.action == "revoke_ordinary" {
                &state.ordinary
            } else {
                &f.key.id
            };
            let active = input.action == "proof_key_active" && input.enabled.ok_or_else(invalid)?;
            db.collection::<Document>(KEYS)
                .update_one(doc! {"_id":key}, doc! {"$set":{"is_active":active}})
                .await?;
            Ok(Json(json!({"ok":true})))
        }
        "stop_turn" => {
            db.collection::<Document>(CONVERSATIONS)
                .update_one(
                    doc! {"_id":&f.conversation},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .await?;
            Ok(Json(json!({"ok":true})))
        }
        "shutdown" => {
            state.stopped.notify_one();
            Ok(Json(json!({"ok":true})))
        }
        _ => Err(invalid()),
    }
}

async fn require_dedicated_mongo() {
    use mongodb::options::{ClientOptions, ServerAddress};
    let uri =
        std::env::var("NYXID_TEST_DATABASE_URL").expect("explicit dedicated Mongo URI required");
    let options = ClientOptions::parse(&uri).await.expect("valid Mongo URI");
    assert!(
        options.direct_connection == Some(true)
            && options.credential.is_none()
            && matches!(options.hosts.as_slice(), [ServerAddress::Tcp { host, port: Some(port) }]
            if host == "127.0.0.1" && *port > 1024 && ![27017, 27018].contains(port)),
        "use a dedicated non-default loopback Mongo port with directConnection=true"
    );
    let client = mongodb::Client::with_options(options).unwrap();
    let names = client.list_database_names().await.unwrap();
    assert!(
        names
            .iter()
            .all(|name| matches!(name.as_str(), "admin" | "config" | "local")),
        "fixture requires a fresh dedicated Mongo instance"
    );
}

struct PrivateManifest(PathBuf);
impl Drop for PrivateManifest {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test]
#[ignore = "paired CMA acceptance only; requires a fresh dedicated loopback Mongo and private manifest"]
async fn serve_joined_cma_acceptance_fixture() {
    let path = PathBuf::from(
        std::env::var_os("NYXID_CMA_JOINED_MANIFEST").expect("manifest path required"),
    );
    let parent =
        std::fs::symlink_metadata(path.parent().unwrap()).expect("private directory required");
    assert!(
        parent.is_dir() && parent.permissions().mode() & 0o077 == 0,
        "manifest directory must be private (0700)"
    );
    assert!(!path.exists(), "manifest must not already exist");
    let staging = path
        .parent()
        .unwrap()
        .join(format!(".cma-fixture-{}", Uuid::new_v4()));
    let mut manifest = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staging)
        .unwrap();
    let _remove_staging = PrivateManifest(staging.clone());
    require_dedicated_mongo().await;
    let mut f = Fixture::new().await;
    f.key.full_key.zeroize(); // The provider needs its key ID, never the raw chat key.
    f.state.http_client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let db = &f.state.db;
    let role = Uuid::new_v4().to_string();
    let now = mongodb::bson::DateTime::now();
    db.collection::<Document>(crate::models::role::COLLECTION_NAME)
        .insert_one(doc! {
            "_id":&role,"name":"CMA fixture","slug":"cma-fixture","description":null,
            "permissions":["cma:sandbox:use"],"is_default":false,"is_system":false,
            "client_id":&f.client.id,"created_at":now,"updated_at":now,
        })
        .await
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&f.subject}, doc! {"$set":{"role_ids":[&role]}})
        .await
        .unwrap();
    let client_secret = Zeroizing::new(generate_random_token());
    db.collection::<Document>(crate::models::oauth_client::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.client.id},
            doc! {"$set":{"client_secret_hash":hash_token(&client_secret)}},
        )
        .await
        .unwrap();
    let mut ordinary = key_service::create_api_key(
        db,
        &f.subject,
        "CMA joined fixture",
        "proxy",
        None,
        None,
        Some(std::slice::from_ref(&f.connection)),
        None,
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let ordinary_key = Zeroizing::new(std::mem::take(&mut ordinary.full_key));
    // Personal NyxBot credentials require their auto-connection identity marker.
    // Use an explicit connection instead, so removing its grant is decisive
    // without converting the conversation credential into an ordinary key.
    db.collection::<Document>(CONNECTIONS)
        .update_one(doc! {"_id":&f.connection}, doc! {"$set":{"source":null}})
        .await
        .unwrap();
    for service_id in [&f.catalog, &f.connection] {
        db.collection::<Document>(crate::models::service_approval_config::COLLECTION_NAME)
            .insert_one(doc! {
                "_id":Uuid::new_v4().to_string(),"user_id":&f.subject,
                "service_id":service_id,"service_name":"CMA fixture",
                "approval_required":false,"default_effect":"auto_allow",
                "created_at":now,"updated_at":now,
            })
            .await
            .unwrap();
    }
    let proof_key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id":&f.key.id})
        .await
        .unwrap()
        .unwrap();
    assert!(crate::mw::auth::is_assistant_conversation_key_candidate(
        &proof_key
    ));
    let token = Zeroizing::new(generate_random_token());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let discovery = json!({"issuer":f.state.config.jwt_issuer,
        "authorization_endpoint":format!("{base_url}/oauth/authorize"),
        "token_endpoint":format!("{base_url}/oauth/token"),
        "introspection_endpoint":format!("{base_url}/oauth/introspect")});
    let data = Zeroizing::new(
        serde_json::to_vec(&json!({"base_url":base_url,
        "client_id":f.client.id,"client_secret":client_secret.as_str(),"service_id":f.catalog,
        "owner":f.subject,"ordinary_key":ordinary_key.as_str(),"control_token":token.as_str()}))
        .unwrap(),
    );
    let provider = Router::new()
        .route(
            "/oauth/introspect",
            post(crate::handlers::oauth::introspect),
        )
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let body = discovery.clone();
                async move { Json(body) }
            }),
        )
        .with_state(f.state.clone());
    let state = Arc::new(Joined {
        fixture: f,
        ordinary: ordinary.id,
        role,
        control_hash: hash_token(&token),
        target: Mutex::new(None),
        stopped: Notify::new(),
    });
    let router = provider
        .merge(
            Router::new()
                .route("/fixture/control", post(control))
                .with_state(state.clone()),
        )
        .layer(DefaultBodyLimit::max(64 * 1024));
    manifest.write_all(&data).unwrap();
    manifest.sync_all().unwrap();
    drop(manifest);
    // Publish complete JSON atomically without replacing another fixture's file.
    std::fs::hard_link(&staging, &path).unwrap();
    let _remove_manifest = PrivateManifest(path);
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(600),
                state.stopped.notified(),
            )
            .await;
        })
        .await
        .unwrap();
}
