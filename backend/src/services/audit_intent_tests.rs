use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use mongodb::bson::{Document, doc};

#[derive(Debug)]
struct SimulatedProcessDeath;

/// Stop the request after MongoDB acknowledged its source write, before the
/// request future can run its post-commit detached effects. For a transactional
/// source, wait for that transaction's commit acknowledgement instead.
#[derive(Default)]
struct CommitBarrier {
    armed: bool,
    transaction: Option<i64>,
    completions: HashSet<i32>,
}

async fn monitored_database(
    name: &str,
    source_collection: &'static str,
) -> (mongodb::Database, Arc<Mutex<CommitBarrier>>) {
    let barrier = Arc::new(Mutex::new(CommitBarrier::default()));
    let observed = barrier.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        name,
        mongodb::event::EventHandler::callback(move |event| {
            use mongodb::event::command::CommandEvent;
            let mut barrier = observed.lock().unwrap();
            if !barrier.armed {
                return;
            }
            match event {
                CommandEvent::Started(event) => {
                    let source = matches!(event.command_name.as_str(), "insert" | "findAndModify")
                        && event.command.get_str(&event.command_name).ok()
                            == Some(source_collection);
                    if source {
                        if event.command.get_bool("autocommit") == Ok(false) {
                            barrier.transaction = event.command.get_i64("txnNumber").ok();
                        } else {
                            barrier.completions.insert(event.request_id);
                        }
                    } else if event.command_name == "commitTransaction"
                        && barrier.transaction.is_some()
                        && event.command.get_i64("txnNumber").ok() == barrier.transaction
                    {
                        barrier.completions.insert(event.request_id);
                    }
                }
                CommandEvent::Succeeded(event) if barrier.completions.remove(&event.request_id) => {
                    barrier.armed = false;
                    drop(barrier);
                    // Unwind this request exactly after the source acknowledgement.
                    // Notify/select alone can let it schedule the audit in this poll.
                    std::panic::resume_unwind(Box::new(SimulatedProcessDeath));
                }
                _ => {}
            }
        }),
    )
    .await;
    (db, barrier)
}

#[tokio::test]
async fn committed_revocation_survives_request_death_with_cae_intent() {
    let (db, barrier) = monitored_database("cae_revoke_crash", "oauth_broker_bindings").await;
    let state = crate::test_utils::test_app_state(db.clone());
    let binding_id = "fixture-binding-hash";
    db.collection::<Document>("oauth_broker_bindings")
        .insert_one(doc! {
            "_id":binding_id,"client_id":"fixture-client","user_id":"fixture-user",
            "refresh_token_jti":"fixture-jti","created_at":bson::DateTime::now(),"revoked":false,
        })
        .await
        .unwrap();
    db.collection::<Document>("refresh_tokens")
        .insert_one(doc! {
            "_id":"fixture-refresh","jti":"fixture-jti","revoked":false,
        })
        .await
        .unwrap();
    barrier.lock().unwrap().armed = true;
    {
        let request = Box::pin(
            crate::services::oauth_broker_service::revoke_binding_by_user(
                &db,
                state.encryption_keys.clone(),
                &state.http_client,
                "fixture-user",
                binding_id,
                "user_revoked",
            ),
        );
        let stopped = std::panic::AssertUnwindSafe(request).catch_unwind().await;
        assert!(stopped.unwrap_err().is::<SimulatedProcessDeath>());
    }
    barrier.lock().unwrap().armed = false;
    let binding = db
        .collection::<Document>("oauth_broker_bindings")
        .find_one(doc! {"_id":binding_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.get_bool("revoked"), Ok(true));
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(
        db.collection::<Document>("security_effect_intents")
            .count_documents(doc! {"kind":"cae","binding_id":binding_id})
            .await
            .unwrap(),
        1,
        "committed revocation must retain a notification intent after request death"
    );
    assert_eq!(
        db.collection::<Document>("security_effect_intents")
            .count_documents(
                doc! {"kind":"audit","event.event_type":"oauth_broker_binding_revoked"}
            )
            .await
            .unwrap(),
        1,
        "the source transaction must also retain its required audit"
    );
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"oauth_broker_binding_revoked"})
            .await
            .unwrap(),
        0
    );
    crate::services::security_effect_service::ensure_indexes(&db)
        .await
        .unwrap();
    crate::services::security_effect_service::sweep(&db, &state.encryption_keys)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"oauth_broker_binding_revoked"})
            .await
            .unwrap(),
        1,
        "a replacement worker must append the committed occurrence"
    );
    db.drop().await.unwrap();
}

// Detached work is outside the request's command budget on the old binary.
tokio::task_local! { static REQUEST_COMMANDS: (); }

async fn revocation_fixture(db: &mongodb::Database) -> (crate::AppState, String, String, String) {
    let owner = uuid::Uuid::new_v4().to_string();
    let client = uuid::Uuid::new_v4().to_string();
    let raw = crate::models::oauth_broker_binding::generate_binding_id();
    db.collection::<Document>("oauth_clients")
        .insert_one(doc! {
            "_id":&client,"client_name":"Fixture","client_secret_hash":"","client_type":"public","is_active":true,
            "redirect_uris":[],"allowed_scopes":"openid","grant_types":"authorization_code",
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    db.collection::<Document>("users")
        .insert_one(doc! {"_id":&owner})
        .await
        .unwrap();
    db.collection::<Document>("consents")
        .insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"client_id":&client,
            "scopes":"openid","granted_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    for (index, raw) in [
        &raw,
        &crate::models::oauth_broker_binding::generate_binding_id(),
    ]
    .into_iter()
    .enumerate()
    {
        let jti = format!("{client}-refresh-{index}");
        db.collection::<Document>("oauth_broker_bindings").insert_one(doc! {
            "_id":crate::models::oauth_broker_binding::hash_binding_id(raw),"user_id":&owner,
            "client_id":&client,"refresh_token_jti":&jti,"created_at":bson::DateTime::now(),"revoked":false,
        }).await.unwrap();
        db.collection::<Document>("refresh_tokens")
            .insert_one(doc! {
                "_id":&jti,"jti":&jti,"user_id":&owner,"client_id":&client,"revoked":false,
            })
            .await
            .unwrap();
    }
    (
        crate::test_utils::test_app_state(db.clone()),
        owner,
        client,
        raw,
    )
}

async fn request_revocation(
    state: crate::AppState,
    owner: &str,
    client: &str,
    raw: &str,
    path: &str,
) -> axum::response::Response {
    use axum::{
        extract::{Path, State},
        http::HeaderMap,
        response::IntoResponse,
    };
    use axum_extra::extract::Query;
    match path {
        "client" => crate::handlers::oauth::delete_binding(
            crate::mw::rate_limit::AuditIp(None),
            State(state),
            HeaderMap::new(),
            Path(raw.into()),
            Query(crate::handlers::oauth::GetBindingQuery {
                client_id: Some(client.into()),
                client_secret: None,
            }),
        )
        .await
        .into_response(),
        "user" => crate::handlers::broker_bindings::revoke_my_broker_binding(
            State(state),
            crate::test_utils::test_auth_user(owner),
            Path(crate::models::oauth_broker_binding::hash_binding_id(raw)),
        )
        .await
        .into_response(),
        "consent" => crate::handlers::consent::revoke_my_consent(
            State(state),
            crate::test_utils::test_auth_user(owner),
            Path(client.into()),
        )
        .await
        .into_response(),
        _ => panic!("unknown fixture route"),
    }
}

#[tokio::test]
async fn revocation_request_command_counts() {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let capture = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "foundation_request_counts",
        mongodb::event::EventHandler::callback(move |event| {
            if REQUEST_COMMANDS.try_with(|_| ()).is_err() {
                return;
            }
            if let mongodb::event::command::CommandEvent::Started(event) = event {
                capture.lock().unwrap().push((
                    event.command_name.clone(),
                    event
                        .command
                        .get_str(&event.command_name)
                        .unwrap_or("")
                        .to_owned(),
                ));
            }
        }),
    )
    .await;
    for path in ["client", "user", "consent"] {
        let (state, owner, client, raw) = revocation_fixture(&db).await;
        commands.lock().unwrap().clear();
        let response = REQUEST_COMMANDS
            .scope((), request_revocation(state, &owner, &client, &raw, path))
            .await;
        assert!(
            response.status().is_success(),
            "{path}: {}",
            response.status()
        );
        assert_eq!(
            db.collection::<Document>("oauth_broker_bindings")
                .count_documents(doc! {"client_id": &client, "revoked": true})
                .await
                .unwrap(),
            if path == "consent" { 2 } else { 1 },
            "must reach the source mutation: {path}"
        );
        let measured = std::mem::take(&mut *commands.lock().unwrap());
        assert_eq!(
            measured.len(),
            match path {
                "client" => 5,
                "user" => 4,
                "consent" => 11,
                _ => unreachable!(),
            }
        );
        println!(
            "FOUNDATION_COMMANDS {path} total={} commands={measured:?}",
            measured.len()
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn rejected_intent_rolls_back_binding_and_multi_binding_consent_revocation() {
    for path in ["client", "user", "consent"] {
        let db = crate::test_utils::connect_transaction_test_database("foundation_intent_rejected")
            .await;
        let (state, owner, client, raw) = revocation_fixture(&db).await;
        db.create_collection("security_effect_intents")
            .await
            .unwrap();
        db.run_command(
            doc! {"collMod":"security_effect_intents","validator":{"forbidden":{"$exists":true}}},
        )
        .await
        .unwrap();
        let response = request_revocation(state, &owner, &client, &raw, path).await;
        assert!(
            response.status().is_server_error(),
            "{path}: {}",
            response.status()
        );
        for collection in ["oauth_broker_bindings", "refresh_tokens"] {
            assert_eq!(
                db.collection::<Document>(collection)
                    .count_documents(doc! {"revoked":true})
                    .await
                    .unwrap(),
                0,
                "{path}: {collection}"
            );
        }
        assert_eq!(
            db.collection::<Document>("consents")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<Document>("security_effect_intents")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn rejected_commit_does_not_report_success_or_publish_partial_revocation() {
    let db =
        crate::test_utils::connect_transaction_test_database("foundation_commit_rejected").await;
    let (_, owner, client_id, raw) = revocation_fixture(&db).await;
    let mut options = mongodb::options::ClientOptions::parse(
        std::env::var("NYXID_TEST_DATABASE_URL").expect("owned MongoDB URI"),
    )
    .await
    .unwrap();
    let app_name = uuid::Uuid::new_v4().to_string();
    options.app_name = Some(app_name.clone());
    let client = mongodb::Client::with_options(options).unwrap();
    let state = crate::test_utils::test_app_state(client.database(db.name()));
    let admin = db.client().database("admin");
    admin
        .run_command(
            doc! {"configureFailPoint":"failCommand", "mode":{"times":1},
                "data":{"failCommands":["commitTransaction"],"appName":&app_name,"errorCode":2},
            },
        )
        .await
        .unwrap();
    let response = request_revocation(state, &owner, &client_id, &raw, "consent").await;
    admin
        .run_command(doc! {"configureFailPoint":"failCommand","mode":"off"})
        .await
        .unwrap();
    assert!(response.status().is_server_error());
    assert_eq!(
        db.collection::<Document>("consents")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>("oauth_broker_bindings")
            .count_documents(doc! {"revoked":true})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>("security_effect_intents")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn multi_binding_consent_commits_one_audit_and_exact_cae_occurrences() {
    let db =
        crate::test_utils::connect_transaction_test_database("foundation_consent_effects").await;
    let (state, owner, client, raw) = revocation_fixture(&db).await;
    assert!(
        request_revocation(state.clone(), &owner, &client, &raw, "consent")
            .await
            .status()
            .is_success()
    );
    assert_eq!(
        db.collection::<Document>("oauth_broker_bindings")
            .count_documents(doc! {"revoked":true})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        db.collection::<Document>("refresh_tokens")
            .count_documents(doc! {"revoked":true})
            .await
            .unwrap(),
        2
    );
    let effects = db.collection::<Document>("security_effect_intents");
    assert_eq!(
        effects
            .count_documents(doc! {"kind":"audit","event.event_type":"oauth_consent_revoked"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        effects
            .count_documents(doc! {"kind":"cae","client_id":&client})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        request_revocation(state, &owner, &client, &raw, "consent")
            .await
            .status(),
        axum::http::StatusCode::NOT_FOUND
    );
    assert_eq!(effects.count_documents(doc! {}).await.unwrap(), 3);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn long_user_agent_consent_revocation_keeps_visible_bounded_context() {
    use axum::extract::{Path, State};
    let db = crate::test_utils::connect_transaction_test_database("consent_long_ua").await;
    let (state, owner, client, _) = revocation_fixture(&db).await;
    let mut auth = crate::test_utils::test_auth_user(&owner);
    auth.user_agent = Some("U".repeat(64 * 1024));
    let _ = crate::handlers::consent::revoke_my_consent(State(state.clone()), auth, Path(client))
        .await
        .unwrap();
    crate::services::security_effect_service::ensure_indexes(&db)
        .await
        .unwrap();
    crate::services::security_effect_service::sweep(&db, &state.encryption_keys)
        .await
        .unwrap();
    let row = db
        .collection::<Document>("audit_log")
        .find_one(doc! {"event_type":"oauth_consent_revoked"})
        .await
        .unwrap()
        .unwrap();
    let ua = row.get_str("user_agent").unwrap();
    assert_eq!(ua.len(), 1024);
    assert!(ua.ends_with("...[truncated]"));
    db.drop().await.unwrap();
}
