//! Child-first cleanup for parents whose existing deletion is a standalone write.
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::Database;

use crate::errors::{AppError, AppResult};

pub(super) async fn delete_inactive_parents(
    db: &Database,
    collection: &str,
    owner: Document,
    children: &[(&str, &[&str])],
) -> AppResult<()> {
    let filter = doc! {"$and":[owner, {"is_active":false}]};
    loop {
        let parents: Vec<Document> = db
            .collection::<Document>(collection)
            .find(filter.clone())
            .projection(doc! {"_id":1})
            .sort(doc! {"_id":1})
            .limit(100)
            .max_time(std::time::Duration::from_secs(3))
            .await?
            .try_collect()
            .await?;
        if parents.is_empty() {
            return Ok(());
        }
        let ids: Vec<_> = parents
            .iter()
            .map(|p| {
                p.get_str("_id").map(str::to_owned).map_err(|_| {
                    AppError::Internal("Organization resource identity invalid".into())
                })
            })
            .collect::<AppResult<_>>()?;
        for (child, fields) in children {
            let filters: Vec<_> = fields.iter().map(|f| doc! {*f:{"$in":&ids}}).collect();
            db.collection::<Document>(child)
                .delete_many(doc! {"$or":filters})
                .await?;
        }
        // Never consume a newly-created parent whose children we did not visit.
        let mut exact = filter.clone();
        exact.insert("_id", doc! {"$in":ids});
        db.collection::<Document>(collection)
            .delete_many(exact)
            .await?;
    }
}

/// Each client unit must walk its own children, not every client's sessions.
pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    for (collection, keys) in [
        (
            crate::models::refresh_token::COLLECTION_NAME,
            doc! {"client_id":1},
        ),
        (
            crate::models::consent::COLLECTION_NAME,
            doc! {"client_id":1},
        ),
        (
            crate::models::authorization_code::COLLECTION_NAME,
            doc! {"client_id":1,"used":1},
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(keys)
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .name("org_delete_client_children".to_owned())
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    Ok(())
}
