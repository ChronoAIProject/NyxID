use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Maximum number of services that can be activated per session.
/// Prevents unbounded memory growth.
pub const MAX_ACTIVATED_SERVICES: usize = 20;

/// Maximum idle time for MCP sessions (30 days).
/// Sessions are extended on every request via `touch()`, so active users
/// never need to re-authenticate.
pub const MCP_SESSION_MAX_IDLE_SECS: u64 = 30 * 24 * 3600;

/// MongoDB collection name for persisted MCP sessions.
pub const MCP_SESSION_COLLECTION: &str = "mcp_sessions";

/// Minimum interval between touch() writes to MongoDB (5 minutes).
const TOUCH_DEBOUNCE_SECS: u64 = 300;

/// Maximum number of concurrent MCP sessions per user.
const MAX_PER_USER_SESSIONS: usize = 50;

/// MongoDB-persisted MCP session record.
/// The in-memory `McpSession` holds runtime state (channels, activated services),
/// while this record holds the durable session identity for cross-restart recovery.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpSessionRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub client_info: Option<String>,
    #[serde(default)]
    pub activated_service_ids: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_active_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

/// An in-memory MCP session with runtime state.
pub struct McpSession {
    pub user_id: String,
    pub last_active: DateTime<Utc>,
    /// Service IDs whose tools are currently exposed in tools/list.
    pub activated_service_ids: HashSet<String>,
    /// Channel to send JSON-RPC notifications to the SSE stream.
    /// None if no SSE listener is connected.
    pub notification_tx: Option<mpsc::Sender<serde_json::Value>>,
}

/// Thread-safe, hybrid in-memory + MongoDB store for active MCP sessions.
///
/// In-memory state holds runtime data (notification channels, activated services).
/// MongoDB provides durability so sessions survive server restarts.
/// All DB writes are fire-and-forget (spawned tasks) to keep sync APIs fast.
#[derive(Clone)]
pub struct McpSessionStore {
    sessions: Arc<RwLock<HashMap<String, McpSession>>>,
    /// Pending notification receivers, waiting for SSE connection.
    pending_receivers: Arc<RwLock<HashMap<String, mpsc::Receiver<serde_json::Value>>>>,
    /// Optional MongoDB handle for persistence. None in tests.
    db: Option<mongodb::Database>,
    /// Tracks when each session was last persisted to MongoDB (for touch debouncing).
    last_persisted: Arc<RwLock<HashMap<String, Instant>>>,
}

impl Default for McpSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl McpSessionStore {
    /// Create a store without MongoDB persistence (for tests).
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            pending_receivers: Arc::new(RwLock::new(HashMap::new())),
            db: None,
            last_persisted: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a store with MongoDB persistence.
    pub fn with_db(db: mongodb::Database) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            pending_receivers: Arc::new(RwLock::new(HashMap::new())),
            db: Some(db),
            last_persisted: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load non-expired sessions from MongoDB into memory.
    /// Called once at startup to recover sessions across restarts.
    ///
    /// Filters out MCP sessions for users whose auth sessions have all been
    /// revoked (e.g., due to refresh token reuse detection). This handles the
    /// edge case where the fire-and-forget MongoDB deletion in
    /// `remove_by_user_id` didn't complete before a server restart.
    ///
    /// Returns the number of sessions recovered.
    pub async fn load_from_db(&self) -> Result<usize, mongodb::error::Error> {
        let db = match &self.db {
            Some(db) => db,
            None => return Ok(0),
        };

        let now = bson::DateTime::from_chrono(Utc::now());
        let cursor = db
            .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
            .find(doc! { "expires_at": { "$gt": now } })
            .await?;

        let records: Vec<McpSessionRecord> = cursor.try_collect().await?;

        if records.is_empty() {
            return Ok(0);
        }

        // Collect unique user IDs and check which users still have at least
        // one active (non-revoked, non-expired) session.
        let unique_user_ids: HashSet<&str> = records.iter().map(|r| r.user_id.as_str()).collect();

        let bson_now = bson::DateTime::from_chrono(Utc::now());
        let users_with_sessions_cursor = db
            .collection::<mongodb::bson::Document>("sessions")
            .find(doc! {
                "user_id": { "$in": unique_user_ids.iter().copied().collect::<Vec<_>>() },
                "revoked": false,
                "expires_at": { "$gt": bson_now },
            })
            .await?;

        let active_session_docs: Vec<mongodb::bson::Document> =
            users_with_sessions_cursor.try_collect().await?;

        let users_with_active_sessions: HashSet<String> = active_session_docs
            .iter()
            .filter_map(|d| d.get_str("user_id").ok().map(String::from))
            .collect();

        // Filter: only recover MCP sessions for users who still have a valid
        // auth session. Orphaned records are cleaned up below.
        let mut orphaned_ids: Vec<String> = Vec::new();

        let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        let mut receivers = self
            .pending_receivers
            .write()
            .unwrap_or_else(|e| e.into_inner());

        for record in &records {
            if !users_with_active_sessions.contains(&record.user_id) {
                orphaned_ids.push(record.id.clone());
                continue;
            }

            let (tx, rx) = mpsc::channel(32);
            let activated: HashSet<String> = record.activated_service_ids.iter().cloned().collect();
            sessions.insert(
                record.id.clone(),
                McpSession {
                    user_id: record.user_id.clone(),
                    last_active: record.last_active_at,
                    activated_service_ids: activated,
                    notification_tx: Some(tx),
                },
            );
            receivers.insert(record.id.clone(), rx);
        }

        let count = sessions.len();

        drop(sessions);
        drop(receivers);

        // Clean up orphaned MCP session records from MongoDB
        if !orphaned_ids.is_empty() {
            tracing::info!(
                orphaned = orphaned_ids.len(),
                "Cleaning up orphaned MCP sessions (users with no active auth sessions)"
            );
            let db = db.clone();
            tokio::spawn(async move {
                if let Err(e) = db
                    .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                    .delete_many(doc! { "_id": { "$in": &orphaned_ids } })
                    .await
                {
                    tracing::warn!("Failed to delete orphaned MCP sessions: {e}");
                }
            });
        }

        Ok(count)
    }

    /// Create a new session for the given user, returning the session ID.
    /// Returns `None` if the per-user session limit has been reached.
    /// Internally creates a notification channel; the rx end is stored in
    /// `pending_receivers` for the SSE handler to take.
    /// Also persists to MongoDB (fire-and-forget).
    pub fn create(&self, user_id: &str) -> Option<String> {
        let (tx, rx) = mpsc::channel(32);
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let session = McpSession {
            user_id: user_id.to_string(),
            last_active: now,
            activated_service_ids: HashSet::new(),
            notification_tx: Some(tx),
        };

        {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            // Enforce per-user session limit
            let user_count = sessions.values().filter(|s| s.user_id == user_id).count();
            if user_count >= MAX_PER_USER_SESSIONS {
                return None;
            }
            sessions.insert(session_id.clone(), session);
        }

        self.pending_receivers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session_id.clone(), rx);

        // Persist to MongoDB
        if let Some(db) = &self.db {
            let record = McpSessionRecord {
                id: session_id.clone(),
                user_id: user_id.to_string(),
                client_info: None,
                activated_service_ids: Vec::new(),
                created_at: now,
                last_active_at: now,
                expires_at: now + chrono::Duration::seconds(MCP_SESSION_MAX_IDLE_SECS as i64),
            };
            let db = db.clone();
            tokio::spawn(async move {
                if let Err(e) = db
                    .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                    .insert_one(&record)
                    .await
                {
                    tracing::warn!("Failed to persist MCP session to MongoDB: {e}");
                }
            });
        }

        Some(session_id)
    }

    /// Check that a session exists and belongs to the given user.
    pub fn validate(&self, session_id: &str, user_id: &str) -> bool {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .is_some_and(|s| s.user_id == user_id)
    }

    /// Get the user_id for an existing session, or `None` if it doesn't exist.
    /// Used for session-based auth fallback when JWT has expired.
    pub fn get_user_id(&self, session_id: &str) -> Option<String> {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .map(|s| s.user_id.clone())
    }

    /// Update the `last_active` timestamp to prevent expiry.
    /// MongoDB writes are debounced: only persists if >5 minutes since last DB write.
    pub fn touch(&self, session_id: &str) {
        let now = Utc::now();
        if let Some(session) = self
            .sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(session_id)
        {
            session.last_active = now;
        } else {
            return;
        }

        // Debounce MongoDB write
        if let Some(db) = &self.db {
            let should_persist = {
                let last = self
                    .last_persisted
                    .read()
                    .unwrap_or_else(|e| e.into_inner());
                match last.get(session_id) {
                    Some(instant) => instant.elapsed() > Duration::from_secs(TOUCH_DEBOUNCE_SECS),
                    None => true,
                }
            };

            if should_persist {
                self.last_persisted
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(session_id.to_string(), Instant::now());

                let db = db.clone();
                let sid = session_id.to_string();
                let new_expires = now + chrono::Duration::seconds(MCP_SESSION_MAX_IDLE_SECS as i64);
                tokio::spawn(async move {
                    if let Err(e) = db
                        .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                        .update_one(
                            doc! { "_id": &sid },
                            doc! { "$set": {
                                "last_active_at": bson::DateTime::from_chrono(now),
                                "expires_at": bson::DateTime::from_chrono(new_expires),
                            }},
                        )
                        .await
                    {
                        tracing::warn!("Failed to persist MCP session touch to MongoDB: {e}");
                    }
                });
            }
        }
    }

    /// Remove a session (called on DELETE /mcp).
    /// Also deletes from MongoDB (fire-and-forget).
    pub fn remove(&self, session_id: &str) {
        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
        self.pending_receivers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
        self.last_persisted
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);

        // Delete from MongoDB
        if let Some(db) = &self.db {
            let db = db.clone();
            let sid = session_id.to_string();
            tokio::spawn(async move {
                if let Err(e) = db
                    .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                    .delete_one(doc! { "_id": &sid })
                    .await
                {
                    tracing::warn!("Failed to delete MCP session from MongoDB: {e}");
                }
            });
        }
    }

    /// Remove all sessions for a given user from both memory and MongoDB.
    /// Used for session revocation cascade (e.g., when refresh token reuse is detected).
    pub fn remove_by_user_id(&self, user_id: &str) {
        let removed_ids: Vec<String> = {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            let ids: Vec<String> = sessions
                .iter()
                .filter(|(_, s)| s.user_id == user_id)
                .map(|(id, _)| id.clone())
                .collect();
            for id in &ids {
                sessions.remove(id);
            }
            ids
        };

        if removed_ids.is_empty() {
            return;
        }

        {
            let mut receivers = self
                .pending_receivers
                .write()
                .unwrap_or_else(|e| e.into_inner());
            for id in &removed_ids {
                receivers.remove(id);
            }
        }

        {
            let mut lp = self
                .last_persisted
                .write()
                .unwrap_or_else(|e| e.into_inner());
            for id in &removed_ids {
                lp.remove(id);
            }
        }

        // Delete from MongoDB
        if let Some(db) = &self.db {
            let db = db.clone();
            let uid = user_id.to_string();
            tokio::spawn(async move {
                if let Err(e) = db
                    .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                    .delete_many(doc! { "user_id": &uid })
                    .await
                {
                    tracing::warn!("Failed to delete MCP sessions for user from MongoDB: {e}");
                }
            });
        }
    }

    /// Activate services for a session. Returns true if any were newly activated.
    /// Enforces MAX_ACTIVATED_SERVICES.
    /// Also persists the updated list to MongoDB (fire-and-forget).
    pub fn activate_services(&self, session_id: &str, service_ids: &[String]) -> bool {
        let (changed, activated_list) = {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            let session = match sessions.get_mut(session_id) {
                Some(s) => s,
                None => return false,
            };
            let mut changed = false;
            for id in service_ids {
                if session.activated_service_ids.len() >= MAX_ACTIVATED_SERVICES {
                    break;
                }
                if session.activated_service_ids.insert(id.clone()) {
                    changed = true;
                }
            }
            let list: Vec<String> = session.activated_service_ids.iter().cloned().collect();
            (changed, list)
        };

        // Persist to MongoDB if changed
        if changed
            && let Some(db) = &self.db
        {
            let db = db.clone();
            let sid = session_id.to_string();
            tokio::spawn(async move {
                if let Err(e) = db
                    .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                    .update_one(
                        doc! { "_id": &sid },
                        doc! { "$set": { "activated_service_ids": &activated_list } },
                    )
                    .await
                {
                    tracing::warn!("Failed to persist activated services to MongoDB: {e}");
                }
            });
        }

        changed
    }

    /// Get the set of activated service IDs for a session.
    pub fn get_activated_service_ids(&self, session_id: &str) -> HashSet<String> {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .map(|s| s.activated_service_ids.clone())
            .unwrap_or_default()
    }

    /// Send a JSON-RPC notification to the session's SSE stream.
    /// Returns true if sent successfully, false if no listener or channel full.
    pub fn send_notification(&self, session_id: &str, notification: serde_json::Value) -> bool {
        let sessions = self.sessions.read().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = sessions.get(session_id)
            && let Some(tx) = &session.notification_tx
        {
            return tx.try_send(notification).is_ok();
        }
        false
    }

    /// Take the pending notification receiver for a session.
    /// Returns None if already taken or session doesn't exist.
    pub fn take_notification_rx(
        &self,
        session_id: &str,
    ) -> Option<mpsc::Receiver<serde_json::Value>> {
        self.pending_receivers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id)
    }

    /// Attach a new notification sender (e.g., when SSE reconnects).
    pub fn set_notification_tx(&self, session_id: &str, tx: mpsc::Sender<serde_json::Value>) {
        if let Some(session) = self
            .sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(session_id)
        {
            session.notification_tx = Some(tx);
        }
    }

    /// Remove sessions that have been idle longer than `max_idle`.
    /// Called periodically by a background task.
    /// In-memory cleanup only; MongoDB TTL index handles DB-side expiry.
    pub fn reap_expired(&self, max_idle: Duration) {
        let cutoff =
            Utc::now() - chrono::Duration::from_std(max_idle).unwrap_or(chrono::Duration::hours(1));
        let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());

        // Collect expired session IDs
        let expired_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.last_active <= cutoff)
            .map(|(id, _)| id.clone())
            .collect();

        for id in &expired_ids {
            sessions.remove(id);
        }

        drop(sessions); // Release lock before acquiring pending_receivers lock

        // Also clean up pending receivers for expired sessions
        let mut receivers = self
            .pending_receivers
            .write()
            .unwrap_or_else(|e| e.into_inner());
        for id in &expired_ids {
            receivers.remove(id);
        }

        drop(receivers);

        let removed = expired_ids.len();
        if removed > 0 {
            tracing::info!(removed, "Reaped expired MCP sessions");

            // Clean up last_persisted tracking
            let mut lp = self
                .last_persisted
                .write()
                .unwrap_or_else(|e| e.into_inner());
            for id in &expired_ids {
                lp.remove(id);
            }

            // Delete from MongoDB (fire-and-forget).
            // Note: MongoDB TTL index also removes expired docs, but this
            // keeps in-memory and DB state consistent sooner.
            if let Some(db) = &self.db {
                let db = db.clone();
                let ids = expired_ids;
                tokio::spawn(async move {
                    if let Err(e) = db
                        .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
                        .delete_many(doc! { "_id": { "$in": &ids } })
                        .await
                    {
                        tracing::warn!("Failed to delete reaped MCP sessions from MongoDB: {e}");
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(MCP_SESSION_COLLECTION, "mcp_sessions");
    }

    #[test]
    fn create_and_validate() {
        let store = McpSessionStore::new();
        let session_id = store.create("user-1").expect("create");
        assert!(store.validate(&session_id, "user-1"));
        assert!(!store.validate(&session_id, "user-2"));
    }

    #[test]
    fn validate_nonexistent_returns_false() {
        let store = McpSessionStore::new();
        assert!(!store.validate("nonexistent-id", "user-1"));
    }

    #[test]
    fn remove_session() {
        let store = McpSessionStore::new();
        let session_id = store.create("user-1").expect("create");
        assert!(store.validate(&session_id, "user-1"));
        store.remove(&session_id);
        assert!(!store.validate(&session_id, "user-1"));
    }

    #[test]
    fn remove_cleans_up_pending_receivers() {
        let store = McpSessionStore::new();
        let session_id = store.create("user-1").expect("create");
        // The rx should be in pending_receivers
        assert!(store.take_notification_rx(&session_id).is_some());
        // Put a new rx back for next test
        let (tx, rx) = mpsc::channel(1);
        store.set_notification_tx(&session_id, tx);
        store
            .pending_receivers
            .write()
            .unwrap()
            .insert(session_id.clone(), rx);
        // Now remove should clean up
        store.remove(&session_id);
        assert!(store.take_notification_rx(&session_id).is_none());
    }

    #[test]
    fn touch_does_not_invalidate() {
        let store = McpSessionStore::new();
        let session_id = store.create("user-1").expect("create");
        store.touch(&session_id);
        assert!(store.validate(&session_id, "user-1"));
    }

    #[test]
    fn touch_nonexistent_is_noop() {
        let store = McpSessionStore::new();
        store.touch("nonexistent-id"); // should not panic
    }

    #[test]
    fn remove_nonexistent_is_noop() {
        let store = McpSessionStore::new();
        store.remove("nonexistent-id"); // should not panic
    }

    #[test]
    fn get_user_id_returns_user() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        assert_eq!(store.get_user_id(&sid), Some("user-1".to_string()));
    }

    #[test]
    fn get_user_id_nonexistent_returns_none() {
        let store = McpSessionStore::new();
        assert_eq!(store.get_user_id("no-such-session"), None);
    }

    #[test]
    fn multiple_sessions_independent() {
        let store = McpSessionStore::new();
        let s1 = store.create("user-1").expect("create");
        let s2 = store.create("user-2").expect("create");
        assert!(store.validate(&s1, "user-1"));
        assert!(store.validate(&s2, "user-2"));
        assert!(!store.validate(&s1, "user-2"));
        assert!(!store.validate(&s2, "user-1"));
    }

    #[test]
    fn reap_expired_with_zero_idle() {
        let store = McpSessionStore::new();
        store.create("user-1").expect("create");
        // Reap with 0 duration means everything is expired
        store.reap_expired(Duration::from_secs(0));
    }

    #[test]
    fn reap_expired_keeps_fresh_sessions() {
        let store = McpSessionStore::new();
        let session_id = store.create("user-1").expect("create");
        // Reap with 1 hour idle -- session was just created so it's fresh
        store.reap_expired(Duration::from_secs(3600));
        assert!(store.validate(&session_id, "user-1"));
    }

    #[test]
    fn session_ids_are_unique() {
        let store = McpSessionStore::new();
        let s1 = store.create("user-1").expect("create");
        let s2 = store.create("user-1").expect("create");
        assert_ne!(s1, s2);
    }

    // -- Tests for lazy loading features --

    #[test]
    fn activate_services_returns_true_on_change() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        let changed = store.activate_services(&sid, &["svc-1".to_string(), "svc-2".to_string()]);
        assert!(changed);
        let activated = store.get_activated_service_ids(&sid);
        assert!(activated.contains("svc-1"));
        assert!(activated.contains("svc-2"));
        assert_eq!(activated.len(), 2);
    }

    #[test]
    fn activate_services_returns_false_on_duplicate() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        store.activate_services(&sid, &["svc-1".to_string()]);
        let changed = store.activate_services(&sid, &["svc-1".to_string()]);
        assert!(!changed);
    }

    #[test]
    fn activate_services_enforces_max_limit() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        // Activate MAX services
        let ids: Vec<String> = (0..MAX_ACTIVATED_SERVICES)
            .map(|i| format!("svc-{i}"))
            .collect();
        store.activate_services(&sid, &ids);
        assert_eq!(
            store.get_activated_service_ids(&sid).len(),
            MAX_ACTIVATED_SERVICES
        );
        // Try to add one more -- should not increase
        let changed = store.activate_services(&sid, &["overflow".to_string()]);
        assert!(!changed);
        assert_eq!(
            store.get_activated_service_ids(&sid).len(),
            MAX_ACTIVATED_SERVICES
        );
    }

    #[test]
    fn activate_services_nonexistent_session_returns_false() {
        let store = McpSessionStore::new();
        let changed = store.activate_services("no-such-session", &["svc-1".to_string()]);
        assert!(!changed);
    }

    #[test]
    fn get_activated_service_ids_empty_for_new_session() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        assert!(store.get_activated_service_ids(&sid).is_empty());
    }

    #[test]
    fn get_activated_service_ids_nonexistent_returns_empty() {
        let store = McpSessionStore::new();
        assert!(store.get_activated_service_ids("no-such").is_empty());
    }

    #[tokio::test]
    async fn send_notification_succeeds() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        // Take the rx so the channel is active
        let mut rx = store.take_notification_rx(&sid).unwrap();

        let sent = store.send_notification(
            &sid,
            serde_json::json!({"method": "notifications/tools/list_changed"}),
        );
        assert!(sent);

        let msg = rx.recv().await.unwrap();
        assert_eq!(msg["method"], "notifications/tools/list_changed");
    }

    #[test]
    fn send_notification_returns_false_without_listener() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        // Drop the tx by removing the notification_tx
        {
            let mut sessions = store.sessions.write().unwrap();
            sessions.get_mut(&sid).unwrap().notification_tx = None;
        }
        let sent = store.send_notification(&sid, serde_json::json!({"method": "test"}));
        assert!(!sent);
    }

    #[test]
    fn send_notification_nonexistent_returns_false() {
        let store = McpSessionStore::new();
        let sent = store.send_notification("no-such", serde_json::json!({"method": "test"}));
        assert!(!sent);
    }

    #[test]
    fn take_notification_rx_returns_once() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        assert!(store.take_notification_rx(&sid).is_some());
        assert!(store.take_notification_rx(&sid).is_none());
    }

    #[tokio::test]
    async fn set_notification_tx_replaces_sender() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        // Take original rx (won't receive after replacement)
        let _old_rx = store.take_notification_rx(&sid);

        // Set a new tx
        let (new_tx, mut new_rx) = mpsc::channel(8);
        store.set_notification_tx(&sid, new_tx);

        // Send via the store -- should go to new rx
        let sent = store.send_notification(&sid, serde_json::json!({"method": "reconnect_test"}));
        assert!(sent);

        let msg = new_rx.recv().await.unwrap();
        assert_eq!(msg["method"], "reconnect_test");
    }

    #[test]
    fn reap_expired_cleans_pending_receivers() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1").expect("create");
        // Force last_active to the past
        {
            let mut sessions = store.sessions.write().unwrap();
            sessions.get_mut(&sid).unwrap().last_active = Utc::now() - chrono::Duration::hours(2);
        }
        store.reap_expired(Duration::from_secs(3600)); // 1 hour max idle
        assert!(!store.validate(&sid, "user-1"));
        assert!(store.take_notification_rx(&sid).is_none());
    }

    #[test]
    fn bson_roundtrip_session_record() {
        let record = McpSessionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            client_info: Some("test-client".to_string()),
            activated_service_ids: vec!["svc-1".to_string(), "svc-2".to_string()],
            created_at: Utc::now(),
            last_active_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(30),
        };
        let doc = bson::to_document(&record).expect("serialize");
        let restored: McpSessionRecord = bson::from_document(doc).expect("deserialize");
        assert_eq!(record.id, restored.id);
        assert_eq!(record.user_id, restored.user_id);
        assert_eq!(record.client_info, restored.client_info);
        assert_eq!(record.activated_service_ids, restored.activated_service_ids);
    }

    #[test]
    fn per_user_session_limit() {
        let store = McpSessionStore::new();
        for i in 0..MAX_PER_USER_SESSIONS {
            assert!(
                store.create("user-1").is_some(),
                "should create session {i}"
            );
        }
        // Next create should fail
        assert!(store.create("user-1").is_none(), "should reject over limit");
        // Different user should still work
        assert!(
            store.create("user-2").is_some(),
            "different user should work"
        );
    }

    #[test]
    fn remove_by_user_id_clears_all() {
        let store = McpSessionStore::new();
        let s1 = store.create("user-1").expect("create");
        let s2 = store.create("user-1").expect("create");
        let s3 = store.create("user-2").expect("create");
        store.remove_by_user_id("user-1");
        assert!(!store.validate(&s1, "user-1"));
        assert!(!store.validate(&s2, "user-1"));
        // user-2's session should be unaffected
        assert!(store.validate(&s3, "user-2"));
    }
}
