//! Real owner handlers, their source commit, and the required-audit relay.
use crate::handlers::credential_record_fixtures::KeySecret;
use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use bson::{Document, doc};
use chrono::{Duration, Utc};
use futures::{FutureExt, TryStreamExt};
use mongodb::event::{EventHandler, command::CommandEvent};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use crate::{
    AppState,
    models::{durable_operation_grant::DurableOperationSelection, user::UserType},
    mw::auth::AuthUser,
    services::{
        api_key_scope_service, key_service, permission_policy_service, security_effect_service,
    },
    test_utils::*,
};

const INTENTS: &str = "security_effect_intents";
tokio::task_local! { static REQUEST: (); }

#[derive(Clone, Copy, Debug)]
enum Case {
    Child,
    Rotate,
    Delete,
    Operations,
    Provision,
    Grant,
    Create,
    Pause,
    Revoke,
}
const CASES: [Case; 9] = [
    Case::Child,
    Case::Rotate,
    Case::Delete,
    Case::Operations,
    Case::Provision,
    Case::Grant,
    Case::Create,
    Case::Pause,
    Case::Revoke,
];

impl Case {
    fn event(self) -> &'static str {
        match self {
            Self::Child | Self::Rotate | Self::Delete => "agent_key_credential_revoked",
            Self::Operations => "api_key_operations_changed",
            Self::Provision => "durable_grants_provisioned",
            Self::Grant => "durable_grant_revoked",
            Self::Create => "permission_key_created",
            Self::Pause => "permission_key_pause_changed",
            Self::Revoke => "permission_key_revoked",
        }
    }
    fn source(self, command: &str, collection: &str, body: &Document) -> bool {
        match self {
            Self::Child | Self::Rotate | Self::Delete => {
                command == "findAndModify" && collection == "api_key_credentials"
            }
            Self::Operations | Self::Revoke => command == "update" && collection == "api_keys",
            Self::Provision => {
                command == "update"
                    && collection == "api_keys"
                    && body.get_array("updates").is_ok_and(|updates| {
                        updates.iter().any(|update| {
                            update
                                .as_document()
                                .and_then(|u| u.get_document("u").ok())
                                .and_then(|u| u.get_document("$set").ok())
                                .is_some_and(|u| u.get_bool("scheduled_write_enabled") == Ok(true))
                        })
                    })
            }
            Self::Grant => command == "findAndModify" && collection == "durable_operation_grants",
            Self::Create => command == "insert" && collection == "permission_key_policies",
            Self::Pause => command == "findAndModify" && collection == "permission_key_policies",
        }
    }
    fn count(self) -> usize {
        if matches!(self, Self::Rotate | Self::Delete) {
            3
        } else {
            1
        }
    }
}

#[derive(Default)]
struct Monitor {
    commands: Vec<(String, String)>,
    stop: bool,
    stop_source: Option<fn(&str, &str, &Document) -> bool>,
    source: Option<(i32, bool)>,
    commit: bool,
    transaction_starts: usize,
    transaction_started_at: Option<std::time::Instant>,
    transaction_durations: Vec<std::time::Duration>,
    transaction_write_bytes: usize,
    largest_transaction_write: usize,
}
#[derive(Debug)]
struct ProcessStopped;
struct Fixture {
    case: Case,
    state: AppState,
    owner: String,
    connection: String,
    endpoint: String,
    key: String,
    children: Vec<String>,
    grant: String,
    create_body: Value,
    monitor: Arc<Mutex<Monitor>>,
}

async fn rows(db: &mongodb::Database, collection: &str) -> Vec<Document> {
    db.collection::<Document>(collection)
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn cleanup(db: &mongodb::Database) {
    crate::services::security_readiness_service::stop_test_replica(db).await;
    db.drop().await.unwrap();
}

async fn fixture(case: Case) -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = connect_transaction_test_database_with_command_handler(
        "key_authority_audit",
        EventHandler::callback(move |event| {
            if REQUEST.try_with(|_| ()).is_err() {
                return;
            }
            let mut m = seen.lock().unwrap();
            match event {
                CommandEvent::Started(event) => {
                    let collection = event
                        .command
                        .get_str(&event.command_name)
                        .unwrap_or("")
                        .to_owned();
                    if m.stop
                        && m.stop_source.map_or_else(
                            || case.source(&event.command_name, &collection, &event.command),
                            |source| source(&event.command_name, &collection, &event.command),
                        )
                    {
                        m.source = Some((
                            event.request_id,
                            event.command.get_bool("autocommit") == Ok(false),
                        ));
                    }
                    if event.command.get_bool("startTransaction") == Ok(true) {
                        m.transaction_starts += 1;
                        m.transaction_started_at = Some(std::time::Instant::now());
                    }
                    if event.command.get_bool("autocommit") == Ok(false)
                        && ["insert", "update", "delete", "findAndModify"]
                            .contains(&event.command_name.as_str())
                    {
                        let bytes = bson::to_vec(&event.command).unwrap().len();
                        m.transaction_write_bytes += bytes;
                        m.largest_transaction_write = m.largest_transaction_write.max(bytes);
                    }
                    m.commands.push((event.command_name, collection));
                }
                CommandEvent::Succeeded(event) => {
                    if event.command_name == "commitTransaction"
                        && let Some(started) = m.transaction_started_at.take()
                    {
                        m.transaction_durations.push(started.elapsed());
                    }
                    if !m.stop {
                        return;
                    }
                    let source = m.source.filter(|(id, _)| *id == event.request_id);
                    if source.is_some_and(|(_, tx)| tx) {
                        m.commit = true;
                    }
                    if source.is_some_and(|(_, tx)| !tx)
                        || (m.commit && event.command_name == "commitTransaction")
                    {
                        m.stop = false;
                        drop(m);
                        std::panic::resume_unwind(Box::new(ProcessStopped));
                    }
                }
                _ => {}
            }
        }),
    )
    .await;
    let state = test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    set_agent_operation_scopes_enabled(&db, &owner, true).await;
    let mut catalog = test_auto_connected_catalog_service();
    catalog.slug = "api-google-drive".into();
    catalog.base_url = "https://www.googleapis.com".into();
    catalog.auth_method = "bearer".into();
    catalog.requires_user_credential = true;
    db.collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&catalog)
        .await
        .unwrap();
    let ep = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(test_user_endpoint(
            &ep,
            &owner,
            "Fixture",
            &catalog.base_url,
            None,
            Some(&catalog.id),
        ))
        .await
        .unwrap();
    let credential = uuid::Uuid::new_v4().to_string();
    KeySecret::AccessToken
        .insert(
            &db,
            &state.encryption_keys,
            doc! {
                "_id": &credential,
                "user_id": &owner,
                "label": "Fixture",
                "credential_type": "oauth2",
                "status": "active",
                "credential_epoch": 1_i64,
                "created_at": bson::DateTime::now(),
                "updated_at": bson::DateTime::now(),
            },
            b"fixture-private-provider-token",
        )
        .await;
    let connection = uuid::Uuid::new_v4().to_string();
    let mut service = test_user_service(
        &connection,
        &owner,
        "fixture-drive",
        &ep,
        Some(&catalog.id),
        None,
    );
    service.api_key_id = Some(credential);
    service.auth_method = "bearer".into();
    service.auth_key_name = "Authorization".into();
    db.collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(service)
        .await
        .unwrap();
    let mut endpoint = test_service_endpoint(&catalog.id, "create_item", "POST", "/items");
    endpoint.risk = Some(crate::models::service_endpoint::EndpointRisk::Write);
    db.collection::<crate::models::service_endpoint::ServiceEndpoint>("service_endpoints")
        .insert_one(&endpoint)
        .await
        .unwrap();
    let expires = Utc::now() + Duration::hours(2);
    let services = vec![connection.clone()];
    let mut selections:Vec<DurableOperationSelection>=serde_json::from_value(json!([{
        "user_service_id":connection,"endpoint_id":endpoint.id,"constraints":{},
        "valid_from":(Utc::now()-Duration::minutes(1)).to_rfc3339(),
        "expires_at":(expires-Duration::minutes(1)).to_rfc3339(),"total_limit":10,"replay_policy":"non_replayable"
    }])).unwrap();
    if matches!(case, Case::Provision) {
        let mut second = endpoint.clone();
        second.id = uuid::Uuid::new_v4().to_string();
        second.name = "create_other".into();
        second.path = "/other".into();
        db.collection::<crate::models::service_endpoint::ServiceEndpoint>("service_endpoints")
            .insert_one(&second)
            .await
            .unwrap();
        let mut selection = selections[0].clone();
        selection.endpoint_id = second.id;
        selections.push(selection);
    }
    let plan = api_key_scope_service::build_scope_plan_with_operations(
        &db,
        &state.node_ws_manager,
        &owner,
        None,
        &services,
        &selections,
        Some(expires),
        false,
    )
    .await
    .unwrap();
    let create_body = json!({"name":"Fixture scheduled key","scopes":"proxy","expires_at":expires.to_rfc3339(),
        "allowed_service_ids":services,"selected_operations":selections,"scope_plan_digest":plan.normalized_grant_digest});
    let mut grant = String::new();
    let key = match case {
        Case::Pause | Case::Revoke => {
            permission_policy_service::create(
                &db,
                &state.encryption_keys,
                &owner,
                "Fixture permission key",
                &connection,
                expires,
                nyxid_permissions::drive::example_policy(),
                None,
            )
            .await
            .unwrap()
            .0
            .id
        }
        Case::Grant => {
            let result = crate::services::durable_operation_grant_service::provision_scheduled_key(
                &db,
                &state.node_ws_manager,
                &owner,
                &owner,
                "Fixture scheduled key",
                expires,
                None,
                &services,
                &plan.allowed_node_ids,
                None,
                None,
                None,
                &selections,
                &plan.normalized_grant_digest,
                None,
            )
            .await
            .unwrap();
            grant = result.grants[0].id.clone();
            result.key.id
        }
        _ => {
            key_service::create_api_key(
                &db,
                &owner,
                "Fixture parent",
                "proxy",
                Some(expires),
                None,
                Some(&services),
                Some(&[]),
                Some(false),
                Some(false),
                Some(false),
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap()
            .id
        }
    };
    let children: Vec<String> = (0..3).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    if matches!(case, Case::Child | Case::Rotate | Case::Delete) {
        for child in &children {
            db.collection::<Document>("api_key_credentials").insert_one(doc!{
                "_id":child,"api_key_id":&key,"user_id":&owner,"secret_hash":"fixture-private-child-hash",
                "secret_prefix":"fixture-private-child-prefix","label":"Fixture","login_request_id":uuid::Uuid::new_v4().to_string(),
                "is_active":true,"created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(expires)
            }).await.unwrap();
        }
    }
    security_effect_service::ensure_indexes(&db).await.unwrap();
    assert!(rows(&db, INTENTS).await.is_empty());
    Fixture {
        case,
        state,
        owner,
        connection,
        endpoint: endpoint.id,
        key,
        children,
        grant,
        create_body,
        monitor,
    }
}

mod reauthorization;

async fn request_as(f: &Fixture, auth: AuthUser) -> Response {
    let state = State(f.state.clone());
    match f.case {
        Case::Child => {
        super::auth_agent_key::revoke_credential(
            state,
            auth,
            ConnectInfo("127.0.0.1:1".parse().unwrap()),HeaderMap::new(),Path((f.key.clone(),f.children[0].clone()))
        ).await.into_response()
    },
        Case::Rotate => {
        super::api_keys::rotate_key(
            state,
            auth,
            Default::default(),Path(f.key.clone())
        ).await.into_response()
    },
        Case::Delete => {
            super::api_keys::delete_key(state, auth, Default::default(), Path(f.key.clone())).await.into_response()
        },
        Case::Operations => {
        super::api_keys::set_key_operations(
            state,
            auth,
            Path((f.key.clone(),f.connection.clone())),Json(serde_json::from_value(json!({"expected_revision":0,"endpoint_ids":[f.endpoint]})).unwrap())
        ).await.into_response()
    },
        Case::Provision => {
        super::api_keys::create_key(
            state,
            auth,
            Default::default(),Json(serde_json::from_value(f.create_body.clone()).unwrap())
        ).await.into_response()
    },
        Case::Grant => {
        super::api_keys::revoke_durable_grant(
            state,
            auth,
            Path((f.key.clone(),f.grant.clone()))
        ).await.into_response()
    },
        Case::Create => {
        super::permission_keys::create(
            state,
            auth,
            Json(serde_json::from_value(json!({"name":"Fixture permission key","user_service_id":f.connection,"expires_at":(Utc::now()+Duration::hours(1)).to_rfc3339(),"policy":nyxid_permissions::drive::example_policy()})).unwrap())
        ).await.into_response()
    },
        Case::Pause => {
        super::permission_keys::pause(
            state,
            auth,
            Path(f.key.clone()),Json(serde_json::from_value(json!({"revision":1,"paused":true})).unwrap())
        ).await.into_response()
    },
        Case::Revoke => {
        super::permission_keys::revoke(
            state,
            auth,
            Path(f.key.clone())
        ).await.into_response()
    },
    }
}
async fn request(f: &Fixture) -> Response {
    request_as(f, test_auth_user(&f.owner)).await
}

async fn death(case: Case) {
    let f = fixture(case).await;
    f.monitor.lock().unwrap().stop = true;
    let result = std::panic::AssertUnwindSafe(Box::pin(REQUEST.scope((), request(&f))))
        .catch_unwind()
        .await;
    match result {
        Err(error) => assert!(error.is::<ProcessStopped>()),
        Ok(response) => panic!(
            "{case:?}: did not reach source commit; got HTTP {}",
            response.status()
        ),
    }
    let db = &f.state.db;
    match case {
        Case::Child | Case::Rotate | Case::Delete => assert_eq!(
            db.collection::<Document>("api_key_credentials")
                .count_documents(doc! {"revoked_at":{"$type":"date"}})
                .await
                .unwrap(),
            case.count() as u64
        ),
        Case::Operations => assert_eq!(
            rows(db, "api_keys").await[0]
                .get_document("assistant_operation_scopes")
                .unwrap()
                .len(),
            1
        ),
        Case::Provision => assert_eq!(
            db.collection::<Document>("api_keys")
                .count_documents(doc! {"scheduled_write_enabled":true})
                .await
                .unwrap(),
            1
        ),
        Case::Grant => assert!(
            rows(db, "durable_operation_grants").await[0]
                .get_datetime("revoked_at")
                .is_ok()
        ),
        Case::Create => assert_eq!(rows(db, "permission_key_policies").await.len(), 1),
        Case::Pause => assert_eq!(
            rows(db, "permission_key_policies").await[0].get_bool("paused"),
            Ok(true)
        ),
        Case::Revoke => assert_eq!(
            rows(db, "api_keys").await[0].get_bool("is_active"),
            Ok(false)
        ),
    }
    assert!(rows(db, "audit_log").await.is_empty());
    let intents = rows(db, INTENTS).await;
    assert_eq!(
        intents.len(),
        case.count(),
        "{case:?}: committed authority change has no durable audit evidence"
    );
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    db.collection::<Document>(INTENTS)
        .insert_many(&intents)
        .await
        .unwrap();
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    let audit = rows(db, "audit_log").await;
    assert_eq!(audit.len(), case.count());
    for row in audit {
        assert_eq!(row.get_str("event_type"), Ok(case.event()));
        assert!(row.get_i64("seq").is_ok());
        assert!(row.get_str("entry_hash").is_ok());
    }
    assert!(rows(db, INTENTS).await.is_empty());
    cleanup(db).await;
}
macro_rules! death_test {
    ($name:ident,$case:ident) => {
        #[tokio::test]
        async fn $name() {
            Box::pin(death(Case::$case)).await;
        }
    };
}
death_test!(child_revoke_process_death_and_replay, Child);
death_test!(parent_rotate_process_death_and_replay, Rotate);
death_test!(parent_delete_process_death_and_replay, Delete);
death_test!(operations_process_death_and_replay, Operations);
death_test!(grant_provision_process_death_and_replay, Provision);
death_test!(grant_revoke_process_death_and_replay, Grant);
death_test!(permission_create_process_death_and_replay, Create);
death_test!(permission_pause_process_death_and_replay, Pause);
death_test!(permission_revoke_process_death_and_replay, Revoke);

#[tokio::test]
async fn key_authority_request_command_counts() {
    for case in CASES {
        let f = Box::pin(fixture(case)).await;
        let response = Box::pin(REQUEST.scope((), request(&f))).await;
        let status = response.status();
        assert!(
            status.is_success(),
            "{case:?} got {status}: {:?}",
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
        );
        let commands = f.monitor.lock().unwrap().commands.clone();
        println!(
            "KEY_AUTHORITY_REQUEST {case:?} count={} commands={commands:?}",
            commands.len()
        );
        let expected = match case {
            Case::Child => 7,
            // The successor insert passes the personal erasure fence (#2859).
            Case::Rotate => 29,
            Case::Delete => 23,
            Case::Operations => 17,
            // Provisioned key inserts pass the personal erasure fence (#2859).
            Case::Provision => 73,
            Case::Grant => 4,
            // The new key insert passes the personal erasure fence (#2859).
            Case::Create => 19,
            Case::Pause => 3,
            Case::Revoke => 13,
        };
        assert_eq!(commands.len(), expected, "{case:?}: command budget");
        if matches!(case, Case::Rotate | Case::Delete | Case::Revoke) {
            let window_lookups = commands
                .iter()
                .filter(|(command, collection)| {
                    command == "find" && collection == "assistant_conversations"
                })
                .count();
            assert_eq!(window_lookups, 1, "{case:?}: lookup before revocation");
        }
        cleanup(&f.state.db).await;
    }
}

#[tokio::test]
async fn key_authority_rejected_intent_rolls_back_its_source_unit() {
    let collections = [
        "api_keys",
        "api_key_credentials",
        "agent_service_bindings",
        "machine_revocations",
        "machine_confirmations",
        "durable_operation_grants",
        "permission_key_policies",
    ];
    for case in CASES {
        let f = Box::pin(fixture(case)).await;
        let db = &f.state.db;
        let mut before = Vec::new();
        for collection in collections {
            before.push(rows(db, collection).await);
        }
        db.run_command(doc! {"collMod":INTENTS,"validator":{"reject_fixture":{"$exists":true}}})
            .await
            .unwrap();
        let response = Box::pin(request(&f)).await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{case:?}"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error_code"], 1007, "{case:?}: {body}");
        for (collection, before) in collections.into_iter().zip(before) {
            if matches!(case, Case::Provision)
                && ["api_keys", "durable_operation_grants"].contains(&collection)
            {
                assert_eq!(db.collection::<Document>("api_keys").count_documents(doc!{"purpose":"scheduled_invocation","is_active":false,"scheduled_write_enabled":false}).await.unwrap(),1);
                assert_eq!(rows(db, "durable_operation_grants").await.len(), 2);
            } else {
                assert_eq!(rows(db, collection).await, before, "{case:?}: {collection}");
            }
        }
        assert!(rows(db, INTENTS).await.is_empty());
        assert!(rows(db, "audit_log").await.is_empty());
        cleanup(db).await;
    }
}

fn metadata(f: &Fixture, source: &Document, child: Option<&str>) -> Value {
    match f.case {
        Case::Child | Case::Rotate | Case::Delete => {
            json!({"api_key_id":f.key,"credential_id":child.unwrap(),"reason":match f.case {Case::Child=>"web_revoke",Case::Rotate=>"parent_rotated",_=>"parent_revoked"}})
        }
        Case::Operations => {
            json!({"owner_id":f.owner,"api_key_id":f.key,"service_id":f.connection,"revision":1,"operation_count":1})
        }
        Case::Provision => unreachable!("Provision parity uses the ordered handler receipts"),
        Case::Grant => {
            json!({"api_key_id":f.key,"grant_id":f.grant,"endpoint_id":source.get_str("endpoint_id").unwrap(),"contract_digest":source.get_str("contract_digest").unwrap(),"decision":"revoked"})
        }
        Case::Create => {
            json!({"permission_key_id":source.get_str("_id").unwrap(),"user_service_id":f.connection})
        }
        Case::Pause => json!({"permission_key_id":f.key,"paused":true,"revision":2}),
        Case::Revoke => json!({"permission_key_id":f.key}),
    }
}

async fn parity(case: Case) {
    use crate::services::audit_service;
    let f = Box::pin(fixture(case)).await;
    let mut auth = test_auth_user(&f.owner);
    auth.ip_address = Some("192.0.2.13".into());
    auth.user_agent = Some("Fixture client/1.0 🦀".into());
    if !matches!(case, Case::Create | Case::Pause | Case::Revoke) {
        auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
        auth.scope = "write".into();
        auth.api_key_id = Some(uuid::Uuid::new_v4().to_string());
        auth.api_key_name = Some("Fixture caller 🦀".into());
    }
    // Permission-key administration is restricted to first-party human callers.
    let context = crate::turn_trace::TurnContext::for_turn(&uuid::Uuid::new_v4().to_string());
    let response = crate::turn_trace::scope(context, Box::pin(request_as(&f, auth.clone()))).await;
    assert!(
        response.status().is_success(),
        "{case:?}: {}",
        response.status()
    );
    let receipt = if matches!(case, Case::Provision) {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        Some(serde_json::from_slice::<Value>(&body).unwrap())
    } else {
        None
    };
    let db = &f.state.db;
    let intents = rows(db, INTENTS).await;
    assert_eq!(intents.len(), case.count());
    for intent in &intents {
        let encoded = serde_json::to_string(intent).unwrap();
        for forbidden in [
            "secret_hash",
            "secret_prefix",
            "key_hash",
            "key_prefix",
            "credential_encrypted",
            "access_token",
            "refresh_token",
            "fixture-private-child",
            "fixture-private-provider-token",
        ] {
            assert!(!encoded.contains(forbidden), "{case:?}: {forbidden}");
        }
    }
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    let actual = rows(db, "audit_log").await;
    let source = match case {
        Case::Provision | Case::Grant => rows(db, "durable_operation_grants").await.remove(0),
        Case::Create => rows(db, "permission_key_policies").await.remove(0),
        _ => Document::new(),
    };
    for mut actual in actual {
        let child = actual
            .get_document("event_data")
            .ok()
            .and_then(|data| data.get_str("credential_id").ok())
            .map(str::to_owned);
        let expected = if let Some(receipt) = &receipt {
            let grants = receipt["durable_grants"].as_array().unwrap();
            json!({"api_key_id":receipt["id"],"grant_ids":grants.iter().map(|grant|&grant["id"]).collect::<Vec<_>>(),"operation_count":grants.len(),"owner_user_id":f.owner,"decision":"authorized"})
        } else {
            metadata(&f, &source, child.as_deref())
        };
        let written = audit_service::notify_on_audit_write_for_user(case.event(), &f.owner);
        crate::turn_trace::scope(context, async {
            match case {
                Case::Child | Case::Rotate | Case::Delete => audit_service::log_async(
                    db.clone(),
                    Some(f.owner.clone()),
                    case.event().into(),
                    Some(expected),
                    None,
                    None,
                    None,
                    None,
                ),
                Case::Operations => audit_service::log_actor_event(
                    db.clone(),
                    &audit_service::AuditActor {
                        user_id: f.owner.clone(),
                        ip_address: None,
                        user_agent: None,
                        api_key_id: None,
                        api_key_name: None,
                    },
                    case.event(),
                    Some(expected),
                )
                .await
                .unwrap(),
                _ => audit_service::log_for_user(db.clone(), &auth, case.event(), Some(expected)),
            }
        })
        .await;
        let old_id = written.await.unwrap();
        let mut old = db
            .collection::<Document>("audit_log")
            .find_one(doc! {"_id":old_id})
            .await
            .unwrap()
            .unwrap();
        for field in ["_id", "created_at", "seq", "prev_hash", "entry_hash"] {
            actual.remove(field);
            old.remove(field);
        }
        assert_eq!(actual, old, "{case:?}: audit content parity");
    }
    cleanup(db).await;
}
macro_rules! parity_test {
    ($name:ident,$case:ident) => {
        #[tokio::test]
        async fn $name() {
            Box::pin(parity(Case::$case)).await;
        }
    };
}
parity_test!(child_revoke_content_parity, Child);
parity_test!(parent_rotate_content_parity, Rotate);
parity_test!(parent_delete_content_parity, Delete);
parity_test!(operations_content_parity, Operations);
parity_test!(grant_provision_content_parity, Provision);
parity_test!(grant_revoke_content_parity, Grant);
parity_test!(permission_create_content_parity, Create);
parity_test!(permission_pause_content_parity, Pause);
parity_test!(permission_revoke_content_parity, Revoke);

#[tokio::test]
async fn key_authority_occurrence_caps_and_maxima() {
    use crate::{
        models::{
            api_key_credential::{ApiKeyCredential, CredentialRevokedReason},
            security_effect_intent::SecurityEffect,
        },
        services::{api_key_audit_service as audit, audit_service::AuditActor},
    };
    let raw = "x".repeat(20_000);
    let actor = AuditActor {
        user_id: raw.clone(),
        ip_address: Some(raw.clone()),
        user_agent: Some(raw.clone()),
        api_key_id: Some(raw.clone()),
        api_key_name: Some(raw.clone()),
    };
    let child:ApiKeyCredential=bson::from_document(doc!{"_id":&raw,"user_id":&raw,"api_key_id":&raw,
        "secret_hash":"fixture-secret-hash","secret_prefix":"fixture-secret-prefix","label":"private-label","login_request_id":"private-login-id","is_active":false,"created_at":bson::DateTime::now()}).unwrap();
    let mut ids = vec!["i".repeat(256); 30];
    ids.push("i".repeat(238)); // Exactly 8 KiB including BSON array index/type/length bytes.
    ids.push("i".repeat(256));
    let context = crate::turn_trace::TurnContext::for_turn(&uuid::Uuid::new_v4().to_string());
    let results = crate::turn_trace::scope(context, async {
        [
            audit::child_revoked(&child, CredentialRevokedReason::ParentRotated),
            audit::operations_changed(&raw, &raw, &raw, &raw, i64::MAX, Some(i64::MAX as usize)),
            audit::grants_provisioned(&actor, &raw, &raw, &ids),
            audit::grant_revoked(
                &actor,
                &raw,
                &raw,
                &raw,
                &format!("sha256:{}", "d".repeat(64)),
            ),
            audit::permission_created(&actor, &raw, &raw),
            audit::permission_paused(&actor, &raw, true, i64::MAX),
            audit::permission_revoked(&actor, &raw),
        ]
    })
    .await;
    for result in results {
        let SecurityEffect::Audit { event } = result.unwrap().effect else {
            panic!("audit")
        };
        let bytes = bson::to_vec(&event).unwrap().len();
        let maximum = match event.event_type.as_str() {
            "agent_key_credential_revoked" => 1195,
            "api_key_operations_changed" => 1477,
            "durable_grants_provisioned" => 11391,
            "durable_grant_revoked" => 3383,
            "permission_key_created" => 3009,
            "permission_key_pause_changed" => 2764,
            "permission_key_revoked" => 2731,
            _ => panic!("unexpected occurrence"),
        };
        assert_eq!(bytes, maximum, "{}", event.event_type);
        assert!(bytes < 16384);
        assert!(event.user_id.as_ref().unwrap().ends_with("...[truncated]"));
        if event.event_type == "durable_grants_provisioned" {
            let data = event.event_data.as_ref().unwrap();
            let array_doc = bson::to_document(&json!({"a":data["grant_ids"]})).unwrap();
            assert_eq!(bson::to_vec(&array_doc).unwrap().len() - 8, 8192);
            assert_eq!(data["grant_ids_count"], ids.len());
            assert_eq!(data["grant_ids_truncated"], true);
            use sha2::Digest;
            assert_eq!(
                data["grant_ids_sha256"],
                hex::encode(sha2::Sha256::digest(ids.join("\n").as_bytes()))
            );
        }
        let encoded = serde_json::to_string(&event).unwrap();
        for secret in ["fixture-secret", "private-label", "private-login-id"] {
            assert!(!encoded.contains(secret));
        }
        println!(
            "KEY_AUTHORITY_OCCURRENCE_MAX {} bytes={bytes} margin={}",
            event.event_type,
            16384 - bytes
        );
    }
}

#[tokio::test]
async fn competing_authority_requests_record_only_the_winner() {
    for case in [Case::Child, Case::Operations, Case::Grant, Case::Pause] {
        let f = Box::pin(fixture(case)).await;
        let (a, b) = tokio::join!(Box::pin(request(&f)), Box::pin(request(&f)));
        let mut statuses = [a.status().as_u16(), b.status().as_u16()];
        statuses.sort();
        assert_eq!(
            statuses,
            match case {
                Case::Child => [200, 200],
                Case::Grant => [200, 403],
                _ => [200, 409],
            },
            "{case:?}"
        );
        assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1, "{case:?}");
        cleanup(&f.state.db).await;
    }
}

#[tokio::test]
async fn authority_audit_caps_do_not_reject_accepted_network_context() {
    let f = Box::pin(fixture(Case::Pause)).await;
    let mut auth = test_auth_user(&f.owner);
    auth.ip_address = Some("x".repeat(20_000));
    auth.user_agent = Some("🦀".repeat(5000));
    assert!(Box::pin(request_as(&f, auth)).await.status().is_success());
    let intent = rows(&f.state.db, INTENTS).await.remove(0);
    let intent: crate::models::security_effect_intent::SecurityEffectIntent =
        bson::from_document(intent).unwrap();
    let crate::models::security_effect_intent::SecurityEffect::Audit { event } = intent.effect
    else {
        panic!("audit")
    };
    assert_eq!(event.ip_address.as_ref().unwrap().len(), 256);
    assert!(event.user_agent.as_ref().unwrap().len() <= 1024);
    for value in [event.ip_address, event.user_agent] {
        assert!(value.unwrap().ends_with("...[truncated]"));
    }
    assert_eq!(
        rows(&f.state.db, "permission_key_policies").await[0].get_bool("paused"),
        Ok(true)
    );
    cleanup(&f.state.db).await;
}

#[tokio::test]
#[ignore = "Operating-range probe; cargo +1.98.1 test -p nyxid --bin nyxid-server key_authority_thousand_children_probe -- --ignored --nocapture --test-threads=2 (through cargo-slot.sh)"]
async fn key_authority_thousand_children_probe() {
    for case in [Case::Rotate] {
        let f = Box::pin(fixture(case)).await;
        let db = &f.state.db;
        let template = rows(db, "api_key_credentials").await.remove(0);
        let children: Vec<_> = (3..1000)
            .map(|_| {
                let mut row = template.clone();
                row.insert("_id", uuid::Uuid::new_v4().to_string());
                row
            })
            .collect();
        db.collection::<Document>("api_key_credentials")
            .insert_many(children)
            .await
            .unwrap();
        let start = std::time::Instant::now();
        let response = Box::pin(REQUEST.scope((), request(&f))).await;
        let elapsed = start.elapsed();
        assert!(
            response.status().is_success(),
            "{case:?}: {}",
            response.status()
        );
        assert_eq!(
            db.collection::<Document>("api_key_credentials")
                .count_documents(doc! {"revoked_at":{"$type":"date"}})
                .await
                .unwrap(),
            1000
        );
        assert_eq!(rows(db, INTENTS).await.len(), 1000);
        let commands = f.monitor.lock().unwrap().commands.clone();
        let commits = commands
            .iter()
            .filter(|(name, _)| name == "commitTransaction")
            .count();
        println!(
            "KEY_AUTHORITY_SCALE {case:?} children=1000 elapsed={elapsed:?} commands={} commits={commits}",
            commands.len()
        );
        cleanup(db).await;
    }
}

#[tokio::test]
async fn missing_authority_rows_keep_errors_and_write_no_intent() {
    for case in [
        Case::Child,
        Case::Operations,
        Case::Grant,
        Case::Pause,
        Case::Revoke,
    ] {
        let f = Box::pin(fixture(case)).await;
        let (collection, id) = match case {
            Case::Child => ("api_key_credentials", &f.children[0]),
            Case::Grant => ("durable_operation_grants", &f.grant),
            Case::Pause | Case::Revoke => ("permission_key_policies", &f.key),
            _ => ("api_keys", &f.key),
        };
        f.state
            .db
            .collection::<Document>(collection)
            .delete_one(doc! {"_id":id})
            .await
            .unwrap();
        let response = Box::pin(request(&f)).await;
        assert_eq!(
            response.status(),
            match case {
                Case::Pause => StatusCode::CONFLICT,
                Case::Grant => StatusCode::FORBIDDEN,
                _ => StatusCode::NOT_FOUND,
            },
            "{case:?}"
        );
        assert!(rows(&f.state.db, INTENTS).await.is_empty());
        assert!(rows(&f.state.db, "audit_log").await.is_empty());
        cleanup(&f.state.db).await;
    }
}

#[tokio::test]
async fn permission_revoke_cleanup_failure_keeps_source_and_its_evidence() {
    for case in [Case::Revoke, Case::Delete] {
        let f = Box::pin(fixture(case)).await;
        let db = &f.state.db;
        db.collection::<Document>("agent_service_bindings")
            .drop()
            .await
            .unwrap();
        db.run_command(
        doc! {"create":"agent_service_bindings","viewOn":"fixture_empty_bindings","pipeline":[]},
    )
    .await
    .unwrap();
        let response = Box::pin(request(&f)).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            rows(db, "api_keys").await[0].get_bool("is_active"),
            Ok(false)
        );
        assert_eq!(rows(db, INTENTS).await.len(), case.count());
        security_effect_service::sweep(db, &f.state.encryption_keys)
            .await
            .unwrap();
        assert_eq!(
            rows(db, "audit_log").await[0].get_str("event_type"),
            Ok(case.event())
        );
        cleanup(db).await;
    }
}

#[tokio::test]
#[ignore = "Operating-range probe, 20,000 login credentials; through cargo-slot.sh run cargo +1.98.1 test -p nyxid --bin nyxid-server key_authority_large_parent_delete_probe -- --ignored --nocapture --test-threads=1"]
async fn key_authority_large_parent_delete_probe() {
    let child_count: usize = std::env::var("NYXID_PARENT_DELETE_PROBE_CHILDREN")
        .map_or(20_000, |value| value.parse().expect("probe child count"));
    assert!(child_count >= 3);
    let f = Box::pin(fixture(Case::Delete)).await;
    let db = &f.state.db;
    for child in &f.children {
        db.collection::<Document>("api_key_credentials")
            .update_one(
                doc! {"_id": child},
                doc! {"$set": {"secret_hash": uuid::Uuid::new_v4().to_string()}},
            )
            .await
            .unwrap();
    }
    crate::db::ensure_indexes(db).await.unwrap();
    let template = rows(db, "api_key_credentials").await.remove(0);
    for offset in (3..child_count).step_by(1000) {
        let children: Vec<_> = (offset..(offset + 1000).min(child_count))
            .map(|_| {
                let mut row = template.clone();
                row.insert("_id", uuid::Uuid::new_v4().to_string());
                row.insert("login_request_id", uuid::Uuid::new_v4().to_string());
                row.insert("secret_hash", uuid::Uuid::new_v4().to_string());
                row
            })
            .collect();
        db.collection::<Document>("api_key_credentials")
            .insert_many(children)
            .await
            .unwrap();
    }
    let start = std::time::Instant::now();
    let response = Box::pin(REQUEST.scope((), request(&f))).await;
    let elapsed = start.elapsed();
    let (commands, starts, wire_bytes, largest_write) = {
        let monitor = f.monitor.lock().unwrap();
        (
            monitor.commands.clone(),
            monitor.transaction_starts,
            monitor.transaction_write_bytes,
            monitor.largest_transaction_write,
        )
    };
    let mut total_rows = 0_i64;
    let mut total_bytes = 0_i64;
    let mut largest_row = 0_i64;
    for collection in [
        "api_keys",
        "api_key_credentials",
        "machine_revocations",
        "machine_confirmations",
        INTENTS,
    ] {
        let size: Vec<Document> = db
            .collection::<Document>(collection)
            .aggregate(vec![doc! {"$group": {
                "_id": null, "rows": {"$sum": 1_i64},
                "bytes": {"$sum": {"$bsonSize": "$$ROOT"}},
                "largest": {"$max": {"$bsonSize": "$$ROOT"}}
            }}])
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        if let Some(size) = size.first() {
            let number = |key: &str| {
                size.get_i64(key)
                    .unwrap_or_else(|_| i64::from(size.get_i32(key).unwrap()))
            };
            let count = number("rows");
            let bytes = number("bytes");
            total_rows += count;
            total_bytes += bytes;
            largest_row = largest_row.max(number("largest"));
            println!("KEY_AUTHORITY_SCALE_ROWS collection={collection} rows={count} bytes={bytes}");
        }
    }
    let commits = commands
        .iter()
        .filter(|(name, _)| name == "commitTransaction")
        .count();
    println!(
        "KEY_AUTHORITY_SCALE Delete children={child_count} status={} elapsed={elapsed:?} commands={} transaction_starts={starts} commits={commits} stored_rows={total_rows} stored_bytes={total_bytes} write_command_bytes={wire_bytes} largest_write_command={largest_write} largest_row={largest_row} largest_row_16mib_margin={} aggregate_16mib_difference={}",
        response.status(),
        commands.len(),
        16 * 1024 * 1024 - largest_row,
        16 * 1024 * 1024 - total_bytes
    );
    assert!(response.status().is_success());
    assert_eq!(
        db.collection::<Document>("api_key_credentials")
            .count_documents(doc! {"revoked_at":{"$type":"date"}})
            .await
            .unwrap(),
        child_count as u64
    );
    assert_eq!(
        db.collection::<Document>(INTENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        child_count as u64
    );
    assert_eq!(commits, 1);
    assert_eq!(
        rows(db, "api_keys").await[0].get_bool("is_active"),
        Ok(false)
    );
    cleanup(db).await;
}
