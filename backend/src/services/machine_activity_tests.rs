use super::*;
use crate::{
    services::{assistant_authority_tests::orchestrator_fixture, machine_integration_tests::node},
    test_utils::test_auth_user,
};
use axum::extract::{Path, Query, State};
use serde_json::json;

#[test]
fn machine_receipt_context_mode_is_additive_and_survives_response_projection() {
    let mut value = json!({"operation_id":"operation","node_id":"node","agent_id":"agent",
        "action":"browser.navigate","status":"completed","job_id":null,"exit_code":null,
        "bytes":null,"duration_ms":null,"error_code":null,"screenshot_id":null,
        "preview_id":null,"preview_enabled":false});
    let legacy: MachineReceipt = serde_json::from_value(value.clone()).unwrap();
    assert!(legacy.context_mode.is_none());
    value["context_mode"] = json!("separated");
    let row: MachineReceipt = serde_json::from_value(value).unwrap();
    let response = crate::handlers::assistant_nyxagent::MachineReceiptResponse::from(row);
    assert_eq!(response.context_mode.as_deref(), Some("separated"));
}

#[test]
fn machine_receipt_action_and_excerpt_exclude_privacy_sentinels() {
    assert_eq!(
        action(
            Operation::Browser,
            &json!({"action":"https://private-sentinel.test"})
        ),
        "browser.action"
    );
    let text = excerpt(
        &json!({"stdout":"hello\u{001b}[31m world\u{001b}[0m\nAPI_KEY=private-sentinel\nAuthorization: Bearer credential-sentinel\n","stderr":"Cookie: session=secret-sentinel"}),
    );
    assert!(text.contains("hello world"));
    for secret in [
        "private-sentinel",
        "credential-sentinel",
        "secret-sentinel",
        "\u{001b}",
    ] {
        assert!(!text.contains(secret));
    }
    let text = excerpt(&json!({"stdout":"💡".repeat(5000),"stderr":"x\n".repeat(500)}));
    assert!(text.len() < 3200);
    assert!(text.lines().count() <= 44);
    assert_eq!(
        outcome(&json!({"isError":true}), Operation::Computer),
        "error"
    );
    assert_eq!(
        outcome(&json!({"status":"running"}), Operation::Exec),
        "running"
    );
    assert_eq!(
        outcome(&json!({"status":"finished","exit_code":7}), Operation::Exec),
        "error"
    );
    assert_eq!(
        outcome(&json!({"status":"finished"}), Operation::JobCancel),
        "cancelled"
    );
    assert_eq!(
        transferred_bytes(
            Operation::ReadFile,
            &json!({"offset":4000}),
            &json!({"offset":8096,"size":50000})
        ),
        Some(4096)
    );
    assert_eq!(
        transferred_bytes(
            Operation::ReadFile,
            &json!({"offset":60000}),
            &json!({"offset":50000,"size":50000})
        ),
        Some(0)
    );
    assert_eq!(
        transferred_bytes(Operation::WriteFile, &json!({}), &json!({"bytes":17})),
        Some(17)
    );
    assert_eq!(
        transferred_bytes(Operation::EditFile, &json!({}), &json!({"sha256":"digest"})),
        None
    );
    assert_eq!(
        action(Operation::Browser, &json!({"action":"tabs_new"})),
        "browser.tabs_new"
    );
    assert_eq!(
        action(Operation::Browser, &json!({"action":"press"})),
        "browser.press_key"
    );
}

#[tokio::test]
async fn machine_receipt_jobs_finish_cancel_and_do_not_resurrect_late_outcomes() {
    use crate::models::assistant_conversation::TurnActivity;
    let f = Box::pin(orchestrator_fixture("machine_receipt_jobs")).await;
    let node = Uuid::new_v4().to_string();
    let mut receipts: Vec<_> = (0..4)
        .map(|_| {
            receipt(
                &f.chat,
                &node,
                Operation::Exec,
                &json!({"job_id":Uuid::new_v4().to_string()}),
            )
        })
        .collect();
    for (i, receipt) in receipts.iter().take(3).enumerate() {
        f.state.db.collection::<bson::Document>(crate::models::machine_job::COLLECTION_NAME).insert_one(doc! {
            "_id":&receipt.job_id,"user_id":&f.owner,"state":"finished","exit_code":if i==0 {0_i64}else{2_i64},"receipt_cancelled":i==2,
        }).await.unwrap();
    }
    refresh_jobs(&f.state.db, &f.owner, receipts.iter_mut().collect())
        .await
        .unwrap();
    assert_eq!(
        receipts
            .iter()
            .map(|r| r.status.as_str())
            .collect::<Vec<_>>(),
        ["finished", "error", "cancelled", "unknown"]
    );
    let mut activity = TurnActivity {
        id: Uuid::new_v4().to_string(),
        label: "nyx__machine_exec".into(),
        status: "completed".into(),
        started_at: Utc::now(),
        ended_at: Some(Utc::now()),
        machine: Some(Box::new(receipt(
            &f.chat,
            &node,
            Operation::Exec,
            &json!({}),
        ))),
    };
    // Background jobs remain running after successful tool admission/turn settlement.
    settle_activity(&mut activity, None);
    assert_eq!(activity.machine.as_ref().unwrap().status, "running");
    settle_activity(&mut activity, Some("cancelled"));
    assert_eq!(activity.machine.as_ref().unwrap().status, "cancelled");
    activity.status = "running".into();
    activity.machine.as_mut().unwrap().status = "running".into();
    settle_activity(&mut activity, Some("assistant_unavailable"));
    assert_eq!(activity.machine.as_ref().unwrap().status, "error");
    // Typed MCP failure also wins when post-processing, rather than the node,
    // failed. A successful turn must not leave that card running forever.
    activity.status = "error".into();
    activity.machine.as_mut().unwrap().status = "running".into();
    settle_activity(&mut activity, None);
    assert_eq!(activity.machine.as_ref().unwrap().status, "error");

    let activity_id = super::super::assistant_nyxagent::activity_started(
        &f.state.db,
        &f.owner,
        &f.row.id,
        "nyx__machine_browser",
    )
    .await
    .unwrap()
    .unwrap();
    let mut pending = receipt(&f.chat, &node, Operation::Browser, &json!({}));
    for (initial, expected) in [("running", "error"), ("cancelled", "cancelled")] {
        pending.status = initial.into();
        ACTIVITY_ID
            .scope(
                Some(activity_id.clone()),
                record(&f.state.db, &f.chat, &pending),
            )
            .await
            .unwrap();
        super::super::assistant_nyxagent::activity_finished(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &activity_id,
            false,
        )
        .await
        .unwrap();
        let row = super::super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap();
        let turn = row.active_turn.unwrap();
        let completed = turn
            .activities
            .iter()
            .find(|a| a.id == activity_id)
            .unwrap();
        assert_eq!(completed.status, "error");
        assert_eq!(completed.machine.as_ref().unwrap().status, expected);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_preview_consent_encryption_acl_expiry_and_late_result() {
    let f = Box::pin(crate::services::assistant_authority_tests::fixture(
        "machine_preview_acl",
    ))
    .await;
    let activity = super::super::assistant_nyxagent::activity_started(
        &f.state.db,
        &f.owner,
        &f.row.id,
        "nyx__machine_exec",
    )
    .await
    .unwrap()
    .unwrap();
    let id = Uuid::new_v4().to_string();
    let mut receipt = receipt(&f.chat, &id, Operation::Exec, &json!({}));
    assert!(!preview_enabled(&f.state.db, &f.chat).await.unwrap());
    let auth = test_auth_user(&f.owner);
    let _ = crate::handlers::machine_activity::preview_policy(
        State(f.state.clone()),
        auth,
        Path(f.row.id.clone()),
        axum::Json(crate::handlers::machine_activity::PreviewPolicy { enabled: true }),
    )
    .await
    .unwrap();
    receipt.preview_enabled = preview_enabled(&f.state.db, &f.chat).await.unwrap();
    assert!(receipt.preview_enabled);
    ACTIVITY_ID
        .scope(Some(activity.clone()), async {
            record(&f.state.db, &f.chat, &receipt).await.unwrap();
            store_preview(
                &f.state,
                &f.chat,
                &mut receipt,
                &json!({"stdout":"output-sentinel\nTOKEN=credential-sentinel"}),
            )
            .await
            .unwrap();
        })
        .await;
    let attachment = receipt.preview_id.as_ref().unwrap();
    let row = f
        .state
        .db
        .collection::<bson::Document>(ATTACHMENTS)
        .find_one(doc! {"_id":attachment})
        .await
        .unwrap()
        .unwrap();
    assert!(!format!("{row:?}").contains("output-sentinel"));
    let (_, bytes) = super::super::assistant_nyxagent::read_attachment(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
        attachment,
    )
    .await
    .unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("output-sentinel"));
    assert!(!String::from_utf8_lossy(&bytes).contains("credential-sentinel"));
    let wrong = Uuid::new_v4().to_string();
    assert!(
        super::super::assistant_nyxagent::read_attachment(
            &f.state.db,
            &f.state.encryption_keys,
            &wrong,
            &f.row.id,
            attachment
        )
        .await
        .is_err()
    );
    assert!(
        super::super::assistant_nyxagent::read_attachment(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &f.nyxbot_thread,
            attachment
        )
        .await
        .is_err()
    );
    // Agent keys cannot opt in, including another specialist's key.
    assert!(
        crate::handlers::machine_activity::preview_policy(
            State(f.state.clone()),
            f.auth.clone(),
            Path(f.row.id.clone()),
            axum::Json(crate::handlers::machine_activity::PreviewPolicy { enabled: true })
        )
        .await
        .is_err()
    );
    // A guest task cannot capture output even if private consent was previously on.
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&f.row.id}, doc! {"$set":{"guest_turn":true}})
        .await
        .unwrap();
    assert!(!preview_enabled(&f.state.db, &f.chat).await.unwrap());
    assert!(
        preview_policy(&f.state.db, &f.owner, &f.row.id, Some(true))
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&f.row.id}, doc! {"$set":{"guest_turn":false}})
        .await
        .unwrap();
    // Shorter live document policy applies without waiting for a sweep.
    f.state.db.collection::<bson::Document>(ATTACHMENTS).update_one(doc!{"_id":attachment},doc!{"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::days(31))}}).await.unwrap();
    assert!(matches!(
        super::super::assistant_nyxagent::read_attachment(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &f.row.id,
            attachment
        )
        .await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    receipt.preview_id = None;
    ACTIVITY_ID
        .scope(
            Some(activity),
            store_preview(
                &f.state,
                &f.chat,
                &mut receipt,
                &json!({"stdout":"late-sentinel"}),
            ),
        )
        .await
        .unwrap();
    assert!(receipt.preview_id.is_none());
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn":bson::Bson::Null}},
        )
        .await
        .unwrap();
    Box::pin(super::super::assistant_nyxagent::delete(
        &f.state.db,
        &f.owner,
        &f.row.id,
    ))
    .await
    .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(ATTACHMENTS)
            .count_documents(doc! {"conversation_id":&f.row.id})
            .await
            .unwrap(),
        0
    );
    // Purge uses the same collection regardless of attachment origin.
    f.state
        .db
        .collection::<AssistantAttachment>(ATTACHMENTS)
        .insert_one(AssistantAttachment {
            id: Uuid::new_v4().to_string(),
            origin: "machine_preview".into(),
            user_id: f.owner.clone(),
            conversation_id: f.nyxbot_thread.clone(),
            turn_id: Uuid::new_v4().to_string(),
            content_type: "text/plain".into(),
            size: 4,
            data_encrypted: f.state.encryption_keys.encrypt(b"test").await.unwrap(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    Box::pin(super::super::admin_user_service::delete_current_user_cascade(&f.state.db, &f.owner))
        .await
        .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(ATTACHMENTS)
            .count_documents(doc! {"user_id":&f.owner})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_activity_is_bounded_filtered_private_and_human_only() {
    let f = orchestrator_fixture("machine_activity_page").await;
    let n = node(&f, &f.owner).await;
    let second = Uuid::new_v4().to_string();
    let mut rows = Vec::new();
    for i in 0..105 {
        rows.push(doc!{"_id":Uuid::new_v4().to_string(),"event_type":"machine_operation","user_id":&f.owner,
            "created_at":bson::DateTime::from_millis(10000+i),"event_data":{"node_id":&n.id,"agent_id":if i%2==0 {&f.chat.agent_id}else{&second},
            "operation":"exec","action":"command.exec","outcome":"completed","conversation_id":"private-thread-sentinel","command":"secret-command-sentinel","stdout":"secret-output-sentinel","path":"private-path-sentinel"}});
    }
    f.state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .insert_many(rows)
        .await
        .unwrap();
    let page = list(
        &f.state.db,
        &n.id,
        ActivityQuery {
            limit: Some(999),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.entries.len(), 100);
    let encoded = serde_json::to_string(&page).unwrap();
    assert!(!encoded.contains("sentinel"));
    let tail = list(
        &f.state.db,
        &n.id,
        ActivityQuery {
            before: page.next_cursor,
            limit: Some(100),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(tail.entries.len(), 5);
    assert!(tail.next_cursor.is_none());
    let selected = list(
        &f.state.db,
        &n.id,
        ActivityQuery {
            agent_id: Some(second.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        selected
            .entries
            .iter()
            .all(|e| e.agent_id.as_ref() == Some(&second))
    );
    assert!(
        list(
            &f.state.db,
            &n.id,
            ActivityQuery {
                before: Some("invalid".into()),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert!(
        crate::handlers::machine_activity::list(
            State(f.state.clone()),
            f.auth.clone(),
            Path(n.id.clone()),
            Query(ActivityQuery::default())
        )
        .await
        .is_err()
    );
    assert!(
        crate::handlers::machine_activity::list(
            State(f.state.clone()),
            test_auth_user(&Uuid::new_v4().to_string()),
            Path(n.id.clone()),
            Query(ActivityQuery::default())
        )
        .await
        .is_err()
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"display_name":"Helpful agent"}},
        )
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .insert_one(doc! {"_id":Uuid::new_v4().to_string(),"user_id":Uuid::new_v4().to_string(),"name":"private-agent-sentinel","kind":"specialist"}).await.unwrap();
    let page = crate::handlers::machine_activity::list(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Path(n.id.clone()),
        Query(ActivityQuery::default()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(page.machine_name.as_deref(), Some(n.name.as_str()));
    let agent = page
        .agents
        .iter()
        .find(|a| a.id == f.chat.agent_id)
        .unwrap();
    assert_eq!(agent.display_name.as_deref(), Some("Helpful agent"));
    let dto = serde_json::to_value(&page).unwrap();
    assert!(
        dto["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == f.chat.agent_id)
    );
    assert!(!dto.to_string().contains("private-agent-sentinel"));
    f.state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .insert_one(doc! {"_id":Uuid::new_v4().to_string(),"event_type":"machine_operation","created_at":bson::DateTime::now(),
            "event_data":{"node_id":&n.id,"operation":"exec","outcome":"completed"}}).await.unwrap();
    let unknown = list(
        &f.state.db,
        &n.id,
        ActivityQuery {
            unattributed: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(unknown.entries.len(), 1);
    assert!(unknown.entries[0].agent_id.is_none());
    assert!(
        list(
            &f.state.db,
            &n.id,
            ActivityQuery {
                unattributed: true,
                agent_id: Some(second),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_activity_org_admin_sees_metadata_but_not_private_preview_consent() {
    use crate::{
        models::{org_membership::OrgRole, user::UserType},
        test_utils::{test_membership, test_user},
    };
    let f = orchestrator_fixture("machine_activity_org_acl").await;
    let org = Uuid::new_v4().to_string();
    let admin = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    for (id, kind) in [
        (&org, UserType::Org),
        (&admin, UserType::Person),
        (&member, UserType::Person),
    ] {
        f.state
            .db
            .collection(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(id, kind))
            .await
            .unwrap();
    }
    for (id, role) in [(&admin, OrgRole::Admin), (&member, OrgRole::Member)] {
        f.state
            .db
            .collection(crate::models::org_membership::COLLECTION_NAME)
            .insert_one(test_membership(&org, id, role, None))
            .await
            .unwrap();
    }
    let n = node(&f, &org).await;
    let personal_agent = Uuid::new_v4().to_string();
    let org_agent = Uuid::new_v4().to_string();
    let foreign_agent = Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .insert_many([
            doc! {"_id":&personal_agent,"user_id":&admin,"name":"my-helper","kind":"specialist"},
            doc! {"_id":&org_agent,"user_id":&org,"name":"org-helper","display_name":"Team helper","kind":"specialist"},
            doc! {"_id":&foreign_agent,"user_id":&member,"name":"private-helper","kind":"specialist"},
        ]).await.unwrap();
    let page = crate::handlers::machine_activity::list(
        State(f.state.clone()),
        test_auth_user(&admin),
        Path(n.id.clone()),
        Query(ActivityQuery::default()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(
        page.agents
            .iter()
            .map(|a| a.id.as_str())
            .collect::<std::collections::HashSet<_>>(),
        [personal_agent.as_str(), org_agent.as_str()]
            .into_iter()
            .collect()
    );
    assert_eq!(page.machine_name.as_deref(), Some(n.name.as_str()));
    assert!(
        crate::handlers::machine_activity::list(
            State(f.state.clone()),
            test_auth_user(&member),
            Path(n.id),
            Query(ActivityQuery::default())
        )
        .await
        .is_err()
    );
    assert!(
        crate::handlers::machine_activity::preview_policy(
            State(f.state.clone()),
            test_auth_user(&admin),
            Path(f.row.id.clone()),
            axum::Json(crate::handlers::machine_activity::PreviewPolicy { enabled: true })
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_group_receipts_strip_private_refs_and_refuse_preview_consent() {
    use crate::models::assistant_group::{GroupMessage, MESSAGES_COLLECTION_NAME};
    let f = orchestrator_fixture("machine_receipt_group").await;
    let mut row = f.row.clone();
    let group = Uuid::new_v4().to_string();
    row.group_id = Some(group.clone());
    let mut receipt = receipt(
        &f.chat,
        &Uuid::new_v4().to_string(),
        Operation::Exec,
        &json!({}),
    );
    receipt.preview_id = Some("private-preview-sentinel".into());
    receipt.screenshot_id = Some("private-image-sentinel".into());
    receipt.preview_enabled = true;
    row.active_turn.as_mut().unwrap().activities.push(
        crate::models::assistant_conversation::TurnActivity {
            machine: Some(Box::new(receipt)),
            id: Uuid::new_v4().to_string(),
            label: "nyx__machine_exec".into(),
            status: "completed".into(),
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
        },
    );
    let message = GroupMessage {
        activities: vec![],
        org_group: false,
        author_user_id: None,
        author_display_name: None,
        request_id: None,
        attachments: vec![],
        id: Uuid::new_v4().to_string(),
        group_id: group.clone(),
        user_id: f.owner.clone(),
        seq: 1,
        role: "agent".into(),
        agent_id: row.agent_id.clone(),
        agent_name: None,
        text: "done".into(),
        created_at: Utc::now(),
    };
    f.state
        .db
        .collection::<GroupMessage>(MESSAGES_COLLECTION_NAME)
        .insert_one(&message)
        .await
        .unwrap();
    publish_group(&f.state.db, &row, &message).await.unwrap();
    let stored = f
        .state
        .db
        .collection::<GroupMessage>(MESSAGES_COLLECTION_NAME)
        .find_one(doc! {"_id":&message.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.activities.len(), 1);
    assert!(
        !serde_json::to_string(&stored.activities)
            .unwrap()
            .contains("private-")
    );
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"group_id":group,"machine_previews":true}},
        )
        .await
        .unwrap();
    assert!(!preview_enabled(&f.state.db, &f.chat).await.unwrap());
    assert!(
        crate::handlers::machine_activity::preview_policy(
            State(f.state.clone()),
            test_auth_user(&f.owner),
            Path(f.row.id.clone()),
            axum::Json(crate::handlers::machine_activity::PreviewPolicy { enabled: true })
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_names_batch_respects_node_read_acl_and_enriches_transcript_dtos() {
    use crate::{
        models::{org_membership::OrgRole, user::UserType},
        test_utils::{test_membership, test_user},
    };
    let f = orchestrator_fixture("machine_display_names").await;
    let own = node(&f, &f.owner).await;
    let other = node(&f, &Uuid::new_v4().to_string()).await;
    let org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let membership = test_membership(&org, &f.owner, OrgRole::Member, None);
    f.state
        .db
        .collection::<crate::models::org_membership::OrgMembership>(
            crate::models::org_membership::COLLECTION_NAME,
        )
        .insert_one(&membership)
        .await
        .unwrap();
    let org_node = node(&f, &org).await;
    let missing = Uuid::new_v4().to_string();
    let mut dtos: Vec<crate::handlers::assistant_nyxagent::MachineReceiptResponse> =
        [&own.id, &own.id, &other.id, &org_node.id, &missing]
            .into_iter()
            .map(|id| {
                receipt(
                    &f.chat,
                    id,
                    Operation::Browser,
                    &json!({"action":"snapshot"}),
                )
                .into()
            })
            .collect();
    crate::handlers::assistant_nyxagent::resolve_machine_names(
        &f.state.db,
        &f.owner,
        dtos.iter_mut().collect(),
    )
    .await
    .unwrap();
    assert_eq!(dtos[0].machine_name.as_deref(), Some(own.name.as_str()));
    assert_eq!(dtos[1].machine_name, dtos[0].machine_name);
    assert!(dtos[2].machine_name.is_none());
    assert_eq!(
        dtos[3].machine_name.as_deref(),
        Some(org_node.name.as_str())
    );
    assert!(dtos[4].machine_name.is_none());
    assert_eq!(
        serde_json::to_value(&dtos[0]).unwrap()["machine_name"],
        own.name
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(doc! {"_id":&membership.id}, doc! {"$set":{"role":"viewer"}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&own.id},
            doc! {"$set":{"name":"Renamed computer"}},
        )
        .await
        .unwrap();
    crate::handlers::assistant_nyxagent::resolve_machine_names(
        &f.state.db,
        &f.owner,
        dtos.iter_mut().collect(),
    )
    .await
    .unwrap();
    assert_eq!(dtos[0].machine_name.as_deref(), Some("Renamed computer"));
    assert!(dtos[3].machine_name.is_none());
    // The real history response resolves old receipts too, without persisting names.
    let activity = super::super::assistant_nyxagent::activity_started(
        &f.state.db,
        &f.owner,
        &f.row.id,
        "nyx__machine_browser",
    )
    .await
    .unwrap()
    .unwrap();
    let r = receipt(
        &f.chat,
        &own.id,
        Operation::Browser,
        &json!({"action":"snapshot"}),
    );
    ACTIVITY_ID
        .scope(Some(activity), record(&f.state.db, &f.chat, &r))
        .await
        .unwrap();
    let history = Box::pin(crate::handlers::assistant_nyxagent::history(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Path(f.row.id.clone()),
        Query(Default::default()),
    ))
    .await
    .unwrap()
    .0;
    let value = serde_json::to_value(history).unwrap();
    assert_eq!(
        value["conversation"]["active_turn"]["activities"][0]["machine"]["machine_name"],
        "Renamed computer"
    );
    let stored = f
        .state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .find_one(doc! {"_id":&f.row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!format!("{stored:?}").contains("Renamed computer"));
    f.state.db.drop().await.unwrap();
}
