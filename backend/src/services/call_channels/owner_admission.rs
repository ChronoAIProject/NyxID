//! The person row is the durable serialization point for adapter admission and deletion.
use crate::{
    errors::{AppError, AppResult},
    models::user::COLLECTION_NAME,
};
use mongodb::{
    ClientSession, Database,
    bson::{Document, doc},
};

fn open_owner(owner: &str) -> Document {
    doc! {
        "_id": owner,
        "is_active": true,
        "user_type": {"$ne": "org"},
        "channel_adapter_admission_closed": {"$ne": true},
    }
}

pub async fn check(db: &Database, owner: &str) -> AppResult<()> {
    if db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(open_owner(owner))
        .projection(doc! {"_id": 1})
        .await?
        .is_none()
    {
        return Err(AppError::Conflict(
            "Adapter admission is closed for this owner".into(),
        ));
    }
    Ok(())
}

pub async fn fence(db: &Database, tx: &mut ClientSession, owner: &str) -> AppResult<()> {
    let changed = db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            open_owner(owner),
            doc! {"$inc": {"channel_adapter_admission_fence": 1_i64}},
        )
        .session(tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict(
            "Adapter admission is closed for this owner".into(),
        ));
    }
    Ok(())
}

/// Keep authentication available while already-admitted cleanup is pending.
/// Admission transactions write this same row, so completion of this update
/// precedes the deletion caller's pending-evidence query.
pub async fn close(db: &Database, owner: &str) -> AppResult<()> {
    let changed = db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": owner},
            doc! {
                "$set": {"channel_adapter_admission_closed": true},
                "$inc": {"channel_adapter_admission_fence": 1_i64},
            },
        )
        .await?;
    if changed.matched_count != 1 {
        return Err(AppError::NotFound("User not found".into()));
    }
    Ok(())
}
