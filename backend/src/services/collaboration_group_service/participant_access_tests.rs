use super::*;
use crate::handlers::collaboration_group as http;
use serde_json::json;

async fn foreign_group(name: &str) -> (Fixture, AssistantAgent, AssistantGroup) {
    let (f, agent, _) = setup(name).await;
    share(&f, &agent, &[&f.member]).await;
    let group = membership::create_ready(
        &f.state.db,
        &f.member,
        "Participants",
        std::slice::from_ref(&agent.id),
    )
    .await
    .unwrap();
    let group = join(&f, &group, &f.viewer).await;
    (f, agent, group)
}
async fn reason(f: &Fixture, actor: &str, group: &AssistantGroup, agent: &str) -> Option<String> {
    let response = http::response(&f.state, actor, group.clone())
        .await
        .unwrap();
    response
        .members
        .iter()
        .find(|m| m.id == agent)
        .unwrap()
        .unavailable_reason
        .map(str::to_owned)
}
async fn approve(
    f: &Fixture,
    actor: &str,
    group: &AssistantGroup,
    agent: &AssistantAgent,
    request: Option<&str>,
    revision: i64,
    enabled: bool,
) -> AppResult<Mutation> {
    membership::apply(
        &f.state.db,
        actor,
        &group.id,
        marker(group).unwrap().membership_revision,
        Change::GuestAccess {
            agent_id: agent.id.clone(),
            enabled,
            expected_revision: revision,
            request_id: request.map(str::to_owned),
        },
        true,
    )
    .await
}

#[tokio::test]
async fn collaboration_participant_posts_without_agents_or_requests_and_refuses_unavailable_mentions()
 {
    let (f, agent, group) = foreign_group("collab_people_only").await;
    for recipients in [Some(&[][..]), None] {
        let posted = http::post(
            &f.state,
            &f.viewer,
            &group.id,
            "Hello everyone",
            recipients,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(posted["request_id"], serde_json::Value::Null);
        assert_eq!(posted["addressed_agent_ids"], json!([]));
    }
    let mentioned = format!("@{} can you help?", agent.name);
    http::post(&f.state, &f.viewer, &group.id, &mentioned, Some(&[]), &[])
        .await
        .unwrap();
    let transcript = messages(&f.state.db, &f.member, &group.id, None)
        .await
        .unwrap();
    assert_eq!(transcript.len(), 4);
    assert_eq!(transcript[2].text, mentioned);
    assert_eq!(transcript[3].role, "notice");
    assert_eq!(transcript[3].agent_id.as_deref(), Some(agent.id.as_str()));
    assert_eq!(transcript[3].text, REFUSAL);
    assert!(transcript.iter().all(|m| m.request_id.is_none()
        && m.recipient_agent_ids.is_empty()
        && m.collaboration.is_some()));
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_group::REQUESTS_COLLECTION_NAME)
            .count_documents(doc! {"group_id":&group.id})
            .await
            .unwrap(),
        0
    );
    assert!(threads(&f.state.db, &group.id).await.unwrap().is_empty());
    assert!(
        http::post(
            &f.state,
            &f.outsider,
            &group.id,
            "No access",
            Some(&[]),
            &[]
        )
        .await
        .is_err()
    );
    let upload = uuid::Uuid::new_v4().to_string();
    assert!(
        matches!(http::post(&f.state,&f.viewer,&group.id,"File",Some(&[]),&[upload]).await,Err(AppError::ValidationError(message)) if message.contains("Attachments need an agent recipient"))
    );
    // Even a participant with agent authority can explicitly choose people only.
    assert_eq!(
        http::post(
            &f.state,
            &f.member,
            &group.id,
            "For the people",
            Some(&[]),
            &[]
        )
        .await
        .unwrap()["request_id"],
        serde_json::Value::Null
    );
    let removed = change(
        &f,
        &f.member,
        &group,
        Change::Remove {
            person_id: f.viewer.clone(),
        },
    )
    .await
    .group
    .unwrap();
    assert!(
        http::post(&f.state, &f.viewer, &removed.id, "Removed", Some(&[]), &[])
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_participant_guest_requests_dedup_cancel_expire_and_owner_only_approval() {
    let (f, agent, group) = foreign_group("collab_guest_requests").await;
    assert!(!group.participant_user_ids.contains(&f.admin));
    assert_eq!(
        reason(&f, &f.viewer, &group, &agent.id).await.as_deref(),
        Some("not_shared_with_you")
    );
    let home = crate::handlers::assistant_team::nyxbot_home(&f.state, &f.admin)
        .await
        .unwrap();
    assert!(
        guest_requests::target(&f.state.db, &f.outsider, &group.id, &agent.id)
            .await
            .is_err()
    );
    assert!(
        approve(&f, &f.member, &group, &agent, None, 1, true)
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(
        guest_requests::ask(&f.state.db, &f.viewer, &group.id, &agent.id, &home.id),
        guest_requests::ask(&f.state.db, &f.member, &group.id, &agent.id, &home.id)
    );
    let (a, created_a) = a.unwrap();
    let (b, created_b) = b.unwrap();
    assert_eq!(a.id, b.id);
    assert_ne!(created_a, created_b);
    let notified = svc::assistant_nyxagent::get(&f.state.db, &f.admin, &home.id)
        .await
        .unwrap();
    assert_eq!(
        notified
            .pending_events
            .iter()
            .filter(|e| e.id == a.id)
            .count(),
        1
    );
    assert!(
        notified
            .pending_events
            .iter()
            .any(|e| e.text.contains("details=grants") && e.text.contains(&agent.id))
    );
    assert_eq!(
        reason(&f, &f.viewer, &group, &agent.id).await.as_deref(),
        Some("owner_approval_pending")
    );
    let owner_view = http::owned_guest_groups(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.admin),
        axum::extract::Path(agent.id.clone()),
    )
    .await
    .unwrap()
    .0;
    let owner_group = owner_view["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["group_id"] == group.id)
        .unwrap();
    assert_eq!(owner_group["request"]["id"], a.id);
    assert!(owner_group.get("participants").is_none());
    assert!(owner_group.get("messages").is_none());
    assert!(
        approve(&f, &f.member, &group, &agent, Some(&a.id), 1, true)
            .await
            .is_err()
    );
    assert!(
        guest_requests::cancel(&f.state.db, &f.outsider, &group.id, &agent.id, &a.id)
            .await
            .is_err()
    );
    guest_requests::cancel(&f.state.db, &f.member, &group.id, &agent.id, &a.id)
        .await
        .unwrap();
    assert!(
        approve(&f, &f.admin, &group, &agent, Some(&a.id), 1, true)
            .await
            .is_err()
    );
    let (next, _) = guest_requests::ask(&f.state.db, &f.viewer, &group.id, &agent.id, &home.id)
        .await
        .unwrap();
    assert_ne!(a.id, next.id);
    f.state.db.collection::<bson::Document>(model::AGENT_ACCESS).update_one(doc! {"group_id":&group.id,"agent_id":&agent.id},doc! {"$set":{"guest_request.expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    assert!(
        approve(&f, &f.admin, &group, &agent, Some(&next.id), 1, true)
            .await
            .is_err()
    );
    let policy = f
        .state
        .db
        .collection::<model::AgentAccess>(model::AGENT_ACCESS)
        .find_one(doc! {"group_id":&group.id,"agent_id":&agent.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        guest_requests::projection(&policy, &group, &f.admin).unwrap()["status"],
        "expired"
    );
    let (live, _) = guest_requests::ask(&f.state.db, &f.viewer, &group.id, &agent.id, &home.id)
        .await
        .unwrap();
    approve(&f, &f.admin, &group, &agent, Some(&live.id), 1, true)
        .await
        .unwrap();
    let access = load(&f.state.db, &f.viewer, &group.id, None).await.unwrap();
    assert_eq!(
        access.agent(&agent.id).unwrap().authority().unwrap().lane,
        model::Lane::Guest
    );
    assert!(
        get(&f.state.db, &f.admin, &group.id).await.is_err(),
        "policy approval never grants transcript access"
    );
    assert_eq!(reason(&f, &f.viewer, &group, &agent.id).await, None);
    assert!(
        approve(&f, &f.admin, &group, &agent, Some(&live.id), 2, true)
            .await
            .is_err(),
        "resolved request cannot replay"
    );
    approve(&f, &f.member, &group, &agent, None, 2, false)
        .await
        .unwrap();
    assert!(
        load(&f.state.db, &f.viewer, &group.id, None)
            .await
            .unwrap()
            .agent(&agent.id)
            .unwrap()
            .authority()
            .is_none()
    );
    // Requester cancellation also works; human-only handler refuses a chat key.
    let (last, _) = guest_requests::ask(&f.state.db, &f.viewer, &group.id, &agent.id, &home.id)
        .await
        .unwrap();
    guest_requests::cancel(&f.state.db, &f.viewer, &group.id, &agent.id, &last.id)
        .await
        .unwrap();
    let (departed, _) = guest_requests::ask(&f.state.db, &f.viewer, &group.id, &agent.id, &home.id)
        .await
        .unwrap();
    let removed = change(
        &f,
        &f.member,
        &group,
        Change::Remove {
            person_id: f.viewer.clone(),
        },
    )
    .await
    .group
    .unwrap();
    assert!(
        approve(&f, &f.admin, &removed, &agent, Some(&departed.id), 3, true)
            .await
            .is_err(),
        "a former participant's request cannot enable guest access"
    );
    let mut auth = crate::test_utils::test_auth_user(&f.viewer);
    auth.api_key_id = Some(uuid::Uuid::new_v4().to_string());
    auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    assert!(
        http::request_guest_access(
            axum::extract::State(f.state.clone()),
            auth,
            axum::extract::Path((group.id.clone(), agent.id.clone()))
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_participant_unavailable_reasons_and_org_guest_refusal() {
    let (f, agent, group) = setup("collab_participant_reasons").await;
    let (org, _) = f.create().await;
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::AddAgent {
            agent_id: org.id.clone(),
            proposed_by: f.admin.clone(),
        },
    )
    .await
    .group
    .unwrap();
    let group = join(&f, &group, &f.viewer).await;
    assert_eq!(
        reason(&f, &f.viewer, &group, &agent.id).await.as_deref(),
        Some("guest_access_off")
    );
    assert_eq!(
        reason(&f, &f.viewer, &group, &org.id).await.as_deref(),
        Some("org_agent_not_guestable")
    );
    assert!(
        guest_requests::target(&f.state.db, &f.viewer, &group.id, &org.id)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert_eq!(
        reason(&f, &f.viewer, &group, &agent.id).await.as_deref(),
        Some("destroyed")
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .delete_one(doc! {"_id":&agent.id})
        .await
        .unwrap();
    assert_eq!(
        reason(&f, &f.viewer, &group, &agent.id).await.as_deref(),
        Some("removed")
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_participant_picker_edit_still_refuses_unavailable_mentions() {
    let (f, unavailable, group) = foreign_group("collab_picker_refusal").await;
    let usable = specialist(&f, &f.viewer, "available-worker").await;
    share(&f, &usable, &[&f.member]).await;
    let group = change(
        &f,
        &f.member,
        &group,
        Change::AddAgent {
            agent_id: usable.id.clone(),
            proposed_by: f.member.clone(),
        },
    )
    .await
    .group
    .unwrap();
    let text = format!("@{} and @{} please", usable.name, unavailable.name);
    let request = requests::enqueue_with_uploads(
        &f.state.db,
        &f.viewer,
        &group.id,
        &text,
        Some(std::slice::from_ref(&usable.id)),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(request.pending_agent_ids, vec![usable.id.clone()]);
    let transcript = messages(&f.state.db, &f.viewer, &group.id, None)
        .await
        .unwrap();
    assert_eq!(transcript.len(), 2);
    assert_eq!(transcript[0].recipient_agent_ids, vec![usable.id]);
    assert_eq!(
        transcript[1].agent_id.as_deref(),
        Some(unavailable.id.as_str())
    );
    assert_eq!(transcript[1].role, "notice");
    f.state.db.drop().await.unwrap();
}
