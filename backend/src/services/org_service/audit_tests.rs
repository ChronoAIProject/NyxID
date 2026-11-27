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

use crate::handlers::orgs as handlers;
use crate::models::{
    org_invite::OrgInvite,
    org_membership::{MemberScopeSource, OrgMembership, OrgRole},
    user::{User, UserType},
};
use crate::services::{audit_service, security_effect_service};
use crate::test_utils::{test_app_state, test_auth_user, test_user};

const INTENTS: &str = "security_effect_intents";
tokio::task_local! { static REQUEST: (); }

#[derive(Clone, Copy, Debug)]
enum Action {
    Add,
    Update,
    Revoke,
    Issue,
    Cancel,
    Join,
}
impl Action {
    const ALL: [Self; 6] = [
        Self::Add,
        Self::Update,
        Self::Revoke,
        Self::Issue,
        Self::Cancel,
        Self::Join,
    ];
    fn event(self) -> &'static str {
        match self {
            Self::Add => "org_member_added",
            Self::Update => "org_member_updated",
            Self::Revoke => "org_member_revoked",
            Self::Issue => "org_invite_created",
            Self::Cancel => "org_invite_cancelled",
            Self::Join => "org_member_joined",
        }
    }
    fn source_command(self) -> (&'static str, &'static str) {
        match self {
            Self::Add | Self::Join => ("insert", "org_memberships"),
            Self::Update => ("findAndModify", "org_memberships"),
            Self::Revoke => ("update", "org_memberships"),
            Self::Issue => ("insert", "org_invites"),
            Self::Cancel => ("delete", "org_invites"),
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
    target: String,
    org: String,
    invite: String,
    nonce: String,
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
    let invite = uuid::Uuid::new_v4().to_string();
    let nonce = format!("ORGINV-{}", uuid::Uuid::new_v4().simple());
    db.collection::<User>("users")
        .insert_many([
            test_user(&actor, UserType::Person),
            test_user(&target, UserType::Person),
            test_user(&org, UserType::Org),
        ])
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
    if matches!(action, Action::Update | Action::Revoke) {
        db.collection::<OrgMembership>("org_memberships")
            .insert_one(crate::test_utils::test_membership(
                &org,
                &target,
                OrgRole::Member,
                None,
            ))
            .await
            .unwrap();
    }
    let now = chrono::Utc::now();
    db.collection::<OrgInvite>("org_invites")
        .insert_one(OrgInvite {
            id: invite.clone(),
            org_user_id: org.clone(),
            nonce: nonce.clone(),
            role: OrgRole::Member,
            scope_source: MemberScopeSource::Override,
            allowed_service_ids: None,
            created_by: actor.clone(),
            expires_at: now + chrono::Duration::hours(1),
            redeemed_by: None,
            redeemed_at: None,
            created_at: now,
        })
        .await
        .unwrap();
    security_effect_service::ensure_indexes(&db).await.unwrap();
    Fixture {
        state: test_app_state(db),
        actor,
        target,
        org,
        invite,
        nonce,
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
fn event_actor(f: &Fixture, action: Action) -> &str {
    if matches!(action, Action::Join) {
        &f.target
    } else {
        &f.actor
    }
}
async fn request(f: &Fixture, action: Action) -> axum::response::Response {
    let state = State(f.state.clone());
    let auth = audit_auth(event_actor(f, action));
    match action {
        Action::Add => handlers::add_member(
            state,
            auth,
            Path(f.org.clone()),
            Json(handlers::AddMemberRequest {
                user_id: f.target.clone(),
                role: handlers::OrgRoleWire::Member,
                scope_source: Some(handlers::MemberScopeSourceWire::Override),
                allowed_service_ids: None,
            }),
        )
        .await
        .into_response(),
        Action::Update => handlers::update_member(
            state,
            auth,
            Path((f.org.clone(), f.target.clone())),
            Json(handlers::UpdateMemberRequest {
                role: Some(handlers::OrgRoleWire::Viewer),
                scope_source: None,
                allowed_service_ids: None,
            }),
        )
        .await
        .into_response(),
        Action::Revoke => {
            handlers::remove_member(state, auth, Path((f.org.clone(), f.target.clone())))
                .await
                .into_response()
        }
        Action::Issue => handlers::create_invite(
            state,
            auth,
            Path(f.org.clone()),
            Json(handlers::CreateInviteRequest {
                role: handlers::OrgRoleWire::Member,
                scope_source: Some(handlers::MemberScopeSourceWire::Override),
                allowed_service_ids: None,
                ttl_hours: Some(24),
            }),
        )
        .await
        .into_response(),
        Action::Cancel => {
            handlers::cancel_invite(state, auth, Path((f.org.clone(), f.invite.clone())))
                .await
                .into_response()
        }
        Action::Join => handlers::redeem_invite(state, auth, Path(f.nonce.clone()))
            .await
            .into_response(),
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
        Action::Add | Action::Join => assert_eq!(
            db.collection::<Document>("org_memberships")
                .count_documents(
                    doc! {"org_user_id":&f.org,"member_user_id":&f.target,"revoked_at":null}
                )
                .await
                .unwrap(),
            1
        ),
        Action::Update => assert_eq!(
            db.collection::<Document>("org_memberships")
                .count_documents(
                    doc! {"org_user_id":&f.org,"member_user_id":&f.target,"role":"viewer"}
                )
                .await
                .unwrap(),
            1
        ),
        Action::Revoke => assert_eq!(
            db.collection::<Document>("org_memberships")
                .count_documents(
                    doc! {"org_user_id":&f.org,"member_user_id":&f.target,"revoked_at":{"$ne":null}}
                )
                .await
                .unwrap(),
            1
        ),
        Action::Issue => assert_eq!(snapshot(db, "org_invites").await.len(), 2),
        Action::Cancel => assert!(snapshot(db, "org_invites").await.is_empty()),
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
crash_test!(org_add_audit_survives_process_death, Add);
crash_test!(org_update_audit_survives_process_death, Update);
crash_test!(org_revoke_audit_survives_process_death, Revoke);
crash_test!(org_issue_audit_survives_process_death, Issue);
crash_test!(org_cancel_audit_survives_process_death, Cancel);
crash_test!(org_join_audit_survives_process_death, Join);
#[tokio::test]
async fn org_audit_request_command_counts() {
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
            "ORG_COMMANDS {action:?} total={} commands={commands:?}",
            commands.len()
        );
        // Add and Join read both schedule timings. Issue fences the org; Issue
        // and Join each fence the issuer and read the active admin membership.
        let expected = match action {
            Action::Add => 20,
            Action::Update => 19,
            Action::Revoke => 16,
            Action::Issue => 10,
            Action::Cancel => 6,
            Action::Join => 19,
        };
        assert_eq!(commands.len(), expected, "{action:?}");
        let policy_commands: Vec<_> = commands
            .iter()
            .filter(|(_, collection)| {
                collection == crate::models::service_approval_config::COLLECTION_NAME
            })
            .map(|(command, _)| command.as_str())
            .collect();
        if matches!(action, Action::Revoke) {
            assert_eq!(policy_commands, ["update"]);
        } else {
            assert!(policy_commands.is_empty());
        }
        let specialist_commands: Vec<_> = commands
            .iter()
            .filter(|(_, collection)| collection == crate::models::assistant_agent::COLLECTION_NAME)
            .map(|(command, _)| command.as_str())
            .collect();
        if matches!(action, Action::Update | Action::Revoke) {
            // The demotion and removal each prune explicit specialist selections in one write.
            assert_eq!(specialist_commands, ["update"], "{action:?}");
        } else {
            assert!(specialist_commands.is_empty(), "{action:?}");
        }
        assert_eq!(
            commands
                .iter()
                .filter(|(command, collection)| command == "insert" && collection == INTENTS)
                .count(),
            1
        );
        f.state.db.drop().await.unwrap();
    }
}

async fn audit_content_matches_legacy(action: Action) {
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME};
    let f = fixture(action).await;
    let response = request(&f, action).await;
    assert!(response.status().is_success());
    let body = if matches!(action, Action::Cancel | Action::Revoke) {
        serde_json::Value::Null
    } else {
        json(response).await
    };
    let intents = snapshot(&f.state.db, INTENTS).await;
    assert_eq!(intents.len(), 1);
    let serialized = serde_json::to_string(&intents[0]).unwrap();
    assert!(!serialized.contains(&f.nonce));
    if let Some(nonce) = body.get("nonce").and_then(serde_json::Value::as_str) {
        assert!(!serialized.contains(nonce));
    }
    assert!(!serialized.contains("ORGINV-"));
    // Verbatim fields from each former handler-side log_for_user constructor.
    let membership = f
        .state
        .db
        .collection::<OrgMembership>("org_memberships")
        .find_one(doc! {"org_user_id":&f.org,"member_user_id":&f.target})
        .await
        .unwrap();
    let metadata = match action {
        Action::Add => {
            serde_json::json!({"org_user_id":f.org,"member_user_id":f.target,"role":"member"})
        }
        Action::Update | Action::Revoke => {
            serde_json::json!({"org_user_id":f.org,"member_user_id":f.target})
        }
        Action::Issue => serde_json::json!({"org_user_id":f.org,"invite_id":body["id"]}),
        Action::Cancel => serde_json::json!({"org_user_id":f.org,"invite_id":f.invite}),
        Action::Join => {
            let m = membership.unwrap();
            serde_json::json!({"org_user_id":m.org_user_id,"membership_id":m.id,"role":m.role})
        }
    };
    let written =
        audit_service::notify_on_audit_write_for_user(action.event(), event_actor(&f, action));
    audit_service::log_for_user(
        f.state.db.clone(),
        &audit_auth(event_actor(&f, action)),
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
content_test!(org_add_audit_content_matches_legacy, Add);
content_test!(org_update_audit_content_matches_legacy, Update);
content_test!(org_revoke_audit_content_matches_legacy, Revoke);
content_test!(org_issue_audit_content_matches_legacy, Issue);
content_test!(org_cancel_audit_content_matches_legacy, Cancel);
content_test!(org_join_audit_content_matches_legacy, Join);
#[tokio::test]
async fn org_rejected_intent_rolls_back_every_source() {
    for action in Action::ALL {
        let f = fixture(action).await;
        let db = &f.state.db;
        let mut before = Vec::new();
        for collection in ["users", "org_memberships", "org_invites"] {
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
        for (index, collection) in ["users", "org_memberships", "org_invites"]
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
async fn org_audit_maximum_occurrences_and_utf8_caps() {
    use crate::models::security_effect_intent::SecurityEffect;
    use crate::services::audit_service::{
        AuditActor,
        required::{self, TextField},
    };
    let raw = "界".repeat(17 * 1024);
    let actor = AuditActor {
        user_id: raw.clone(),
        ip_address: Some(raw.clone()),
        user_agent: Some(raw.clone()),
        api_key_id: Some(raw.clone()),
        api_key_name: Some(raw.clone()),
    };
    let mut member = crate::test_utils::test_membership(&raw, &raw, OrgRole::Member, None);
    member.id = raw.clone();
    for action in Action::ALL {
        let build = || match action {
            Action::Issue | Action::Cancel => {
                super::audit::invite_intent(&actor, action.event(), &raw, &raw)
            }
            _ => super::audit::member_intent(&actor, action.event(), &member),
        };
        let turn = uuid::Uuid::new_v4().to_string();
        let intent =
            crate::turn_trace::scope(crate::turn_trace::TurnContext::for_turn(&turn), async {
                build()
            })
            .await
            .unwrap();
        let SecurityEffect::Audit { event } = intent.effect else {
            panic!("audit")
        };
        let data = event.event_data.as_ref().unwrap();
        for key in [
            "org_user_id",
            "member_user_id",
            "membership_id",
            "invite_id",
        ] {
            if let Some(value) = data.get(key) {
                assert_eq!(
                    value.as_str().unwrap(),
                    required::text(TextField::Identifier, &raw)
                );
                assert!(value.as_str().unwrap().ends_with("...[truncated]"));
            }
        }
        assert!(
            event
                .user_agent
                .as_ref()
                .unwrap()
                .ends_with("...[truncated]")
        );
        assert!(bson::to_vec(&event).unwrap().len() < 16384);
    }
    // ASCII reaches every cap exactly (UTF-8 truncation can leave 1-2 unused bytes).
    let raw = "x".repeat(17 * 1024);
    let actor = AuditActor {
        user_id: raw.clone(),
        ip_address: Some(raw.clone()),
        user_agent: Some(raw.clone()),
        api_key_id: Some(raw.clone()),
        api_key_name: Some(raw.clone()),
    };
    member.id = raw.clone();
    member.org_user_id = raw.clone();
    member.member_user_id = raw.clone();
    for action in Action::ALL {
        let turn = uuid::Uuid::new_v4().to_string();
        let intent =
            crate::turn_trace::scope(crate::turn_trace::TurnContext::for_turn(&turn), async {
                match action {
                    Action::Issue | Action::Cancel => {
                        super::audit::invite_intent(&actor, action.event(), &raw, &raw)
                    }
                    _ => super::audit::member_intent(&actor, action.event(), &member),
                }
            })
            .await
            .unwrap();
        let SecurityEffect::Audit { event } = intent.effect else {
            panic!("audit")
        };
        let bytes = bson::to_vec(&event).unwrap().len();
        println!(
            "ORG_AUDIT_MAX {} bytes={bytes} margin={}",
            action.event(),
            16384 - bytes
        );
        assert!(bytes < 16384);
    }
}

#[tokio::test]
async fn org_missing_sources_do_not_publish_occurrences() {
    use crate::errors::AppError;
    use crate::services::{org_invite_service, org_service};
    let f = fixture(Action::Add).await;
    let db = &f.state.db;
    let actor = audit_service::AuditActor::from_auth_user(&audit_auth(&f.actor));
    let missing = uuid::Uuid::new_v4().to_string();
    assert!(
        matches!(org_service::update_membership_as(db,&missing,Some(OrgRole::Viewer),None,None,&actor).await,Err(AppError::NotFound(message)) if message=="Active membership not found")
    );
    let before = snapshot(db, "users").await;
    assert!(
        matches!(org_service::revoke_membership_as(db,&f.org,&missing,&actor).await,Err(AppError::NotFound(message)) if message=="Active membership not found")
    );
    assert_eq!(
        before,
        snapshot(db, "users").await,
        "org fence must abort too"
    );
    assert!(
        matches!(org_invite_service::cancel_invite_as(db,&f.org,&missing,&actor).await,Err(AppError::OrgInviteInvalid(message)) if message=="invite not found")
    );
    assert!(
        matches!(org_invite_service::redeem_invite_as(db,"unknown",&f.target,&actor).await,Err(AppError::OrgInviteInvalid(message)) if message=="unknown invite")
    );
    assert!(snapshot(db, INTENTS).await.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_duplicate_rejoin_noop_and_invite_errors_keep_existing_semantics() {
    use crate::{
        errors::AppError,
        services::{org_invite_service, org_service},
    };
    let f = fixture(Action::Update).await;
    let db = &f.state.db;
    let actor = audit_service::AuditActor::from_auth_user(&audit_auth(&f.actor));
    let before = snapshot(db, "org_memberships").await;
    assert!(
        matches!(org_invite_service::redeem_invite_as(db,&f.nonce,&f.target,&actor).await,Err(AppError::Conflict(message)) if message=="User is already a member of this org")
    );
    assert_eq!(before, snapshot(db, "org_memberships").await);
    assert_eq!(
        snapshot(db, "org_invites").await[0].get("redeemed_at"),
        Some(&bson::Bson::Null)
    );
    assert!(snapshot(db, INTENTS).await.is_empty());
    let member = org_service::get_active_membership(db, &f.org, &f.target)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(org_service::update_membership_as(db,&member.id,None,None,None,&actor).await,Err(AppError::BadRequest(message)) if message=="No fields to update")
    );
    assert!(snapshot(db, INTENTS).await.is_empty());
    let unchanged = org_service::update_membership_as(
        db,
        &member.id,
        Some(OrgRole::Member),
        None,
        None,
        &actor,
    )
    .await
    .unwrap();
    assert_eq!(unchanged.role, member.role);
    assert_eq!(
        snapshot(db, INTENTS).await.len(),
        1,
        "same-value save was audited before too"
    );
    org_service::revoke_membership_as(db, &f.org, &f.target, &actor)
        .await
        .unwrap();
    let rejoined = org_invite_service::redeem_invite_as(db, &f.nonce, &f.target, &actor)
        .await
        .unwrap();
    assert_eq!(rejoined.id, member.id);
    assert!(rejoined.created_at > member.created_at);
    let count = snapshot(db, INTENTS).await.len();
    assert!(
        matches!(org_invite_service::redeem_invite_as(db,&f.nonce,&f.target,&actor).await,Err(AppError::OrgInviteInvalid(message)) if message=="invite already redeemed")
    );
    assert!(
        matches!(org_invite_service::cancel_invite_as(db,&f.org,&f.invite,&actor).await,Err(AppError::OrgInviteInvalid(message)) if message=="cannot cancel a redeemed invite")
    );
    assert_eq!(snapshot(db, INTENTS).await.len(), count);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_invite_cancel_and_join_publish_only_the_winner() {
    use crate::services::org_invite_service;
    let f = fixture(Action::Join).await;
    let db = &f.state.db;
    let independent =
        mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap()
            .database(db.name());
    let actor = audit_service::AuditActor::from_auth_user(&audit_auth(&f.actor));
    let person = audit_service::AuditActor::from_auth_user(&audit_auth(&f.target));
    let (cancel, join) = tokio::join!(
        org_invite_service::cancel_invite_as(db, &f.org, &f.invite, &actor),
        org_invite_service::redeem_invite_as(&independent, &f.nonce, &f.target, &person),
    );
    assert_eq!(usize::from(cancel.is_ok()) + usize::from(join.is_ok()), 1);
    assert_eq!(snapshot(db, INTENTS).await.len(), 1);
    if join.is_ok() {
        assert_eq!(snapshot(db, "org_invites").await.len(), 1);
        assert_eq!(
            db.collection::<Document>("org_memberships")
                .count_documents(doc! {"member_user_id":&f.target})
                .await
                .unwrap(),
            1
        );
    } else {
        assert!(snapshot(db, "org_invites").await.is_empty());
        assert_eq!(
            db.collection::<Document>("org_memberships")
                .count_documents(doc! {"member_user_id":&f.target})
                .await
                .unwrap(),
            0
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_hot_owner_resolution_has_no_audit_commands() {
    let f = fixture(Action::Add).await;
    let direct = REQUEST
        .scope(
            (),
            super::resolve_owner_access(&f.state.db, &f.actor, &f.actor),
        )
        .await
        .unwrap();
    assert!(matches!(direct, super::OwnerAccess::Direct));
    assert!(f.monitor.lock().unwrap().commands.is_empty());
    let org = REQUEST
        .scope(
            (),
            super::resolve_owner_access(&f.state.db, &f.actor, &f.org),
        )
        .await
        .unwrap();
    assert!(matches!(org, super::OwnerAccess::AsOrgAdmin { .. }));
    let commands = f.monitor.lock().unwrap().commands.clone();
    assert_eq!(
        commands,
        [
            ("find".into(), "users".into()),
            ("find".into(), "org_memberships".into())
        ]
    );
    println!("ORG_HOT_COMMANDS direct=0 org_override=2 added=0");
    assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}
