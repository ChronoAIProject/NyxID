use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

/// Maximum number of services that can be activated per session.
/// Prevents unbounded memory growth.
pub const MAX_ACTIVATED_SERVICES: usize = 20;

/// Maximum idle time for MCP sessions (30 days).
/// Sessions are extended on every request via `touch()`, so active users
/// never need to re-authenticate.
pub const MCP_SESSION_MAX_IDLE_SECS: u64 = 30 * 24 * 3600;

/// An ephemeral MCP session (in-memory only, not persisted to MongoDB).
pub struct McpSession {
    pub user_id: String,
    pub last_active: DateTime<Utc>,
    /// Service IDs whose tools are currently exposed in tools/list.
    pub activated_service_ids: HashSet<String>,
    /// Channel to send JSON-RPC notifications to the SSE stream.
    /// None if no SSE listener is connected.
    pub notification_tx: Option<mpsc::Sender<serde_json::Value>>,
}

/// Thread-safe, in-memory store for active MCP sessions.
///
/// Uses `Arc<RwLock<HashMap>>` (zero new dependencies) rather than DashMap.
/// Session operations are infrequent and fast, so lock contention is negligible.
#[derive(Clone)]
pub struct McpSessionStore {
    sessions: Arc<RwLock<HashMap<String, McpSession>>>,
    /// Pending notification receivers, waiting for SSE connection.
    /// Key: session_id, Value: Receiver
    pending_receivers: Arc<RwLock<HashMap<String, mpsc::Receiver<serde_json::Value>>>>,
}

impl Default for McpSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl McpSessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            pending_receivers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new session for the given user, returning the session ID.
    /// Internally creates a notification channel; the rx end is stored in
    /// `pending_receivers` for the SSE handler to take.
    pub fn create(&self, user_id: &str) -> String {
        let (tx, rx) = mpsc::channel(32);
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = McpSession {
            user_id: user_id.to_string(),
            last_active: Utc::now(),
            activated_service_ids: HashSet::new(),
            notification_tx: Some(tx),
        };
        self.sessions
            .write()
            .expect("session store lock poisoned")
            .insert(session_id.clone(), session);
        self.pending_receivers
            .write()
            .expect("pending_receivers lock poisoned")
            .insert(session_id.clone(), rx);
        session_id
    }

    /// Check that a session exists and belongs to the given user.
    pub fn validate(&self, session_id: &str, user_id: &str) -> bool {
        self.sessions
            .read()
            .expect("session store lock poisoned")
            .get(session_id)
            .is_some_and(|s| s.user_id == user_id)
    }

    /// Get the user_id for an existing session, or `None` if it doesn't exist.
    /// Used for session-based auth fallback when JWT has expired.
    pub fn get_user_id(&self, session_id: &str) -> Option<String> {
        self.sessions
            .read()
            .expect("session store lock poisoned")
            .get(session_id)
            .map(|s| s.user_id.clone())
    }

    /// Update the `last_active` timestamp to prevent expiry.
    pub fn touch(&self, session_id: &str) {
        if let Some(session) = self
            .sessions
            .write()
            .expect("session store lock poisoned")
            .get_mut(session_id)
        {
            session.last_active = Utc::now();
        }
    }

    /// Remove a session (called on DELETE /mcp).
    pub fn remove(&self, session_id: &str) {
        self.sessions
            .write()
            .expect("session store lock poisoned")
            .remove(session_id);
        self.pending_receivers
            .write()
            .expect("pending_receivers lock poisoned")
            .remove(session_id);
    }

    /// Activate services for a session. Returns true if any were newly activated.
    /// Enforces MAX_ACTIVATED_SERVICES.
    pub fn activate_services(&self, session_id: &str, service_ids: &[String]) -> bool {
        let mut sessions = self.sessions.write().expect("session store lock poisoned");
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
        changed
    }

    /// Get the set of activated service IDs for a session.
    pub fn get_activated_service_ids(&self, session_id: &str) -> HashSet<String> {
        self.sessions
            .read()
            .expect("session store lock poisoned")
            .get(session_id)
            .map(|s| s.activated_service_ids.clone())
            .unwrap_or_default()
    }

    /// Send a JSON-RPC notification to the session's SSE stream.
    /// Returns true if sent successfully, false if no listener or channel full.
    pub fn send_notification(
        &self,
        session_id: &str,
        notification: serde_json::Value,
    ) -> bool {
        let sessions = self.sessions.read().expect("session store lock poisoned");
        if let Some(session) = sessions.get(session_id) {
            if let Some(tx) = &session.notification_tx {
                return tx.try_send(notification).is_ok();
            }
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
            .expect("pending_receivers lock poisoned")
            .remove(session_id)
    }

    /// Attach a new notification sender (e.g., when SSE reconnects).
    pub fn set_notification_tx(
        &self,
        session_id: &str,
        tx: mpsc::Sender<serde_json::Value>,
    ) {
        if let Some(session) = self
            .sessions
            .write()
            .expect("session store lock poisoned")
            .get_mut(session_id)
        {
            session.notification_tx = Some(tx);
        }
    }

    /// Remove sessions that have been idle longer than `max_idle`.
    /// Called periodically by a background task.
    pub fn reap_expired(&self, max_idle: Duration) {
        let cutoff =
            Utc::now() - chrono::Duration::from_std(max_idle).unwrap_or(chrono::Duration::hours(1));
        let mut sessions = self.sessions.write().expect("session store lock poisoned");

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
            .expect("pending_receivers lock poisoned");
        for id in &expired_ids {
            receivers.remove(id);
        }

        let removed = expired_ids.len();
        if removed > 0 {
            tracing::info!(removed, "Reaped expired MCP sessions");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_validate() {
        let store = McpSessionStore::new();
        let session_id = store.create("user-1");
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
        let session_id = store.create("user-1");
        assert!(store.validate(&session_id, "user-1"));
        store.remove(&session_id);
        assert!(!store.validate(&session_id, "user-1"));
    }

    #[test]
    fn remove_cleans_up_pending_receivers() {
        let store = McpSessionStore::new();
        let session_id = store.create("user-1");
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
        let session_id = store.create("user-1");
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
        let sid = store.create("user-1");
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
        let s1 = store.create("user-1");
        let s2 = store.create("user-2");
        assert!(store.validate(&s1, "user-1"));
        assert!(store.validate(&s2, "user-2"));
        assert!(!store.validate(&s1, "user-2"));
        assert!(!store.validate(&s2, "user-1"));
    }

    #[test]
    fn reap_expired_with_zero_idle() {
        let store = McpSessionStore::new();
        store.create("user-1");
        // Reap with 0 duration means everything is expired
        store.reap_expired(Duration::from_secs(0));
    }

    #[test]
    fn reap_expired_keeps_fresh_sessions() {
        let store = McpSessionStore::new();
        let session_id = store.create("user-1");
        // Reap with 1 hour idle -- session was just created so it's fresh
        store.reap_expired(Duration::from_secs(3600));
        assert!(store.validate(&session_id, "user-1"));
    }

    #[test]
    fn session_ids_are_unique() {
        let store = McpSessionStore::new();
        let s1 = store.create("user-1");
        let s2 = store.create("user-1");
        assert_ne!(s1, s2);
    }

    // -- New tests for lazy loading features --

    #[test]
    fn activate_services_returns_true_on_change() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1");
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
        let sid = store.create("user-1");
        store.activate_services(&sid, &["svc-1".to_string()]);
        let changed = store.activate_services(&sid, &["svc-1".to_string()]);
        assert!(!changed);
    }

    #[test]
    fn activate_services_enforces_max_limit() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1");
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
        let sid = store.create("user-1");
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
        let sid = store.create("user-1");
        // Take the rx so the channel is active
        let mut rx = store.take_notification_rx(&sid).unwrap();

        let sent = store.send_notification(
            &sid,
            serde_json::json!({"method": "notifications/tools/list_changed"}),
        );
        assert!(sent);

        let msg = rx.recv().await.unwrap();
        assert_eq!(
            msg["method"],
            "notifications/tools/list_changed"
        );
    }

    #[test]
    fn send_notification_returns_false_without_listener() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1");
        // Drop the tx by removing the notification_tx
        {
            let mut sessions = store.sessions.write().unwrap();
            sessions.get_mut(&sid).unwrap().notification_tx = None;
        }
        let sent = store.send_notification(
            &sid,
            serde_json::json!({"method": "test"}),
        );
        assert!(!sent);
    }

    #[test]
    fn send_notification_nonexistent_returns_false() {
        let store = McpSessionStore::new();
        let sent = store.send_notification(
            "no-such",
            serde_json::json!({"method": "test"}),
        );
        assert!(!sent);
    }

    #[test]
    fn take_notification_rx_returns_once() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1");
        assert!(store.take_notification_rx(&sid).is_some());
        assert!(store.take_notification_rx(&sid).is_none());
    }

    #[tokio::test]
    async fn set_notification_tx_replaces_sender() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1");
        // Take original rx (won't receive after replacement)
        let _old_rx = store.take_notification_rx(&sid);

        // Set a new tx
        let (new_tx, mut new_rx) = mpsc::channel(8);
        store.set_notification_tx(&sid, new_tx);

        // Send via the store -- should go to new rx
        let sent = store.send_notification(
            &sid,
            serde_json::json!({"method": "reconnect_test"}),
        );
        assert!(sent);

        let msg = new_rx.recv().await.unwrap();
        assert_eq!(msg["method"], "reconnect_test");
    }

    #[test]
    fn reap_expired_cleans_pending_receivers() {
        let store = McpSessionStore::new();
        let sid = store.create("user-1");
        // Force last_active to the past
        {
            let mut sessions = store.sessions.write().unwrap();
            sessions.get_mut(&sid).unwrap().last_active =
                Utc::now() - chrono::Duration::hours(2);
        }
        store.reap_expired(Duration::from_secs(3600)); // 1 hour max idle
        assert!(!store.validate(&sid, "user-1"));
        assert!(store.take_notification_rx(&sid).is_none());
    }
}
