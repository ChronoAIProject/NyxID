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
