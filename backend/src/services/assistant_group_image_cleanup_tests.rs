//! #1884 claim v2 lifetime on group deletion: a group's attachments and every
//! image delivery claim of theirs go in the deletion transaction; claims of
//! other groups and of ordinary threads are untouched, and the orphan sweep
//! has nothing left to collect for the deleted group.
use super::*;
use crate::{
    models::nyxbot_image_delivery::{COLLECTION_NAME as CLAIMS, ImageDeliveryState},
    services::{assistant_group_service as groups, channel_image_delivery as images},
};

/// A tool image bound to a group transcript (no conversation of its own).
async fn group_image(state: &AppState, owner: &str, group_id: &str) -> String {
    let id = Uuid::new_v4().to_string();
    state
        .db
        .collection::<Document>(ATTACHMENTS)
        .insert_one(doc! {
            "_id": &id, "origin": "tool", "user_id": owner, "conversation_id": "",
            "group_id": group_id, "turn_id": format!("turn-{id}"),
            "content_type": "image/png", "size": 4_i64,
            "data_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: state.encryption_keys.encrypt(b"png!").await.unwrap(),
            },
            "created_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn group_deletion_removes_its_image_claims_and_keeps_other_scopes() {
    let (state, owner, thread) = fixture().await;
    let agent = team::ensure_nyxbot(&state.db, &owner).await.unwrap();
    let group = groups::create(
        &state.db,
        &owner,
        "Photos",
        std::slice::from_ref(&agent.id),
        "user",
    )
    .await
    .unwrap();
    let other = groups::create(
        &state.db,
        &owner,
        "Archive",
        std::slice::from_ref(&agent.id),
        "user",
    )
    .await
    .unwrap();
    let in_group = [
        group_image(&state, &owner, &group.id).await,
        group_image(&state, &owner, &group.id).await,
    ];
    let in_other = group_image(&state, &owner, &other.id).await;
    let in_thread = tool_image(&state, &owner, &thread, "turn-1", Duration::zero()).await;
    let plan = [
        (&in_group[0], &group.id, 3),
        (&in_group[1], &group.id, 2),
        (&in_other, &other.id, 2),
        (&in_thread, &thread, 1),
    ];
    for (attachment, conversation, audiences) in plan {
        for i in 0..audiences {
            let claimed = images::claim(
                &state.db,
                &owner,
                conversation,
                attachment,
                &format!("bot\u{1f}chat-{i}\u{1f}"),
            )
            .await
            .unwrap();
            // The first copy of each image was delivered; the rest are in flight.
            if i == 0
                && let images::ImageClaim::Dispatch { id, attempt } = claimed
            {
                images::finish(&state.db, &id, &attempt, ImageDeliveryState::Sent, None)
                    .await
                    .unwrap();
            }
        }
    }
    let claims = state.db.collection::<Document>(CLAIMS);
    let attachments = state.db.collection::<Document>(ATTACHMENTS);
    assert_eq!(claims.count_documents(doc! {}).await.unwrap(), 8);
    // Someone else's deletion attempt changes nothing.
    assert!(matches!(
        groups::delete(&state.db, "someone-else", &group.id).await,
        Err(AppError::NotFound(_))
    ));
    assert_eq!(claims.count_documents(doc! {}).await.unwrap(), 8);
    groups::delete(&state.db, &owner, &group.id).await.unwrap();
    let group_filter = doc! {"attachment_id": {"$in": [&in_group[0], &in_group[1]]}};
    assert_eq!(
        claims.count_documents(group_filter.clone()).await.unwrap(),
        0
    );
    assert_eq!(
        attachments
            .count_documents(doc! {"_id": {"$in": [&in_group[0], &in_group[1]]}})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        claims
            .count_documents(doc! {"attachment_id": &in_other})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        claims
            .count_documents(doc! {"attachment_id": &in_thread})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        attachments
            .count_documents(doc! {"_id": {"$in": [&in_other, &in_thread]}})
            .await
            .unwrap(),
        2
    );
    assert!(groups::get(&state.db, &owner, &group.id).await.is_err());
    assert!(groups::get(&state.db, &owner, &other.id).await.is_ok());
    // A later sender finds no claim for the deleted images and the orphan
    // sweep has nothing of theirs left; live claims survive a full pass.
    assert!(
        images::read(&state.db, &owner, &in_group[0], "bot\u{1f}chat-0\u{1f}")
            .await
            .unwrap()
            .is_none()
    );
    let mut cursor = String::new();
    loop {
        cursor = images::sweep_orphans(&state.db, &cursor, 3).await.unwrap();
        if cursor.is_empty() {
            break;
        }
    }
    assert_eq!(claims.count_documents(doc! {}).await.unwrap(), 3);
    assert_eq!(claims.count_documents(group_filter).await.unwrap(), 0);
    state.db.drop().await.unwrap();
}
