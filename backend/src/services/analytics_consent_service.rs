//! Default-off account analytics permission, checked at admission and dispatch.
use crate::{
    errors::{AppError, AppResult},
    models::user::{AnalyticsConsent, COLLECTION_NAME, User},
};
use mongodb::{
    Database,
    bson::{Document, doc},
    options::{
        CollectionOptions, ReadConcern, ReadPreference, ReturnDocument, SelectionCriteria,
        WriteConcern,
    },
};
use std::future::IntoFuture;

/// Version of the optional, account-linked product analytics permission.
pub const POLICY_VERSION: &str = "1";

fn options() -> CollectionOptions {
    CollectionOptions::builder()
        .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
        .read_concern(ReadConcern::majority())
        .write_concern(WriteConcern::majority())
        .build()
}

fn active_person(user_id: &str) -> Document {
    doc! {"_id": user_id, "is_active": true, "user_type": {"$ne": "org"}, "platform_personal_deleting": {"$ne": true}}
}

pub async fn read(db: &Database, user_id: &str) -> AppResult<AnalyticsConsent> {
    db.collection_with_options::<User>(COLLECTION_NAME, options())
        .find_one(active_person(user_id))
        .await?
        .map(|user| user.profile_config.analytics)
        .ok_or_else(|| AppError::NotFound("Active personal account not found".into()))
}

pub async fn set(db: &Database, user_id: &str, enabled: bool) -> AppResult<AnalyticsConsent> {
    // A random revision invalidates previously admitted work even on same-ms
    // withdraw/regrant or wall-clock rollback. Majority acknowledgement prevents
    // an acknowledged withdrawal being rolled back on ordinary primary failover.
    let revision = uuid::Uuid::new_v4().to_string();
    let user = db
        .collection_with_options::<User>(COLLECTION_NAME, options())
        .find_one_and_update(
            active_person(user_id),
            vec![doc! {"$set": {"profile_config.analytics": {
                "enabled": enabled, "changed_at": "$$NOW", "revision": &revision,
                "policy_version": POLICY_VERSION
            }}}],
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::NotFound("Active personal account not found".into()))?;
    Ok(user.profile_config.analytics)
}

/// Never cache positive permission. The caller saves this revision in the
/// bounded queue at admission; queued jobs never acquire a replacement revision.
pub async fn permitted_revision(db: &Database, user_id: &str) -> Option<String> {
    if uuid::Uuid::parse_str(user_id).is_err() {
        return None;
    }
    let mut filter = active_person(user_id);
    filter.insert("profile_config.analytics.enabled", true);
    filter.insert(
        "profile_config.analytics.revision",
        doc! {"$type": "string", "$ne": ""},
    );
    // Unlike a majority snapshot, this indexed single-user read must observe
    // majority-acknowledged choices preceding it even across primary changes.
    let mut gate_options = options();
    gate_options.read_concern = Some(ReadConcern::linearizable());
    let row = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        db.collection_with_options::<Document>(COLLECTION_NAME, gate_options)
            .find_one(filter)
            .max_time(std::time::Duration::from_millis(200))
            .projection(doc! {"profile_config.analytics.revision": 1})
            .into_future(),
    )
    .await
    .ok()?
    .ok()??;
    Some(
        row.get_document("profile_config")
            .ok()?
            .get_document("analytics")
            .ok()?
            .get_str("revision")
            .ok()?
            .to_owned(),
    )
}

pub async fn permits_dispatch(db: &Database, user_id: &str, revision: &str) -> bool {
    permitted_revision(db, user_id).await.as_deref() == Some(revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserType;
    use crate::test_utils::{connect_test_database, test_user};
    use mongodb::bson;

    async fn account() -> (Database, String) {
        let db = connect_test_database("analytics_consent")
            .await
            .expect("isolated MongoDB required");
        let id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(COLLECTION_NAME)
            .insert_one(test_user(&id, UserType::Person))
            .await
            .unwrap();
        (db, id)
    }

    #[tokio::test]
    async fn choices_persist_policy_version_and_database_change_time() {
        let (db, id) = account().await;
        // A legacy row must not invent consent or a policy acknowledgement.
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &id},
                doc! {"$unset": {"profile_config.analytics": ""}},
            )
            .await
            .unwrap();
        let absent = read(&db, &id).await.unwrap();
        assert!(!absent.enabled);
        assert!(absent.changed_at.is_none());
        assert!(absent.policy_version.is_none());
        assert!(absent.revision.is_empty());
        for enabled in [true, false] {
            let before = db.run_command(doc! {"hello": 1}).await.unwrap();
            let saved = set(&db, &id, enabled).await.unwrap();
            let after = db.run_command(doc! {"hello": 1}).await.unwrap();
            assert_eq!(saved.enabled, enabled);
            assert_eq!(saved.policy_version.as_deref(), Some(POLICY_VERSION));
            let changed = saved.changed_at.unwrap().timestamp_millis();
            assert!(changed >= before.get_datetime("localTime").unwrap().timestamp_millis());
            assert!(changed <= after.get_datetime("localTime").unwrap().timestamp_millis());
            assert_eq!(read(&db, &id).await.unwrap(), saved);
            let stored = db
                .collection::<Document>(COLLECTION_NAME)
                .find_one(doc! {"_id": &id})
                .await
                .unwrap()
                .unwrap();
            let analytics = stored
                .get_document("profile_config")
                .unwrap()
                .get_document("analytics")
                .unwrap();
            assert_eq!(analytics.get_str("policy_version").unwrap(), POLICY_VERSION);
            assert_eq!(
                analytics
                    .get_datetime("changed_at")
                    .unwrap()
                    .timestamp_millis(),
                changed
            );
        }
    }

    #[tokio::test]
    async fn security_audit_persists_without_analytics_permission() {
        use crate::services::audit_service::{self, AuditActor};
        let (db, id) = account().await;
        let actor = AuditActor::from_auth_user(&crate::test_utils::test_auth_user(&id));
        for choice in [None, Some(false), Some(true), Some(false)] {
            if let Some(enabled) = choice {
                set(&db, &id, enabled).await.unwrap();
            }
            audit_service::log_actor_event(
                db.clone(),
                &actor,
                "security.consent_independence",
                None,
            )
            .await
            .unwrap();
        }
        let rows = db
            .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"user_id": &id, "event_type": "security.consent_independence"})
            .await
            .unwrap();
        assert_eq!(
            rows, 4,
            "product consent must never suppress security evidence"
        );
        let verified = crate::services::audit_chain_service::verify_chain(
            &db,
            audit_service::audit_chain_hmac_key().unwrap(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            verified.status,
            crate::services::audit_chain_service::AuditChainStatus::Ok
        );
        assert_eq!(verified.checked_count, 4);
    }

    #[tokio::test]
    async fn permission_commands_use_linearizable_gate_and_majority_writes() {
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{Arc, Mutex};
        let commands = Arc::new(Mutex::new(Vec::<Document>::new()));
        let recorder = commands.clone();
        let db = crate::test_utils::connect_test_database_with_command_handler(
            "analytics_concerns",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event {
                    recorder.lock().unwrap().push(event.command);
                }
            }),
        )
        .await
        .expect("isolated MongoDB required");
        let id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(COLLECTION_NAME)
            .insert_one(test_user(&id, UserType::Person))
            .await
            .unwrap();
        commands.lock().unwrap().clear();
        let consent = set(&db, &id, true).await.unwrap();
        assert!(permits_dispatch(&db, &id, &consent.revision).await);
        let recorded = commands.lock().unwrap();
        let write = recorded
            .iter()
            .find(|command| command.contains_key("findAndModify"))
            .unwrap();
        assert_eq!(
            write
                .get_document("writeConcern")
                .unwrap()
                .get_str("w")
                .unwrap(),
            "majority"
        );
        let read = recorded
            .iter()
            .find(|command| command.contains_key("find"))
            .unwrap();
        assert_eq!(
            read.get_document("readConcern")
                .unwrap()
                .get_str("level")
                .unwrap(),
            "linearizable"
        );
        let max_time = match read.get("maxTimeMS") {
            Some(bson::Bson::Int32(value)) => i64::from(*value),
            Some(bson::Bson::Int64(value)) => *value,
            other => panic!("maxTimeMS must be a BSON integer: {other:?}"),
        };
        assert_eq!(max_time, 200);
    }

    #[tokio::test]
    async fn unavailable_gate_is_bounded_and_denies() {
        let mut config = mongodb::options::ClientOptions::parse("mongodb://127.0.0.1:1")
            .await
            .unwrap();
        config.server_selection_timeout = Some(std::time::Duration::from_secs(30));
        let db = mongodb::Client::with_options(config)
            .unwrap()
            .database("unavailable");
        let started = std::time::Instant::now();
        assert!(
            permitted_revision(&db, &uuid::Uuid::new_v4().to_string())
                .await
                .is_none()
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "optional analytics lookup must not wait for server selection"
        );
    }

    #[tokio::test]
    async fn absence_withdrawal_regrant_and_account_fences_are_live() {
        let (db, id) = account().await;
        assert!(!read(&db, &id).await.unwrap().enabled);
        assert!(permitted_revision(&db, &id).await.is_none());
        assert!(
            permitted_revision(&db, &uuid::Uuid::new_v4().to_string())
                .await
                .is_none()
        );
        assert!(permitted_revision(&db, "pre-login").await.is_none());
        let first = set(&db, &id, true).await.unwrap();
        assert!(permits_dispatch(&db, &id, &first.revision).await);
        set(&db, &id, false).await.unwrap();
        assert!(!permits_dispatch(&db, &id, &first.revision).await);
        let second = set(&db, &id, true).await.unwrap();
        assert_ne!(first.revision, second.revision);
        assert!(!permits_dispatch(&db, &id, &first.revision).await);
        assert!(permits_dispatch(&db, &id, &second.revision).await);
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &id},
                doc! {"$set": {"platform_personal_deleting": true}},
            )
            .await
            .unwrap();
        assert!(!permits_dispatch(&db, &id, &second.revision).await);
        assert!(set(&db, &id, true).await.is_err());
    }

    #[tokio::test]
    async fn clock_skew_and_same_timestamp_do_not_resurrect_old_permission() {
        let (db, id) = account().await;
        let first = set(&db, &id, true).await.unwrap();
        for millis in [0_i64, i64::MAX / 2, 0] {
            set(&db, &id, false).await.unwrap();
            let current = set(&db, &id, true).await.unwrap();
            db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id": &id}, doc! {"$set": {"profile_config.analytics.changed_at": bson::DateTime::from_millis(millis)}}).await.unwrap();
            assert!(!permits_dispatch(&db, &id, &first.revision).await);
            assert!(permits_dispatch(&db, &id, &current.revision).await);
        }
    }

    #[tokio::test]
    async fn reads_override_inherited_secondary_only_preference() {
        let (db, id) = account().await;
        let current = set(&db, &id, true).await.unwrap();
        let secondary_only = db.client().database_with_options(
            db.name(),
            mongodb::options::DatabaseOptions::builder()
                .selection_criteria(SelectionCriteria::ReadPreference(
                    ReadPreference::Secondary {
                        options: Default::default(),
                    },
                ))
                .build(),
        );
        // The isolated replica set has one primary and no secondary. An inherited
        // secondary-only read cannot succeed; the gate must select primary.
        assert!(permits_dispatch(&secondary_only, &id, &current.revision).await);
    }

    #[tokio::test]
    async fn concurrent_choices_do_not_reuse_revisions_or_cross_accounts() {
        let (db, id) = account().await;
        let other = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(COLLECTION_NAME)
            .insert_one(test_user(&other, UserType::Person))
            .await
            .unwrap();
        let choices =
            futures::future::join_all((0..16).map(|index| set(&db, &id, index % 2 == 0))).await;
        let mut revisions = std::collections::HashSet::new();
        for choice in choices {
            assert!(revisions.insert(choice.unwrap().revision));
        }
        assert_eq!(revisions.len(), 16);
        for revision in &revisions {
            assert!(!permits_dispatch(&db, &other, revision).await);
        }
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(doc! {"_id": &id}, doc! {"$set": {"is_active": false}})
            .await
            .unwrap();
        assert!(permitted_revision(&db, &id).await.is_none());
    }
}
