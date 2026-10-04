use super::{
    assistant_acknowledgement_service as acks, assistant_group_service as personal,
    assistant_nyxagent as engine, assistant_upload_service as uploads,
    feature_flag_service as flags, org_agent_tests::Fixture, org_group_service as groups,
};
use crate::{
    errors::AppError,
    models::{
        api_key::ApiKey,
        assistant_agent::AssistantAgent,
        assistant_conversation::{AssistantConversation, TurnOrigin},
        assistant_group::{GroupRequest, REQUESTS_COLLECTION_NAME as REQUESTS},
    },
    test_utils::*,
};
use axum::extract::{Path, State};
use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;

async fn create(f: &Fixture, agent: &AssistantAgent) -> groups::Access {
    groups::create(
        &f.state.db,
        &f.admin,
        &f.org,
        "Shared research",
        std::slice::from_ref(&agent.id),
        std::slice::from_ref(&f.member),
        "user",
    )
    .await
    .unwrap()
}
async fn access(f: &Fixture, actor: &str, group: &str) -> groups::Access {
    groups::get(&f.state.db, actor, group, None).await.unwrap()
}
async fn enqueue(
    f: &Fixture,
    access: &groups::Access,
    agent: &AssistantAgent,
    uploads: &[String],
) -> String {
    let id = Uuid::new_v4().to_string();
    let request = GroupRequest {
        id: id.clone(),
        group_id: access.group.id.clone(),
        user_id: f.org.clone(),
        actor_user_id: access.actor.clone(),
        message_seq: 0,
        attachment_ids: uploads.to_vec(),
        pending_agent_ids: vec![agent.id.clone()],
        hops_remaining: 3,
        created_at: Utc::now(),
    };
    groups::append(
        &f.state.db,
        access,
        "user",
        None,
        "Research this request",
        Some(&id),
        uploads,
        Some(request),
    )
    .await
    .unwrap();
    id
}
async fn start(
    f: &Fixture,
    access: &groups::Access,
    agent: &AssistantAgent,
    request: &str,
) -> AssistantConversation {
    let row = groups::member_thread(
        &f.state.db,
        &f.state.encryption_keys,
        access,
        agent,
        request,
    )
    .await
    .unwrap();
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        conversation_id: Some(row.id),
        attachment_ids: vec![],
        text: "Shared request".into(),
        agent_id: None,
        model: None,
        access_mode: None,
    });
    start.origin = TurnOrigin::Group;
    start.group_request_id = Some(request.into());
    start.org_access = access.org.clone();
    start.group_attachments = groups::request_attachments(&f.state.db, access, request)
        .await
        .unwrap();
    engine::begin_turn(&f.state.db, &access.actor, start, &f.state.encryption_keys)
        .await
        .unwrap()
}
async fn key(f: &Fixture, row: &AssistantConversation) -> ApiKey {
    f.state
        .db
        .collection(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":&row.credential_api_key_id})
        .await
        .unwrap()
        .unwrap()
}
async fn finish(f: &Fixture, row: &AssistantConversation) {
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn":bson::Bson::Null}},
        )
        .await
        .unwrap();
}
async fn engine_flag(f: &Fixture) {
    flags::set_platform_override(
        &f.state.db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.admin,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn org_group_creation_acl_management_and_legacy_defaults() {
    let f = Fixture::new("org_group_acl").await;
    let (agent, _) = f.create().await;
    flags::set_platform_override(
        &f.state.db,
        "assistant:org-agents",
        &flags::FlagTarget::Global,
        false,
        &f.admin,
    )
    .await
    .unwrap();
    assert!(
        groups::create(
            &f.state.db,
            &f.admin,
            &f.org,
            "Group",
            std::slice::from_ref(&agent.id),
            &[],
            "user"
        )
        .await
        .is_err()
    );
    flags::set_platform_override(
        &f.state.db,
        "assistant:org-agents",
        &flags::FlagTarget::Global,
        true,
        &f.admin,
    )
    .await
    .unwrap();
    for people in [
        vec![f.viewer.clone()],
        vec![f.outsider.clone()],
        vec![f.org.clone()],
        vec![f.member.clone(); 17],
    ] {
        assert!(
            groups::create(
                &f.state.db,
                &f.admin,
                &f.org,
                "Group",
                std::slice::from_ref(&agent.id),
                &people,
                "user"
            )
            .await
            .is_err()
        );
    }
    let nyxbot = super::assistant_team_service::ensure_nyxbot(&f.state.db, &f.admin)
        .await
        .unwrap();
    assert!(
        groups::create(
            &f.state.db,
            &f.admin,
            &f.org,
            "Group",
            &[nyxbot.id],
            &[],
            "user"
        )
        .await
        .is_err()
    );
    let group = create(&f, &agent).await.group;
    assert_eq!(access(&f, &f.admin, &group.id).await.role(), "creator");
    assert_eq!(access(&f, &f.member, &group.id).await.role(), "participant");
    assert!(
        access(&f, &f.member, &group.id)
            .await
            .require_manage()
            .is_err()
    );
    for actor in [&f.viewer, &f.outsider] {
        assert!(matches!(
            groups::get(&f.state.db, actor, &group.id, None).await,
            Err(AppError::NotFound(_))
        ));
        assert!(groups::list(&f.state.db, actor).await.unwrap().is_empty());
        assert!(
            uploads::owner_scope(&f.state.db, actor, &group.id)
                .await
                .is_err()
        );
    }
    // Active org membership alone does not reveal a group.
    let private = groups::create(
        &f.state.db,
        &f.admin,
        &f.org,
        "Private",
        std::slice::from_ref(&agent.id),
        &[],
        "user",
    )
    .await
    .unwrap();
    assert!(matches!(
        groups::get(&f.state.db, &f.member, &private.group.id, None).await,
        Err(AppError::NotFound(_))
    ));
    // A Member creator may manage, and a participating Admin may manage.
    let member_group = groups::create(
        &f.state.db,
        &f.member,
        &f.org,
        "Member group",
        std::slice::from_ref(&agent.id),
        std::slice::from_ref(&f.admin),
        "user",
    )
    .await
    .unwrap();
    assert_eq!(member_group.role(), "creator");
    assert_eq!(
        access(&f, &f.admin, &member_group.group.id).await.role(),
        "admin"
    );
    let personal = personal::create(&f.state.db, &f.admin, "Personal", &[agent.id], "user")
        .await
        .unwrap();
    let mut legacy = bson::to_document(&personal).unwrap();
    legacy.remove("participant_user_ids");
    legacy.remove("created_by_user_id");
    let legacy =
        bson::from_document::<crate::models::assistant_group::AssistantGroup>(legacy).unwrap();
    assert!(!groups::is_org(&legacy));
    groups::authorize(&f.state.db, &f.admin, legacy, None)
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_group_threads_keys_billing_uploads_and_single_auth_resolution() {
    use std::sync::atomic::Ordering;
    let (f, memberships, _) = Fixture::observed("org_group_threads").await;
    let (agent, _) = f.create().await;
    let group = create(&f, &agent).await.group;
    let mut rows = Vec::new();
    let mut uploaded: Vec<String> = Vec::new();
    for actor in [&f.admin, &f.member] {
        let access = access(&f, actor, &group.id).await;
        let upload = uploads::upload(
            &f.state.db,
            &f.state.encryption_keys,
            actor,
            &group.id,
            "note.txt",
            b"Request-local file".to_vec(),
        )
        .await
        .unwrap();
        assert!(
            uploads::owner_read(
                &f.state.db,
                &f.state.encryption_keys,
                if actor == &f.admin {
                    &f.member
                } else {
                    &f.admin
                },
                &group.id,
                &upload.id
            )
            .await
            .is_err()
        );
        let request = enqueue(&f, &access, &agent, std::slice::from_ref(&upload.id)).await;
        memberships.store(0, Ordering::SeqCst);
        let row = start(&f, &access, &agent, &request).await;
        assert_eq!(
            memberships.load(Ordering::SeqCst),
            0,
            "turn admission reuses the request snapshot"
        );
        let key = key(&f, &row).await;
        assert_eq!(&row.user_id, actor);
        assert_eq!(&key.user_id, actor);
        assert_eq!(key.assistant_group_id.as_deref(), Some(group.id.as_str()));
        assert_eq!(
            key.assistant_agent_owner_id.as_deref(),
            Some(f.org.as_str())
        );
        let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .unwrap();
        let chat = acks::for_key_with_access(
            &f.state.db,
            actor,
            Some(&key.id),
            auth.org_agent_access.as_ref(),
        )
        .await
        .unwrap()
        .unwrap();
        uploads::for_chat(&f.state.db, &chat, &upload.id)
            .await
            .unwrap();
        assert_eq!(
            memberships.load(Ordering::SeqCst),
            1,
            "auth, group binding and tool share one live org resolution"
        );
        for other in &uploaded {
            assert!(uploads::for_chat(&f.state.db, &chat, other).await.is_err());
        }
        let payer = super::billing::owner_resolver::BillingOwnerResolver::new(f.state.db.clone())
            .resolve_for_execution(
                actor,
                &f.org,
                crate::models::usage_meter::CredentialClass::NyxidManagedMaster,
            )
            .await
            .unwrap();
        assert_eq!(&payer.owner_id, actor);
        let other = if actor == &f.admin {
            &f.member
        } else {
            &f.admin
        };
        assert!(
            engine::history_page(&f.state.db, other, &row.id, 20, None)
                .await
                .is_err()
        );
        let (_, data) = uploads::owner_read(
            &f.state.db,
            &f.state.encryption_keys,
            other,
            &group.id,
            &upload.id,
        )
        .await
        .unwrap();
        assert_eq!(data, b"Request-local file");
        uploaded.push(upload.id);
        rows.push(row);
    }
    assert_ne!(rows[0].id, rows[1].id);
    let chat = acks::for_key(&f.state.db, &f.admin, Some(&rows[0].credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(
        uploads::for_chat(&f.state.db, &chat, &uploaded[1])
            .await
            .is_err()
    );
    for row in &rows {
        finish(&f, row).await;
    }
    groups::delete(&f.state.db, &access(&f, &f.admin, &group.id).await)
        .await
        .unwrap();
    for row in rows {
        assert!(
            f.state
                .db
                .collection::<ApiKey>(crate::models::api_key::COLLECTION_NAME)
                .find_one(doc! {"_id":row.credential_api_key_id,"is_active":true})
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
            .count_documents(doc! {"group_id":group.id})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_group_cards_leave_revocation_and_queue_drop() {
    let f = Fixture::new("org_group_cards").await;
    let (agent, _) = f.create().await;
    let group = create(&f, &agent).await.group;
    let b = access(&f, &f.member, &group.id).await;
    let request = enqueue(&f, &b, &agent, &[]).await;
    let row = start(&f, &b, &agent, &request).await;
    let chat = acks::for_key(&f.state.db, &f.member, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let card = acks::request(
        &f.state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("test_action"),
            arguments: Some(&serde_json::json!({"id":"resource"})),
            summary: "Change a resource",
            platform: false,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        acks::decide(&f.state.db, &f.admin, &row.id, &card.id, true).await,
        Err(AppError::NotFound(_))
    ));
    acks::decide(&f.state.db, &f.member, &row.id, &card.id, true)
        .await
        .unwrap();
    let card = acks::request(
        &f.state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("another_action"),
            arguments: None,
            summary: "Another change",
            platform: false,
        },
    )
    .await
    .unwrap();
    let queued = enqueue(&f, &b, &agent, &[]).await;
    assert!(matches!(
        groups::update(
            &f.state.db,
            access(&f, &f.member, &group.id).await,
            None,
            None,
            None,
            None,
            true
        )
        .await,
        Err(AppError::AssistantTurnActive)
    ));
    // The failed transaction leaves both participation and credentials intact.
    groups::get(&f.state.db, &f.member, &group.id, None)
        .await
        .unwrap();
    let thread_key = key(&f, &row).await;
    assert!(thread_key.is_active);
    finish(&f, &row).await;
    // Retry immediately: the active-turn refusal must have awaited its abort.
    groups::update(&f.state.db, b, None, None, None, None, true)
        .await
        .unwrap();
    assert_removed(&f, &row).await;
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &thread_key, None, None, None)
            .await
            .is_err()
    );
    assert!(engine::get(&f.state.db, &f.member, &row.id).await.is_err());
    assert!(
        engine::history_page(&f.state.db, &f.member, &row.id, 20, None)
            .await
            .is_err()
    );
    assert!(
        acks::decide(&f.state.db, &f.member, &row.id, &card.id, true)
            .await
            .is_err()
    );
    assert!(
        acks::decide_as(
            &f.state.db,
            &f.member,
            None,
            &card.id,
            true,
            acks::Decider::User,
            None
        )
        .await
        .is_err()
    );
    assert!(
        uploads::owner_scope(&f.state.db, &f.member, &group.id)
            .await
            .is_err()
    );
    crate::handlers::org_group::advance(&f.state, &group).await;
    let pending = f
        .state
        .db
        .collection::<GroupRequest>(REQUESTS)
        .find_one(doc! {"_id":queued})
        .await
        .unwrap();
    assert!(pending.is_none());
    // Org revocation also invalidates an existing key even without editing participants.
    let a = access(&f, &f.admin, &group.id).await;
    let request = enqueue(&f, &a, &agent, &[]).await;
    let admin_row = start(&f, &a, &agent, &request).await;
    let queued = enqueue(&f, &a, &agent, &[]).await;
    f.revoke(&f.admin).await;
    crate::handlers::org_group::advance(&f.state, &group).await;
    let pending = f
        .state
        .db
        .collection::<GroupRequest>(REQUESTS)
        .find_one(doc! {"_id":queued})
        .await
        .unwrap()
        .unwrap();
    assert!(pending.pending_agent_ids.is_empty());
    let notices = personal::messages(&f.state.db, &f.org, &group.id, 20, None)
        .await
        .unwrap();
    assert!(notices.iter().any(|m| m.org_group
        && m.author_user_id.is_none()
        && m.role == "notice"
        && m.text == "Queued request dropped: participant access ended."));
    assert!(
        crate::mw::auth::api_key_auth_user(
            &f.state.db,
            &key(&f, &admin_row).await,
            None,
            None,
            None
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_group_live_fanout_rechecks_delivery_and_excludes_nonparticipants() {
    use super::assistant_live::LiveEvent;
    use http_body_util::BodyExt;
    let f = Fixture::new("org_group_live").await;
    let (agent, _) = f.create().await;
    let group = create(&f, &agent).await.group;
    engine_flag(&f).await;
    f.state.assistant_live.set_open_for_tests(true);
    let mut bodies = Vec::new();
    for actor in [&f.admin, &f.member, &f.viewer, &f.outsider] {
        let response = crate::handlers::assistant_nyxagent::live(
            State(f.state.clone()),
            test_auth_user(actor),
        )
        .await
        .unwrap();
        let mut body = response.into_body();
        let ready = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert!(std::str::from_utf8(&ready).unwrap().contains("ready"));
        bodies.push(body);
    }
    f.state
        .assistant_live
        .publish_resolved(
            &f.state.db,
            LiveEvent::OrgGroup {
                id: group.id.clone(),
                user_id: f.org.clone(),
            },
        )
        .await;
    // Already queued for B, but B leaves before the stream delivers it.
    groups::update(
        &f.state.db,
        access(&f, &f.member, &group.id).await,
        None,
        None,
        None,
        None,
        true,
    )
    .await
    .unwrap();
    let data = bodies[0]
        .frame()
        .await
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    let frame = std::str::from_utf8(&data).unwrap();
    assert!(frame.contains(&group.id));
    assert!(!frame.contains("Shared research"));
    for body in &mut bodies[1..] {
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), body.frame())
                .await
                .is_err()
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_group_http_tools_and_demoted_or_inactive_members_are_refused() {
    use crate::handlers::{assistant_group as handler, assistant_team};
    let f = Fixture::new("org_group_routes").await;
    let (agent, _) = f.create().await;
    engine_flag(&f).await;
    let group = create(&f, &agent).await.group;
    let nyxbot = super::assistant_team_service::ensure_nyxbot(&f.state.db, &f.member)
        .await
        .unwrap();
    let home = super::assistant_team_service::home_thread_for(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &nyxbot,
    )
    .await
    .unwrap();
    let chat = acks::for_key(&f.state.db, &f.member, Some(&home.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let (listing, error) = assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__list_groups",
        &serde_json::json!({"org":f.org}),
    )
    .await;
    assert!(!error);
    assert_eq!(listing["groups"][0]["id"], group.id);
    let mut automation = chat.clone();
    automation.confirmation_policy =
        Some(crate::models::trigger_schedule::ConfirmationPolicy::Destructive);
    let (card, error) = assistant_team::execute_tool(
        &f.state,
        &automation,
        "nyxid__update_group",
        &serde_json::json!({"group":group.id,"leave":true}),
    )
    .await;
    assert!(error);
    assert_eq!(card["error"], "acknowledgement_required");
    groups::get(&f.state.db, &f.member, &group.id, None)
        .await
        .unwrap();
    let (_, error) = assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__post_to_group",
        &serde_json::json!({"group":group.id,"text":"post"}),
    )
    .await;
    assert!(error);
    let _ = handler::get_group(
        State(f.state.clone()),
        test_auth_user(&f.member),
        Path(group.id.clone()),
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id":&f.org,"member_user_id":&f.member},
            doc! {"$set":{"role":"viewer"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        handler::get_group(
            State(f.state.clone()),
            test_auth_user(&f.member),
            Path(group.id.clone())
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert!(
        crate::handlers::assistant_group::post(&f.state, &f.member, &group.id, "no", None)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.org}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        groups::get(&f.state.db, &f.admin, &group.id, None)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_group_handoff_keeps_request_actor_budget_and_thread() {
    let f = Fixture::new("org_group_handoff").await;
    let (agent, _) = f.create().await;
    let mut other = agent.clone();
    other.id = Uuid::new_v4().to_string();
    other.name = "reviewer".into();
    other.home_conversation_id = None;
    f.state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .insert_one(&other)
        .await
        .unwrap();
    let group = groups::create(
        &f.state.db,
        &f.admin,
        &f.org,
        "Handoff",
        &[agent.id.clone(), other.id.clone()],
        std::slice::from_ref(&f.member),
        "user",
    )
    .await
    .unwrap()
    .group;
    let a = access(&f, &f.admin, &group.id).await;
    let b = access(&f, &f.member, &group.id).await;
    let ra = enqueue(&f, &a, &agent, &[]).await;
    let rb = enqueue(&f, &b, &agent, &[]).await;
    let ta = start(&f, &a, &agent, &ra).await;
    let tb = start(&f, &b, &agent, &rb).await;
    finish(&f, &ta).await;
    finish(&f, &tb).await;
    // Keep server turns queued so the test exercises hand-off persistence
    // without talking to an external model.
    let limit = crate::handlers::assistant_team::team_pool_limit(&f.state, &f.member).await + 1;
    let mut permits = Vec::new();
    for _ in 0..limit {
        permits.push(
            f.state
                .direct_chat_limiter
                .try_acquire_pool("assistant_team", &f.member, limit)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    crate::handlers::org_group::member_settled(&f.state, b, &tb, "@reviewer please review", None)
        .await;
    let request = f
        .state
        .db
        .collection::<GroupRequest>(REQUESTS)
        .find_one(doc! {"_id":&rb})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.actor_user_id, f.member);
    assert_eq!(request.pending_agent_ids, vec![other.id.clone()]);
    assert_eq!(request.hops_remaining, 2);
    let untouched = f
        .state
        .db
        .collection::<GroupRequest>(REQUESTS)
        .find_one(doc! {"_id":&ra})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.actor_user_id, f.admin);
    assert_eq!(untouched.hops_remaining, 3);
    assert!(untouched.pending_agent_ids.is_empty());
    let reviewer = personal::member_thread(&f.state.db, &f.member, &group.id, &other.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(key(&f, &reviewer).await.user_id, f.member);
    assert!(
        personal::member_thread(&f.state.db, &f.admin, &group.id, &other.id)
            .await
            .unwrap()
            .is_none()
    );
    drop(permits);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_group_proxy_cards_bind_person_chain_and_live_participation() {
    use super::{approval_service as approvals, operation_descriptor};
    use crate::models::service_approval_config::ApprovalMode;
    let f = Fixture::new("org_group_proxy_cards").await;
    let (agent, _) = f.create().await;
    let group = create(&f, &agent).await.group;
    let b = access(&f, &f.member, &group.id).await;
    let request = enqueue(&f, &b, &agent, &[]).await;
    let row = start(&f, &b, &agent, &request).await;
    let binding = groups::approval_binding(
        &f.state.db,
        Some(&group.id),
        &f.member,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(binding.request_id, request);
    let mut operation = approvals::ApprovalRequestOperation::from_descriptor(
        &operation_descriptor::build_http_descriptor("POST", "/task", None),
        None,
    );
    operation.assistant_group = Some(binding.clone());
    let card = approvals::create_approval_request(
        &f.state.db,
        &f.state.config,
        &f.state.http_client,
        None,
        None,
        &f.member,
        "service",
        "Task service",
        "task",
        "api_key",
        &f.member,
        None,
        operation.clone(),
        ApprovalMode::PerRequest,
        300,
        vec![f.admin.clone()],
        false,
    )
    .await
    .unwrap();
    assert_eq!(card.assistant_group.as_ref(), Some(&binding));
    assert_eq!(card.notify_user_ids, vec![f.member.clone()]);
    assert!(matches!(
        groups::authorize_approval(&f.state.db, &card, &f.admin, None).await,
        Err(AppError::NotFound(_))
    ));
    assert!(
        crate::handlers::approvals::decide_request(
            State(f.state.clone()),
            test_auth_user(&f.admin),
            Path(card.id.clone()),
            Default::default(),
            axum::http::HeaderMap::new(),
            axum::Json(crate::handlers::approvals::DecideRequest {
                approved: true,
                duration_sec: None
            })
        )
        .await
        .is_err()
    );
    let _ = crate::handlers::approvals::decide_request(
        State(f.state.clone()),
        test_auth_user(&f.member),
        Path(card.id.clone()),
        Default::default(),
        axum::http::HeaderMap::new(),
        axum::Json(crate::handlers::approvals::DecideRequest {
            approved: false,
            duration_sec: None,
        }),
    )
    .await
    .unwrap();
    let card = approvals::create_approval_request(
        &f.state.db,
        &f.state.config,
        &f.state.http_client,
        None,
        None,
        &f.member,
        "service",
        "Task service",
        "task",
        "api_key",
        &f.member,
        None,
        operation,
        ApprovalMode::PerRequest,
        300,
        vec![],
        false,
    )
    .await
    .unwrap();
    let requester =
        crate::mw::auth::api_key_auth_user(&f.state.db, &key(&f, &row).await, None, None, None)
            .await
            .unwrap();
    let _ = crate::handlers::approvals::get_request_status(
        State(f.state.clone()),
        requester.clone(),
        Path(card.id.clone()),
    )
    .await
    .unwrap();
    finish(&f, &row).await;
    groups::update(&f.state.db, b, None, None, None, None, true)
        .await
        .unwrap();
    assert!(
        approvals::process_decision(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            None,
            None,
            &card.id,
            true,
            None,
            None,
            "telegram"
        )
        .await
        .is_err()
    );
    assert!(approvals::get_request(&f.state.db, &card.id).await.is_err());
    assert!(
        crate::handlers::approvals::get_request_status(
            State(f.state.clone()),
            requester,
            Path(card.id.clone()),
        )
        .await
        .is_err()
    );
    finish(&f, &row).await;
    groups::delete(&f.state.db, &access(&f, &f.admin, &group.id).await)
        .await
        .unwrap();
    assert!(approvals::get_request(&f.state.db, &card.id).await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_group_request_upload_isolation_retention_and_rotated_key_revocation() {
    let f = Fixture::new("org_group_lifecycle").await;
    let (agent, _) = f.create().await;
    let group = create(&f, &agent).await.group;
    let b = access(&f, &f.member, &group.id).await;
    let upload = uploads::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        "note.txt",
        b"Only this request receives this file".to_vec(),
    )
    .await
    .unwrap();
    let first = enqueue(&f, &b, &agent, std::slice::from_ref(&upload.id)).await;
    let row = start(&f, &b, &agent, &first).await;
    finish(&f, &row).await;
    let mut resumed = engine::TurnStart::from(&engine::TurnRequest {
        conversation_id: Some(row.id.clone()),
        attachment_ids: vec![],
        text: "Direct hidden-thread post".into(),
        agent_id: None,
        model: None,
        access_mode: None,
    });
    resumed.org_access = b.org.clone();
    assert!(
        engine::begin_turn(
            &f.state.db,
            &f.member,
            resumed.clone(),
            &f.state.encryption_keys
        )
        .await
        .is_err()
    );
    let event =
        super::assistant_team_service::event("action_decided", "Action decided".into(), None);
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$push":{"pending_events":bson::to_bson(&event).unwrap()}},
        )
        .await
        .unwrap();
    resumed.origin = TurnOrigin::Event;
    resumed.text = "New shared transcript context".into();
    let resumed = engine::begin_turn(&f.state.db, &f.member, resumed, &f.state.encryption_keys)
        .await
        .unwrap();
    let message: crate::models::assistant_message::AssistantMessage = f
        .state
        .db
        .collection(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc! {"conversation_id":&row.id,"role":"event"})
        .await
        .unwrap()
        .unwrap();
    assert!(message.text.contains("New shared transcript context"));
    finish(&f, &resumed).await;
    let next = enqueue(&f, &b, &agent, &[]).await;
    let row = start(&f, &b, &agent, &next).await;
    let chat = acks::for_key(&f.state.db, &f.member, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(
        uploads::for_chat(&f.state.db, &chat, &upload.id)
            .await
            .is_err(),
        "a later request from the same person cannot read an earlier upload"
    );
    uploads::owner_read(
        &f.state.db,
        &f.state.encryption_keys,
        &f.admin,
        &group.id,
        &upload.id,
    )
    .await
    .unwrap();
    finish(&f, &row).await;
    let rotated = super::key_service::rotate_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &row.credential_api_key_id,
    )
    .await
    .unwrap();
    let key: ApiKey = f
        .state
        .db
        .collection(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":&rotated.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(key.assistant_group_id.as_deref(), Some(group.id.as_str()));
    assert_eq!(
        key.assistant_agent_owner_id.as_deref(),
        Some(f.org.as_str())
    );
    crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();

    let old = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::days(31));
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .update_one(doc! {"_id":&upload.id}, doc! {"$set":{"bound_at":old}})
        .await
        .unwrap();
    assert!(
        uploads::owner_read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.admin,
            &group.id,
            &upload.id
        )
        .await
        .is_err()
    );
    assert_eq!(
        super::assistant_upload_retention::sweep(&f.state.db)
            .await
            .unwrap(),
        1
    );
    let expired = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload_retention::TOMBSTONES)
        .find_one(doc! {"_id":&upload.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(expired.get_str("user_id").unwrap(), f.member);
    assert_eq!(expired.get_str("group_id").unwrap(), group.id);
    let ids = std::slice::from_ref(&upload.id);
    assert!(
        super::assistant_upload_retention::expired_ids(&f.state.db, &f.member, ids)
            .await
            .unwrap()
            .contains(&upload.id)
    );
    assert!(
        super::assistant_upload_retention::expired_ids(&f.state.db, &f.org, ids)
            .await
            .unwrap()
            .is_empty()
    );

    // Removing just the agent invalidates even the rotated key while the
    // person remains an eligible participant and organization member.
    let mut replacement = agent.clone();
    replacement.id = Uuid::new_v4().to_string();
    replacement.name = "replacement".into();
    f.state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .insert_one(&replacement)
        .await
        .unwrap();
    let a = access(&f, &f.admin, &group.id).await;
    groups::update(
        &f.state.db,
        a,
        None,
        Some(std::slice::from_ref(&replacement.id)),
        None,
        None,
        false,
    )
    .await
    .unwrap();
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_err()
    );
    groups::delete(&f.state.db, &access(&f, &f.admin, &group.id).await)
        .await
        .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload_retention::TOMBSTONES)
            .count_documents(doc! {"group_id":&group.id})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

async fn assert_removed(f: &Fixture, row: &AssistantConversation) {
    use crate::models::{
        api_key, assistant_acknowledgement, assistant_agent_credential, assistant_conversation,
    };
    for (collection, filter) in [
        (
            assistant_conversation::COLLECTION_NAME,
            doc! {"_id":&row.id},
        ),
        (
            api_key::COLLECTION_NAME,
            doc! {"assistant_group_id":&row.group_id,"user_id":&row.user_id},
        ),
        (
            assistant_agent_credential::COLLECTION_NAME,
            doc! {"conversation_id":&row.id},
        ),
        (
            assistant_acknowledgement::COLLECTION_NAME,
            doc! {"conversation_id":&row.id},
        ),
        (
            REQUESTS,
            doc! {"group_id":&row.group_id,"actor_user_id":&row.user_id},
        ),
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(filter)
                .await
                .unwrap(),
            0,
            "removed participant retains rows in {collection}"
        );
    }
}

#[tokio::test]
async fn org_group_participant_removal_is_atomic_and_purges_rotated_keys() {
    let f = Fixture::new("org_group_removal").await;
    let (agent, _) = f.create().await;
    let group = create(&f, &agent).await.group;
    let b = access(&f, &f.member, &group.id).await;
    let request = enqueue(&f, &b, &agent, &[]).await;
    let member = start(&f, &b, &agent, &request).await;
    let a = access(&f, &f.admin, &group.id).await;
    assert!(matches!(
        groups::update(
            &f.state.db,
            a,
            None,
            None,
            Some(std::slice::from_ref(&f.admin)),
            None,
            false
        )
        .await,
        Err(AppError::AssistantTurnActive)
    ));
    groups::get(&f.state.db, &f.member, &group.id, None)
        .await
        .unwrap();
    assert!(key(&f, &member).await.is_active);
    finish(&f, &member).await;
    super::key_service::rotate_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &member.credential_api_key_id,
    )
    .await
    .unwrap();
    let a = access(&f, &f.admin, &group.id).await;
    let request = enqueue(&f, &a, &agent, &[]).await;
    let admin = start(&f, &a, &agent, &request).await;
    groups::update(
        &f.state.db,
        a,
        None,
        None,
        Some(std::slice::from_ref(&f.admin)),
        None,
        false,
    )
    .await
    .unwrap();
    assert_removed(&f, &member).await;
    assert!(
        key(&f, &admin).await.is_active,
        "another participant's live turn is preserved"
    );
    assert_eq!(groups::threads(&f.state.db, &group).await.unwrap().len(), 1);
    // Shared messages survive a participant's departure.
    assert_eq!(
        personal::messages(&f.state.db, &f.org, &group.id, 20, None)
            .await
            .unwrap()
            .len(),
        2
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_group_manager_must_remain_and_last_leave_cascades() {
    let f = Fixture::new("org_group_last_leave").await;
    let (agent, _) = f.create().await;
    let group = create(&f, &agent).await.group;
    for leave in [true, false] {
        let a = access(&f, &f.admin, &group.id).await;
        assert!(matches!(groups::update(&f.state.db, a, None, None,
            if leave {None} else {Some(std::slice::from_ref(&f.member))}, None, leave).await,
            Err(AppError::ValidationError(message)) if message.contains("participating Admin")));
        assert_eq!(
            access(&f, &f.admin, &group.id)
                .await
                .group
                .participant_user_ids
                .len(),
            2
        );
    }
    // A Member creator can leave if a participating Admin remains to manage/delete.
    let b = groups::create(
        &f.state.db,
        &f.member,
        &f.org,
        "Managed",
        std::slice::from_ref(&agent.id),
        std::slice::from_ref(&f.admin),
        "user",
    )
    .await
    .unwrap();
    let managed = b.group.id.clone();
    groups::update(&f.state.db, b, None, None, None, None, true)
        .await
        .unwrap();
    let a = access(&f, &f.admin, &managed).await;
    a.require_manage().unwrap();
    let upload = uploads::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.admin,
        &managed,
        "shared.txt",
        b"Shared".to_vec(),
    )
    .await
    .unwrap();
    let request = enqueue(&f, &a, &agent, std::slice::from_ref(&upload.id)).await;
    let row = start(&f, &a, &agent, &request).await;
    assert!(matches!(
        groups::update(
            &f.state.db,
            access(&f, &f.admin, &managed).await,
            None,
            None,
            None,
            None,
            true
        )
        .await,
        Err(AppError::AssistantTurnActive)
    ));
    assert!(key(&f, &row).await.is_active);
    finish(&f, &row).await;
    // Last-participant deletion must release its rejected transaction too.
    groups::update(&f.state.db, a, None, None, None, None, true)
        .await
        .unwrap();
    assert_removed(&f, &row).await;
    assert!(
        groups::get(&f.state.db, &f.admin, &managed, None)
            .await
            .is_err()
    );
    for collection in [
        crate::models::assistant_group::MESSAGES_COLLECTION_NAME,
        crate::models::assistant_attachment::COLLECTION_NAME,
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {"group_id":&managed})
                .await
                .unwrap(),
            0
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_group_pending_cards_batch_acknowledgements_and_bound_approvals() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let ack_queries = Arc::new(AtomicUsize::new(0));
    let approval_queries = Arc::new(AtomicUsize::new(0));
    let a = ack_queries.clone();
    let p = approval_queries.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            match event.command.get_str("find").ok() {
                Some(crate::models::assistant_acknowledgement::COLLECTION_NAME) => {
                    assert_eq!(
                        event
                            .command
                            .get_document("filter")
                            .unwrap()
                            .get_document("conversation_id")
                            .unwrap()
                            .get_array("$in")
                            .unwrap()
                            .len(),
                        2
                    );
                    a.fetch_add(1, Ordering::SeqCst);
                }
                Some(crate::models::approval_request::COLLECTION_NAME) => {
                    assert_eq!(event.command.get_i64("limit").unwrap(), 100);
                    p.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    });
    // Only monitor after fixtures/cards are prepared; setup does point reads.
    let f = Fixture::new("org_group_card_batch").await;
    let (agent, _) = f.create().await;
    let group = create(&f, &agent).await.group;
    for actor in [&f.admin, &f.member] {
        let access = access(&f, actor, &group.id).await;
        let request = enqueue(&f, &access, &agent, &[]).await;
        let row = start(&f, &access, &agent, &request).await;
        let chat = acks::for_key(&f.state.db, actor, Some(&row.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
        acks::request(
            &f.state.db,
            &chat,
            acks::Request {
                kind: "action",
                service: None,
                tool: Some("test_action"),
                arguments: None,
                summary: "Change a resource",
                platform: false,
            },
        )
        .await
        .unwrap();
    }
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(handler);
    let mut state = f.state.clone();
    state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name());
    let a = access(&f, &f.admin, &group.id).await;
    let cards = crate::handlers::assistant_group::org_pending_actions(&state, &a)
        .await
        .unwrap();
    assert_eq!(ack_queries.load(Ordering::SeqCst), 1);
    assert_eq!(approval_queries.load(Ordering::SeqCst), 1);
    assert_eq!(cards.len(), 2);
    for actor in [&f.admin, &f.member] {
        let card = cards
            .iter()
            .find(|card| card["triggering_person"]["id"].as_str() == Some(actor))
            .unwrap();
        assert_eq!(card["agent_id"].as_str(), Some(agent.id.as_str()));
        assert_eq!(card["can_decide"].as_bool(), Some(actor == &f.admin));
    }
    f.state.db.drop().await.unwrap();
}
