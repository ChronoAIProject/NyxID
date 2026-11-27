//! Durable admission for MCP sessions and their per-owner slots.
use std::time::Duration;

use chrono::Utc;
use mongodb::{
    bson::{self, doc},
    options::ReturnDocument,
};

use crate::models::{
    mcp_notification::{COLLECTION_NAME as MCP_NOTIFICATION_COLLECTION, McpSessionNotification},
    mcp_session::{
        MAX_PER_USER_SESSIONS, MCP_SESSION_COLLECTION, MCP_SESSION_MAX_IDLE_SECS,
        MCP_SESSION_SLOT_COLLECTION, McpSessionRecord, McpSessionSlot, McpSessionStore,
        SESSION_RECLAIM_IDLE_SECS, is_duplicate_key_error,
    },
};

use super::{
    api_key_mutation_service::transaction_result,
    erasure_admission::{OwnerFenceCreationRace, fence_in_session},
};

const SESSION_CREATE_ATTEMPTS: usize = 5;

impl McpSessionStore {
    pub async fn create(&self, user_id: &str) -> Result<Option<String>, mongodb::error::Error> {
        self.create_with_proxy_access(user_id, false, false).await
    }

    pub async fn create_with_proxy_access(
        &self,
        user_id: &str,
        proxy_authorized: bool,
        session_fallback_allowed: bool,
    ) -> Result<Option<String>, mongodb::error::Error> {
        self.create_with_parent_session(user_id, proxy_authorized, session_fallback_allowed, None)
            .await
    }

    pub async fn create_with_parent_session(
        &self,
        user_id: &str,
        proxy_authorized: bool,
        session_fallback_allowed: bool,
        parent_session_id: Option<&str>,
    ) -> Result<Option<String>, mongodb::error::Error> {
        if self.db.is_none() {
            return Ok(self.create_in_memory(
                user_id,
                proxy_authorized,
                session_fallback_allowed,
                parent_session_id,
            ));
        }

        // Admission can contend on the owner fence or a slot across replicas.
        // Only retry transactions MongoDB identifies as aborted; an unknown
        // commit result must not create a second session.
        for attempt in 0..SESSION_CREATE_ATTEMPTS {
            match self
                .create_durable_session(
                    user_id,
                    proxy_authorized,
                    session_fallback_allowed,
                    parent_session_id,
                )
                .await
            {
                Err(error)
                    if attempt + 1 < SESSION_CREATE_ATTEMPTS
                        && (error.contains_label("TransientTransactionError")
                            || error.get_custom::<OwnerFenceCreationRace>().is_some()
                            || error
                                .get_custom::<super::mcp_session_storage::ConcurrentMcpClaim>()
                                .is_some())
                        && !error.contains_label("UnknownTransactionCommitResult") =>
                {
                    tokio::time::sleep(Duration::from_millis(10 << attempt)).await;
                }
                result => return result,
            }
        }
        unreachable!("the final attempt returns its result")
    }

    async fn create_durable_session(
        &self,
        user_id: &str,
        proxy_authorized: bool,
        session_fallback_allowed: bool,
        parent_session_id: Option<&str>,
    ) -> Result<Option<String>, mongodb::error::Error> {
        let db = self.db.as_ref().expect("durable session store");

        for slot in 0..MAX_PER_USER_SESSIONS {
            let now = Utc::now();
            let expires_at = now + chrono::Duration::seconds(MCP_SESSION_MAX_IDLE_SECS as i64);
            let session_id = uuid::Uuid::new_v4().to_string();
            let slot_id = format!("{user_id}:{slot}");
            let mut transaction = db.client().start_session().await?;
            transaction.start_transaction().await?;
            let storage_id =
                super::mcp_session_storage::issuance_key(db, &mut transaction, &session_id).await?;
            let record = McpSessionRecord {
                id: storage_id.clone(),
                user_id: user_id.to_string(),
                client_info: None,
                activated_service_ids: Vec::new(),
                proxy_authorized,
                session_fallback_allowed,
                parent_session_id: parent_session_id.map(str::to_owned),
                notification_sequence: 0,
                created_at: now,
                absolute_expires_at: None,
                last_active_at: now,
                expires_at,
            };
            let claim = db
                .collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION)
                .find_one_and_update(
                    doc! {
                        "_id": &slot_id,
                        "$or": [
                            { "expires_at": { "$lte": bson::DateTime::from_chrono(now) } },
                            { "session_id": &storage_id },
                        ],
                    },
                    doc! { "$set": {
                        "user_id": user_id,
                        "session_id": &storage_id,
                        "slot": slot as i32,
                        "expires_at": bson::DateTime::from_chrono(expires_at),
                    }},
                )
                .upsert(true)
                .return_document(ReturnDocument::After)
                .session(&mut transaction)
                .await;
            match claim {
                Ok(Some(_)) => {}
                Ok(None) => {
                    transaction.abort_transaction().await?;
                    continue;
                }
                Err(error) if is_duplicate_key_error(&error) => {
                    let _ = transaction.abort_transaction().await;
                    continue;
                }
                Err(error) => {
                    let _ = transaction.abort_transaction().await;
                    return Err(error);
                }
            }
            if storage_id == session_id {
                super::mcp_session_storage::fence_activation(db, &mut transaction).await?;
            }
            if let Err(error) =
                transaction_result(fence_in_session(db, user_id, &mut transaction).await)
            {
                let _ = transaction.abort_transaction().await;
                return Err(error);
            }
            if let Err(error) = db
                .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                .insert_one(&record)
                .session(&mut transaction)
                .await
            {
                let _ = transaction.abort_transaction().await;
                return Err(error);
            }
            transaction.commit_transaction().await?;
            self.hydrate(record);
            return Ok(Some(session_id));
        }
        self.replace_oldest_idle_session(
            user_id,
            proxy_authorized,
            session_fallback_allowed,
            parent_session_id,
        )
        .await
    }

    /// At capacity, replace one idle session atomically with its admission slot.
    /// Touches update the same record in a transaction, so concurrent activity
    /// conflicts with reclamation instead of reviving an evicted session.
    async fn replace_oldest_idle_session(
        &self,
        user_id: &str,
        proxy_authorized: bool,
        session_fallback_allowed: bool,
        parent_session_id: Option<&str>,
    ) -> Result<Option<String>, mongodb::error::Error> {
        let db = self.db.as_ref().expect("durable session store");
        let now = Utc::now();
        let cutoff = now - chrono::Duration::seconds(SESSION_RECLAIM_IDLE_SECS);
        let mut transaction = db.client().start_session().await?;
        transaction.start_transaction().await?;
        if let Err(error) =
            transaction_result(fence_in_session(db, user_id, &mut transaction).await)
        {
            let _ = transaction.abort_transaction().await;
            return Err(error);
        }
        let old = db
            .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .find_one_and_delete(doc! {
                "user_id": user_id,
                "last_active_at": { "$lte": bson::DateTime::from_chrono(cutoff) },
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            })
            .sort(doc! { "last_active_at": 1, "_id": 1 })
            .session(&mut transaction)
            .await?;
        let Some(old) = old else {
            transaction.abort_transaction().await?;
            return Ok(None);
        };
        let id = uuid::Uuid::new_v4().to_string();
        let storage_id =
            super::mcp_session_storage::issuance_key(db, &mut transaction, &id).await?;
        if storage_id == id {
            super::mcp_session_storage::fence_activation(db, &mut transaction).await?;
        }
        let expires_at = now + chrono::Duration::seconds(MCP_SESSION_MAX_IDLE_SECS as i64);
        let slot = db
            .collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION)
            .update_one(
                doc! { "user_id": user_id, "session_id": &old.id },
                doc! { "$set": {
                    "session_id": &storage_id,
                    "expires_at": bson::DateTime::from_chrono(expires_at),
                }},
            )
            .session(&mut transaction)
            .await?;
        if slot.matched_count != 1 {
            transaction.abort_transaction().await?;
            return Ok(None);
        }
        let record = McpSessionRecord {
            id: storage_id,
            user_id: user_id.to_string(),
            client_info: None,
            activated_service_ids: Vec::new(),
            proxy_authorized,
            session_fallback_allowed,
            parent_session_id: parent_session_id.map(str::to_owned),
            notification_sequence: 0,
            created_at: now,
            absolute_expires_at: None,
            last_active_at: now,
            expires_at,
        };
        db.collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .insert_one(&record)
            .session(&mut transaction)
            .await?;
        db.collection::<McpSessionNotification>(MCP_NOTIFICATION_COLLECTION)
            .delete_many(doc! { "session_id": &old.id })
            .session(&mut transaction)
            .await?;
        transaction.commit_transaction().await?;
        self.evict(&old.id);
        self.hydrate(record);
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn durable_admission_refuses_account_erasure_without_creating_or_replacing_a_session() {
        use crate::errors::AppError;
        use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
        use crate::services::{api_key_mutation_service, personal_erasure_service};
        use crate::test_utils::{connect_transaction_test_database, test_user};

        let db = connect_transaction_test_database("mcp_erasure_admission").await;
        personal_erasure_service::ensure_indexes(&db).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&owner, UserType::Person),
                test_user(&other, UserType::Person),
            ])
            .await
            .unwrap();
        let store = McpSessionStore::with_db(db.clone());
        let existing = store.create(&owner).await.unwrap().expect("active owner");
        let sessions = db.collection::<bson::Document>(MCP_SESSION_COLLECTION);
        sessions
            .update_one(
                doc! {"_id": &existing},
                doc! {"$set": {"last_active_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(2))}},
            )
            .await
            .unwrap();
        let slots = db.collection::<bson::Document>(MCP_SESSION_SLOT_COLLECTION);
        let owner_filter = doc! {"user_id": &owner};
        let session_before = sessions.find_one(owner_filter.clone()).await.unwrap();
        let slot_before = slots.find_one(owner_filter.clone()).await.unwrap();
        personal_erasure_service::accept(&db, &owner, &owner, None)
            .await
            .unwrap();

        let replica = McpSessionStore::with_db(db.clone());
        let new_session = replica.create(&owner).await.unwrap_err();
        let replacement = replica
            .replace_oldest_idle_session(&owner, true, false, None)
            .await
            .unwrap_err();
        for error in [new_session, replacement] {
            assert!(matches!(
                api_key_mutation_service::map_transaction_error(error),
                AppError::Conflict(_)
            ));
        }
        assert_eq!(
            sessions
                .count_documents(owner_filter.clone())
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            slots.count_documents(owner_filter.clone()).await.unwrap(),
            1
        );
        assert_eq!(
            sessions.find_one(owner_filter.clone()).await.unwrap(),
            session_before
        );
        assert_eq!(slots.find_one(owner_filter).await.unwrap(), slot_before);
        assert!(replica.create(&other).await.unwrap().is_some());
    }
}
