use crate::{
    errors::{AppError, AppResult},
    models::machine_desktop::{COLLECTION_NAME, MachineDesktop},
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};

pub async fn get(db: &Database, node: &str) -> AppResult<Option<MachineDesktop>> {
    Ok(db
        .collection::<MachineDesktop>(COLLECTION_NAME)
        .find_one(doc! {"_id":node})
        .await?)
}

pub async fn agent_allowed(db: &Database, node: &str) -> AppResult<()> {
    if get(db, node).await?.is_some_and(|row| {
        matches!(
            row.status.as_str(),
            "owner" | "requested" | "taking" | "returning"
        )
    }) {
        return Err(AppError::MachineOwnerInControl);
    }
    Ok(())
}

pub async fn open(
    db: &Database,
    owner: &str,
    node: &str,
    conversation: Option<&str>,
) -> AppResult<MachineDesktop> {
    let fresh = MachineDesktop {
        node_id: node.into(),
        session_id: uuid::Uuid::new_v4().to_string(),
        user_id: owner.into(),
        conversation_id: conversation.map(str::to_owned),
        status: "agent".into(),
        revision: 0,
        controller: None,
        reason: None,
        handback_note: None,
        updated_at: Utc::now(),
        controller_expires_at: None,
    };
    let encoded = bson::to_document(&fresh)
        .map_err(|_| AppError::Internal("Desktop state encoding failed".into()))?;
    db.collection::<MachineDesktop>(COLLECTION_NAME)
        .update_one(doc! {"_id":node}, doc! {"$setOnInsert":encoded})
        .upsert(true)
        .await?;
    // A previous owner's idle session may be retired; active owner control
    // never expires into agent access without an explicit hand-back.
    db.collection::<MachineDesktop>(COLLECTION_NAME).update_one(
        doc!{
            "_id":node,
            "status":{
                "$in":["agent","closed"]
            },
            "updated_at":{
                "$lt":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(40))
            }
        },
        doc!{
            "$set":bson::to_document(&fresh).map_err(|_|AppError::Internal("Desktop metadata encoding failed".into()))?
        },
    ).await?;
    let mut row = get(db, node)
        .await?
        .ok_or(AppError::MachineBrowserUnavailable)?;
    if row.user_id != owner {
        return Err(AppError::MachineNotAllowed);
    }
    if row.status == "agent" {
        let mut set = doc! {"updated_at":bson::DateTime::now()};
        if let Some(conversation) = conversation {
            set.insert("conversation_id", conversation);
        }
        db.collection::<MachineDesktop>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":node,"session_id":&row.session_id,"status":"agent"},
                doc! {"$set":set},
            )
            .await?;
        if let Some(conversation) = conversation {
            row.conversation_id = Some(conversation.into());
        }
    }
    Ok(row)
}

pub async fn list(
    db: &Database,
    owner: &str,
    conversation: Option<&str>,
) -> AppResult<Vec<MachineDesktop>> {
    let mut filter = doc! {
        "user_id":owner,
        "status":{
            "$ne":"closed"
        },
        "$or":[{
            "status":{
                "$ne":"agent"
            }
        },{
            "updated_at":{
                "$gt":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(40))
            }
        }]
    };
    if let Some(conversation) = conversation {
        filter.insert("conversation_id", conversation);
    }
    Ok(db
        .collection::<MachineDesktop>(COLLECTION_NAME)
        .find(filter)
        .limit(32)
        .await?
        .try_collect()
        .await?)
}

pub async fn request(
    db: &Database,
    row: &MachineDesktop,
    reason: &str,
) -> AppResult<MachineDesktop> {
    if reason.is_empty() || reason.len() > 1000 {
        return Err(AppError::ValidationError(
            "Give a short reason for owner control".into(),
        ));
    }
    transition(
        db,
        row,
        doc! {"status":"agent"},
        doc! {
            "status":"requested",
            "reason":reason,
            "handback_note":bson::Bson::Null
        },
    )
    .await
}

pub async fn take(db: &Database, row: &MachineDesktop, viewer: &str) -> AppResult<MachineDesktop> {
    transition(
        db,
        row,
        doc! {
            "$or":[{
                "status":{
                    "$in":["agent","requested"]
                }
            },{
                "status":{
                    "$in":["owner","taking","returning"]
                },"controller_expires_at":{
                    "$lte":bson::DateTime::now()
                }
            },{
                "status":"owner","controller":viewer
            }]
        },
        doc! {
            "status":"taking",
            "controller":viewer,
            "controller_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(40))
        },
    )
    .await
}

pub async fn controlled(
    db: &Database,
    row: &MachineDesktop,
    viewer: &str,
) -> AppResult<MachineDesktop> {
    acknowledge(
        db,
        row,
        doc! {"status":"taking","controller":viewer},
        doc! {"status":"owner"},
    )
    .await
}

pub async fn release(
    db: &Database,
    row: &MachineDesktop,
    viewer: &str,
    note: &str,
) -> AppResult<MachineDesktop> {
    if note.len() > 2000 {
        return Err(AppError::ValidationError(
            "Hand-back note is too long".into(),
        ));
    }
    transition(
        db,
        row,
        doc! {"status":"owner","controller":viewer},
        doc! {"status":"returning","handback_note":note},
    )
    .await
}

pub async fn returned(db: &Database, row: &MachineDesktop) -> AppResult<MachineDesktop> {
    acknowledge(
        db,
        row,
        doc! {"status":"returning"},
        doc! {
            "status":"agent",
            "controller":bson::Bson::Null,
            "controller_expires_at":bson::Bson::Null
        },
    )
    .await
}

pub async fn touch(db: &Database, row: &MachineDesktop) -> AppResult<()> {
    db.collection::<MachineDesktop>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id":&row.node_id,
                "session_id":&row.session_id,
                "user_id":&row.user_id
            },
            doc! {"$set":{"updated_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

pub async fn refresh(db: &Database, row: &MachineDesktop, viewer: &str) -> AppResult<()> {
    db.collection::<MachineDesktop>(COLLECTION_NAME).update_one(doc!{
        "_id":&row.node_id,
        "session_id":&row.session_id,
        "controller":viewer,
        "status":"owner"
    },doc!{
        "$set":{
            "updated_at":bson::DateTime::now(),
            "controller_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(40))
        }
    }).await?;
    Ok(())
}

async fn transition(
    db: &Database,
    row: &MachineDesktop,
    mut filter: bson::Document,
    mut set: bson::Document,
) -> AppResult<MachineDesktop> {
    filter.insert("_id", &row.node_id);
    filter.insert("session_id", &row.session_id);
    filter.insert("user_id", &row.user_id);
    set.insert("updated_at", bson::DateTime::now());
    db.collection::<MachineDesktop>(COLLECTION_NAME)
        .find_one_and_update(filter, doc! {"$set":set,"$inc":{"revision":1}})
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("Desktop controller changed; refresh the panel".into()))
}

/// Completing a node-acknowledged transition retains the revision sent to the
/// node. Browser input must carry that same fence, and a delayed acknowledgement
/// must never finish a newer controller's transition.
async fn acknowledge(
    db: &Database,
    row: &MachineDesktop,
    mut filter: bson::Document,
    mut set: bson::Document,
) -> AppResult<MachineDesktop> {
    filter.insert("_id", &row.node_id);
    filter.insert("session_id", &row.session_id);
    filter.insert("user_id", &row.user_id);
    filter.insert("revision", row.revision);
    set.insert("updated_at", bson::DateTime::now());
    db.collection::<MachineDesktop>(COLLECTION_NAME)
        .find_one_and_update(filter, doc! {"$set":set})
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("Desktop controller changed; refresh the panel".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn controller_races_recover_without_releasing_agent_authority() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_desktop_control").await;
        let row = open(&db, "owner", "machine", Some("thread")).await.unwrap();
        agent_allowed(&db, "machine").await.unwrap();
        let requested = request(&db, &row, "Please sign in").await.unwrap();
        assert!(matches!(
            agent_allowed(&db, "machine").await,
            Err(AppError::MachineOwnerInControl)
        ));
        assert!(open(&db, "other", "machine", None).await.is_err());
        let (a, b) = tokio::join!(
            take(&db, &requested, "tab-a"),
            take(&db, &requested, "tab-b")
        );
        assert_ne!(a.is_ok(), b.is_ok(), "one controller wins atomically");
        let taken = a.or(b).unwrap();
        let viewer = taken.controller.as_deref().unwrap();
        assert!(controlled(&db, &taken, "wrong-tab").await.is_err());
        let owner = controlled(&db, &taken, viewer).await.unwrap();
        assert_eq!(
            owner.revision, taken.revision,
            "input uses the node's acknowledged revision"
        );
        assert!(release(&db, &owner, "wrong-tab", "done").await.is_err());
        let releasing = release(&db, &owner, viewer, "logged in").await.unwrap();
        assert!(agent_allowed(&db, "machine").await.is_err());
        let returned = returned(&db, &releasing).await.unwrap();
        assert_eq!(returned.revision, releasing.revision);
        assert_eq!(returned.handback_note.as_deref(), Some("logged in"));
        assert!(returned.revision > taken.revision);
        agent_allowed(&db, "machine").await.unwrap();
        // A replica dying between the durable claim and the node ack must be
        // recoverable by the human, while the agent remains locked out.
        let stalled = take(&db, &returned, "dead-tab").await.unwrap();
        db.collection::<MachineDesktop>(COLLECTION_NAME).update_one(doc!{"_id":"machine"},doc!{
            "$set":{
                "controller_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))
            }
        }).await.unwrap();
        assert!(agent_allowed(&db, "machine").await.is_err());
        let recovered = take(&db, &stalled, "new-tab").await.unwrap();
        assert_eq!(recovered.controller.as_deref(), Some("new-tab"));
        assert!(recovered.revision > stalled.revision);
        assert!(controlled(&db, &stalled, "dead-tab").await.is_err());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn idle_viewers_do_not_reserve_an_org_machine_forever() {
        let db = crate::test_utils::connect_transaction_test_database("machine_desktop_idle").await;
        let row = open(&db, "admin-one", "machine", Some("thread-one"))
            .await
            .unwrap();
        db.collection::<MachineDesktop>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":"machine"},
                doc! {
                    "$set":{
                        "updated_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(41))
                    }
                },
            )
            .await
            .unwrap();
        assert!(list(&db, "admin-one", None).await.unwrap().is_empty());
        let next = open(&db, "admin-two", "machine", Some("thread-two"))
            .await
            .unwrap();
        assert_ne!(row.session_id, next.session_id);
        let taking = take(&db, &next, "tab").await.unwrap();
        controlled(&db, &taking, "tab").await.unwrap();
        db.collection::<MachineDesktop>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":"machine"},
                doc! {
                    "$set":{
                        "updated_at":bson::DateTime::from_chrono(Utc::now()-Duration::hours(1))
                    }
                },
            )
            .await
            .unwrap();
        assert!(
            open(&db, "admin-one", "machine", None).await.is_err(),
            "idle owner sessions never silently return to the agent"
        );
        db.drop().await.unwrap();
    }
}
