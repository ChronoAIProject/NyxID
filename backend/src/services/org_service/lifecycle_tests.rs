use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use bson::{Document, doc};
use futures::{FutureExt, TryStreamExt};
use mongodb::event::{EventHandler, command::CommandEvent};

use crate::handlers::{org_role_scopes as scopes, orgs as handlers};
use crate::models::{
    org_membership::{OrgMembership, OrgRole},
    user::{User, UserType},
};
use crate::services::{audit_service, security_effect_service};
use crate::test_utils::{test_app_state, test_auth_user, test_user};

const INTENTS: &str = "security_effect_intents";
tokio::task_local! { static REQUEST: (); }

#[derive(Clone, Copy, Debug)]
enum Action {
    Create,
    Profile,
    Set,
    Clear,
}
impl Action {
    const ALL: [Self; 4] = [Self::Create, Self::Profile, Self::Set, Self::Clear];
    fn event(self) -> &'static str {
        match self {
            Self::Create => "org_created",
            Self::Profile => "org_updated",
            Self::Set => "org_role_scope_set",
            Self::Clear => "org_role_scope_cleared",
        }
    }
    fn source_command(self) -> (&'static str, &'static str) {
        match self {
            Self::Create => ("insert", "org_memberships"),
            Self::Profile => ("update", "users"),
            Self::Set => ("findAndModify", "org_role_scopes"),
            Self::Clear => ("delete", "org_role_scopes"),
        }
    }
}
#[derive(Debug)]
struct ProcessStopped;
#[derive(Default)]
struct Monitor {
    stop: Option<(&'static str, &'static str)>,
    transaction: Option<i64>,
    completions: HashSet<i32>,
    commands: Vec<(String, String)>,
}
struct Fixture {
    state: crate::AppState,
    actor: String,
    org: String,
    monitor: Arc<Mutex<Monitor>>,
}

async fn fixture(action: Action) -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "org_audit",
        EventHandler::callback(move |event| {
            if REQUEST.try_with(|_| ()).is_err() {
                return;
            }
            let mut seen = seen.lock().unwrap();
            match event {
                CommandEvent::Started(e) => {
                    let collection = e.command.get_str(&e.command_name).unwrap_or("").to_owned();
                    seen.commands
                        .push((e.command_name.clone(), collection.clone()));
                    let Some(source) = seen.stop else {
                        return;
                    };
                    if (e.command_name.as_str(), collection.as_str()) == source {
                        if e.command.get_bool("autocommit") == Ok(false) {
                            seen.transaction = e.command.get_i64("txnNumber").ok();
                        } else {
                            seen.completions.insert(e.request_id);
                        }
                    } else if e.command_name == "commitTransaction"
                        && seen.transaction.is_some()
                        && e.command.get_i64("txnNumber").ok() == seen.transaction
                    {
                        seen.completions.insert(e.request_id);
                    }
                }
                CommandEvent::Succeeded(e)
                    if seen.stop.is_some() && seen.completions.remove(&e.request_id) =>
                {
                    seen.stop = None;
                    drop(seen);
                    std::panic::resume_unwind(Box::new(ProcessStopped));
                }
                _ => {}
            }
        }),
    )
    .await;
    let actor = uuid::Uuid::new_v4().to_string();
    let target = uuid::Uuid::new_v4().to_string();
    let org = uuid::Uuid::new_v4().to_string();
    db.collection::<User>("users")
        .insert_many([
            test_user(&actor, UserType::Person),
            test_user(&target, UserType::Person),
        ])
        .await
        .unwrap();
    if !matches!(action, Action::Create) {
        db.collection::<User>("users")
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        db.collection::<OrgMembership>("org_memberships")
            .insert_one(crate::test_utils::test_membership(
                &org,
                &actor,
                OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
    }
    if matches!(action, Action::Clear) {
        db.collection::<Document>("org_role_scopes")
            .insert_one(doc! {
                "_id": uuid::Uuid::new_v4().to_string(),
                "org_user_id": &org,
                "role": "member",
                "allowed_service_ids": [],
                "updated_at": bson::DateTime::now(),
                "updated_by": &actor,
            })
            .await
            .unwrap();
    }
    security_effect_service::ensure_indexes(&db).await.unwrap();
    Fixture {
        state: test_app_state(db),
        actor,
        org,
        monitor,
    }
}
fn audit_auth(actor: &str) -> crate::mw::auth::AuthUser {
    let mut auth = test_auth_user(actor);
    auth.ip_address = Some("192.0.2.10".into());
    auth.user_agent = Some(format!("NyxID-Test/1.0 {}", "x".repeat(300)));
    auth.api_key_id = Some("fixture-key-id".into());
    auth.api_key_name = Some("Administrator test key".into());
    auth
}
async fn request(f: &Fixture, action: Action) -> axum::response::Response {
    let state = State(f.state.clone());
    let auth = audit_auth(&f.actor);
    match action {
        Action::Create => handlers::create_org_with_id(
            state,
            auth,
            Json(handlers::CreateOrgRequest {
                display_name: "  Evidence org  ".into(),
                contact_email: Some("contact@example.test".into()),
                avatar_url: Some("https://example.test/avatar?token=fixture".into()),
            }),
            Some(&f.org),
        )
        .await
        .into_response(),
        Action::Profile => handlers::update_org(
            state,
            auth,
            Path(f.org.clone()),
            Json(handlers::UpdateOrgRequest {
                display_name: Some("Profile updated".into()),
                slug: None,
                avatar_url: Some("https://example.test/avatar?token=fixture".into()),
                contact_email: Some("new@example.test".into()),
                remote_credential_integrity_verification_opt_out: Some(true),
            }),
        )
        .await
        .into_response(),
        Action::Set => scopes::set_role_scope(
            state,
            auth,
            Path((f.org.clone(), "member".into())),
            Json(scopes::UpdateRoleScopeRequest {
                allowed_service_ids: Some(vec![]),
            }),
        )
        .await
        .into_response(),
        Action::Clear => {
            scopes::clear_role_scope(state, auth, Path((f.org.clone(), "member".into())))
                .await
                .into_response()
        }
    }
}
async fn snapshot(db: &mongodb::Database, collection: &str) -> Vec<Document> {
    db.collection::<Document>(collection)
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 65_536)
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn crash_after_commit(action: Action) {
    let f = fixture(action).await;
    f.monitor.lock().unwrap().stop = Some(action.source_command());
    let stopped = std::panic::AssertUnwindSafe(Box::pin(REQUEST.scope((), request(&f, action))))
        .catch_unwind()
        .await;
    assert!(stopped.unwrap_err().is::<ProcessStopped>());
    let db = &f.state.db;
    match action {
        Action::Create => assert_eq!(
            db.collection::<Document>("org_memberships")
                .count_documents(doc! {"org_user_id":&f.org,"member_user_id":&f.actor})
                .await
                .unwrap(),
            1
        ),
        Action::Profile => assert_eq!(
            db.collection::<Document>("users")
                .find_one(doc! {"_id":&f.org})
                .await
                .unwrap()
                .unwrap()
                .get_str("display_name"),
            Ok("Profile updated")
        ),
        Action::Set => assert_eq!(snapshot(db, "org_role_scopes").await.len(), 1),
        Action::Clear => assert!(snapshot(db, "org_role_scopes").await.is_empty()),
    }
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(
        db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":action.event()})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>(INTENTS)
            .count_documents(doc! {"event.event_type":action.event()})
            .await
            .unwrap(),
        1,
        "committed {action:?} lost its audit evidence"
    );
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":action.event()})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}
macro_rules! crash_test {
    ($name:ident,$action:ident) => {
        #[tokio::test]
        async fn $name() {
            crash_after_commit(Action::$action).await;
        }
    };
}
crash_test!(org_create_audit_survives_process_death, Create);
crash_test!(org_profile_audit_survives_process_death, Profile);
crash_test!(org_set_audit_survives_process_death, Set);
crash_test!(org_clear_audit_survives_process_death, Clear);
#[tokio::test]
async fn org_lifecycle_request_command_counts() {
    for action in Action::ALL {
        let f = fixture(action).await;
        let response = REQUEST.scope((), request(&f, action)).await;
        assert!(
            response.status().is_success(),
            "{action:?}: {}",
            response.status()
        );
        let commands = f.monitor.lock().unwrap().commands.clone();
        println!(
            "ORG_LIFECYCLE_COMMANDS {action:?} total={} commands={commands:?}",
            commands.len()
        );
        let expected = match action {
            Action::Create => 24,
            Action::Profile => 15,
            Action::Set | Action::Clear => 5,
        };
        assert_eq!(commands.len(), expected, "{action:?}");
        f.state.db.drop().await.unwrap();
    }
}

async fn audit_content_matches_legacy(action: Action) {
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME};
    let f = fixture(action).await;
    let response = request(&f, action).await;
    assert!(response.status().is_success());
    let metadata = match action {
        Action::Create => {
            serde_json::json!({"org_user_id":f.org,"display_name":"  Evidence org  "})
        }
        Action::Profile => {
            serde_json::json!({"org_user_id":f.org,"contact_email_changed":true,"remote_credential_integrity_verification_opt_out_changed":true})
        }
        Action::Set => {
            serde_json::json!({"org_user_id":f.org,"role":"member","allowed_service_ids":[]})
        }
        Action::Clear => serde_json::json!({"org_user_id":f.org,"role":"member"}),
    };
    let written = audit_service::notify_on_audit_write_for_user(action.event(), &f.actor);
    audit_service::log_for_user(
        f.state.db.clone(),
        &audit_auth(&f.actor),
        action.event(),
        Some(metadata),
    );
    let old_id = tokio::time::timeout(std::time::Duration::from_secs(5), written)
        .await
        .unwrap()
        .unwrap();
    let collection = f.state.db.collection::<AuditLog>(COLLECTION_NAME);
    let old = collection
        .find_one(doc! {"_id":&old_id})
        .await
        .unwrap()
        .unwrap();
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let new = collection
        .find_one(doc! {"event_type":action.event(),"_id":{"$ne":old_id}})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        new.event_data, old.event_data,
        "{action:?}: no intentional metadata differences"
    );
    assert_eq!(new.user_id, old.user_id);
    assert_eq!(new.api_key_id, old.api_key_id);
    assert_eq!(new.api_key_name, old.api_key_name);
    assert_eq!(new.ip_address, old.ip_address);
    assert_eq!(new.user_agent, old.user_agent);
    f.state.db.drop().await.unwrap();
}
macro_rules! content_test {
    ($name:ident,$action:ident) => {
        #[tokio::test]
        async fn $name() {
            audit_content_matches_legacy(Action::$action).await;
        }
    };
}
content_test!(org_create_audit_content_matches_legacy, Create);
content_test!(org_profile_audit_content_matches_legacy, Profile);
content_test!(org_set_audit_content_matches_legacy, Set);
content_test!(org_clear_audit_content_matches_legacy, Clear);
#[tokio::test]
async fn org_lifecycle_rejected_intent_rolls_back_every_source() {
    rejected_intents(&Action::ALL).await;
}

#[tokio::test]
async fn org_profile_and_creation_rejected_intents_roll_back() {
    rejected_intents(&[Action::Create, Action::Profile]).await;
}

async fn rejected_intents(actions: &[Action]) {
    for &action in actions {
        let f = fixture(action).await;
        let db = &f.state.db;
        let mut before = Vec::new();
        for collection in ["users", "org_memberships", "org_role_scopes"] {
            before.push(snapshot(db, collection).await);
        }
        db.run_command(doc! {"collMod":INTENTS,"validator":{"forbidden":{"$exists":true}}})
            .await
            .unwrap();
        let response = REQUEST.scope((), request(&f, action)).await;
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "{action:?}"
        );
        assert_eq!(json(response).await["error_code"], 1007);
        let commands = f.monitor.lock().unwrap().commands.clone();
        let intent = commands
            .iter()
            .position(|(cmd, col)| cmd == "insert" && col == INTENTS)
            .unwrap();
        let source = action.source_command();
        let position = commands
            .iter()
            .position(|(cmd, col)| cmd == source.0 && col == source.1)
            .unwrap();
        assert!(
            position < intent,
            "source must succeed before rejected intent"
        );
        for (index, collection) in ["users", "org_memberships", "org_role_scopes"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                snapshot(db, collection).await,
                before[index],
                "{action:?}: {collection}"
            );
        }
        assert!(snapshot(db, INTENTS).await.is_empty());
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_lifecycle_response_failure_preserves_committed_action_and_occurrence() {
    for action in [Action::Create, Action::Profile] {
        let f = fixture(action).await;
        let db = &f.state.db;
        // This unrelated malformed override is read by response enrichment,
        // not the enrollment hint's narrow feature-key filter.
        db.collection::<Document>("feature_flag_overrides")
            .insert_one(doc! {"_id":"malformed-response-fixture","org_user_id":&f.org})
            .await
            .unwrap();
        let response = request(&f, action).await;
        assert_eq!(response.status(), 500);
        assert_eq!(json(response).await["error_code"], 1007);
        assert_eq!(
            db.collection::<Document>("users")
                .count_documents(doc! {"_id":&f.org})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<Document>("org_memberships")
                .count_documents(doc! {"org_user_id":&f.org,"member_user_id":&f.actor})
                .await
                .unwrap(),
            1
        );
        assert_eq!(snapshot(db, INTENTS).await.len(), 1);
        security_effect_service::sweep(db, &f.state.encryption_keys)
            .await
            .unwrap();
        assert_eq!(snapshot(db, "audit_log").await.len(), 1);
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_creation_membership_failure_rolls_back_org_and_slug() {
    let f = fixture(Action::Create).await;
    let db = &f.state.db;
    db.collection::<Document>("users")
        .update_one(doc! {"_id":&f.actor}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    let response = request(&f, Action::Create).await;
    assert_eq!(response.status(), 409);
    assert_eq!(
        db.collection::<Document>("users")
            .count_documents(doc! {"_id":&f.org})
            .await
            .unwrap(),
        0
    );
    assert!(snapshot(db, "org_memberships").await.is_empty());
    assert!(snapshot(db, INTENTS).await.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_profile_missing_source_has_existing_error_and_no_occurrence() {
    let f = fixture(Action::Create).await;
    let actor = audit_service::AuditActor::from_auth_user(&audit_auth(&f.actor));
    let result = super::profile::update(
        &f.state.db,
        &f.org,
        super::profile::Update {
            display_name: Some("new"),
            slug: None,
            avatar_url: None,
            contact_email: None,
            remote_credential_integrity_verification_opt_out: None,
        },
        &actor,
    )
    .await;
    assert!(matches!(result,Err(crate::errors::AppError::OrgNotFound(id)) if id==f.org));
    assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_profile_and_create_maximum_occurrences_fit_backstop() {
    use crate::{
        models::security_effect_intent::SecurityEffect,
        services::audit_service::{
            AuditActor,
            required::{self, TextField},
        },
    };
    let raw = "x".repeat(17000);
    let actor = AuditActor {
        user_id: raw.clone(),
        ip_address: Some(raw.clone()),
        user_agent: Some(raw.clone()),
        api_key_id: Some(raw.clone()),
        api_key_name: Some(raw.clone()),
    };
    let patch = super::profile::Update {
        display_name: Some(&raw),
        slug: Some(&raw),
        avatar_url: Some(&raw),
        contact_email: Some(&raw),
        remote_credential_integrity_verification_opt_out: Some(false),
    };
    for event in ["org_created", "org_updated"] {
        let turn = uuid::Uuid::new_v4().to_string();
        let intent =
            crate::turn_trace::scope(crate::turn_trace::TurnContext::for_turn(&turn), async {
                if event == "org_created" {
                    super::profile::created_intent(&actor, &raw, &raw)
                } else {
                    super::profile::updated_intent(&actor, &raw, &patch)
                }
            })
            .await
            .unwrap();
        let SecurityEffect::Audit { event } = intent.effect else {
            panic!("audit")
        };
        let bytes = bson::to_vec(&event).unwrap().len();
        println!(
            "ORG_PROFILE_MAX {} bytes={bytes} margin={}",
            event.event_type,
            16384 - bytes
        );
        assert!(bytes < 16384);
        assert_eq!(
            event.event_data.as_ref().unwrap()["org_user_id"],
            required::text(TextField::Identifier, &raw)
        );
        if event.event_type == "org_created" {
            assert_eq!(
                event.event_data.as_ref().unwrap()["display_name"],
                required::text(TextField::DisplayName, &raw)
            );
        }
        assert!(
            !bson::to_document(&event)
                .unwrap()
                .to_string()
                .contains(&raw)
        );
    }
}

#[tokio::test]
async fn org_creation_deferred_enrollment_and_intent_share_commit() {
    for reject in [false, true] {
        let f = fixture(Action::Create).await;
        let db = &f.state.db;
        // A malformed unrelated membership makes the best-effort hint read fail.
        // Its org/pair differs from the new membership, so source validation
        // still succeeds and production takes the durable repair branch.
        db.collection::<Document>("org_memberships")
            .insert_one(doc! {"_id":"bad-hint-fixture","member_user_id":&f.actor,"revoked_at":null})
            .await
            .unwrap();
        let collections = [
            "users",
            "org_memberships",
            "execution_enrollment_jobs",
            "execution_enrollment_repairs",
        ];
        let mut before = Vec::new();
        for collection in collections {
            before.push(snapshot(db, collection).await);
        }
        if reject {
            db.run_command(doc! {"collMod":INTENTS,"validator":{"forbidden":{"$exists":true}}})
                .await
                .unwrap();
        }
        let response = request(&f, Action::Create).await;
        assert_eq!(response.status().as_u16(), if reject { 500 } else { 201 });
        if reject {
            for (index, collection) in collections.into_iter().enumerate() {
                assert_eq!(
                    snapshot(db, collection).await,
                    before[index],
                    "{collection}"
                );
            }
            assert!(snapshot(db, INTENTS).await.is_empty());
        } else {
            assert_eq!(snapshot(db, "execution_enrollment_jobs").await.len(), 1);
            assert_eq!(snapshot(db, "execution_enrollment_repairs").await.len(), 1);
            assert_eq!(snapshot(db, INTENTS).await.len(), 1);
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_creation_reserved_identity_retry_does_not_duplicate_membership_or_intent() {
    let f = fixture(Action::Create).await;
    let first = request(&f, Action::Create).await;
    assert_eq!(first.status(), 201);
    let first = json(first).await;
    assert_eq!(first["id"], f.org);
    let members = snapshot(&f.state.db, "org_memberships").await;
    let intents = snapshot(&f.state.db, INTENTS).await;
    // The handler has never made an existing reserved UUID a second creation;
    // the assistant's receipt readback recovers it before invoking this path.
    let second = request(&f, Action::Create).await;
    assert_eq!(second.status(), 500);
    assert_eq!(json(second).await["error_code"], 1007);
    assert_eq!(snapshot(&f.state.db, "org_memberships").await, members);
    assert_eq!(snapshot(&f.state.db, INTENTS).await, intents);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn role_scope_clear_absent_emits_no_occurrence() {
    let f = fixture(Action::Set).await;
    let response = request(&f, Action::Clear).await;
    assert_eq!(response.status(), 204);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
    assert!(snapshot(&f.state.db, "audit_log").await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn role_scope_same_value_emits_no_second_occurrence() {
    let f = fixture(Action::Set).await;
    assert!(request(&f, Action::Set).await.status().is_success());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let before =
        snapshot(&f.state.db, INTENTS).await.len() + snapshot(&f.state.db, "audit_log").await.len();
    assert!(request(&f, Action::Set).await.status().is_success());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let after =
        snapshot(&f.state.db, INTENTS).await.len() + snapshot(&f.state.db, "audit_log").await.len();
    assert_eq!(before, after);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_scope_nonempty_and_unrestricted_content_matches_legacy() {
    use crate::models::{audit_log::AuditLog, user_service::UserService};
    for unrestricted in [false, true] {
        let f = fixture(Action::Set).await;
        let id = uuid::Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<UserService>("user_services")
            .insert_one(crate::test_utils::test_user_service(
                &id,
                &f.org,
                "audit-service",
                &uuid::Uuid::new_v4().to_string(),
                None,
                None,
            ))
            .await
            .unwrap();
        let ids = if unrestricted {
            None
        } else {
            Some(vec![id.clone(), id.clone()])
        };
        let response = scopes::set_role_scope(
            State(f.state.clone()),
            audit_auth(&f.actor),
            Path((f.org.clone(), "viewer".into())),
            Json(scopes::UpdateRoleScopeRequest {
                allowed_service_ids: ids.clone(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.0.allowed_service_ids, ids);
        let old_metadata =
            serde_json::json!({"org_user_id":&f.org,"role":"viewer","allowed_service_ids":ids});
        let written = audit_service::notify_on_audit_write_for_user(Action::Set.event(), &f.actor);
        audit_service::log_for_user(
            f.state.db.clone(),
            &audit_auth(&f.actor),
            Action::Set.event(),
            Some(old_metadata),
        );
        let old_id = tokio::time::timeout(std::time::Duration::from_secs(5), written)
            .await
            .unwrap()
            .unwrap();
        security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
            .await
            .unwrap();
        let rows = f.state.db.collection::<AuditLog>("audit_log");
        let old = rows.find_one(doc! {"_id":&old_id}).await.unwrap().unwrap();
        let new = rows
            .find_one(doc! {"_id":{"$ne":&old_id},"event_type":Action::Set.event()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(new.event_data, old.event_data);
        assert_eq!(new.user_id, old.user_id);
        assert_eq!(new.ip_address, old.ip_address);
        assert_eq!(new.user_agent, old.user_agent);
        assert_eq!(new.api_key_id, old.api_key_id);
        assert_eq!(new.api_key_name, old.api_key_name);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn role_scope_noop_preserves_author_time_and_bypasses_rejected_intents() {
    let f = fixture(Action::Set).await;
    assert!(request(&f, Action::Set).await.status().is_success());
    let row = snapshot(&f.state.db, "org_role_scopes").await;
    let intents = snapshot(&f.state.db, INTENTS).await;
    f.state
        .db
        .run_command(doc! {"collMod":INTENTS,"validator":{"forbidden":{"$exists":true}}})
        .await
        .unwrap();
    assert!(request(&f, Action::Set).await.status().is_success());
    assert_eq!(snapshot(&f.state.db, "org_role_scopes").await, row);
    assert_eq!(snapshot(&f.state.db, INTENTS).await, intents);
    f.state
        .db
        .collection::<Document>("org_role_scopes")
        .delete_many(doc! {})
        .await
        .unwrap();
    assert_eq!(request(&f, Action::Clear).await.status(), 204);
    assert_eq!(snapshot(&f.state.db, INTENTS).await, intents);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn role_scope_list_over_budget_succeeds_and_keeps_full_source() {
    use sha2::{Digest, Sha256};
    let f = fixture(Action::Set).await;
    let ids: Vec<String> = (0..80)
        .map(|i| format!("service-{i:03}-{}", "x".repeat(220)))
        .collect();
    for id in &ids {
        f.state
            .db
            .collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(crate::test_utils::test_user_service(
                id,
                &f.org,
                id,
                &uuid::Uuid::new_v4().to_string(),
                None,
                None,
            ))
            .await
            .unwrap();
    }
    let response = scopes::set_role_scope(
        State(f.state.clone()),
        audit_auth(&f.actor),
        Path((f.org.clone(), "member".into())),
        Json(scopes::UpdateRoleScopeRequest {
            allowed_service_ids: Some(ids.clone()),
        }),
    )
    .await
    .unwrap();
    assert_eq!(response.0.allowed_service_ids.as_ref().unwrap(), &ids);
    let row = f
        .state
        .db
        .collection::<Document>(INTENTS)
        .find_one(doc! {"event.event_type":"org_role_scope_set"})
        .await
        .unwrap()
        .unwrap();
    let event = row.get_document("event").unwrap();
    let data = event.get_document("event_data").unwrap();
    let event_bytes = bson::to_vec(event).unwrap().len();
    assert!(event_bytes < 16 * 1024);
    assert_eq!(data.get_i64("allowed_service_ids_count").unwrap(), 80);
    assert_eq!(data.get_bool("allowed_service_ids_truncated"), Ok(true));
    assert_eq!(
        data.get_str("allowed_service_ids_sha256").unwrap(),
        hex::encode(Sha256::digest(ids.join("\n").as_bytes()))
    );
    let kept: Vec<String> = bson::from_bson(bson::Bson::Array(
        data.get_array("allowed_service_ids").unwrap().clone(),
    ))
    .unwrap();
    assert!(kept.len() < ids.len());
    assert_eq!(kept, ids[..kept.len()]);
    let encoded = 5 + kept
        .iter()
        .enumerate()
        .map(|(index, value)| 7 + index.to_string().len() + value.len())
        .sum::<usize>();
    assert!(encoded <= audit_service::required::MAX_IDENTIFIER_LIST_BYTES);
    let stored = f
        .state
        .db
        .collection::<Document>("org_role_scopes")
        .find_one(doc! {"org_user_id":&f.org,"role":"member"})
        .await
        .unwrap()
        .unwrap();
    let stored_ids: Vec<String> = bson::from_bson(bson::Bson::Array(
        stored.get_array("allowed_service_ids").unwrap().clone(),
    ))
    .unwrap();
    assert_eq!(stored_ids, ids);
    assert_eq!(
        data.get_str("allowed_service_ids_sha256").unwrap(),
        hex::encode(Sha256::digest(stored_ids.join("\n").as_bytes()))
    );
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let events = snapshot(&f.state.db, "audit_log").await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].get_document("event_data").unwrap(), data);
    f.state.db.drop().await.unwrap();
}
