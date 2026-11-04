use super::super::tests::{change, join, setup};
use super::*;
use crate::models::notification_channel::{
    COLLECTION_NAME as CHANNELS, DeviceToken, NotificationChannel,
};
use crate::services::notification_service;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn push_on(db: &Database, user: &str) {
    let channel = notification_service::get_or_create_channel(db, user)
        .await
        .unwrap();
    let device = DeviceToken {
        device_id: uuid::Uuid::new_v4().to_string(),
        platform: "fcm".into(),
        token: "test-token".into(),
        device_name: None,
        app_id: None,
        registered_at: Utc::now(),
        last_used_at: None,
    };
    db.collection::<NotificationChannel>(CHANNELS).update_one(doc! {"_id":channel.id},doc! {"$set":{"push_enabled":true,"push_devices":[bson::to_document(&device).unwrap()]}}).await.unwrap();
}
async fn post(db: &Database, actor: &str, group: &AssistantGroup, target: &str) -> GroupMessage {
    let people = participants(db, group).await.unwrap();
    let handle = people
        .iter()
        .find(|p| p.id == target)
        .unwrap()
        .handle
        .as_ref()
        .unwrap();
    participant_posts::try_post(
        db,
        actor,
        &group.id,
        &format!("@{handle} private preview sentinel"),
        Some(&[]),
        &[],
    )
    .await
    .unwrap()
    .unwrap()
}
async fn item(db: &Database, message: &GroupMessage, user: &str) -> model::MentionNotification {
    db.collection(model::MENTION_NOTIFICATIONS)
        .find_one(doc! {"message_id":&message.id,"user_id":user})
        .await
        .unwrap()
        .unwrap()
}
async fn send(
    state: &crate::AppState,
    item: &model::MentionNotification,
    calls: &Arc<AtomicUsize>,
) -> AppResult<()> {
    deliver_one_with(state, item, |user, group, name, sender| {
        assert_eq!(user, item.user_id);
        assert_eq!(group, item.group_id);
        assert!(!name.contains("private preview sentinel"));
        assert!(!sender.contains("private preview sentinel"));
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(true) }
    })
    .await
}

#[tokio::test]
async fn collaboration_read_markers_count_mentions_exclude_self_and_fence_departure() {
    let (f, _, group) = setup("collab_notification_reads").await;
    let group = join(&f, &group, &f.member).await;
    let message = post(&f.state.db, &f.admin, &group, &f.member).await;
    post(&f.state.db, &f.member, &group, &f.member).await;
    let count = counts(&f.state.db, &f.member, &group.id).await.unwrap();
    assert_eq!((count.unread_count, count.mention_count), (1, 1));
    let response =
        crate::handlers::collaboration_group::response(&f.state, &f.member, group.clone())
            .await
            .unwrap();
    assert_eq!(response.collaboration.as_ref().unwrap()["mention_count"], 1);
    assert_eq!(
        f.state
            .db
            .collection::<model::MentionNotification>(model::MENTION_NOTIFICATIONS)
            .count_documents(doc! {"user_id":&f.member})
            .await
            .unwrap(),
        1
    );
    // Opening only the received transcript advances monotonically, never beyond
    // the public sequence. Older tabs cannot regress it.
    assert_eq!(
        crate::handlers::collaboration_group::mark_read(
            axum::extract::State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.member),
            axum::extract::Path(group.id.clone()),
            axum::Json(crate::handlers::collaboration_group::ReadRequest {
                last_seen_seq: message.seq
            })
        )
        .await
        .unwrap(),
        axum::http::StatusCode::NO_CONTENT
    );
    mark_read(&f.state.db, &f.member, &group.id, 0)
        .await
        .unwrap();
    let count = counts(&f.state.db, &f.member, &group.id).await.unwrap();
    assert_eq!(
        (count.last_seen_seq, count.unread_count, count.mention_count),
        (message.seq, 0, 0)
    );
    mark_read(&f.state.db, &f.member, &group.id, i64::MAX)
        .await
        .unwrap();
    assert_eq!(
        counts(&f.state.db, &f.member, &group.id)
            .await
            .unwrap()
            .last_seen_seq,
        get(&f.state.db, &f.member, &group.id)
            .await
            .unwrap()
            .message_count
    );
    assert!(counts(&f.state.db, &f.viewer, &group.id).await.is_err());
    assert!(
        mark_read(&f.state.db, &f.viewer, &group.id, 100)
            .await
            .is_err()
    );
    change(
        &f,
        &f.admin,
        &group,
        membership::Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    assert!(counts(&f.state.db, &f.member, &group.id).await.is_err());
    assert!(
        mark_read(&f.state.db, &f.member, &group.id, 100)
            .await
            .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(model::MENTION_NOTIFICATIONS)
            .count_documents(doc! {"group_id":&group.id,"user_id":&f.member})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_mention_push_preferences_coalescing_and_live_membership() {
    let (f, _, group) = setup("collab_notification_push").await;
    let group = join(&f, &group, &f.member).await;
    push_on(&f.state.db, &f.member).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let first = post(&f.state.db, &f.admin, &group, &f.member).await;
    let first = item(&f.state.db, &first, &f.member).await;
    // Off never dispatches or consumes the rate window.
    f.state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"user_id":&f.member},
            doc! {"$set":{"group_mentions_enabled":false}},
        )
        .await
        .unwrap();
    send(&f.state, &first, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    f.state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"user_id":&f.member},
            doc! {"$set":{"group_mentions_enabled":true}},
        )
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        send(&f.state, &first, &calls),
        send(&f.state, &first, &calls)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let second = post(&f.state.db, &f.admin, &group, &f.member).await;
    let second = item(&f.state.db, &second, &f.member).await;
    send(&f.state, &second, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        counts(&f.state.db, &f.member, &group.id)
            .await
            .unwrap()
            .mention_count,
        2
    );
    let third = post(&f.state.db, &f.admin, &group, &f.member).await;
    let third = item(&f.state.db, &third, &f.member).await;
    crate::services::feature_flag_service::set_platform_override(
        &f.state.db,
        crate::services::shared_agent_service::FLAG,
        &crate::services::feature_flag_service::FlagTarget::User(f.member.clone()),
        false,
        &f.admin,
    )
    .await
    .unwrap();
    assert!(send(&f.state, &third, &calls).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    crate::services::feature_flag_service::set_platform_override(
        &f.state.db,
        crate::services::shared_agent_service::FLAG,
        &crate::services::feature_flag_service::FlagTarget::User(f.member.clone()),
        true,
        &f.admin,
    )
    .await
    .unwrap();
    let remaining = change(
        &f,
        &f.admin,
        &group,
        membership::Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await
    .group
    .unwrap();
    assert!(send(&f.state, &third, &calls).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let rejoined = join(&f, &remaining, &f.member).await;
    let message = post(&f.state.db, &f.admin, &rejoined, &f.member).await;
    send(
        &f.state,
        &item(&f.state.db, &message, &f.member).await,
        &calls,
    )
    .await
    .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "leave/rejoin must not reset the push limiter"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_agent_mentions_share_the_rate_limit_and_read_posts_do_not_push() {
    let (f, agent, group) = setup("collab_agent_mention_push").await;
    let group = join(&f, &group, &f.member).await;
    push_on(&f.state.db, &f.member).await;
    let calls = Arc::new(AtomicUsize::new(0));
    for n in 0..3 {
        let people = participants(&f.state.db, &group).await.unwrap();
        let handle = people
            .iter()
            .find(|p| p.id == f.member)
            .unwrap()
            .handle
            .as_ref()
            .unwrap();
        let request = requests::enqueue_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            "Reply to the group",
            Some(std::slice::from_ref(&agent.id)),
        )
        .await
        .unwrap();
        let started = runtime::begin_ready(
            &f.state.db,
            &f.state.encryption_keys,
            &f.admin,
            &group.id,
            &request.id,
            &agent.id,
        )
        .await
        .unwrap();
        crate::services::assistant_nyxagent::finish_turn(
            &f.state.db,
            &started.claim.conversation,
            &started.claim.conversation.credential_api_key_id,
            &uuid::Uuid::new_v4().to_string(),
            &crate::services::assistant_nyxagent::TurnResult {
                text: format!("@{handle} hello"),
                session_id: None,
                response_id: None,
                error: None,
            },
        )
        .await
        .unwrap();
        let message = messages(&f.state.db, &f.member, &group.id, None)
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.request_id.as_deref() == Some(&request.id) && m.role == "agent")
            .unwrap();
        assert_eq!(message.mentioned_participant_ids, vec![f.member.clone()]);
        let notification = item(&f.state.db, &message, &f.member).await;
        send(&f.state, &notification, &calls).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "agent mention {n}");
    }
    assert_eq!(
        counts(&f.state.db, &f.member, &group.id)
            .await
            .unwrap()
            .mention_count,
        3
    );
    f.state.db.collection::<model::ReadState>(model::READ_STATES).update_one(doc! {"group_id":&group.id,"user_id":&f.member},doc! {"$set":{"last_push_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(6))}}).await.unwrap();
    let message = post(&f.state.db, &f.admin, &group, &f.member).await;
    let notification = item(&f.state.db, &message, &f.member).await;
    mark_read(&f.state.db, &f.member, &group.id, message.seq)
        .await
        .unwrap();
    send(&f.state, &notification, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let message = post(&f.state.db, &f.admin, &group, &f.member).await;
    send(
        &f.state,
        &item(&f.state.db, &message, &f.member).await,
        &calls,
    )
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    change(&f, &f.admin, &group, membership::Change::Delete).await;
    for collection in [model::READ_STATES, model::MENTION_NOTIFICATIONS] {
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {"group_id":&group.id})
                .await
                .unwrap(),
            0
        );
    }
    f.state.db.drop().await.unwrap();
}
