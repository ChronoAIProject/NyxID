use super::*;
use std::future::IntoFuture;

async fn changed_before_write(action: Action, competing_assignment: bool) {
    let mut f = fixture(action).await;
    let original = f.state.db.clone();
    let id = if matches!(action, Action::Update | Action::Delete) {
        f.group.clone()
    } else {
        f.target.clone()
    };
    let group = f.group.clone();
    let source = action.source_command();
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = fired.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        let CommandEvent::Started(event) = event else {
            return;
        };
        if event.command_name == source.0
            && event.command.get_str(source.0) == Ok(source.1)
            && !observed.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    if competing_assignment {
                        let result = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            original
                                .collection::<Document>("users")
                                .update_one(
                                    doc! {"_id":&id},
                                    doc! {"$addToSet":{"group_ids":&group}},
                                )
                                .into_future(),
                        )
                        .await
                        .unwrap()
                        .unwrap();
                        assert_eq!(result.modified_count, 1);
                    } else {
                        let result = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            original
                                .collection::<Document>(source.1)
                                .delete_one(doc! {"_id":&id})
                                .into_future(),
                        )
                        .await
                        .unwrap()
                        .unwrap();
                        assert_eq!(result.deleted_count, 1);
                    }
                })
            });
        }
    }));
    f.state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name());
    let response = request(&f, action).await;
    assert!(fired.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        response.status().as_u16(),
        if competing_assignment { 409 } else { 404 }
    );
    assert_eq!(
        json(response).await["error_code"],
        if competing_assignment {
            4004
        } else if matches!(action, Action::Add) {
            1003
        } else {
            4001
        }
    );
    assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    assert!(snapshot(&f.state.db, "audit_log").await.is_empty());
    if matches!(action, Action::Delete) {
        // Failed deletion must also roll back preceding membership cleanup.
        assert_eq!(
            f.state
                .db
                .collection::<Document>("users")
                .count_documents(doc! {"group_ids":&f.group})
                .await
                .unwrap(),
            1
        );
    }
    f.state.db.drop().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disappeared_group_update_has_no_audit() {
    changed_before_write(Action::Update, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disappeared_group_delete_has_no_audit() {
    changed_before_write(Action::Delete, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disappeared_assignment_user_has_no_audit() {
    changed_before_write(Action::Add, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_assignment_loser_has_no_audit() {
    changed_before_write(Action::Add, true).await;
}

#[tokio::test]
async fn removing_absent_membership_is_success_without_an_intent() {
    let f = fixture(Action::Add).await;
    let written = audit_service::notify_on_audit_write_for_user(Action::Remove.event(), &f.actor);
    assert!(request(&f, Action::Remove).await.status().is_success());
    assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), written)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<Document>("users")
        .delete_one(doc! {"_id":&f.target})
        .await
        .unwrap();
    assert!(request(&f, Action::Remove).await.status().is_success());
    assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn maximum_group_inputs_preserve_source_and_bound_audit() {
    let f = fixture(Action::Create).await;
    let mut auth = audit_auth(&f.actor);
    auth.ip_address = Some("p".repeat(64 * 1024));
    auth.user_agent = Some("u".repeat(64 * 1024));
    auth.api_key_name = Some("n".repeat(64 * 1024));
    let name = "n".repeat(32 * 1024);
    let description = "d".repeat(32 * 1024);
    let written = audit_service::notify_on_audit_write_for_user("admin.group.created", &f.actor);
    let Json(response) = handlers::create_group(
        State(f.state.clone()),
        auth,
        HeaderMap::new(),
        Json(handlers::CreateGroupRequest {
            name: name.clone(),
            slug: "s".repeat(32 * 1024),
            description: Some(description.clone()),
            role_ids: vec![],
            parent_group_id: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(response.name, name);
    assert_eq!(response.description, Some(description));
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), written)
        .await
        .unwrap()
        .unwrap();
    let row = f
        .state
        .db
        .collection::<Document>("audit_log")
        .find_one(doc! {"event_type":"admin.group.created"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("ip_address").unwrap().len(), 256);
    assert_eq!(row.get_str("api_key_name").unwrap().len(), 256);
    assert_eq!(row.get_str("user_agent").unwrap().len(), 1024);
    assert!(
        row.get_str("ip_address")
            .unwrap()
            .ends_with("...[truncated]")
    );
    assert_eq!(
        row.get_document("event_data")
            .unwrap()
            .get_str("group_slug")
            .unwrap(),
        format!("{}...[truncated]", "s".repeat(242))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn maximum_group_occurrences_fit_backstop() {
    use super::super::audit;
    use crate::{
        models::security_effect_intent::SecurityEffect,
        services::audit_service::{
            AuditActor,
            required::{TextField, text},
        },
    };
    let raw = "x".repeat(64 * 1024);
    let max = |field| text(field, &raw);
    let actor = AuditActor {
        user_id: max(TextField::Identifier),
        ip_address: Some(max(TextField::ForwardedIp)),
        user_agent: Some(max(TextField::UserAgent)),
        api_key_id: Some(max(TextField::Identifier)),
        api_key_name: Some(max(TextField::KeyName)),
    };
    for action in Action::ALL {
        let turn = uuid::Uuid::new_v4().to_string();
        let intent =
            crate::turn_trace::scope(crate::turn_trace::TurnContext::for_turn(&turn), async {
                match action {
                    Action::Create | Action::Update => {
                        audit::group_intent(&actor, action.event(), &raw, Some(&raw))
                    }
                    Action::Delete => audit::group_intent(&actor, action.event(), &raw, None),
                    Action::Add => audit::Membership::Add.intent(&actor, &raw, &raw),
                    Action::Remove => audit::Membership::Remove.intent(&actor, &raw, &raw),
                }
            })
            .await
            .unwrap();
        let SecurityEffect::Audit { event } = intent.effect else {
            panic!("expected audit")
        };
        let bytes = bson::to_vec(&event).unwrap().len();
        assert!(bytes < 16 * 1024);
        let metadata = event.event_data.as_ref().unwrap();
        assert_eq!(metadata["group_id"].as_str().unwrap().len(), 256);
        assert!(
            metadata["group_id"]
                .as_str()
                .unwrap()
                .ends_with("...[truncated]")
        );
        for key in ["group_slug", "target_user_id"] {
            if let Some(value) = metadata.get(key) {
                assert_eq!(value.as_str().unwrap(), max(TextField::Identifier));
            }
        }
        println!(
            "GROUP_AUDIT_MAX {} bytes={bytes} margin={}",
            action.event(),
            16 * 1024 - bytes
        );
    }
}
