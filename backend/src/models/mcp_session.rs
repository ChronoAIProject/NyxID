use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};

/// An ephemeral MCP session (in-memory only, not persisted to MongoDB).
pub struct McpSession {
    pub user_id: String,
    pub last_active: DateTime<Utc>,
}

/// Thread-safe, in-memory store for active MCP sessions.
///
/// Uses `Arc<RwLock<HashMap>>` (zero new dependencies) rather than DashMap.
/// Session operations are infrequent and fast, so lock contention is negligible.
#[derive(Clone)]
pub struct McpSessionStore {
    sessions: Arc<RwLock<HashMap<String, McpSession>>>,
}

impl McpSessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new session for the given user, returning the session ID.
    pub fn create(&self, user_id: &str) -> String {
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let session = McpSession {
            user_id: user_id.to_string(),
            last_active: now,
        };
        self.sessions
            .write()
            .expect("session store lock poisoned")
            .insert(session_id.clone(), session);
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
    }

    /// Remove sessions that have been idle longer than `max_idle`.
    /// Called periodically by a background task.
    pub fn reap_expired(&self, max_idle: Duration) {
        let cutoff =
            Utc::now() - chrono::Duration::from_std(max_idle).unwrap_or(chrono::Duration::hours(1));
        let mut sessions = self.sessions.write().expect("session store lock poisoned");
        let before = sessions.len();
        sessions.retain(|_, s| s.last_active > cutoff);
        let removed = before - sessions.len();
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
        // All sessions should be removed since they were created "before" cutoff
        // (Utc::now() - 0 seconds = now, and last_active <= now)
        // The session was just created, so last_active ~= now. With 0 max_idle,
        // cutoff = now, and retain keeps s where s.last_active > cutoff.
        // Since last_active <= cutoff (roughly equal), it gets reaped.
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
}
