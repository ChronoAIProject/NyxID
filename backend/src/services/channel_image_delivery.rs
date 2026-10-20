use std::collections::HashSet;

use bson::{Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{Database, IndexModel, options::IndexOptions};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_attachment::COLLECTION_NAME as ATTACHMENTS,
        nyxbot_image_delivery::{
            COLLECTION_NAME as CLAIMS, ImageDeliveryState, LEGACY_COLLECTION_NAME as LEGACY,
            NyxbotImageDelivery,
        },
    },
};

pub fn image_delivery_id(owner: &str, attachment: &str, audience: &str) -> String {
    hex::encode(Sha256::digest(
        format!("v2\0{owner}\0{attachment}\0{audience}").as_bytes(),
    ))
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Document>(LEGACY)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "conversation_id": 1, "turn_id": 1, "channel_id": 1})
                .build(),
        ])
        .await?;
    db.collection::<Document>(CLAIMS)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"attachment_id": 1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "conversation_id": 1})
                .build(),
        ])
        .await?;
    Ok(())
}

pub enum ImageClaim {
    Dispatch { id: String, attempt: String },
    Existing(NyxbotImageDelivery),
    Legacy(LegacyImageOutcome),
    Unknown,
}

pub struct LegacyImageOutcome {
    pub state: ImageDeliveryState,
    pub code: Option<String>,
}

/// V1 aliases do not prove an audience. Any possible prior effect on this
/// channel suppresses the image for all its audiences until the row expires.
pub async fn read_legacy(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: &str,
    channel: &str,
    attachment: &str,
) -> AppResult<Option<LegacyImageOutcome>> {
    let mut rows = db
        .collection::<Document>(LEGACY)
        .find(doc! {
            "user_id": owner, "conversation_id": conversation, "turn_id": turn,
            "channel_id": channel, "items.id": attachment,
        })
        .projection(doc! {"state": 1, "items": 1})
        .await?;
    let mut result: Option<LegacyImageOutcome> = None;
    while let Some(row) = rows.try_next().await? {
        let Ok(items) = row.get_array("items") else {
            result = Some(LegacyImageOutcome {
                state: ImageDeliveryState::Sending,
                code: None,
            });
            continue;
        };
        for item in items
            .iter()
            .filter_map(bson::Bson::as_document)
            .filter(|item| item.get_str("id").ok() == Some(attachment))
        {
            let code = item.get_str("code").ok();
            let state = match item.get_str("state").ok() {
                Some("sent") => {
                    return Ok(Some(LegacyImageOutcome {
                        state: ImageDeliveryState::Sent,
                        code: code.map(str::to_owned),
                    }));
                }
                Some("pending") if row.get_str("state").ok() == Some("pending") => continue,
                // V1 assigned these skip codes before calling send_one. Failure
                // codes also came from adapters, so they cannot prove no I/O.
                Some("skipped")
                    if matches!(code, Some("deadline" | "after_unknown" | "billing")) =>
                {
                    continue;
                }
                Some("failed") => ImageDeliveryState::Failed,
                Some("skipped") => ImageDeliveryState::Skipped,
                _ => ImageDeliveryState::Sending,
            };
            let rank = |state| match state {
                ImageDeliveryState::Sending => 3,
                ImageDeliveryState::Failed => 2,
                _ => 1,
            };
            if result
                .as_ref()
                .is_none_or(|previous| rank(state) > rank(previous.state))
            {
                result = Some(LegacyImageOutcome {
                    state,
                    code: code.map(str::to_owned),
                });
            }
        }
    }
    Ok(result)
}

pub async fn claim_for_turn(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: &str,
    channel: &str,
    attachment: &str,
    audience: &str,
) -> AppResult<ImageClaim> {
    if let Some(outcome) = read_legacy(db, owner, conversation, turn, channel, attachment).await? {
        return Ok(ImageClaim::Legacy(outcome));
    }
    claim_v2(db, owner, conversation, attachment, audience).await
}

pub async fn read(
    db: &Database,
    owner: &str,
    attachment: &str,
    audience: &str,
) -> AppResult<Option<NyxbotImageDelivery>> {
    Ok(db
        .collection::<NyxbotImageDelivery>(CLAIMS)
        .find_one(doc! {"_id":image_delivery_id(owner, attachment, audience)})
        .await?)
}

async fn claim_v2(
    db: &Database,
    owner: &str,
    conversation: &str,
    attachment: &str,
    audience: &str,
) -> AppResult<ImageClaim> {
    let id = image_delivery_id(owner, attachment, audience);
    let collection = db.collection::<NyxbotImageDelivery>(CLAIMS);
    let row = NyxbotImageDelivery {
        id: id.clone(),
        user_id: owner.into(),
        conversation_id: conversation.into(),
        attachment_id: attachment.into(),
        audience_key: audience.into(),
        state: ImageDeliveryState::Pending,
        attempt: None,
        code: None,
        created_at: Utc::now(),
    };
    match collection.insert_one(row).await {
        Ok(_) => {}
        Err(error) if duplicate(&error) => {}
        Err(_) => return Ok(ImageClaim::Unknown),
    }
    let attempt = Uuid::new_v4().to_string();
    match collection
        .update_one(
            doc! {"_id":&id,"state":"pending"},
            doc! {"$set":{"state":"sending","attempt":&attempt}},
        )
        .await
    {
        Ok(result) if result.matched_count == 1 => Ok(ImageClaim::Dispatch { id, attempt }),
        Ok(_) => Ok(match collection.find_one(doc! {"_id":id}).await {
            Ok(Some(row)) => ImageClaim::Existing(row),
            _ => ImageClaim::Unknown,
        }),
        // The CAS may have committed. Never dispatch on an ambiguous response.
        Err(_) => Ok(ImageClaim::Unknown),
    }
}

#[cfg(test)]
pub async fn claim(
    db: &Database,
    owner: &str,
    conversation: &str,
    attachment: &str,
    audience: &str,
) -> AppResult<ImageClaim> {
    claim_v2(db, owner, conversation, attachment, audience).await
}

pub enum ImageCompletion {
    Recorded,
    Existing(NyxbotImageDelivery),
    Unknown,
}

pub async fn finish(
    db: &Database,
    id: &str,
    attempt: &str,
    state: ImageDeliveryState,
    code: Option<&str>,
) -> AppResult<ImageCompletion> {
    if matches!(
        state,
        ImageDeliveryState::Pending | ImageDeliveryState::Sending
    ) {
        return Err(AppError::Internal("Nonterminal image outcome".into()));
    }
    let collection = db.collection::<NyxbotImageDelivery>(CLAIMS);
    match collection
        .update_one(
            doc! {"_id":id,"state":"sending","attempt":attempt},
            doc! {"$set":{"state":state.as_str(),"code":code}},
        )
        .await
    {
        Ok(result) if result.matched_count == 1 => Ok(ImageCompletion::Recorded),
        Ok(_) => Ok(match collection.find_one(doc! {"_id":id}).await {
            Ok(Some(row)) => ImageCompletion::Existing(row),
            _ => ImageCompletion::Unknown,
        }),
        Err(_) => Ok(ImageCompletion::Unknown),
    }
}

/// Release only an attempt whose caller proved that no provider I/O started.
/// An old attempt token cannot release a later sender's claim.
pub async fn release(db: &Database, id: &str, attempt: &str) -> AppResult<ImageCompletion> {
    let collection = db.collection::<NyxbotImageDelivery>(CLAIMS);
    match collection
        .update_one(
            doc! {"_id": id, "state": "sending", "attempt": attempt},
            doc! {
                "$set": {"state": "pending"},
                "$unset": {"attempt": "", "code": ""},
            },
        )
        .await
    {
        Ok(result) if result.matched_count == 1 => Ok(ImageCompletion::Recorded),
        Ok(_) => Ok(match collection.find_one(doc! {"_id": id}).await {
            Ok(Some(row)) => ImageCompletion::Existing(row),
            _ => ImageCompletion::Unknown,
        }),
        Err(_) => Ok(ImageCompletion::Unknown),
    }
}

pub async fn source_exists(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: &str,
    attachment: &str,
) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(ATTACHMENTS)
        .find_one(doc! {
            "_id":attachment, "user_id":owner, "conversation_id":conversation,
            "turn_id":turn, "origin":"tool",
        })
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}

pub async fn remove(db: &Database, id: &str) -> AppResult<()> {
    db.collection::<Document>(CLAIMS)
        .delete_one(doc! {"_id":id})
        .await?;
    Ok(())
}

/// Unique claim IDs, rather than attachment IDs, bound each sweep page.
pub async fn sweep_orphans(db: &Database, cursor: &str, limit: i64) -> AppResult<String> {
    let rows: Vec<NyxbotImageDelivery> = db
        .collection::<NyxbotImageDelivery>(CLAIMS)
        .find(doc! {"_id":{"$gt":cursor}})
        .sort(doc! {"_id":1})
        .limit(limit)
        .await?
        .try_collect()
        .await?;
    let ids: Vec<&str> = rows.iter().map(|row| row.attachment_id.as_str()).collect();
    if !ids.is_empty() {
        let present: Vec<Document> = db
            .collection::<Document>(ATTACHMENTS)
            .find(doc! {"_id":{"$in":ids}})
            .projection(doc! {"_id":1})
            .await?
            .try_collect()
            .await?;
        let present: HashSet<&str> = present
            .iter()
            .filter_map(|row| row.get_str("_id").ok())
            .collect();
        let orphan_ids: Vec<&str> = rows
            .iter()
            .filter(|row| !present.contains(row.attachment_id.as_str()))
            .map(|row| row.id.as_str())
            .collect();
        if !orphan_ids.is_empty() {
            db.collection::<Document>(CLAIMS)
                .delete_many(doc! {"_id":{"$in":orphan_ids}})
                .await?;
        }
    }
    Ok(if rows.len() == limit as usize {
        rows.last().map(|row| row.id.clone()).unwrap_or_default()
    } else {
        String::new()
    })
}

fn duplicate(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(w)) if w.code == 11000)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn source(db: &Database, id: &str) {
        db.collection::<Document>(ATTACHMENTS).insert_one(doc! {
            "_id":id,"user_id":"owner","conversation_id":"conversation","turn_id":"turn","origin":"tool",
        }).await.unwrap();
    }

    fn legacy_row(
        attachment: &str,
        alias: &str,
        state: &str,
        item_state: &str,
        code: Option<&str>,
    ) -> Document {
        doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": "owner",
            "conversation_id": "conversation", "turn_id": "turn", "channel_id": "channel",
            "destination_key": format!("{alias}:destination"), "state": state,
            "claim_id": "old-attempt", "items": [{"id": attachment, "state": item_state, "code": code}],
            "created_at": bson::DateTime::now(),
            "expires_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::days(7)),
        }
    }

    #[tokio::test]
    async fn legacy_image_claims_block_all_aliases_for_sent_and_uncertain_items() {
        let db = crate::test_utils::connect_transaction_test_database("image_v1_barriers").await;
        for alias in ["inbound", "thread", "chat", "initiated"] {
            for (state, item_state, expected) in [
                ("done", "sent", ImageDeliveryState::Sent),
                ("sending", "sending", ImageDeliveryState::Sending),
                ("done", "unknown", ImageDeliveryState::Sending),
                ("sending", "pending", ImageDeliveryState::Sending),
            ] {
                let attachment = format!("{alias}-{item_state}");
                let row = legacy_row(&attachment, alias, state, item_state, None);
                db.collection::<Document>(LEGACY)
                    .insert_one(&row)
                    .await
                    .unwrap();
                for audience in ["first-chat", "second-chat-topic"] {
                    assert!(matches!(
                        claim_for_turn(&db, "owner", "conversation", "turn", "channel", &attachment, audience).await.unwrap(),
                        ImageClaim::Legacy(outcome) if outcome.state == expected
                    ));
                }
                assert_eq!(
                    db.collection::<Document>(LEGACY)
                        .find_one(doc! {"_id": &row["_id"]})
                        .await
                        .unwrap(),
                    Some(row)
                );
            }
        }
        let uncertain = legacy_row("mixed", "inbound", "sending", "unknown", None);
        let sent = legacy_row("mixed", "initiated", "done", "sent", None);
        db.collection::<Document>(LEGACY)
            .insert_many([uncertain, sent])
            .await
            .unwrap();
        assert!(matches!(
            claim_for_turn(&db, "owner", "conversation", "turn", "channel", "mixed", "audience").await.unwrap(),
            ImageClaim::Legacy(outcome) if outcome.state == ImageDeliveryState::Sent
        ));
        assert_eq!(
            db.collection::<Document>(CLAIMS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_image_claims_ignore_other_sources_and_allow_absent_images() {
        let db = crate::test_utils::connect_transaction_test_database("image_v1_sources").await;
        for field in ["user_id", "conversation_id", "turn_id", "channel_id"] {
            let mut row = legacy_row("A", "inbound", "done", "sent", None);
            row.insert(field, "other");
            db.collection::<Document>(LEGACY)
                .insert_one(row)
                .await
                .unwrap();
        }
        db.collection::<Document>(LEGACY)
            .insert_one(legacy_row("another-image", "chat", "done", "sent", None))
            .await
            .unwrap();
        for attachment in ["A", "absent"] {
            assert!(matches!(
                claim_for_turn(
                    &db,
                    "owner",
                    "conversation",
                    "turn",
                    "channel",
                    attachment,
                    "audience"
                )
                .await
                .unwrap(),
                ImageClaim::Dispatch { .. }
            ));
        }
        assert_eq!(
            db.collection::<Document>(CLAIMS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_image_claims_retry_only_proven_pre_dispatch_states() {
        let db = crate::test_utils::connect_transaction_test_database("image_v1_preflight").await;
        for (index, (state, item_state, code, retry)) in [
            ("done", "skipped", Some("deadline"), true),
            ("done", "skipped", Some("after_unknown"), true),
            ("done", "skipped", Some("billing"), true),
            ("pending", "pending", None, true),
            ("done", "failed", Some("billing"), false),
            ("done", "failed", Some("platform_error"), false),
            ("done", "skipped", Some("unrecognized"), false),
            ("done", "pending", None, false),
            ("done", "unrecognized", None, false),
        ]
        .into_iter()
        .enumerate()
        {
            let attachment = index.to_string();
            let row = legacy_row(&attachment, "thread", state, item_state, code);
            db.collection::<Document>(LEGACY)
                .insert_one(&row)
                .await
                .unwrap();
            let result = claim_for_turn(
                &db,
                "owner",
                "conversation",
                "turn",
                "channel",
                &attachment,
                "audience",
            )
            .await
            .unwrap();
            if retry {
                assert!(matches!(result, ImageClaim::Dispatch { .. }));
            } else {
                let ImageClaim::Legacy(outcome) = result else {
                    panic!("legacy barrier")
                };
                assert_eq!(outcome.code.as_deref(), code);
                assert_eq!(
                    outcome.state,
                    match item_state {
                        "failed" => ImageDeliveryState::Failed,
                        "skipped" => ImageDeliveryState::Skipped,
                        _ => ImageDeliveryState::Sending,
                    }
                );
            }
            assert_eq!(
                db.collection::<Document>(LEGACY)
                    .find_one(doc! {"_id": &row["_id"]})
                    .await
                    .unwrap(),
                Some(row)
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_image_ttl_and_v2_documents_stay_separate() {
        let db = crate::test_utils::connect_transaction_test_database("image_v1_v2_indexes").await;
        ensure_indexes(&db).await.unwrap();
        source(&db, "A").await;
        let mut row = legacy_row("A", "chat", "done", "sent", None);
        row.insert("_id", image_delivery_id("owner", "A", "audience"));
        row.insert("turn_id", "another-turn");
        db.collection::<Document>(LEGACY)
            .insert_one(&row)
            .await
            .unwrap();
        assert!(matches!(
            claim_for_turn(
                &db,
                "owner",
                "conversation",
                "turn",
                "channel",
                "A",
                "audience"
            )
            .await
            .unwrap(),
            ImageClaim::Dispatch { .. }
        ));
        ensure_indexes(&db).await.unwrap();
        let claim = read(&db, "owner", "A", "audience").await.unwrap().unwrap();
        assert_eq!(claim.state, ImageDeliveryState::Sending);
        assert!(sweep_orphans(&db, "", 10).await.unwrap().is_empty());
        assert_eq!(
            read(&db, "owner", "A", "audience")
                .await
                .unwrap()
                .unwrap()
                .id,
            claim.id
        );
        db.collection::<Document>(ATTACHMENTS)
            .delete_one(doc! {"_id": "A"})
            .await
            .unwrap();
        assert!(sweep_orphans(&db, "", 10).await.unwrap().is_empty());
        assert!(read(&db, "owner", "A", "audience").await.unwrap().is_none());
        assert_eq!(
            db.collection::<Document>(LEGACY)
                .find_one(doc! {"_id": &row["_id"]})
                .await
                .unwrap(),
            Some(row)
        );
        assert_eq!(
            db.collection::<Document>(CLAIMS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let old_indexes: Vec<_> = db
            .collection::<Document>(LEGACY)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(old_indexes.iter().any(|index| {
            index.keys == doc! {"expires_at": 1}
                && index
                    .options
                    .as_ref()
                    .and_then(|options| options.expire_after)
                    == Some(std::time::Duration::ZERO)
        }));
        let new_indexes: Vec<_> = db
            .collection::<Document>(CLAIMS)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(new_indexes.iter().all(|index| {
            index
                .options
                .as_ref()
                .is_none_or(|options| options.expire_after.is_none())
        }));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn channel_image_claims_arbitrate_each_image_and_audience_and_report_durable_states() {
        let db = crate::test_utils::connect_transaction_test_database("channel_image_v2").await;
        ensure_indexes(&db).await.unwrap();
        source(&db, "A").await;
        source(&db, "B").await;
        let acquire = || claim(&db, "owner", "conversation", "A", "bot\u{1f}chat\u{1f}");
        let results = futures::future::join_all((0..12).map(|_| acquire())).await;
        let mut winner = None;
        let mut peers = 0;
        for result in results {
            match result.unwrap() {
                ImageClaim::Dispatch { id, attempt } => {
                    assert!(winner.is_none());
                    winner = Some((id, attempt));
                }
                ImageClaim::Existing(row) => {
                    assert_eq!(row.state, ImageDeliveryState::Sending);
                    peers += 1;
                }
                ImageClaim::Unknown | ImageClaim::Legacy(_) => {
                    panic!("Acknowledged competing claims must have a durable winner")
                }
            }
        }
        assert_eq!(peers, 11);
        let (id, attempt) = winner.unwrap();
        assert_eq!(
            id,
            hex::encode(Sha256::digest(b"v2\0owner\0A\0bot\x1fchat\x1f"))
        );
        assert!(
            matches!(finish(&db,&id,"stale",ImageDeliveryState::Sent,None).await.unwrap(),ImageCompletion::Existing(row) if row.state==ImageDeliveryState::Sending)
        );
        assert!(matches!(
            finish(&db, &id, &attempt, ImageDeliveryState::Sent, None)
                .await
                .unwrap(),
            ImageCompletion::Recorded
        ));
        assert!(
            matches!(acquire().await.unwrap(),ImageClaim::Existing(row) if row.state==ImageDeliveryState::Sent)
        );
        assert!(matches!(
            claim(&db, "owner", "conversation", "B", "bot\u{1f}chat\u{1f}")
                .await
                .unwrap(),
            ImageClaim::Dispatch { .. }
        ));
        assert!(matches!(
            claim(&db, "owner", "conversation", "A", "bot\u{1f}other\u{1f}")
                .await
                .unwrap(),
            ImageClaim::Dispatch { .. }
        ));
        for (state, code) in [
            (ImageDeliveryState::Failed, Some("refused")),
            (ImageDeliveryState::Skipped, Some("deadline")),
            (ImageDeliveryState::Unknown, None),
        ] {
            let audience = format!("{state:?}");
            let ImageClaim::Dispatch { id, attempt } =
                claim(&db, "owner", "conversation", "A", &audience)
                    .await
                    .unwrap()
            else {
                panic!("new audience")
            };
            assert!(matches!(
                finish(&db, &id, &attempt, state, code).await.unwrap(),
                ImageCompletion::Recorded
            ));
            assert!(
                matches!(claim(&db,"owner","conversation","A",&audience).await.unwrap(),ImageClaim::Existing(row) if row.state==state && row.code.as_deref()==code)
            );
        }
        let id = image_delivery_id("owner", "A", "pending");
        claim(&db, "owner", "conversation", "A", "pending")
            .await
            .unwrap();
        db.collection::<Document>(CLAIMS)
            .update_one(doc! {"_id":&id}, doc! {"$set":{"state":"pending"}})
            .await
            .unwrap();
        assert!(matches!(
            claim(&db, "owner", "conversation", "A", "pending")
                .await
                .unwrap(),
            ImageClaim::Dispatch { .. }
        ));
        let indexes: Vec<_> = db
            .collection::<Document>(CLAIMS)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(indexes.iter().all(|index| {
            index
                .options
                .as_ref()
                .is_none_or(|options| options.expire_after.is_none())
        }));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn channel_image_orphan_sweep_visits_every_claim_and_restarts_after_a_pass() {
        let db =
            crate::test_utils::connect_transaction_test_database("channel_image_orphans").await;
        source(&db, "live").await;
        source(&db, "late-delete").await;
        for image in ["live", "gone", "late-delete"] {
            for destination in 0..7 {
                claim(
                    &db,
                    "owner",
                    "conversation",
                    image,
                    &destination.to_string(),
                )
                .await
                .unwrap();
            }
        }
        let mut cursor = String::new();
        loop {
            cursor = sweep_orphans(&db, &cursor, 3).await.unwrap();
            if cursor.is_empty() {
                break;
            }
        }
        assert_eq!(
            db.collection::<Document>(CLAIMS)
                .count_documents(doc! {"attachment_id":"gone"})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(CLAIMS)
                .count_documents(doc! {"attachment_id":"live"})
                .await
                .unwrap(),
            7
        );
        db.collection::<Document>(ATTACHMENTS)
            .delete_one(doc! {"_id":"late-delete"})
            .await
            .unwrap();
        loop {
            cursor = sweep_orphans(&db, &cursor, 3).await.unwrap();
            if cursor.is_empty() {
                break;
            }
        }
        assert_eq!(
            db.collection::<Document>(CLAIMS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            7
        );
        // A crash between claim insertion and source recheck leaves only an orphan.
        claim(&db, "owner", "conversation", "never-present", "crash")
            .await
            .unwrap();
        assert!(
            !source_exists(&db, "owner", "conversation", "turn", "never-present")
                .await
                .unwrap()
        );
        loop {
            cursor = sweep_orphans(&db, &cursor, 3).await.unwrap();
            if cursor.is_empty() {
                break;
            }
        }
        assert_eq!(
            db.collection::<Document>(CLAIMS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            7
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn channel_image_release_is_attempt_fenced_and_cannot_reset_terminal_effects() {
        let db =
            crate::test_utils::connect_transaction_test_database("channel_image_release").await;
        let ImageClaim::Dispatch { id, attempt: first } =
            claim(&db, "owner", "conversation", "A", "audience")
                .await
                .unwrap()
        else {
            panic!("new claim")
        };
        assert!(
            matches!(release(&db, &id, "stale").await.unwrap(), ImageCompletion::Existing(row) if row.state == ImageDeliveryState::Sending)
        );
        assert!(matches!(
            release(&db, &id, &first).await.unwrap(),
            ImageCompletion::Recorded
        ));
        let pending = read(&db, "owner", "A", "audience").await.unwrap().unwrap();
        assert_eq!(pending.state, ImageDeliveryState::Pending);
        assert!(pending.attempt.is_none() && pending.code.is_none());
        let ImageClaim::Dispatch {
            attempt: second, ..
        } = claim(&db, "owner", "conversation", "A", "audience")
            .await
            .unwrap()
        else {
            panic!("released claim")
        };
        assert_ne!(first, second);
        assert!(
            matches!(release(&db, &id, &first).await.unwrap(), ImageCompletion::Existing(row) if row.state == ImageDeliveryState::Sending)
        );
        for state in [
            ImageDeliveryState::Sent,
            ImageDeliveryState::Failed,
            ImageDeliveryState::Unknown,
        ] {
            let audience = state.as_str();
            let ImageClaim::Dispatch { id, attempt } =
                claim(&db, "owner", "conversation", "B", audience)
                    .await
                    .unwrap()
            else {
                panic!("new claim")
            };
            finish(&db, &id, &attempt, state, None).await.unwrap();
            assert!(
                matches!(release(&db, &id, &attempt).await.unwrap(), ImageCompletion::Existing(row) if row.state == state)
            );
            assert!(
                matches!(claim(&db, "owner", "conversation", "B", audience).await.unwrap(), ImageClaim::Existing(row) if row.state == state)
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn channel_image_claims_fail_closed_on_uncommitted_and_committed_uncertain_writes() {
        let base =
            crate::test_utils::connect_transaction_test_database("channel_image_write_faults")
                .await;
        let app = format!("image-cas-{}", Uuid::new_v4());
        let mut options = mongodb::options::ClientOptions::parse(
            std::env::var("NYXID_TEST_DATABASE_URL").expect("MongoDB test URL"),
        )
        .await
        .unwrap();
        options.app_name = Some(app.clone());
        options.retry_writes = Some(false);
        let client = mongodb::Client::with_options(options).unwrap();
        let db = client.database(base.name());
        let admin = base.client().database("admin");
        let fail = |committed: bool| {
            let mut data = doc! {"failCommands":["update"],"appName":&app};
            if committed {
                data.insert(
                    "writeConcernError",
                    doc! {"code":64,"errmsg":"injected uncertain acknowledgement"},
                );
            } else {
                data.insert("errorCode", 91);
            }
            admin.run_command(
                doc! {"configureFailPoint":"failCommand","mode":{"times":1},"data":data},
            )
        };
        // A rejected acquisition never authorizes dispatch; the pending claim
        // can safely be acquired by a later acknowledged CAS.
        if let Err(error) = fail(false).await {
            if matches!(
                error.kind.as_ref(),
                mongodb::error::ErrorKind::Command(command)
                    if matches!(command.code, 13 | 59 | 115)
            ) {
                eprintln!(
                    "Skipping image uncertain-write fault test: configureFailPoint was rejected; \
                     enable MongoDB test commands and grant failpoint authority ({error})"
                );
                base.drop().await.unwrap();
                return;
            }
            panic!("Failed to configure image write fault: {error}");
        }
        assert!(matches!(
            claim(&db, "owner", "conversation", "A", "rejected")
                .await
                .unwrap(),
            ImageClaim::Unknown
        ));
        assert_eq!(
            read(&base, "owner", "A", "rejected")
                .await
                .unwrap()
                .unwrap()
                .state,
            ImageDeliveryState::Pending
        );
        let ImageClaim::Dispatch { id, attempt } =
            claim(&db, "owner", "conversation", "A", "rejected")
                .await
                .unwrap()
        else {
            panic!("pending is reacquirable")
        };
        // A rejected finish retains the durable sending barrier.
        fail(false).await.unwrap();
        assert!(matches!(
            finish(&db, &id, &attempt, ImageDeliveryState::Sent, None)
                .await
                .unwrap(),
            ImageCompletion::Unknown
        ));
        assert!(
            matches!(claim(&base,"owner","conversation","A","rejected").await.unwrap(),ImageClaim::Existing(row) if row.state==ImageDeliveryState::Sending)
        );
        // The update commits but its acknowledgement reports uncertainty.
        // This caller reports unknown; subsequent readers trust durable sent.
        fail(true).await.unwrap();
        assert!(matches!(
            finish(&db, &id, &attempt, ImageDeliveryState::Sent, None)
                .await
                .unwrap(),
            ImageCompletion::Unknown
        ));
        assert!(
            matches!(claim(&base,"owner","conversation","A","rejected").await.unwrap(),ImageClaim::Existing(row) if row.state==ImageDeliveryState::Sent)
        );
        fail(true).await.unwrap();
        assert!(matches!(
            claim(&db, "owner", "conversation", "B", "committed")
                .await
                .unwrap(),
            ImageClaim::Unknown
        ));
        assert!(
            matches!(claim(&base,"owner","conversation","B","committed").await.unwrap(),ImageClaim::Existing(row) if row.state==ImageDeliveryState::Sending)
        );
        admin
            .run_command(doc! {"configureFailPoint":"failCommand","mode":"off"})
            .await
            .unwrap();
        base.drop().await.unwrap();
    }
}
