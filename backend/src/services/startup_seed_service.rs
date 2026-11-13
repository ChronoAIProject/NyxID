use mongodb::{Collection, bson::Document};
use serde::{Serialize, de::DeserializeOwned};

use crate::errors::AppResult;

/// On a duplicate, return the row matching the caller's original existence
/// predicate so it can follow its existing-row branch. None means inserted.
pub(super) async fn insert_or_read<T>(
    collection: &Collection<T>,
    seed: &T,
    winner_filter: Document,
) -> AppResult<Option<T>>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    match collection.insert_one(seed).await {
        Ok(_) => Ok(None),
        Err(error) if super::telegram_new_service::is_duplicate_key_error(&error) => {
            match collection.find_one(winner_filter).await? {
                Some(winner) => Ok(Some(winner)),
                None => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}
