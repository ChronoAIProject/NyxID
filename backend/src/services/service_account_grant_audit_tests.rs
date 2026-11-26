use super::*;
use crate::{
    handlers::{admin_service_accounts as curation, service_account_key_reads as key_reads},
    models::{
        service_account_key_read_grant::{
            COLLECTION_NAME as GRANTS, KeyReadTarget, ServiceAccountKeyReadGrant,
        },
        user::UserType,
    },
    services::{
        audit_service, security_effect_service, service_account_key_read_service as reads,
        service_account_service as accounts,
    },
    test_utils::{self, test_app_state, test_auth_user, test_user},
};
use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use futures::{FutureExt, TryStreamExt};
use mongodb::event::{EventHandler, command::CommandEvent};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
const INTENTS: &str = "security_effect_intents";
tokio::task_local! { static REQUEST: (); }
#[derive(Clone, Copy, Debug)]
enum Action {
    CurationIssue,
    CurationRevoke,
    KeyIssue,
    KeyRevoke,
}
impl Action {
    const ALL: [Self; 4] = [
        Self::CurationIssue,
        Self::CurationRevoke,
        Self::KeyIssue,
        Self::KeyRevoke,
    ];
    fn event(self) -> &'static str {
        match self {
            Self::CurationIssue => "admin.sa.curation_granted",
            Self::CurationRevoke => "admin.sa.curation_revoked",
            Self::KeyIssue => "admin.sa.key_read_granted",
            Self::KeyRevoke => "admin.sa.key_read_revoked",
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
    catalog: String,
    connection: String,
    previous: ServiceAccountKeyReadGrant,
    monitor: Arc<Mutex<Monitor>>,
}
async fn fixture() -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "sa_grant_audit",
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
                        && matches!(collection.as_str(), ACCOUNTS | GRANTS)
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
    db.collection::<User>(USERS)
        .insert_one(admin)
        .await
        .unwrap();
    let (mut sa, secret) = accounts::create_service_account(
        &db,
        "Grant fixture",
        Some("Never copy this description"),
        "catalog:skills:read catalog:skills:write user-services:read",
        &[],
        None,
        &actor,
    )
    .await
    .unwrap();
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.created_by = actor.clone();
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
    let endpoint = Uuid::new_v4().to_string();
    let connection = Uuid::new_v4().to_string();
    db.collection::<bson::Document>("user_endpoints")
        .insert_one(
            bson::to_document(&test_utils::test_user_endpoint(
                &endpoint,
                &actor,
                "Secret-free metadata",
                "https://example.invalid/?secret=do-not-copy",
                None,
                None,
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    db.collection::<bson::Document>("user_services")
        .insert_one(
            bson::to_document(&test_utils::test_user_service(
                &connection,
                &actor,
                "fixture",
                &endpoint,
                None,
                None,
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    let now = Utc::now();
    sa.purpose = ServiceAccountPurpose::Curation;
    sa.platform_protected = true;
    sa.curation_grant = Some(CurationGrant {
        id: Uuid::new_v4().to_string(),
        service_ids: vec![service.id.clone()],
        ornn_proxy_service_id: None,
        issued_by: actor.clone(),
        issued_at: now,
        expires_at: Some(now + chrono::Duration::hours(2)),
        max_writes: 50,
        window_seconds: 3600,
        window_started_at: now,
        writes_used: 7,
    });
    db.collection::<ServiceAccount>(ACCOUNTS)
        .replace_one(doc! {"_id":&sa.id}, &sa)
        .await
        .unwrap();
    let previous = ServiceAccountKeyReadGrant {
        service_account_id: sa.id.clone(),
        owner_id: actor.clone(),
        targets: vec![KeyReadTarget {
            user_service_id: connection.clone(),
            owner_id: actor.clone(),
        }],
        issued_by: actor.clone(),
        issued_at: now,
        expires_at: Some(now + chrono::Duration::hours(2)),
    };
    db.collection::<ServiceAccountKeyReadGrant>(GRANTS)
        .insert_one(&previous)
        .await
        .unwrap();
    security_effect_service::ensure_indexes(&db).await.unwrap();
    Fixture {
        state: test_app_state(db),
        actor,
        sa,
        secret,
        catalog: service.id,
        connection,
        previous,
        monitor,
    }
}
fn auth(f: &Fixture) -> crate::mw::auth::AuthUser {
    let mut auth = test_auth_user(&f.actor);
    auth.ip_address = Some("192.0.2.10".into());
    auth.user_agent = Some("Grant audit fixture".into());
    auth.api_key_id = Some("fixture-key".into());
    auth.api_key_name = Some("Fixture administrator".into());
    auth
}
fn input(f: &Fixture) -> IssueCurationGrant {
    IssueCurationGrant {
        service_ids: vec![f.catalog.clone()],
        ornn_proxy_service_id: None,
        expires_at: Some(Utc::now() + chrono::Duration::hours(3)),
        max_writes: 100,
        window_seconds: 3600,
    }
}
async fn request(f: &Fixture, action: Action) -> axum::response::Response {
    let state = State(f.state.clone());
    let auth = auth(f);
    let id = Path(f.sa.id.clone());
    match action {
        Action::CurationIssue => curation::issue_curation_grant(state, auth, id, Json(input(f)))
            .await
            .into_response(),
        Action::CurationRevoke => curation::revoke_curation_grant(state, auth, id)
            .await
            .into_response(),
        Action::KeyIssue => key_reads::issue_grant(
            state,
            auth,
            id,
            Json(key_reads::GrantRequest {
                user_service_ids: vec![f.connection.clone()],
                expires_at: Some(Utc::now() + chrono::Duration::hours(3)),
            }),
        )
        .await
        .into_response(),
        Action::KeyRevoke => key_reads::revoke_grant(state, auth, id)
            .await
            .into_response(),
    }
}
async fn rows(db: &Database, name: &str) -> Vec<bson::Document> {
    db.collection::<bson::Document>(name)
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
    let sa = accounts::get_service_account(db, &f.sa.id).await.unwrap();
    let grant = reads::get_grant(db, &f.sa.id).await.unwrap();
    match action {
        Action::CurationIssue => {
            let g = sa.curation_grant.unwrap();
            assert_eq!(g.max_writes, 100);
            assert_eq!(g.writes_used, 0);
            assert_ne!(g.id, f.sa.curation_grant.as_ref().unwrap().id);
        }
        Action::CurationRevoke => assert!(sa.curation_grant.is_none()),
        Action::KeyIssue => assert_ne!(grant.unwrap().expires_at, f.previous.expires_at),
        Action::KeyRevoke => assert!(grant.is_none()),
    }
    assert!(rows(db, "audit_log").await.is_empty());
    let intents = rows(db, INTENTS).await;
    assert_eq!(
        intents.len(),
        1,
        "{} committed without evidence",
        action.event()
    );
    assert!(!intents[0].to_string().contains(&f.secret));
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
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
death_test!(curation_granted_survives_process_death, CurationIssue);
death_test!(curation_revoked_survives_process_death, CurationRevoke);
death_test!(key_read_granted_survives_process_death, KeyIssue);
death_test!(key_read_revoked_survives_process_death, KeyRevoke);
#[tokio::test]
async fn grant_command_counts() {
    for action in Action::ALL {
        let f = fixture().await;
        let result = REQUEST.scope((), request(&f, action)).await;
        assert!(
            result.status().is_success(),
            "{action:?}: {}",
            result.status()
        );
        let commands = f.monitor.lock().unwrap().commands.clone();
        assert_eq!(
            commands.len(),
            match action {
                Action::CurationIssue => 9,
                Action::CurationRevoke => 7,
                Action::KeyIssue => 24,
                Action::KeyRevoke => 7,
            },
            "{action:?}: {commands:?}"
        );
        println!(
            "GRANT_COMMANDS {action:?} count={} commands={commands:?}",
            commands.len()
        );
        f.state.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn rejected_intent_rolls_back_every_grant_source() {
    for action in Action::ALL {
        let f = fixture().await;
        let db = &f.state.db;
        let mut before = Vec::new();
        for c in [ACCOUNTS, GRANTS, USERS, "personal_owner_write_fences"] {
            before.push((c, rows(db, c).await));
        }
        db.run_command(doc! {"collMod":INTENTS,"validator":{"reject_fixture":{"$exists":true}}})
            .await
            .unwrap();
        assert_eq!(
            request(&f, action).await.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "{action:?}"
        );
        for (c, old) in before {
            assert_eq!(rows(db, c).await, old, "{action:?}: {c}");
        }
        assert!(rows(db, INTENTS).await.is_empty());
        db.drop().await.unwrap();
    }
}

#[path = "service_account_grant_audit_cases.rs"]
mod cases;
