use std::collections::HashSet;

use bson::{Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{Database, IndexModel};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_attachment::COLLECTION_NAME as ATTACHMENTS,
        nyxbot_image_delivery::{
            COLLECTION_NAME as CLAIMS, ImageDeliveryState, NyxbotImageDelivery,
        },
    },
};

pub fn image_delivery_id(owner: &str, attachment: &str, audience: &str) -> String {
    hex::encode(Sha256::digest(
        format!("v2\0{owner}\0{attachment}\0{audience}").as_bytes(),
    ))
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
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
    Unknown,
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

pub async fn claim(
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
                ImageClaim::Unknown => {
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
    #[ignore = "requires a private MongoDB replica set with enableTestCommands=true; run explicitly"]
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
        fail(false).await.unwrap();
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
