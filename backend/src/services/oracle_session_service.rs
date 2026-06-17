//! Oracle session reads and lifecycle: list/get a consumer's multi-turn
//! conversations, fetch a transcript (the session's tasks), close a
//! session. Continuation itself happens in `oracle_task_service::submit_task`
//! via `conversation_id`.

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::errors::{AppError, AppResult};
use crate::models::oracle_session::{COLLECTION_NAME as ORACLE_SESSIONS, OracleSession};
use crate::models::oracle_task::{COLLECTION_NAME as ORACLE_TASKS, OracleTask};
use crate::services::oracle_pool_service;

/// Load a session the actor may read: its owner, or the pool's manager.
/// Unauthorized reads 404 (don't leak session existence).
pub async fn get_session_for_consumer(
    db: &mongodb::Database,
    actor_user_id: &str,
    conversation_id: &str,
) -> AppResult<OracleSession> {
    let session = db
        .collection::<OracleSession>(ORACLE_SESSIONS)
        .find_one(doc! { "_id": conversation_id })
        .await?
        .ok_or_else(|| AppError::OracleSessionNotFound(conversation_id.to_string()))?;

    if session.owner_user_id != actor_user_id {
        let pool = oracle_pool_service::get_pool(db, &session.pool_id).await?;
        oracle_pool_service::ensure_can_manage(db, actor_user_id, &pool)
            .await
            .map_err(|_| AppError::OracleSessionNotFound(conversation_id.to_string()))?;
    }
    Ok(session)
}

/// Sessions owned by the actor, newest first, optionally scoped to a pool.
pub async fn list_own_sessions(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool_id: Option<&str>,
    limit: i64,
) -> AppResult<Vec<OracleSession>> {
    let mut filter = doc! { "owner_user_id": actor_user_id };
    if let Some(pool_id) = pool_id {
        filter.insert("pool_id", pool_id);
    }
    let sessions = db
        .collection::<OracleSession>(ORACLE_SESSIONS)
        .find(filter)
        .sort(doc! { "updated_at": -1 })
        .limit(limit.clamp(1, 200))
        .await?
        .try_collect()
        .await?;
    Ok(sessions)
}

/// The session's tasks (turns), oldest first.
pub async fn list_session_tasks(
    db: &mongodb::Database,
    actor_user_id: &str,
    conversation_id: &str,
) -> AppResult<(OracleSession, Vec<OracleTask>)> {
    let session = get_session_for_consumer(db, actor_user_id, conversation_id).await?;
    let tasks = db
        .collection::<OracleTask>(ORACLE_TASKS)
        .find(doc! {
            "conversation_id": conversation_id,
            "kind": { "$ne": "scrape" },
        })
        .sort(doc! { "created_at": 1 })
        .await?
        .try_collect()
        .await?;
    Ok((session, tasks))
}

/// Close a session: further continuations are rejected. Idempotent.
pub async fn close_session(
    db: &mongodb::Database,
    actor_user_id: &str,
    conversation_id: &str,
) -> AppResult<OracleSession> {
    let session = get_session_for_consumer(db, actor_user_id, conversation_id).await?;
    if session.closed_at.is_some() {
        return Ok(session);
    }
    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<OracleSession>(ORACLE_SESSIONS)
        .update_one(
            doc! { "_id": conversation_id },
            doc! { "$set": { "closed_at": now, "updated_at": now } },
        )
        .await?;
    get_session_for_consumer(db, actor_user_id, conversation_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::oracle_pool::{OraclePool, OraclePoolVisibility};
    use crate::services::oracle_task_service::{SubmitTaskInput, SubmitterIdentity, submit_task};
    use crate::test_utils::connect_test_database;
    use chrono::Utc;

    fn test_pool(owner: &str) -> OraclePool {
        let now = Utc::now();
        OraclePool {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: owner.to_string(),
            slug: format!("pool-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            name: "Test Pool".to_string(),
            description: None,
            visibility: OraclePoolVisibility::Platform,
            worker_token_hash: "h".repeat(64),
            chatgpt_project_url: None,
            default_model_label: None,
            allow_extract: false,
            max_workers: 3,
            max_queue_length: 50,
            per_user_max_inflight: 10,
            task_timeout_secs: 3600,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn session_lifecycle_and_acl() {
        let Some(db) = connect_test_database("oracle_session_lifecycle").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let stranger = uuid::Uuid::new_v4().to_string();
        let pool = test_pool(&owner);
        db.collection::<OraclePool>(crate::models::oracle_pool::COLLECTION_NAME)
            .insert_one(&pool)
            .await
            .unwrap();
        let identity = SubmitterIdentity {
            user_id: owner.clone(),
            api_key_id: None,
            api_key_name: None,
        };

        let outcome = submit_task(
            &db,
            &pool,
            &identity,
            SubmitTaskInput {
                prompt: "open".to_string(),
                conversation_id: Some(String::new()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let conv_id = outcome.task.conversation_id.clone().unwrap();

        // Owner reads; stranger gets 404-shaped error.
        let session = get_session_for_consumer(&db, &owner, &conv_id)
            .await
            .unwrap();
        assert_eq!(session.pool_id, pool.id);
        assert!(matches!(
            get_session_for_consumer(&db, &stranger, &conv_id).await,
            Err(AppError::OracleSessionNotFound(_))
        ));

        // Transcript holds the opening turn.
        let (_, tasks) = list_session_tasks(&db, &owner, &conv_id).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].prompt, "open");

        // Listing.
        let sessions = list_own_sessions(&db, &owner, Some(&pool.id), 50)
            .await
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(
            list_own_sessions(&db, &stranger, None, 50)
                .await
                .unwrap()
                .is_empty()
        );

        // Close is idempotent and blocks continuation.
        let closed = close_session(&db, &owner, &conv_id).await.unwrap();
        assert!(closed.closed_at.is_some());
        let closed_again = close_session(&db, &owner, &conv_id).await.unwrap();
        assert_eq!(
            closed.closed_at.map(|t| t.timestamp()),
            closed_again.closed_at.map(|t| t.timestamp())
        );
        let cont = submit_task(
            &db,
            &pool,
            &identity,
            SubmitTaskInput {
                prompt: "continue".to_string(),
                conversation_id: Some(conv_id.clone()),
                ..Default::default()
            },
        )
        .await;
        assert!(matches!(cont, Err(AppError::OracleSessionClosed(_))));

        db.drop().await.ok();
    }
}
