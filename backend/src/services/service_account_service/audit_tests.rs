use super::*;
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
};
use futures::FutureExt;
use mongodb::event::{EventHandler, command::CommandEvent};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use crate::{
    handlers::admin_service_accounts as handlers,
    models::user::{User, UserType},
    services::{audit_service, security_effect_service},
    test_utils::{test_app_state, test_auth_user, test_user},
};

const INTENTS: &str = "security_effect_intents";
tokio::task_local! { static REQUEST: (); }

#[derive(Clone, Copy, Debug)]
enum Action {
    Create,
    Update,
    Delete,
    Rotate,
    Revoke,
}
impl Action {
    const ALL: [Self; 5] = [
        Self::Create,
        Self::Update,
        Self::Delete,
        Self::Rotate,
        Self::Revoke,
    ];
    fn event(self) -> &'static str {
        match self {
            Self::Create => "admin.sa.created",
            Self::Update => "admin.sa.updated",
            Self::Delete => "admin.sa.deleted",
            Self::Rotate => "admin.sa.secret_rotated",
            Self::Revoke => "admin.sa.tokens_revoked",
        }
    }
}
#[derive(Debug)]
struct ProcessStopped;
#[derive(Default)]
struct Monitor {
    stop: bool,
    source_transactions: HashSet<String>,
    stop_requests: HashSet<i32>,
    commands: Vec<(String, String)>,
    transactions: HashMap<i32, String>,
}
struct Fixture {
    state: crate::AppState,
    actor: String,
    sa: ServiceAccount,
    secret: String,
    created: String,
    monitor: Arc<Mutex<Monitor>>,
}
async fn fixture() -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "sa_required_audit",
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
                    if let Some(key) = key.as_ref() {
                        m.transactions.insert(e.request_id, key.clone());
                    }
                    if m.stop
                        && collection == SERVICE_ACCOUNTS
                        && matches!(
                            e.command_name.as_str(),
                            "insert" | "update" | "findAndModify"
                        )
                    {
                        if let Some(key) = key {
                            m.source_transactions.insert(key);
                        } else {
                            m.stop_requests.insert(e.request_id);
                        }
                    } else if m.stop
                        && e.command_name == "commitTransaction"
                        && key
                            .as_ref()
                            .is_some_and(|k| m.source_transactions.contains(k))
                    {
                        m.stop_requests.insert(e.request_id);
                    }
                }
                CommandEvent::Succeeded(e) => {
                    m.transactions.remove(&e.request_id);
                    if m.stop && m.stop_requests.remove(&e.request_id) {
                        m.stop = false;
                        drop(m);
                        std::panic::resume_unwind(Box::new(ProcessStopped));
                    }
                }
                CommandEvent::Failed(e) => {
                    m.transactions.remove(&e.request_id);
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
    let mut admin = test_user(&actor, UserType::Person);
    admin.role_ids = vec![roles.admin];
    db.collection::<User>("users")
        .insert_one(admin)
        .await
        .unwrap();
    let (sa, secret) = create_service_account(
        &db,
        "Existing account",
        Some("Never copy this description"),
        "proxy",
        &[],
        None,
        &actor,
    )
    .await
    .unwrap();
    db.collection::<bson::Document>(SA_TOKENS).insert_many((0..3).map(|i| doc!{
        "_id":Uuid::new_v4().to_string(),"jti":Uuid::new_v4().to_string(),"service_account_id":&sa.id,
        "scope":"proxy","revoked":i==2,"credential_generation":0_i64,
        "created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(1))
    })).await.unwrap();
    security_effect_service::ensure_indexes(&db).await.unwrap();
    Fixture {
        state: test_app_state(db),
        actor,
        sa,
        secret,
        created: Uuid::new_v4().to_string(),
        monitor,
    }
}
fn auth(f: &Fixture) -> crate::mw::auth::AuthUser {
    let mut auth = test_auth_user(&f.actor);
    auth.ip_address = Some("192.0.2.9".into());
    auth.user_agent = Some("Audit parity fixture".into());
    auth.api_key_id = Some("fixture-key".into());
    auth.api_key_name = Some("Fixture administrator".into());
    auth
}
async fn request(f: &Fixture, action: Action) -> axum::response::Response {
    let state = State(f.state.clone());
    let auth = auth(f);
    let headers = HeaderMap::new();
    let id = Path(f.sa.id.clone());
    let tele = crate::telemetry::TelemetryContext::default();
    match action {
        Action::Create => handlers::create_service_account_with_id(
            state,
            auth,
            tele,
            headers,
            Json(handlers::CreateServiceAccountRequest {
                name: "Created account".into(),
                description: None,
                allowed_scopes: "proxy".into(),
                role_ids: None,
                rate_limit_override: None,
                target_org_id: None,
            }),
            Some(&f.created),
        )
        .await
        .into_response(),
        Action::Update => handlers::update_service_account(
            state,
            auth,
            headers,
            id,
            Json(handlers::UpdateServiceAccountRequest {
                name: Some("Changed account".into()),
                description: None,
                allowed_scopes: None,
                role_ids: None,
                rate_limit_override: None,
                is_active: None,
                expected_access: None,
            }),
        )
        .await
        .into_response(),
        Action::Delete => handlers::delete_service_account(state, auth, tele, headers, id)
            .await
            .into_response(),
        Action::Rotate => handlers::rotate_secret(state, auth, tele, headers, id)
            .await
            .into_response(),
        Action::Revoke => handlers::revoke_tokens(state, auth, headers, id)
            .await
            .into_response(),
    }
}
async fn rows(db: &Database, collection: &str) -> Vec<bson::Document> {
    db.collection::<bson::Document>(collection)
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn crash(action: Action) {
    let f = fixture().await;
    f.monitor.lock().unwrap().stop = true;
    let result = std::panic::AssertUnwindSafe(Box::pin(REQUEST.scope((), request(&f, action))))
        .catch_unwind()
        .await;
    assert!(result.unwrap_err().is::<ProcessStopped>());
    let db = &f.state.db;
    let sa = get_service_account(
        db,
        if matches!(action, Action::Create) {
            &f.created
        } else {
            &f.sa.id
        },
    )
    .await
    .unwrap();
    match action {
        Action::Create => assert_eq!(sa.name, "Created account"),
        Action::Update => assert_eq!(sa.name, "Changed account"),
        Action::Delete => {
            assert!(!sa.is_active);
            assert_eq!(sa.credential_generation, 1);
        }
        Action::Rotate => {
            assert_ne!(sa.client_secret_hash, f.sa.client_secret_hash);
            assert_eq!(sa.credential_generation, 1);
        }
        Action::Revoke => assert_eq!(sa.credential_generation, 1),
    }
    assert!(rows(db, "audit_log").await.is_empty());
    let intents = rows(db, INTENTS).await;
    assert_eq!(
        intents.len(),
        1,
        "{} source committed without durable audit",
        action.event()
    );
    assert!(!intents[0].to_string().contains(&f.secret));
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    // Replay the exact durable UUID after the receipt deletion was lost.
    db.collection::<bson::Document>(INTENTS)
        .insert_one(&intents[0])
        .await
        .unwrap();
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    let logs = rows(db, "audit_log").await;
    assert_eq!(logs.len(), 1);
    assert!(logs[0].get_i64("seq").is_ok());
    assert!(logs[0].get_str("entry_hash").is_ok());
    assert!(rows(db, INTENTS).await.is_empty());
    db.drop().await.unwrap();
}
macro_rules! death_test {
    ($name:ident,$action:ident) => {
        #[tokio::test]
        async fn $name() {
            crash(Action::$action).await;
        }
    };
}
death_test!(sa_created_survives_process_death, Create);
death_test!(sa_updated_survives_process_death, Update);
death_test!(sa_deleted_survives_process_death, Delete);
death_test!(sa_rotated_survives_process_death, Rotate);
death_test!(sa_revoked_survives_process_death, Revoke);

#[tokio::test]
async fn sa_audit_command_counts() {
    for action in Action::ALL {
        let f = fixture().await;
        let started = std::time::Instant::now();
        let response = REQUEST.scope((), request(&f, action)).await;
        assert!(
            response.status().is_success(),
            "{action:?}: {}",
            response.status()
        );
        let commands = f.monitor.lock().unwrap().commands.clone();
        assert_eq!(
            commands.len(),
            match action {
                Action::Create => 14,
                Action::Update | Action::Delete => 12,
                Action::Rotate => 13,
                Action::Revoke => 11,
            },
            "{action:?}: {commands:?}"
        );
        println!(
            "SA_COMMANDS {action:?} count={} elapsed_us={} commands={commands:?}",
            commands.len(),
            started.elapsed().as_micros()
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn sa_idle_relay_is_one_read() {
    let f = fixture().await;
    REQUEST
        .scope(
            (),
            security_effect_service::sweep(&f.state.db, &f.state.encryption_keys),
        )
        .await
        .unwrap();
    assert_eq!(
        f.monitor.lock().unwrap().commands,
        vec![("find".into(), INTENTS.into())]
    );
    println!("SA_IDLE_RELAY 1 find, 0 writes");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn sa_rejected_intent_rolls_back_effective_change() {
    for action in Action::ALL {
        let f = fixture().await;
        let db = &f.state.db;
        let owners = rows(db, "users").await;
        let fences = rows(db, "personal_owner_write_fences").await;
        let accounts = rows(db, SERVICE_ACCOUNTS).await;
        let tokens = rows(db, SA_TOKENS).await;
        db.run_command(doc! {"collMod":INTENTS,"validator":{"rejected_fixture":{"$exists":true}}})
            .await
            .unwrap();
        let response = request(&f, action).await;
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "{action:?}"
        );
        assert_eq!(rows(db, "users").await, owners);
        assert_eq!(rows(db, "personal_owner_write_fences").await, fences);
        assert_eq!(rows(db, SERVICE_ACCOUNTS).await, accounts);
        assert_eq!(rows(db, SA_TOKENS).await, tokens);
        assert!(rows(db, INTENTS).await.is_empty());
        db.drop().await.unwrap();
    }
}

#[path = "audit_tests_cases.rs"]
mod cases;
#[path = "audit_tests_scale.rs"]
mod scale;
