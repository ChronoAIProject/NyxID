use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::ReturnDocument;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::models::mcp_notification::{
    COLLECTION_NAME as MCP_NOTIFICATION_COLLECTION, McpSessionNotification,
};

pub const MAX_ACTIVATED_SERVICES: usize = 20;
pub const MCP_SESSION_MAX_IDLE_SECS: u64 = 30 * 24 * 3600;
pub const MCP_SESSION_COLLECTION: &str = "mcp_sessions";
pub const MCP_SESSION_SLOT_COLLECTION: &str = "mcp_session_slots";
pub const DEFAULT_MCP_NOTIFICATION_TTL_SECS: u64 = 30 * 24 * 3600;
pub const DEFAULT_MCP_NOTIFICATION_POLL_INTERVAL_MILLIS: u64 = 250;

const MAX_PER_USER_SESSIONS: usize = 50;
const NOTIFICATION_PAGE_SIZE: i64 = 100;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpSessionRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub client_info: Option<String>,
    #[serde(default)]
    pub activated_service_ids: Vec<String>,
    #[serde(default)]
    pub proxy_authorized: bool,
    #[serde(default)]
    pub notification_sequence: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_active_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpSessionSlot {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub session_id: String,
    pub slot: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpSessionAccess {
    pub user_id: String,
    pub proxy_authorized: bool,
}

pub struct McpSession {
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub activated_service_ids: HashSet<String>,
    pub proxy_authorized: bool,
    pub notification_sequence: i64,
    pub notification_tx: Option<mpsc::Sender<serde_json::Value>>,
}

#[derive(Clone)]
pub struct McpSessionStore {
    sessions: Arc<RwLock<HashMap<String, McpSession>>>,
    pending_receivers: Arc<RwLock<HashMap<String, mpsc::Receiver<serde_json::Value>>>>,
    db: Option<mongodb::Database>,
    notification_ttl: Duration,
    notification_poll_interval: Duration,
}

impl Default for McpSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl McpSessionStore {
    pub fn new() -> Self {
        Self::with_options(
            None,
            Duration::from_secs(DEFAULT_MCP_NOTIFICATION_TTL_SECS),
            Duration::from_millis(DEFAULT_MCP_NOTIFICATION_POLL_INTERVAL_MILLIS),
        )
    }

    pub fn with_db(db: mongodb::Database) -> Self {
        Self::with_db_options(
            db,
            Duration::from_secs(DEFAULT_MCP_NOTIFICATION_TTL_SECS),
            Duration::from_millis(DEFAULT_MCP_NOTIFICATION_POLL_INTERVAL_MILLIS),
        )
    }

    pub fn with_db_options(
        db: mongodb::Database,
        notification_ttl: Duration,
        notification_poll_interval: Duration,
    ) -> Self {
        Self::with_options(Some(db), notification_ttl, notification_poll_interval)
    }

    fn with_options(
        db: Option<mongodb::Database>,
        notification_ttl: Duration,
        notification_poll_interval: Duration,
    ) -> Self {
        assert!(
            !notification_ttl.is_zero(),
            "MCP notification TTL must be positive"
        );
        assert!(
            !notification_poll_interval.is_zero(),
            "MCP notification poll interval must be positive"
        );
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            pending_receivers: Arc::new(RwLock::new(HashMap::new())),
            db,
            notification_ttl,
            notification_poll_interval,
        }
    }

    pub fn notification_poll_interval(&self) -> Duration {
        self.notification_poll_interval
    }

    pub async fn load_from_db(&self) -> Result<usize, mongodb::error::Error> {
        let Some(db) = &self.db else {
            return Ok(0);
        };
        let now = bson::DateTime::from_chrono(Utc::now());
        let records: Vec<McpSessionRecord> = db
            .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .find(doc! { "expires_at": { "$gt": now } })
            .await?
            .try_collect()
            .await?;
        if records.is_empty() {
            return Ok(0);
        }

        let user_ids: HashSet<&str> = records
            .iter()
            .map(|record| record.user_id.as_str())
            .collect();
        let active_auth_sessions: Vec<mongodb::bson::Document> = db
            .collection::<mongodb::bson::Document>("sessions")
            .find(doc! {
                "user_id": { "$in": user_ids.iter().copied().collect::<Vec<_>>() },
                "revoked": false,
                "expires_at": { "$gt": now },
            })
            .await?
            .try_collect()
            .await?;
        let active_users: HashSet<String> = active_auth_sessions
            .iter()
            .filter_map(|session| session.get_str("user_id").ok().map(str::to_string))
            .collect();

        let mut records = records;
        records.sort_by_key(|record| (record.created_at, record.id.clone()));
        let mut orphaned_ids = Vec::new();
        let mut loaded = 0;
        for record in records {
            if active_users.contains(&record.user_id) && self.ensure_recovered_slot(&record).await?
            {
                self.hydrate(record);
                loaded += 1;
            } else {
                orphaned_ids.push(record.id);
            }
        }
        if !orphaned_ids.is_empty() {
            db.collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                .delete_many(doc! { "_id": { "$in": &orphaned_ids } })
                .await?;
            db.collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION)
                .delete_many(doc! { "session_id": { "$in": &orphaned_ids } })
                .await?;
            db.collection::<McpSessionNotification>(MCP_NOTIFICATION_COLLECTION)
                .delete_many(doc! { "session_id": { "$in": &orphaned_ids } })
                .await?;
        }
        Ok(loaded)
    }

    pub async fn create(&self, user_id: &str) -> Result<Option<String>, mongodb::error::Error> {
        self.create_with_proxy_access(user_id, false).await
    }

    pub async fn create_with_proxy_access(
        &self,
        user_id: &str,
        proxy_authorized: bool,
    ) -> Result<Option<String>, mongodb::error::Error> {
        let Some(db) = &self.db else {
            return Ok(self.create_in_memory(user_id, proxy_authorized));
        };

        for slot in 0..MAX_PER_USER_SESSIONS {
            let now = Utc::now();
            let expires_at = now + chrono::Duration::seconds(MCP_SESSION_MAX_IDLE_SECS as i64);
            let session_id = uuid::Uuid::new_v4().to_string();
            let slot_id = format!("{user_id}:{slot}");
            let record = McpSessionRecord {
                id: session_id.clone(),
                user_id: user_id.to_string(),
                client_info: None,
                activated_service_ids: Vec::new(),
                proxy_authorized,
                notification_sequence: 0,
                created_at: now,
                last_active_at: now,
                expires_at,
            };
            let mut transaction = db.client().start_session().await?;
            transaction.start_transaction().await?;
            let claim = db
                .collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION)
                .find_one_and_update(
                    doc! {
                        "_id": &slot_id,
                        "$or": [
                            { "expires_at": { "$lte": bson::DateTime::from_chrono(now) } },
                            { "session_id": &session_id },
                        ],
                    },
                    doc! { "$set": {
                        "user_id": user_id,
                        "session_id": &session_id,
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
        Ok(None)
    }

    pub async fn get_for_auth(
        &self,
        session_id: &str,
    ) -> Result<Option<McpSessionAccess>, mongodb::error::Error> {
        Ok(self
            .authoritative_record(session_id)
            .await?
            .map(|record| McpSessionAccess {
                user_id: record.user_id,
                proxy_authorized: record.proxy_authorized,
            }))
    }

    pub async fn validate(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        Ok(self
            .authoritative_record(session_id)
            .await?
            .is_some_and(|record| record.user_id == user_id))
    }

    pub async fn get_user_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, mongodb::error::Error> {
        Ok(self
            .authoritative_record(session_id)
            .await?
            .map(|record| record.user_id))
    }

    pub async fn allows_proxy_access(
        &self,
        session_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        Ok(self
            .authoritative_record(session_id)
            .await?
            .is_some_and(|record| record.proxy_authorized))
    }

    pub async fn touch(&self, session_id: &str) -> Result<bool, mongodb::error::Error> {
        let now = Utc::now();
        let expires_at = now + chrono::Duration::seconds(MCP_SESSION_MAX_IDLE_SECS as i64);
        let Some(db) = &self.db else {
            let mut sessions = self
                .sessions
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let Some(session) = sessions.get_mut(session_id) else {
                return Ok(false);
            };
            session.last_active = now;
            session.expires_at = expires_at;
            return Ok(true);
        };

        let mut transaction = db.client().start_session().await?;
        transaction.start_transaction().await?;
        let updated = db
            .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .find_one_and_update(
                doc! {
                    "_id": session_id,
                    "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                },
                doc! { "$set": {
                    "last_active_at": bson::DateTime::from_chrono(now),
                    "expires_at": bson::DateTime::from_chrono(expires_at),
                }},
            )
            .return_document(ReturnDocument::After)
            .session(&mut transaction)
            .await?;
        let Some(record) = updated else {
            transaction.abort_transaction().await?;
            self.evict(session_id);
            return Ok(false);
        };
        db.collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION)
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(expires_at) } },
            )
            .session(&mut transaction)
            .await?;
        transaction.commit_transaction().await?;
        self.hydrate(record);
        Ok(true)
    }

    pub async fn remove(&self, session_id: &str) -> Result<(), mongodb::error::Error> {
        if let Some(db) = &self.db {
            let mut transaction = db.client().start_session().await?;
            transaction.start_transaction().await?;
            db.collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                .delete_one(doc! { "_id": session_id })
                .session(&mut transaction)
                .await?;
            db.collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION)
                .delete_many(doc! { "session_id": session_id })
                .session(&mut transaction)
                .await?;
            db.collection::<McpSessionNotification>(MCP_NOTIFICATION_COLLECTION)
                .delete_many(doc! { "session_id": session_id })
                .session(&mut transaction)
                .await?;
            transaction.commit_transaction().await?;
        }
        self.evict(session_id);
        Ok(())
    }

    pub async fn remove_by_user_id(&self, user_id: &str) -> Result<(), mongodb::error::Error> {
        if let Some(db) = &self.db {
            let mut transaction = db.client().start_session().await?;
            transaction.start_transaction().await?;
            let mut cursor = db
                .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                .find(doc! { "user_id": user_id })
                .session(&mut transaction)
                .await?;
            let ids: Vec<String> = cursor
                .stream(&mut transaction)
                .map_ok(|record| record.id)
                .try_collect()
                .await?;
            db.collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                .delete_many(doc! { "user_id": user_id })
                .session(&mut transaction)
                .await?;
            db.collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION)
                .delete_many(doc! { "user_id": user_id })
                .session(&mut transaction)
                .await?;
            if !ids.is_empty() {
                db.collection::<McpSessionNotification>(MCP_NOTIFICATION_COLLECTION)
                    .delete_many(doc! { "session_id": { "$in": &ids } })
                    .session(&mut transaction)
                    .await?;
            }
            transaction.commit_transaction().await?;
        }
        self.evict_user(user_id);
        Ok(())
    }

    pub async fn activate_services(
        &self,
        session_id: &str,
        service_ids: &[String],
    ) -> Result<bool, mongodb::error::Error> {
        self.activate_services_with_optional_notification(session_id, service_ids, None)
            .await
    }

    pub async fn activate_services_and_notify(
        &self,
        session_id: &str,
        service_ids: &[String],
        notification: serde_json::Value,
    ) -> Result<bool, mongodb::error::Error> {
        self.activate_services_with_optional_notification(
            session_id,
            service_ids,
            Some(notification),
        )
        .await
    }

    async fn activate_services_with_optional_notification(
        &self,
        session_id: &str,
        service_ids: &[String],
        notification: Option<serde_json::Value>,
    ) -> Result<bool, mongodb::error::Error> {
        if service_ids.is_empty() {
            return Ok(false);
        }
        let Some(db) = &self.db else {
            let mut sessions = self
                .sessions
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let Some(session) = sessions.get_mut(session_id) else {
                return Ok(false);
            };
            let mut changed = false;
            for id in service_ids {
                if session.activated_service_ids.len() >= MAX_ACTIVATED_SERVICES {
                    break;
                }
                changed |= session.activated_service_ids.insert(id.clone());
            }
            if changed
                && let Some(notification) = notification
                && let Some(sender) = session.notification_tx.as_ref()
            {
                let _ = sender.try_send(notification);
            }
            return Ok(changed);
        };

        let now = Utc::now();
        let requested_service_ids = bson::to_bson(service_ids).expect("MCP service ids serialize");
        let mut transaction = db.client().start_session().await?;
        transaction.start_transaction().await?;
        let before = db
            .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .find_one_and_update(
                doc! {
                    "_id": session_id,
                    "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                },
                vec![doc! { "$set": {
                    "activated_service_ids": {
                        "$let": {
                            "vars": {
                                "existing": { "$ifNull": ["$activated_service_ids", []] },
                            },
                            "in": {
                                "$reduce": {
                                    "input": requested_service_ids,
                                    "initialValue": "$$existing",
                                    "in": {
                                        "$cond": [
                                            { "$or": [
                                                { "$in": ["$$this", "$$value"] },
                                                { "$gte": [
                                                    { "$size": "$$value" },
                                                    MAX_ACTIVATED_SERVICES as i64,
                                                ] },
                                            ] },
                                            "$$value",
                                            { "$concatArrays": ["$$value", ["$$this"]] },
                                        ]
                                    }
                                }
                            }
                        }
                    }
                }}],
            )
            .return_document(ReturnDocument::Before)
            .session(&mut transaction)
            .await?;
        let Some(before) = before else {
            transaction.abort_transaction().await?;
            self.evict(session_id);
            return Ok(false);
        };
        let after = db
            .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .find_one(doc! { "_id": session_id })
            .session(&mut transaction)
            .await?
            .expect("activation update returned a session");
        let before_ids: HashSet<&str> = before
            .activated_service_ids
            .iter()
            .map(String::as_str)
            .collect();
        let after_ids: HashSet<&str> = after
            .activated_service_ids
            .iter()
            .map(String::as_str)
            .collect();
        let changed = service_ids
            .iter()
            .any(|id| !before_ids.contains(id.as_str()) && after_ids.contains(id.as_str()));

        let after = if changed && let Some(payload) = notification {
            let sequenced = db
                .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                .find_one_and_update(
                    doc! { "_id": session_id },
                    doc! { "$inc": { "notification_sequence": 1_i64 } },
                )
                .return_document(ReturnDocument::After)
                .session(&mut transaction)
                .await?
                .expect("activated session still exists in transaction");
            self.insert_notification_in_transaction(
                db,
                &mut transaction,
                session_id,
                sequenced.notification_sequence,
                payload,
                now,
            )
            .await?;
            sequenced
        } else {
            after
        };
        transaction.commit_transaction().await?;
        self.hydrate(after);
        Ok(changed)
    }

    pub async fn get_activated_service_ids(
        &self,
        session_id: &str,
    ) -> Result<HashSet<String>, mongodb::error::Error> {
        Ok(self
            .authoritative_record(session_id)
            .await?
            .map(|record| record.activated_service_ids.into_iter().collect())
            .unwrap_or_default())
    }

    pub async fn send_notification(
        &self,
        session_id: &str,
        payload: serde_json::Value,
    ) -> Result<bool, mongodb::error::Error> {
        let Some(db) = &self.db else {
            let sessions = self
                .sessions
                .read()
                .unwrap_or_else(|error| error.into_inner());
            let Some(sender) = sessions
                .get(session_id)
                .and_then(|session| session.notification_tx.as_ref())
            else {
                return Ok(false);
            };
            return Ok(sender.try_send(payload).is_ok());
        };

        let now = Utc::now();
        let mut transaction = db.client().start_session().await?;
        transaction.start_transaction().await?;
        let session = db
            .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .find_one_and_update(
                doc! {
                    "_id": session_id,
                    "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                },
                doc! { "$inc": { "notification_sequence": 1_i64 } },
            )
            .return_document(ReturnDocument::After)
            .session(&mut transaction)
            .await?;
        let Some(session) = session else {
            transaction.abort_transaction().await?;
            self.evict(session_id);
            return Ok(false);
        };
        self.insert_notification_in_transaction(
            db,
            &mut transaction,
            session_id,
            session.notification_sequence,
            payload,
            now,
        )
        .await?;
        transaction.commit_transaction().await?;
        self.hydrate(session);
        Ok(true)
    }

    pub async fn notifications_after(
        &self,
        session_id: &str,
        after_sequence: i64,
    ) -> Result<Option<Vec<McpSessionNotification>>, mongodb::error::Error> {
        let Some(db) = &self.db else {
            return Ok(self
                .sessions
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .contains_key(session_id)
                .then(Vec::new));
        };
        if self.authoritative_record(session_id).await?.is_none() {
            return Ok(None);
        }
        let notifications = db
            .collection::<McpSessionNotification>(MCP_NOTIFICATION_COLLECTION)
            .find(doc! {
                "session_id": session_id,
                "sequence": { "$gt": after_sequence },
                "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
            })
            .sort(doc! { "sequence": 1 })
            .limit(NOTIFICATION_PAGE_SIZE)
            .await?
            .try_collect()
            .await?;
        Ok(Some(notifications))
    }

    pub fn take_notification_rx(
        &self,
        session_id: &str,
    ) -> Option<mpsc::Receiver<serde_json::Value>> {
        self.pending_receivers
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(session_id)
    }

    pub fn set_notification_tx(&self, session_id: &str, tx: mpsc::Sender<serde_json::Value>) {
        if let Some(session) = self
            .sessions
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(session_id)
        {
            session.notification_tx = Some(tx);
        }
    }

    pub async fn reap_expired(&self, max_idle: Duration) -> Result<usize, mongodb::error::Error> {
        let cutoff = Utc::now()
            - chrono::Duration::from_std(max_idle).unwrap_or_else(|_| chrono::Duration::hours(1));
        let expired_ids: Vec<String> = self
            .sessions
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|(_, session)| session.last_active <= cutoff)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired_ids {
            self.evict(id);
        }
        if let Some(db) = &self.db {
            let now = bson::DateTime::from_chrono(Utc::now());
            db.collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                .delete_many(doc! { "expires_at": { "$lte": now } })
                .await?;
            db.collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION)
                .delete_many(doc! { "expires_at": { "$lte": now } })
                .await?;
        }
        Ok(expired_ids.len())
    }

    async fn authoritative_record(
        &self,
        session_id: &str,
    ) -> Result<Option<McpSessionRecord>, mongodb::error::Error> {
        let Some(db) = &self.db else {
            return Ok(self.local_record(session_id));
        };
        let record = db
            .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .find_one(doc! {
                "_id": session_id,
                "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
            })
            .await?;
        match record {
            Some(record) => {
                self.hydrate(record.clone());
                Ok(Some(record))
            }
            None => {
                self.evict(session_id);
                Ok(None)
            }
        }
    }

    async fn insert_notification_in_transaction(
        &self,
        db: &mongodb::Database,
        transaction: &mut mongodb::ClientSession,
        session_id: &str,
        sequence: i64,
        payload: serde_json::Value,
        now: DateTime<Utc>,
    ) -> Result<(), mongodb::error::Error> {
        let expires_at = now
            + chrono::Duration::from_std(self.notification_ttl)
                .expect("validated MCP notification TTL fits chrono duration");
        db.collection::<McpSessionNotification>(MCP_NOTIFICATION_COLLECTION)
            .insert_one(McpSessionNotification {
                id: format!("{session_id}:{sequence}"),
                session_id: session_id.to_string(),
                sequence,
                payload,
                created_at: now,
                expires_at,
            })
            .session(transaction)
            .await?;
        Ok(())
    }

    async fn ensure_recovered_slot(
        &self,
        record: &McpSessionRecord,
    ) -> Result<bool, mongodb::error::Error> {
        let Some(db) = &self.db else {
            return Ok(true);
        };
        let slots = db.collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION);
        if slots
            .find_one(doc! { "session_id": &record.id })
            .await?
            .is_some()
        {
            slots
                .update_one(
                    doc! { "session_id": &record.id },
                    doc! { "$set": {
                        "expires_at": bson::DateTime::from_chrono(record.expires_at),
                    }},
                )
                .await?;
            return Ok(true);
        }

        let now = bson::DateTime::from_chrono(Utc::now());
        for slot in 0..MAX_PER_USER_SESSIONS {
            let slot_id = format!("{}:{slot}", record.user_id);
            let result = slots
                .find_one_and_update(
                    doc! {
                        "_id": &slot_id,
                        "$or": [
                            { "expires_at": { "$lte": now } },
                            { "session_id": &record.id },
                        ],
                    },
                    doc! { "$set": {
                        "user_id": &record.user_id,
                        "session_id": &record.id,
                        "slot": slot as i32,
                        "expires_at": bson::DateTime::from_chrono(record.expires_at),
                    }},
                )
                .upsert(true)
                .return_document(ReturnDocument::After)
                .await;
            match result {
                Ok(Some(_)) => return Ok(true),
                Ok(None) => continue,
                Err(error) if is_duplicate_key_error(&error) => {
                    if slots
                        .find_one(doc! { "session_id": &record.id })
                        .await?
                        .is_some()
                    {
                        return Ok(true);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(false)
    }

    fn create_in_memory(&self, user_id: &str, proxy_authorized: bool) -> Option<String> {
        let mut sessions = self
            .sessions
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if sessions
            .values()
            .filter(|session| session.user_id == user_id)
            .count()
            >= MAX_PER_USER_SESSIONS
        {
            return None;
        }
        let now = Utc::now();
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::channel(32);
        sessions.insert(
            id.clone(),
            McpSession {
                user_id: user_id.to_string(),
                created_at: now,
                last_active: now,
                expires_at: now + chrono::Duration::seconds(MCP_SESSION_MAX_IDLE_SECS as i64),
                activated_service_ids: HashSet::new(),
                proxy_authorized,
                notification_sequence: 0,
                notification_tx: Some(tx),
            },
        );
        drop(sessions);
        self.pending_receivers
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.clone(), rx);
        Some(id)
    }

    fn hydrate(&self, record: McpSessionRecord) {
        let mut sessions = self
            .sessions
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(session) = sessions.get_mut(&record.id) {
            session.user_id = record.user_id;
            session.created_at = record.created_at;
            session.last_active = record.last_active_at;
            session.expires_at = record.expires_at;
            session.activated_service_ids = record.activated_service_ids.into_iter().collect();
            session.proxy_authorized = record.proxy_authorized;
            session.notification_sequence = record.notification_sequence;
            return;
        }
        let (tx, rx) = mpsc::channel(32);
        let id = record.id.clone();
        sessions.insert(
            id.clone(),
            McpSession {
                user_id: record.user_id,
                created_at: record.created_at,
                last_active: record.last_active_at,
                expires_at: record.expires_at,
                activated_service_ids: record.activated_service_ids.into_iter().collect(),
                proxy_authorized: record.proxy_authorized,
                notification_sequence: record.notification_sequence,
                notification_tx: Some(tx),
            },
        );
        drop(sessions);
        self.pending_receivers
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .entry(id)
            .or_insert(rx);
    }

    fn local_record(&self, session_id: &str) -> Option<McpSessionRecord> {
        self.sessions
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(session_id)
            .map(|session| McpSessionRecord {
                id: session_id.to_string(),
                user_id: session.user_id.clone(),
                client_info: None,
                activated_service_ids: session.activated_service_ids.iter().cloned().collect(),
                proxy_authorized: session.proxy_authorized,
                notification_sequence: session.notification_sequence,
                created_at: session.created_at,
                last_active_at: session.last_active,
                expires_at: session.expires_at,
            })
    }

    fn evict(&self, session_id: &str) {
        self.sessions
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(session_id);
        self.pending_receivers
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(session_id);
    }

    fn evict_user(&self, user_id: &str) {
        let ids: Vec<String> = self
            .sessions
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|(_, session)| session.user_id == user_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.evict(&id);
        }
    }
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    error.to_string().contains("E11000") || error.to_string().contains("duplicate key")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(user_id: &str) -> McpSessionRecord {
        let now = Utc::now();
        McpSessionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            client_info: None,
            activated_service_ids: Vec::new(),
            proxy_authorized: true,
            notification_sequence: 0,
            created_at: now,
            last_active_at: now,
            expires_at: now + chrono::Duration::hours(1),
        }
    }

    #[test]
    fn collection_names_are_stable() {
        assert_eq!(MCP_SESSION_COLLECTION, "mcp_sessions");
        assert_eq!(MCP_SESSION_SLOT_COLLECTION, "mcp_session_slots");
        assert_eq!(MCP_NOTIFICATION_COLLECTION, "mcp_session_notifications");
    }

    #[tokio::test]
    async fn in_memory_session_lifecycle() {
        let store = McpSessionStore::new();
        let id = store.create("user-1").await.unwrap().expect("create");
        assert!(store.validate(&id, "user-1").await.unwrap());
        assert!(store.touch(&id).await.unwrap());
        store.remove(&id).await.unwrap();
        assert!(!store.validate(&id, "user-1").await.unwrap());
    }

    #[tokio::test]
    async fn session_identity_and_proxy_authorization_are_independent() {
        let store = McpSessionStore::new();
        let ordinary = store.create("user-1").await.unwrap().expect("create");
        let proxy = store
            .create_with_proxy_access("user-2", true)
            .await
            .unwrap()
            .expect("create proxy session");

        assert_eq!(
            store.get_user_id(&ordinary).await.unwrap().as_deref(),
            Some("user-1")
        );
        assert!(!store.allows_proxy_access(&ordinary).await.unwrap());
        assert!(store.allows_proxy_access(&proxy).await.unwrap());
        assert!(!store.validate(&ordinary, "user-2").await.unwrap());
        assert!(store.get_user_id("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn in_memory_limit_and_activation_limit_are_preserved() {
        let store = McpSessionStore::new();
        let first = store.create("user-1").await.unwrap().expect("create");
        for _ in 1..MAX_PER_USER_SESSIONS {
            assert!(store.create("user-1").await.unwrap().is_some());
        }
        assert!(store.create("user-1").await.unwrap().is_none());

        let service_ids: Vec<String> = (0..MAX_ACTIVATED_SERVICES + 1)
            .map(|index| format!("service-{index}"))
            .collect();
        assert!(store.activate_services(&first, &service_ids).await.unwrap());
        assert_eq!(
            store.get_activated_service_ids(&first).await.unwrap().len(),
            MAX_ACTIVATED_SERVICES
        );
        assert!(
            !store
                .activate_services(&first, &["overflow".to_string()])
                .await
                .unwrap()
        );
        assert!(
            !store
                .activate_services("missing", &["service".to_string()])
                .await
                .unwrap()
        );
        assert!(
            store
                .get_activated_service_ids("missing")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn duplicate_activation_is_not_reported_as_a_change() {
        let store = McpSessionStore::new();
        let id = store.create("user-1").await.unwrap().expect("create");
        let services = ["service-1".to_string()];
        assert!(store.activate_services(&id, &services).await.unwrap());
        assert!(!store.activate_services(&id, &services).await.unwrap());
    }

    #[tokio::test]
    async fn local_notification_channel_supports_reconnect() {
        let store = McpSessionStore::new();
        let id = store.create("user-1").await.unwrap().expect("create");
        let mut first_receiver = store
            .take_notification_rx(&id)
            .expect("initial notification receiver");
        assert!(
            store
                .send_notification(&id, serde_json::json!({"value": 1}))
                .await
                .unwrap()
        );
        assert_eq!(first_receiver.recv().await.unwrap()["value"], 1);

        let (sender, mut replacement_receiver) = mpsc::channel(1);
        store.set_notification_tx(&id, sender);
        assert!(
            store
                .send_notification(&id, serde_json::json!({"value": 2}))
                .await
                .unwrap()
        );
        assert_eq!(replacement_receiver.recv().await.unwrap()["value"], 2);
    }

    #[tokio::test]
    async fn remove_by_user_only_removes_matching_local_sessions() {
        let store = McpSessionStore::new();
        let first = store.create("user-1").await.unwrap().expect("create");
        let second = store.create("user-2").await.unwrap().expect("create");
        store.remove_by_user_id("user-1").await.unwrap();

        assert!(!store.validate(&first, "user-1").await.unwrap());
        assert!(store.validate(&second, "user-2").await.unwrap());
    }

    #[tokio::test]
    async fn reaper_removes_idle_local_sessions_and_receivers() {
        let store = McpSessionStore::new();
        let id = store.create("user-1").await.unwrap().expect("create");
        store
            .sessions
            .write()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .last_active = Utc::now() - chrono::Duration::hours(2);

        assert_eq!(
            store.reap_expired(Duration::from_secs(3600)).await.unwrap(),
            1
        );
        assert!(!store.validate(&id, "user-1").await.unwrap());
        assert!(store.take_notification_rx(&id).is_none());
    }

    #[tokio::test]
    async fn load_without_database_is_empty() {
        assert_eq!(McpSessionStore::new().load_from_db().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn validation_reads_through_on_a_replica_local_miss() {
        let Some(db) = crate::test_utils::connect_test_database("mcp_read_through").await else {
            return;
        };
        let record = record(&uuid::Uuid::new_v4().to_string());
        db.collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .insert_one(&record)
            .await
            .expect("insert remote session");

        let other_replica = McpSessionStore::with_db(db);
        assert!(
            other_replica
                .validate(&record.id, &record.user_id)
                .await
                .unwrap()
        );
        assert_eq!(
            other_replica.get_user_id(&record.id).await.unwrap(),
            Some(record.user_id)
        );
    }

    #[tokio::test]
    async fn validation_rejects_a_cached_session_deleted_by_another_replica() {
        let Some(db) = crate::test_utils::connect_test_database("mcp_stale_positive").await else {
            return;
        };
        let record = record(&uuid::Uuid::new_v4().to_string());
        db.collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .insert_one(&record)
            .await
            .expect("insert session");
        let cached_replica = McpSessionStore::with_db(db.clone());
        assert!(
            cached_replica
                .validate(&record.id, &record.user_id)
                .await
                .unwrap()
        );

        db.collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .delete_one(doc! { "_id": &record.id })
            .await
            .expect("delete from another replica");

        assert!(
            !cached_replica
                .validate(&record.id, &record.user_id)
                .await
                .unwrap()
        );
        assert!(
            cached_replica
                .get_user_id(&record.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn startup_recovery_hydrates_valid_sessions_and_backfills_admission_slots() {
        let Some(db) = crate::test_utils::connect_test_database("mcp_recovery_slots").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let mut persisted = record(&user_id);
        persisted.activated_service_ids = vec!["service-1".to_string()];
        db.collection::<mongodb::bson::Document>("sessions")
            .insert_one(doc! {
                "_id": uuid::Uuid::new_v4().to_string(),
                "user_id": &user_id,
                "revoked": false,
                "expires_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(1)),
            })
            .await
            .expect("insert auth session");
        db.collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .insert_one(&persisted)
            .await
            .expect("insert MCP session");

        let store = McpSessionStore::with_db(db.clone());
        assert_eq!(store.load_from_db().await.unwrap(), 1);
        assert!(store.validate(&persisted.id, &user_id).await.unwrap());
        assert!(
            store
                .get_activated_service_ids(&persisted.id)
                .await
                .unwrap()
                .contains("service-1")
        );
        assert!(
            db.collection::<McpSessionSlot>(MCP_SESSION_SLOT_COLLECTION)
                .find_one(doc! { "session_id": &persisted.id })
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn cross_replica_notifications_are_durable_and_strictly_ordered() {
        let db =
            crate::test_utils::connect_transaction_test_database("mcp_notification_outbox").await;
        let owner = McpSessionStore::with_db_options(
            db.clone(),
            Duration::from_secs(3600),
            Duration::from_millis(10),
        );
        let producer = McpSessionStore::with_db_options(
            db,
            Duration::from_secs(3600),
            Duration::from_millis(10),
        );
        let id = owner
            .create_with_proxy_access("user-1", true)
            .await
            .unwrap()
            .expect("create");

        assert!(
            producer
                .activate_services_and_notify(
                    &id,
                    &["service-1".to_string()],
                    serde_json::json!({"value": 1}),
                )
                .await
                .unwrap()
        );
        assert!(
            producer
                .activate_services_and_notify(
                    &id,
                    &["service-2".to_string()],
                    serde_json::json!({"value": 2}),
                )
                .await
                .unwrap()
        );
        let notifications = owner
            .notifications_after(&id, 0)
            .await
            .unwrap()
            .expect("active session");

        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].sequence, 1);
        assert_eq!(notifications[0].payload["value"], 1);
        assert_eq!(notifications[1].sequence, 2);
        assert_eq!(notifications[1].payload["value"], 2);
    }

    #[tokio::test]
    async fn deletion_removes_durable_notifications_and_stops_polling() {
        let db =
            crate::test_utils::connect_transaction_test_database("mcp_notification_delete").await;
        let store = McpSessionStore::with_db(db);
        let id = store.create("user-1").await.unwrap().expect("create");
        store
            .send_notification(&id, serde_json::json!({"value": 1}))
            .await
            .unwrap();
        store.remove(&id).await.unwrap();

        assert!(store.notifications_after(&id, 0).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn mongo_admission_limit_is_shared_across_replicas() {
        let db = crate::test_utils::connect_transaction_test_database("mcp_global_limit").await;
        let first = McpSessionStore::with_db(db.clone());
        let second = McpSessionStore::with_db(db);
        for index in 0..MAX_PER_USER_SESSIONS {
            let store = if index % 2 == 0 { &first } else { &second };
            assert!(store.create("user-1").await.unwrap().is_some());
        }
        assert!(second.create("user-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn mongo_activation_overflow_preserves_existing_services() {
        let db = crate::test_utils::connect_transaction_test_database("mcp_activation_limit").await;
        let store = McpSessionStore::with_db(db);
        let id = store.create("user-1").await.unwrap().expect("create");
        let service_ids: Vec<String> = (0..MAX_ACTIVATED_SERVICES)
            .map(|index| format!("service-{index}"))
            .collect();

        assert!(store.activate_services(&id, &service_ids).await.unwrap());
        let before = store.get_activated_service_ids(&id).await.unwrap();
        assert!(
            !store
                .activate_services(&id, &["overflow".to_string()])
                .await
                .unwrap()
        );
        assert_eq!(store.get_activated_service_ids(&id).await.unwrap(), before);
    }

    #[test]
    fn bson_roundtrip_session_record_includes_notification_sequence() {
        let mut record = record("user-1");
        record.notification_sequence = 7;
        let document = bson::to_document(&record).expect("serialize");
        let restored: McpSessionRecord = bson::from_document(document).expect("deserialize");
        assert_eq!(restored.notification_sequence, 7);
    }
}
