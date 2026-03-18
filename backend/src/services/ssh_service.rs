use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use mongodb::bson::doc;

use crate::errors::{AppError, AppResult};
use crate::models::ssh_service::{COLLECTION_NAME as SSH_SERVICES, SshService};

#[derive(Debug)]
pub struct SshSessionManager {
    concurrent_by_user: Arc<DashMap<String, usize>>,
    max_sessions_per_user: usize,
}

impl SshSessionManager {
    pub fn new(max_sessions_per_user: usize) -> Self {
        Self {
            concurrent_by_user: Arc::new(DashMap::new()),
            max_sessions_per_user,
        }
    }

    pub fn try_acquire(&self, user_id: &str) -> AppResult<SshSessionGuard> {
        let mut entry = self
            .concurrent_by_user
            .entry(user_id.to_string())
            .or_insert(0);
        if *entry >= self.max_sessions_per_user {
            return Err(AppError::RateLimited);
        }

        *entry += 1;
        drop(entry);

        Ok(SshSessionGuard {
            manager: self.concurrent_by_user.clone(),
            user_id: user_id.to_string(),
        })
    }

    pub fn release(&self, user_id: &str) {
        if let Some(mut entry) = self.concurrent_by_user.get_mut(user_id) {
            if *entry > 1 {
                *entry -= 1;
            } else {
                drop(entry);
                self.concurrent_by_user.remove(user_id);
            }
        }
    }

    pub fn active_sessions_for_user(&self, user_id: &str) -> usize {
        self.concurrent_by_user
            .get(user_id)
            .map(|entry| *entry)
            .unwrap_or(0)
    }

    pub fn max_sessions_per_user(&self) -> usize {
        self.max_sessions_per_user
    }
}

pub struct SshSessionGuard {
    manager: Arc<DashMap<String, usize>>,
    user_id: String,
}

impl Drop for SshSessionGuard {
    fn drop(&mut self) {
        if let Some(mut entry) = self.manager.get_mut(&self.user_id) {
            if *entry > 1 {
                *entry -= 1;
            } else {
                drop(entry);
                self.manager.remove(&self.user_id);
            }
        }
    }
}

pub async fn get_ssh_service(db: &mongodb::Database, service_id: &str) -> AppResult<SshService> {
    db.collection::<SshService>(SSH_SERVICES)
        .find_one(doc! { "_id": service_id, "enabled": true })
        .await?
        .ok_or_else(|| AppError::NotFound("SSH service not found".to_string()))
}

pub async fn get_ssh_service_optional(
    db: &mongodb::Database,
    service_id: &str,
) -> AppResult<Option<SshService>> {
    db.collection::<SshService>(SSH_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await
        .map_err(Into::into)
}

pub async fn upsert_ssh_service(
    db: &mongodb::Database,
    service_id: &str,
    host: &str,
    port: u16,
    created_by: &str,
) -> AppResult<SshService> {
    let now = Utc::now();
    let existing = get_ssh_service_optional(db, service_id).await?;

    let service = match existing {
        Some(existing) => SshService {
            id: existing.id,
            host: host.to_string(),
            port,
            enabled: true,
            created_by: existing.created_by,
            created_at: existing.created_at,
            updated_at: now,
        },
        None => SshService {
            id: service_id.to_string(),
            host: host.to_string(),
            port,
            enabled: true,
            created_by: created_by.to_string(),
            created_at: now,
            updated_at: now,
        },
    };

    db.collection::<SshService>(SSH_SERVICES)
        .replace_one(doc! { "_id": service_id }, &service)
        .upsert(true)
        .await?;

    Ok(service)
}

pub async fn disable_ssh_service(db: &mongodb::Database, service_id: &str) -> AppResult<()> {
    db.collection::<SshService>(SSH_SERVICES)
        .update_one(
            doc! { "_id": service_id },
            doc! {
                "$set": {
                    "enabled": false,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;
    Ok(())
}

pub fn validate_ssh_target(host: &str, port: u16) -> AppResult<()> {
    let trimmed = host.trim();
    if trimmed.is_empty() || trimmed.len() > 255 {
        return Err(AppError::ValidationError(
            "host must be between 1 and 255 characters".to_string(),
        ));
    }

    if port == 0 {
        return Err(AppError::ValidationError(
            "port must be greater than 0".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SshSessionManager, validate_ssh_target};

    #[test]
    fn validates_ssh_target() {
        assert!(validate_ssh_target("ssh.internal", 22).is_ok());
        assert!(validate_ssh_target("", 22).is_err());
    }

    #[test]
    fn enforces_concurrent_session_limit() {
        let manager = SshSessionManager::new(1);
        let guard = manager.try_acquire("user-1").expect("first acquire");
        assert_eq!(manager.active_sessions_for_user("user-1"), 1);
        assert!(manager.try_acquire("user-1").is_err());
        drop(guard);
    }
}
