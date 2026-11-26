use crate::{
    AppState,
    handlers::{
        admin_sa_connections as admin_connections, admin_sa_providers as admin_providers,
        connections, user_tokens,
    },
    models::{provider_config::ProviderConfig, user::UserType},
    services::{
        audit_service, connection_service, security_effect_service, service_account_service,
        user_token_service,
    },
    test_utils::{self, test_app_state, test_auth_user, test_user},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use bson::{Document, doc};
use futures::{FutureExt, TryStreamExt};
use mongodb::{
    Database,
    event::{EventHandler, command::CommandEvent},
};
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use uuid::Uuid;
const INTENTS: &str = "security_effect_intents";
const TOKENS: &str = "user_provider_tokens";
const STATES: &str = "oauth_states";
const CONNECTIONS: &str = "user_service_connections";
const CREDENTIAL: &str = "never-audit-raw-credential";
const ACCESS: &str = "never-audit-provider-access";
const REFRESH: &str = "never-audit-provider-refresh";
const DEVICE: &str = "never-audit-device-code";
const USER_CODE: &str = "never-audit-user-code";

tokio::task_local! { static REQUEST: (); }
#[derive(Clone, Copy, Debug)]
enum Action {
    ApiKey,
    DeviceComplete,
    ProviderDisconnect,
    OAuthStart,
    DeviceStart,
    ServiceConnect,
    CredentialUpdate,
    ServiceDisconnect,
}
impl Action {
    const ALL: [Self; 8] = [
        Self::ApiKey,
        Self::DeviceComplete,
        Self::ProviderDisconnect,
        Self::OAuthStart,
        Self::DeviceStart,
        Self::ServiceConnect,
        Self::CredentialUpdate,
        Self::ServiceDisconnect,
    ];
    fn event(self) -> &'static str {
        match self {
            Self::ApiKey | Self::DeviceComplete => "admin.sa.provider_connected",
            Self::ProviderDisconnect => "admin.sa.provider_disconnected",
            Self::OAuthStart => "admin.sa.oauth_initiated",
            Self::DeviceStart => "admin.sa.device_code_initiated",
            Self::ServiceConnect => "admin.sa.service_connected",
            Self::CredentialUpdate => "admin.sa.service_credential_updated",
            Self::ServiceDisconnect => "admin.sa.service_disconnected",
        }
    }
    fn source(self) -> &'static str {
        match self {
            Self::ApiKey | Self::DeviceComplete | Self::ProviderDisconnect => TOKENS,
            Self::OAuthStart | Self::DeviceStart => STATES,
            _ => CONNECTIONS,
        }
    }
}
#[derive(Debug)]
struct ProcessStopped;
#[derive(Default)]
struct Monitor {
    stop: Option<&'static str>,
    transactions_with_source: HashSet<String>,
    open_transactions: HashSet<String>,
    completed_transactions: std::collections::HashMap<i32, String>,
    stop_requests: HashSet<i32>,
    commands: Vec<(String, String)>,
}
#[derive(Default)]
struct ProviderControl {
    requests: AtomicUsize,
    fail: AtomicBool,
    hold: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
struct ProviderServer {
    base: String,
    control: Arc<ProviderControl>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for ProviderServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(monitor: Arc<Mutex<Monitor>>) -> ProviderServer {
    let control = Arc::new(ProviderControl::default());
    let ctl = control.clone();
    let app=axum::Router::new().fallback(move |uri: axum::http::Uri| {
        let ctl=ctl.clone(); let monitor=monitor.clone();
        async move {
            assert!(monitor.lock().unwrap().open_transactions.is_empty(), "provider I/O inside transaction");
            ctl.requests.fetch_add(1,Ordering::SeqCst);
            ctl.entered.notify_one();
            if ctl.hold.load(Ordering::SeqCst) { ctl.release.notified().await; }
            if ctl.fail.load(Ordering::SeqCst) {
                return (StatusCode::BAD_GATEWAY,Json(serde_json::json!({"error":"fixture_provider_failure"})));
            }
            let body=if uri.path()=="/device" { serde_json::json!({
                "device_code":DEVICE,"user_code":USER_CODE,"verification_uri":"https://example.invalid/verify",
                "expires_in":900,"interval":1
            }) } else { serde_json::json!({"access_token":ACCESS,"refresh_token":REFRESH,"expires_in":3600}) };
            (StatusCode::OK,Json(body))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = crate::services::shutdown::process().spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    ProviderServer {
        base,
        control,
        task,
    }
}
struct Fixture {
    state: AppState,
    actor: String,
    sa: String,
    provider: String,
    service: String,
    oauth_state: String,
    server: ProviderServer,
    monitor: Arc<Mutex<Monitor>>,
}
async fn fixture(action: Action) -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = test_utils::connect_transaction_test_database_with_command_handler(
        "sa_connection_audit",
        EventHandler::callback(move |event| {
            if REQUEST.try_with(|_| ()).is_err() {
                return;
            }
            let mut m = seen.lock().unwrap();
            match event {
                CommandEvent::Started(e) => {
                    let collection = e.command.get_str(&e.command_name).unwrap_or("").to_owned();
                    m.commands
                        .push((e.command_name.clone(), collection.clone()));
                    let key = e
                        .command
                        .get_i64("txnNumber")
                        .ok()
                        .filter(|_| e.command.get_bool("autocommit") == Ok(false))
                        .map(|n| format!("{:?}:{n}", e.command.get_document("lsid").unwrap()));
                    if let Some(key) = &key {
                        m.open_transactions.insert(key.clone());
                        if matches!(
                            e.command_name.as_str(),
                            "commitTransaction" | "abortTransaction"
                        ) {
                            m.completed_transactions.insert(e.request_id, key.clone());
                        }
                    }
                    if m.stop == Some(collection.as_str())
                        && matches!(
                            e.command_name.as_str(),
                            "insert" | "update" | "findAndModify"
                        )
                    {
                        if let Some(key) = key {
                            m.transactions_with_source.insert(key);
                        } else {
                            m.stop_requests.insert(e.request_id);
                        }
                    } else if m.stop.is_some()
                        && e.command_name == "commitTransaction"
                        && key
                            .as_ref()
                            .is_some_and(|k| m.transactions_with_source.contains(k))
                    {
                        m.stop_requests.insert(e.request_id);
                    }
                }
                CommandEvent::Succeeded(e) => {
                    if let Some(key) = m.completed_transactions.remove(&e.request_id) {
                        m.open_transactions.remove(&key);
                    }
                    if m.stop.is_some() && m.stop_requests.remove(&e.request_id) {
                        m.stop = None;
                        drop(m);
                        std::panic::resume_unwind(Box::new(ProcessStopped));
                    }
                }
                CommandEvent::Failed(e) => {
                    m.stop_requests.remove(&e.request_id);
                }
                _ => {}
            }
        }),
    )
    .await;
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&db)
        .await
        .unwrap();
    let actor = Uuid::new_v4().to_string();
    let mut user = test_user(&actor, UserType::Person);
    user.role_ids = vec![roles.admin];
    db.collection::<crate::models::user::User>("users")
        .insert_one(user)
        .await
        .unwrap();
    let (sa, _) = service_account_service::create_service_account(
        &db,
        "Connection fixture",
        None,
        "proxy",
        &[],
        None,
        &actor,
    )
    .await
    .unwrap();
    let server = server(monitor.clone()).await;
    let state = test_app_state(db.clone());
    let provider = Uuid::new_v4().to_string();
    let kind = match action {
        Action::DeviceStart | Action::DeviceComplete => "device_code",
        Action::OAuthStart => "oauth2",
        _ => "api_key",
    };
    let p:ProviderConfig=bson::from_document(doc! {"_id":&provider,"slug":"audit-local-provider","name":"Audit fixture",
        "provider_type":kind,"is_active":true,"created_by":&actor,
        "authorization_url":format!("{}/authorize",server.base),"token_url":format!("{}/token",server.base),
        "device_code_url":format!("{}/device",server.base),"device_token_url":format!("{}/token",server.base),
        "client_id_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:state.encryption_keys.encrypt(b"fixture-client").await.unwrap()},
        "client_secret_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:state.encryption_keys.encrypt(b"never-audit-client-secret").await.unwrap()},
        "supports_pkce":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
    db.collection::<ProviderConfig>("provider_configs")
        .insert_one(p)
        .await
        .unwrap();
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.service_category = "connection".into();
    catalog.requires_user_credential = true;
    catalog.auth_type = Some("api_key".into());
    let service = catalog.id.clone();
    db.collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(catalog)
        .await
        .unwrap();
    if matches!(action, Action::CredentialUpdate | Action::ServiceDisconnect) {
        connection_service::connect_user(
            &db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &sa.id,
            &service,
            Some("previous-credential"),
            None,
        )
        .await
        .unwrap();
    }
    if matches!(action, Action::ProviderDisconnect) {
        user_token_service::store_api_key(
            &db,
            &state.encryption_keys,
            &sa.id,
            &provider,
            CREDENTIAL,
            None,
            None,
        )
        .await
        .unwrap();
    }
    let oauth_state = if matches!(action, Action::DeviceComplete) {
        user_token_service::request_device_code(
            &db,
            &state.encryption_keys,
            &actor,
            &provider,
            Some(&sa.id),
            &[],
            None,
            None,
        )
        .await
        .unwrap()
        .state
    } else {
        String::new()
    };
    security_effect_service::ensure_indexes(&db).await.unwrap();
    Fixture {
        state,
        actor,
        sa: sa.id,
        provider,
        service,
        oauth_state,
        server,
        monitor,
    }
}
fn auth(f: &Fixture) -> crate::mw::auth::AuthUser {
    let mut a = test_auth_user(&f.actor);
    a.ip_address = Some("192.0.2.11".into());
    a.user_agent = Some("SA connection audit fixture".into());
    a.api_key_id = Some("admin-key-id".into());
    a.api_key_name = Some("Administrator key".into());
    a
}
async fn request(f: &Fixture, action: Action) -> axum::response::Response {
    let state = State(f.state.clone());
    let actor = auth(f);
    let ids = Path((f.sa.clone(), f.provider.clone()));
    let services = Path((f.sa.clone(), f.service.clone()));
    let headers = HeaderMap::new();
    match action {
        Action::ApiKey => admin_providers::connect_api_key_for_sa(
            state,
            actor,
            headers,
            ids,
            Json(admin_providers::AdminConnectApiKeyRequest {
                api_key: CREDENTIAL.into(),
                label: Some("Unrecorded label".into()),
            }),
        )
        .await
        .into_response(),
        Action::DeviceComplete => admin_providers::poll_device_code_for_sa(
            state,
            actor,
            headers,
            ids,
            Json(admin_providers::AdminSaDeviceCodePollRequest {
                state: f.oauth_state.clone(),
            }),
        )
        .await
        .into_response(),
        Action::ProviderDisconnect => admin_providers::disconnect_sa_provider(
            state,
            actor,
            crate::telemetry::TelemetryContext::default(),
            headers,
            ids,
            Query(Default::default()),
        )
        .await
        .into_response(),
        Action::OAuthStart => admin_providers::initiate_oauth_for_sa(state, actor, headers, ids)
            .await
            .into_response(),
        Action::DeviceStart => {
            admin_providers::initiate_device_code_for_sa(state, actor, headers, ids)
                .await
                .into_response()
        }
        Action::ServiceConnect => admin_connections::connect_sa_service(
            state,
            actor,
            headers,
            services,
            Json(admin_connections::AdminSaConnectRequest {
                credential: Some(CREDENTIAL.into()),
                credential_label: None,
            }),
        )
        .await
        .into_response(),
        Action::CredentialUpdate => admin_connections::update_sa_connection_credential(
            state,
            actor,
            headers,
            services,
            Json(admin_connections::AdminSaUpdateCredentialRequest {
                credential: CREDENTIAL.into(),
                credential_label: Some("Updated label".into()),
            }),
        )
        .await
        .into_response(),
        Action::ServiceDisconnect => {
            admin_connections::disconnect_sa_service(state, actor, headers, services)
                .await
                .into_response()
        }
    }
}
async fn rows(db: &Database, collection: &str) -> Vec<Document> {
    db.collection::<Document>(collection)
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn committed(f: &Fixture, action: Action) {
    let docs = rows(&f.state.db, action.source()).await;
    assert_eq!(docs.len(), 1, "{action:?}");
    match action {
        Action::ApiKey | Action::DeviceComplete => {
            assert_eq!(docs[0].get_str("status"), Ok("active"))
        }
        Action::ProviderDisconnect => assert_eq!(docs[0].get_str("status"), Ok("revoked")),
        Action::OAuthStart | Action::DeviceStart => {
            assert_eq!(docs[0].get_str("target_user_id"), Ok(f.sa.as_str()))
        }
        Action::ServiceDisconnect => assert_eq!(docs[0].get_bool("is_active"), Ok(false)),
        Action::ServiceConnect | Action::CredentialUpdate => {
            assert_eq!(docs[0].get_bool("is_active"), Ok(true));
            let row: bson::Document = docs[0].clone();
            let row = bson::from_document(row).unwrap();
            assert_eq!(
                crate::services::connection_record_service::text(&f.state.encryption_keys, &row)
                    .await
                    .unwrap()
                    .as_str(),
                CREDENTIAL
            );
        }
    }
}
async fn death(action: Action) {
    let f = fixture(action).await;
    f.monitor.lock().unwrap().stop = Some(action.source());
    let result = std::panic::AssertUnwindSafe(Box::pin(REQUEST.scope((), request(&f, action))))
        .catch_unwind()
        .await;
    assert!(result.unwrap_err().is::<ProcessStopped>());
    committed(&f, action).await;
    assert!(rows(&f.state.db, "audit_log").await.is_empty());
    let intents = rows(&f.state.db, INTENTS).await;
    assert_eq!(
        intents.len(),
        1,
        "{} committed without evidence ({action:?})",
        action.event()
    );
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(INTENTS)
        .insert_one(&intents[0])
        .await
        .unwrap();
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let logs = rows(&f.state.db, "audit_log").await;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].get_str("event_type"), Ok(action.event()));
    assert!(logs[0].get_i64("seq").is_ok());
    assert!(logs[0].get_str("entry_hash").is_ok());
    assert!(rows(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}
macro_rules! death_test {
    ($name:ident,$action:ident) => {
        #[tokio::test]
        async fn $name() {
            death(Action::$action).await;
        }
    };
}
death_test!(api_key_connected_survives_process_death, ApiKey);
death_test!(device_connected_survives_process_death, DeviceComplete);
death_test!(
    provider_disconnected_survives_process_death,
    ProviderDisconnect
);
death_test!(oauth_initiated_survives_process_death, OAuthStart);
death_test!(device_initiated_survives_process_death, DeviceStart);
death_test!(service_connected_survives_process_death, ServiceConnect);
death_test!(credential_updated_survives_process_death, CredentialUpdate);
death_test!(
    service_disconnected_survives_process_death,
    ServiceDisconnect
);

#[tokio::test]
async fn rejected_intent_rolls_back_source_unit() {
    for action in Action::ALL {
        let f = fixture(action).await;
        let before = rows(&f.state.db, action.source()).await;
        f.state
            .db
            .run_command(doc! {"collMod":INTENTS,"validator":{"reject_fixture":{"$exists":true}}})
            .await
            .unwrap();
        let response = request(&f, action).await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{action:?}"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error_code"], 1007, "{action:?}");
        assert_eq!(body["error"], "database_error", "{action:?}");
        assert_eq!(body["message"], "An internal error occurred", "{action:?}");
        assert_eq!(
            rows(&f.state.db, action.source()).await,
            before,
            "{action:?}"
        );
        assert!(rows(&f.state.db, INTENTS).await.is_empty());
        if matches!(action, Action::DeviceComplete) {
            assert_eq!(
                rows(&f.state.db, STATES).await.len(),
                1,
                "failed publication must not consume state"
            );
        }
        f.state.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn admin_command_counts() {
    for action in Action::ALL {
        let f = fixture(action).await;
        let response = REQUEST.scope((), request(&f, action)).await;
        assert!(
            response.status().is_success(),
            "{action:?}: {}",
            response.status()
        );
        let cmds = f.monitor.lock().unwrap().commands.clone();
        let expected = match action {
            Action::ApiKey => 17,
            Action::DeviceComplete => 46,
            Action::ProviderDisconnect => 11,
            Action::OAuthStart | Action::DeviceStart => 8,
            Action::ServiceConnect | Action::CredentialUpdate => 19,
            Action::ServiceDisconnect => 18,
        };
        assert_eq!(cmds.len(), expected, "{action:?}: {cmds:?}");
        println!(
            "ADMIN_CONNECTION_COMMANDS {action:?} count={} commands={cmds:?}",
            cmds.len()
        );
        f.state.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn ordinary_user_command_counts_and_no_required_intents() {
    let f = fixture(Action::ApiKey).await;
    let actions = [
        "service_connect",
        "service_disconnect",
        "provider_connect",
        "provider_disconnect",
    ];
    for action in actions {
        f.monitor.lock().unwrap().commands.clear();
        let result = REQUEST
            .scope((), async {
                match action {
                    "service_connect" => connections::connect_service(
                        State(f.state.clone()),
                        auth(&f),
                        Path(f.service.clone()),
                        Json(connections::ConnectRequest {
                            credential: Some(CREDENTIAL.into()),
                            credential_label: None,
                        }),
                    )
                    .await
                    .into_response(),
                    "service_disconnect" => connections::disconnect_service(
                        State(f.state.clone()),
                        auth(&f),
                        Path(f.service.clone()),
                    )
                    .await
                    .into_response(),
                    "provider_connect" => user_tokens::connect_api_key(
                        State(f.state.clone()),
                        auth(&f),
                        Path(f.provider.clone()),
                        Json(user_tokens::ConnectApiKeyRequest {
                            api_key: CREDENTIAL.into(),
                            label: None,
                            gateway_url: None,
                        }),
                    )
                    .await
                    .into_response(),
                    _ => user_tokens::disconnect_provider(
                        State(f.state.clone()),
                        auth(&f),
                        crate::telemetry::TelemetryContext::default(),
                        Path(f.provider.clone()),
                        Query(Default::default()),
                    )
                    .await
                    .into_response(),
                }
            })
            .await;
        assert!(
            result.status().is_success(),
            "{action}: {}",
            result.status()
        );
        let cmds = f.monitor.lock().unwrap().commands.clone();
        let expected = match action {
            "service_connect" => 10,
            "service_disconnect" => 9,
            "provider_connect" => 12,
            _ => 6,
        };
        assert_eq!(cmds.len(), expected, "{action}: {cmds:?}");
        println!(
            "ORDINARY_CONNECTION_COMMANDS {action} count={} commands={cmds:?}",
            cmds.len()
        );
        assert!(rows(&f.state.db, INTENTS).await.is_empty());
    }
    f.state.db.drop().await.unwrap();
}

fn old_metadata(f: &Fixture, action: Action) -> serde_json::Value {
    // Literal projections from the eight original handler log_for_user calls.
    match action {
        Action::ApiKey => {
            serde_json::json!({"target_sa_id":f.sa,"provider_id":f.provider,"token_type":"api_key"})
        }
        Action::DeviceComplete => {
            serde_json::json!({"target_sa_id":f.sa,"provider_id":f.provider,"token_type":"device_code"})
        }
        Action::ProviderDisconnect | Action::OAuthStart | Action::DeviceStart => {
            serde_json::json!({"target_sa_id":f.sa,"provider_id":f.provider})
        }
        Action::ServiceConnect => {
            serde_json::json!({"target_sa_id":f.sa,"service_id":f.service,"has_credential":true})
        }
        Action::CredentialUpdate | Action::ServiceDisconnect => {
            serde_json::json!({"target_sa_id":f.sa,"service_id":f.service})
        }
    }
}
async fn parity(action: Action) {
    let f = fixture(action).await;
    assert!(request(&f, action).await.status().is_success());
    let intents = rows(&f.state.db, INTENTS).await;
    assert_eq!(intents.len(), 1);
    let encoded = serde_json::to_string(&intents[0]).unwrap();
    for secret in [
        CREDENTIAL,
        ACCESS,
        REFRESH,
        DEVICE,
        USER_CODE,
        "never-audit-client-secret",
    ] {
        assert!(!encoded.contains(secret));
    }
    assert!(!encoded.contains(&f.server.base));
    if !f.oauth_state.is_empty() {
        assert!(!encoded.contains(&f.oauth_state));
    }
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let mut actual = rows(&f.state.db, "audit_log").await.remove(0);
    audit_service::log_for_user(
        f.state.db.clone(),
        &auth(&f),
        action.event(),
        Some(old_metadata(&f, action)),
    );
    let logs = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let logs = rows(&f.state.db, "audit_log").await;
            if logs.len() == 2 {
                break logs;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut old = logs
        .into_iter()
        .find(|row| row.get_str("_id") != actual.get_str("_id"))
        .unwrap();
    for field in ["_id", "created_at", "seq", "prev_hash", "entry_hash"] {
        actual.remove(field);
        old.remove(field);
    }
    assert_eq!(
        actual, old,
        "{action:?}: every immutable field except identity/time must match"
    );
    f.state.db.drop().await.unwrap();
}
macro_rules! parity_test {
    ($name:ident,$action:ident) => {
        #[tokio::test]
        async fn $name() {
            parity(Action::$action).await;
        }
    };
}
parity_test!(api_key_audit_content_parity, ApiKey);
parity_test!(device_complete_audit_content_parity, DeviceComplete);
parity_test!(provider_disconnect_audit_content_parity, ProviderDisconnect);
parity_test!(oauth_start_audit_content_parity, OAuthStart);
parity_test!(device_start_audit_content_parity, DeviceStart);
parity_test!(service_connect_audit_content_parity, ServiceConnect);
parity_test!(credential_update_audit_content_parity, CredentialUpdate);
parity_test!(service_disconnect_audit_content_parity, ServiceDisconnect);

#[tokio::test]
async fn occurrence_maxima_and_utf8_caps() {
    use super::service_account_connection_audit::{Event, intent};
    use crate::models::security_effect_intent::SecurityEffect;
    let huge = "🦀".repeat(5000);
    let actor = audit_service::AuditActor {
        user_id: huge.clone(),
        ip_address: Some(huge.clone()),
        user_agent: Some(huge.clone()),
        api_key_id: Some(huge.clone()),
        api_key_name: Some(huge.clone()),
    };
    for event in [
        Event::ApiKeyConnected,
        Event::DeviceConnected,
        Event::ProviderDisconnected,
        Event::OAuthInitiated,
        Event::DeviceInitiated,
        Event::ServiceConnected {
            has_credential: true,
        },
        Event::CredentialUpdated,
        Event::ServiceDisconnected,
    ] {
        let SecurityEffect::Audit { event: occurrence } =
            intent(&actor, &huge, &huge, event).unwrap().effect
        else {
            panic!("audit")
        };
        let bytes = bson::to_vec(&occurrence).unwrap().len();
        println!(
            "SA_CONNECTION_OCCURRENCE_MAX {} bytes={bytes} margin={}",
            occurrence.event_type,
            16384 - bytes
        );
        assert!(bytes < 16384);
        assert!(
            occurrence
                .user_id
                .as_ref()
                .unwrap()
                .ends_with("...[truncated]")
        );
        assert!(
            occurrence
                .user_agent
                .as_ref()
                .unwrap()
                .ends_with("...[truncated]")
        );
        // ASCII fills every cap exactly (UTF-8 input can finish up to 3 bytes short).
        let fill = "x".repeat(20_000);
        let actor = audit_service::AuditActor {
            user_id: fill.clone(),
            ip_address: Some(fill.clone()),
            user_agent: Some(fill.clone()),
            api_key_id: Some(fill.clone()),
            api_key_name: Some(fill.clone()),
        };
        let SecurityEffect::Audit { event: max } =
            intent(&actor, &fill, &fill, event).unwrap().effect
        else {
            panic!("audit")
        };
        let bytes = bson::to_vec(&max).unwrap().len();
        println!(
            "SA_CONNECTION_OCCURRENCE_ASCII_MAX {} bytes={bytes} margin={}",
            max.event_type,
            16384 - bytes
        );
        assert!(bytes < 16384);
        let context = crate::turn_trace::TurnContext::for_turn(&Uuid::new_v4().to_string());
        let traced = crate::turn_trace::scope(context, async {
            intent(&actor, &fill, &fill, event).unwrap()
        })
        .await;
        let SecurityEffect::Audit { event: traced } = traced.effect else {
            panic!("audit")
        };
        let traced_bytes = bson::to_vec(&traced).unwrap().len();
        assert_eq!(traced_bytes, bytes + 192);
        println!(
            "SA_CONNECTION_OCCURRENCE_WITH_TRACE {} bytes={traced_bytes} margin={}",
            traced.event_type,
            16384 - traced_bytes
        );
    }
}

#[tokio::test]
async fn provider_failure_or_death_before_local_commit_has_no_success_event() {
    for action in [Action::DeviceStart, Action::DeviceComplete] {
        let f = fixture(action).await;
        f.server.control.fail.store(true, Ordering::SeqCst);
        assert!(
            !REQUEST
                .scope((), request(&f, action))
                .await
                .status()
                .is_success()
        );
        assert!(rows(&f.state.db, INTENTS).await.is_empty());
        assert!(rows(&f.state.db, TOKENS).await.is_empty());
        f.state.db.drop().await.unwrap();
        let f = fixture(action).await;
        f.server.control.hold.store(true, Ordering::SeqCst);
        let mut request = Box::pin(REQUEST.scope((), request(&f, action)));
        // Fixture setup may have signalled this Notify. Consume it before polling.
        let _ = f.server.control.entered.notified().now_or_never();
        tokio::select! {
            _=f.server.control.entered.notified()=>{},
            response=&mut request=>panic!("completed before provider barrier: {}",response.status()),
        }
        drop(request);
        f.server.control.release.notify_one();
        assert!(rows(&f.state.db, INTENTS).await.is_empty());
        assert!(rows(&f.state.db, TOKENS).await.is_empty());
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn missing_and_inactive_connections_emit_nothing() {
    for action in [
        Action::CredentialUpdate,
        Action::ServiceDisconnect,
        Action::ProviderDisconnect,
    ] {
        let f = fixture(action).await;
        f.state
            .db
            .collection::<Document>(action.source())
            .delete_many(doc! {})
            .await
            .unwrap();
        let response = request(&f, action).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{action:?}");
        assert!(rows(&f.state.db, INTENTS).await.is_empty());
        f.state.db.drop().await.unwrap();
    }
    let f = fixture(Action::ServiceDisconnect).await;
    assert!(
        request(&f, Action::ServiceDisconnect)
            .await
            .status()
            .is_success()
    );
    let before = rows(&f.state.db, INTENTS).await;
    assert_eq!(
        request(&f, Action::ServiceDisconnect).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(rows(&f.state.db, INTENTS).await, before);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_connection_commit_noop_or_stale_snapshot_emits_nothing() {
    use crate::models::user_service_connection::UserServiceConnection;
    let f = fixture(Action::CredentialUpdate).await;
    let raw = rows(&f.state.db, CONNECTIONS).await.remove(0);
    let row: UserServiceConnection = bson::from_document(raw.clone()).unwrap();
    super::connection_record_service::commit(&f.state.db, Some(&raw), &row)
        .await
        .unwrap();
    let raw = rows(&f.state.db, CONNECTIONS).await.remove(0);
    let intent = super::service_account_connection_audit::intent(
        &audit_service::AuditActor::from_auth_user(&auth(&f)),
        &f.sa,
        &f.service,
        super::service_account_connection_audit::Event::CredentialUpdated,
    )
    .unwrap();
    super::connection_record_service::commit_with_audit(
        &f.state.db,
        Some(&raw),
        &row,
        Some(&intent),
    )
    .await
    .unwrap();
    assert!(rows(&f.state.db, INTENTS).await.is_empty());
    f.state
        .db
        .collection::<Document>(CONNECTIONS)
        .update_one(doc! {"_id":&row.id}, doc! {"$inc":{"state_version":1}})
        .await
        .unwrap();
    assert!(matches!(
        super::connection_record_service::commit_with_audit(
            &f.state.db,
            Some(&raw),
            &row,
            Some(&intent)
        )
        .await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    assert!(rows(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn provider_disconnect_later_cleanup_error_keeps_committed_audit() {
    use crate::models::user_provider_token::UserProviderToken;
    let f = fixture(Action::ProviderDisconnect).await;
    let token: UserProviderToken =
        bson::from_document(rows(&f.state.db, TOKENS).await.remove(0)).unwrap();
    let key = super::user_api_key_service::create_api_key_from_provider_token(
        &f.state.db,
        &f.state.encryption_keys,
        &f.sa,
        "fixture copy",
        &f.provider,
        &token,
    )
    .await
    .unwrap();
    f.state
        .db
        .run_command(doc! {"collMod":"user_api_keys","validator":{"status":{"$ne":"revoked"}}})
        .await
        .unwrap();
    assert_eq!(
        request(&f, Action::ProviderDisconnect).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    committed(&f, Action::ProviderDisconnect).await;
    assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
    let unchanged = f
        .state
        .db
        .collection::<Document>("user_api_keys")
        .find_one(doc! {"_id":key.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.get_str("status"), Ok("active"));
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_eq!(
        rows(&f.state.db, "audit_log")
            .await
            .iter()
            .filter(|r| r.get_str("event_type") == Ok(Action::ProviderDisconnect.event()))
            .count(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn api_key_replacement_rejected_intent_keeps_previous_material() {
    let f = fixture(Action::ProviderDisconnect).await;
    let before = rows(&f.state.db, TOKENS).await;
    f.state
        .db
        .run_command(doc! {"collMod":INTENTS,"validator":{"reject_fixture":{"$exists":true}}})
        .await
        .unwrap();
    assert_eq!(
        request(&f, Action::ApiKey).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(rows(&f.state.db, TOKENS).await, before);
    assert!(rows(&f.state.db, INTENTS).await.is_empty());
    f.state
        .db
        .run_command(doc! {"collMod":INTENTS,"validator":{}})
        .await
        .unwrap();
    assert!(request(&f, Action::ApiKey).await.status().is_success());
    let after = rows(&f.state.db, TOKENS).await;
    assert_eq!(after[0].get_str("_id"), before[0].get_str("_id"));
    assert_eq!(
        after[0].get_i64("state_version").unwrap(),
        before[0].get_i64("state_version").unwrap() + 1
    );
    assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn idle_relay_is_one_indexed_read() {
    let f = fixture(Action::ApiKey).await;
    REQUEST
        .scope(
            (),
            security_effect_service::sweep(&f.state.db, &f.state.encryption_keys),
        )
        .await
        .unwrap();
    let commands = f.monitor.lock().unwrap().commands.clone();
    assert_eq!(commands, vec![("find".into(), INTENTS.into())]);
    println!("SA_CONNECTION_IDLE_RELAY count=1 commands={commands:?}");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn device_poll_target_mismatch_is_read_only_then_correct_url_completes() {
    let mut f = fixture(Action::DeviceComplete).await;
    let target = f.sa.clone();
    let (other, _) = service_account_service::create_service_account(
        &f.state.db,
        "Other SA",
        None,
        "proxy",
        &[],
        None,
        &f.actor,
    )
    .await
    .unwrap();
    f.sa = other.id;
    let before = rows(&f.state.db, STATES).await;
    f.server.control.requests.store(0, Ordering::SeqCst);
    let response = REQUEST.scope((), request(&f, Action::DeviceComplete)).await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "mismatched URL must not install the state's credentials"
    );
    assert_eq!(f.server.control.requests.load(Ordering::SeqCst), 0);
    assert_eq!(rows(&f.state.db, STATES).await, before);
    assert!(rows(&f.state.db, TOKENS).await.is_empty());
    assert!(rows(&f.state.db, INTENTS).await.is_empty());
    let commands = f.monitor.lock().unwrap().commands.clone();
    assert_eq!(commands.len(), 5);
    assert!(commands.iter().all(|(command, _)| command == "find"));
    f.sa = target;
    f.monitor.lock().unwrap().commands.clear();
    assert!(
        REQUEST
            .scope((), request(&f, Action::DeviceComplete))
            .await
            .status()
            .is_success()
    );
    assert_eq!(f.monitor.lock().unwrap().commands.len(), 46);
    assert_eq!(f.server.control.requests.load(Ordering::SeqCst), 1);
    committed(&f, Action::DeviceComplete).await;
    assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
    assert!(rows(&f.state.db, STATES).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn device_poll_target_mismatch_does_not_even_delete_an_expired_state() {
    let mut f = fixture(Action::DeviceComplete).await;
    let (other, _) = service_account_service::create_service_account(
        &f.state.db,
        "Other SA",
        None,
        "proxy",
        &[],
        None,
        &f.actor,
    )
    .await
    .unwrap();
    f.sa = other.id;
    f.state
        .db
        .collection::<Document>(STATES)
        .update_one(
            doc! {"_id":&f.oauth_state},
            doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let before = rows(&f.state.db, STATES).await;
    f.server.control.requests.store(0, Ordering::SeqCst);
    assert_eq!(
        request(&f, Action::DeviceComplete).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(f.server.control.requests.load(Ordering::SeqCst), 0);
    assert_eq!(rows(&f.state.db, STATES).await, before);
    assert!(rows(&f.state.db, TOKENS).await.is_empty());
    assert!(rows(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}
