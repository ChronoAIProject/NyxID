//! NyxBot Notifications produced by real collaboration-group transitions.
use super::super::tests::{join, setup};
use super::*;
use crate::{
    models::assistant_notification::{
        AssistantNotification, COLLECTION_NAME as NOTIFICATIONS, NotificationKind, SourceType,
    },
    services::{
        assistant_notification_service as feed,
        feature_flag_service::{self as flags, FlagTarget},
    },
};

async fn feed_rows(db: &Database, user: &str) -> Vec<AssistantNotification> {
    db.collection::<AssistantNotification>(NOTIFICATIONS)
        .find(doc! {"user_id": user})
        .sort(doc! {"created_at": 1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn mention(db: &Database, actor: &str, group: &AssistantGroup, target: &str, text: &str) {
    let people = participants(db, group).await.unwrap();
    let handle = people
        .iter()
        .find(|p| p.id == target)
        .unwrap()
        .handle
        .clone()
        .unwrap();
    participant_posts::try_post(
        db,
        actor,
        &group.id,
        &format!("@{handle} {text}"),
        Some(&[]),
        &[],
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn collaboration_mentions_notify_the_mentioned_person_in_the_post_transaction() {
    let (f, _, group) = setup("collab_feed_mentions").await;
    let db = &f.state.db;
    feed::ensure_indexes(db).await.unwrap();
    let group = join(&f, &group, &f.member).await;
    // Flag off for everyone: the mention queue still works, the feed is silent.
    mention(db, &f.admin, &group, &f.member, "first").await;
    assert!(feed_rows(db, &f.member).await.is_empty());
    flags::set_platform_override(db, feed::FLAG_KEY, &FlagTarget::Global, true, &f.admin)
        .await
        .unwrap();
    feed::reset_flag_cache();
    mention(db, &f.admin, &group, &f.member, "second").await;
    mention(db, &f.admin, &group, &f.member, "third").await;
    let rows = feed_rows(db, &f.member).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::GroupMention);
    assert_eq!(rows[0].source.source_type, SourceType::CollaborationGroup);
    assert_eq!(rows[0].source.id, group.id);
    assert_eq!(rows[0].source.group_id.as_deref(), Some(group.id.as_str()));
    assert_eq!(rows[0].count, 2);
    // The author is never notified about their own mention.
    assert!(feed_rows(db, &f.admin).await.is_empty());
    let listed = feed::list(
        &f.state,
        &f.member,
        &feed::ListQuery {
            limit: 10,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].title, "Mention in Research");
    assert!(listed.items[0].preview.ends_with("third"));
    assert!(listed.items[0].url.contains(&format!("c={}", group.id)));
    // Opening the group reads it by the group id alone; the next mention is new.
    assert_eq!(
        feed::mark_read(
            db,
            &f.member,
            feed::ReadScope::Source {
                source_type: SourceType::Group,
                id: group.id.clone(),
            },
        )
        .await
        .unwrap(),
        1
    );
    mention(db, &f.admin, &group, &f.member, "fourth").await;
    let rows = feed_rows(db, &f.member).await;
    assert_eq!(rows.len(), 2);
    assert!(rows[1].unread && rows[1].count == 1);
    // A person who is no longer a participant sees nothing in the summary.
    let summary = feed::summary(db, &f.member).await.unwrap();
    assert_eq!(summary.unread_count, 1);
    db.collection::<bson::Document>(GROUPS)
        .update_one(
            doc! {"_id": &group.id},
            doc! {"$pull": {"participant_user_ids": &f.member}},
        )
        .await
        .unwrap();
    assert_eq!(feed::summary(db, &f.member).await.unwrap().unread_count, 0);
    assert!(
        feed::list(
            &f.state,
            &f.member,
            &feed::ListQuery {
                limit: 10,
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .items
        .is_empty()
    );
    assert_eq!(feed_rows(db, &f.member).await.len(), 2);
}
