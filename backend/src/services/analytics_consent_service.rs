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
pub const POLICY_VERSION_V1: &str = "1";
pub const POLICY_VERSION_DIAGNOSTICS: &str = "2";
#[cfg(test)]
pub const POLICY_VERSION: &str = POLICY_VERSION_V1;

#[derive(Clone, Copy)]
pub enum PolicyVersion {
    V1,
    Diagnostics,
}

impl PolicyVersion {
    pub fn parse(value: Option<&str>) -> AppResult<Self> {
        match value {
            None | Some("1") => Ok(Self::V1),
            Some("2") => Ok(Self::Diagnostics),
            _ => Err(AppError::ValidationError(
                "Invalid analytics policy version".into(),
            )),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::V1 => POLICY_VERSION_V1,
            Self::Diagnostics => POLICY_VERSION_DIAGNOSTICS,
        }
    }
}

/// Existing explicit choices already answered the prompt before the marker
/// existed. This is display state only and never grants analytics permission.
pub fn prompt_seen(consent: &AnalyticsConsent) -> bool {
    consent.prompt_seen
        || consent.enabled
        || !consent.revision.is_empty()
        || consent.policy_version.is_some()
        || consent.changed_at.is_some()
}

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
    set_acknowledging(db, user_id, enabled, PolicyVersion::V1).await
}

pub async fn set_acknowledging(
    db: &Database,
    user_id: &str,
    enabled: bool,
    acknowledged: PolicyVersion,
) -> AppResult<AnalyticsConsent> {
    if matches!(acknowledged, PolicyVersion::Diagnostics) {
        use super::crash_report_service::DiagnosticsStatus;
        match super::crash_report_service::eligibility(db, user_id).await {
            DiagnosticsStatus::Unknown => return Err(AppError::SecurityReadinessUnavailable),
            DiagnosticsStatus::NotOffered => {
                return Err(AppError::ValidationError(
                    "Diagnostics permission is not currently offered".into(),
                ));
            }
            DiagnosticsStatus::Available | DiagnosticsStatus::ConsentRequired => {}
        }
    }
    // A random revision invalidates previously admitted work even on same-ms
    // withdraw/regrant or wall-clock rollback. Majority acknowledgement prevents
    // an acknowledged withdrawal being rolled back on ordinary primary failover.
    let revision = uuid::Uuid::new_v4().to_string();
    let user = db
        .collection_with_options::<User>(COLLECTION_NAME, options())
        .find_one_and_update(
            active_person(user_id),
            vec![doc! {"$set": {"profile_config.analytics": {
                "prompt_seen": true,
                "enabled": enabled, "changed_at": "$$NOW", "revision": &revision,
                "policy_version": acknowledged.as_str()
            }}}],
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::NotFound("Active personal account not found".into()))?;
    Ok(user.profile_config.analytics)
}

/// A failed permission read is not evidence of withdrawal.
#[derive(Debug, PartialEq, Eq)]
pub enum ConsentPermission<T = ()> {
    Permitted(T),
    Denied,
    Unknown,
}
impl ConsentPermission<String> {
    fn revision(self) -> Option<String> {
        match self {
            Self::Permitted(revision) => Some(revision),
            // Legacy categorical analytics continues to fail closed on unknown.
            Self::Denied | Self::Unknown => None,
        }
    }
    pub(crate) fn for_revision(self, expected: &str) -> ConsentPermission {
        match self {
            Self::Permitted(revision) if revision == expected => ConsentPermission::Permitted(()),
            Self::Unknown => ConsentPermission::Unknown,
            _ => ConsentPermission::Denied,
        }
    }
}

/// Never cache positive permission. Queued work retains its admitted revision.
pub async fn permitted_revision(db: &Database, user_id: &str) -> Option<String> {
    permitted_revision_for(db, user_id, false).await.revision()
}

#[cfg(test)]
pub async fn permitted_diagnostics_revision(db: &Database, user_id: &str) -> Option<String> {
    permitted_revision_for(db, user_id, true).await.revision()
}

pub async fn permits_diagnostics(
    db: &Database,
    user_id: &str,
    revision: &str,
) -> ConsentPermission {
    permitted_revision_for(db, user_id, true)
        .await
        .for_revision(revision)
}

async fn permitted_revision_for(
    db: &Database,
    user_id: &str,
    diagnostics: bool,
) -> ConsentPermission<String> {
    if uuid::Uuid::parse_str(user_id).is_err() {
        return ConsentPermission::Unknown;
    }
    let mut filter = active_person(user_id);
    filter.insert("profile_config.analytics.enabled", true);
    if diagnostics {
        filter.insert(
            "profile_config.analytics.policy_version",
            POLICY_VERSION_DIAGNOSTICS,
        );
    }
    filter.insert(
        "profile_config.analytics.revision",
        doc! {"$type": "string", "$ne": ""},
    );
    // Linearizable reads observe acknowledged choices across primary changes.
    let mut gate_options = options();
    gate_options.read_concern = Some(ReadConcern::linearizable());
    bounded_consent_read(
        db.collection_with_options::<Document>(COLLECTION_NAME, gate_options)
            .find_one(filter)
            .max_time(std::time::Duration::from_millis(200))
            .projection(doc! {"profile_config.analytics.revision": 1})
            .into_future(),
        std::time::Duration::from_millis(250),
    )
    .await
}

pub(crate) async fn bounded_consent_read(
    read: impl std::future::Future<Output = mongodb::error::Result<Option<Document>>>,
    deadline: std::time::Duration,
) -> ConsentPermission<String> {
    match tokio::time::timeout(deadline, read).await {
        Ok(Ok(None)) => ConsentPermission::Denied,
        Ok(Ok(Some(row))) => row
            .get_document("profile_config")
            .ok()
            .and_then(|profile| profile.get_document("analytics").ok())
            .and_then(|analytics| analytics.get_str("revision").ok())
            .map(|revision| ConsentPermission::Permitted(revision.to_owned()))
            .unwrap_or(ConsentPermission::Unknown),
        Ok(Err(_)) | Err(_) => ConsentPermission::Unknown,
    }
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

    #[test]
    fn prompt_acknowledgement_preserves_legacy_choices_without_enabling_collection() {
        let unseen = AnalyticsConsent::default();
        assert!(!prompt_seen(&unseen));
        for consent in [
            AnalyticsConsent {
                prompt_seen: true,
                ..unseen.clone()
            },
            AnalyticsConsent {
                revision: "legacy-choice".into(),
                ..unseen.clone()
            },
            AnalyticsConsent {
                policy_version: Some("1".into()),
                ..unseen.clone()
            },
            AnalyticsConsent {
                changed_at: Some(chrono::Utc::now()),
                ..unseen.clone()
            },
        ] {
            assert!(prompt_seen(&consent));
            assert!(!consent.enabled);
        }
        // A legacy opt-in is also an answer, even without newer metadata.
        assert!(prompt_seen(&AnalyticsConsent {
            enabled: true,
            ..unseen
        }));
    }

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
        assert!(!prompt_seen(&absent));
        assert!(!absent.enabled);
        assert!(absent.changed_at.is_none());
        assert!(absent.policy_version.is_none());
        assert!(absent.revision.is_empty());
        for enabled in [true, false] {
            let before = db.run_command(doc! {"hello": 1}).await.unwrap();
            let saved = set(&db, &id, enabled).await.unwrap();
            assert!(prompt_seen(&saved));
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
            assert!(analytics.get_bool("prompt_seen").unwrap());
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
    async fn diagnostics_consent_read_preserves_success_denial_database_error_and_timeout() {
        let revision = "acknowledged";
        let row = doc! {"profile_config": {"analytics": {"revision": revision}}};
        let permitted = bounded_consent_read(
            std::future::ready(Ok(Some(row))),
            std::time::Duration::from_secs(1),
        )
        .await;
        assert_eq!(
            permitted.for_revision(revision),
            ConsentPermission::Permitted(())
        );
        assert_eq!(
            bounded_consent_read(
                std::future::ready(Ok(None)),
                std::time::Duration::from_secs(1)
            )
            .await,
            ConsentPermission::Denied
        );
        let failure = mongodb::error::Error::custom("fixed test failure");
        assert_eq!(
            bounded_consent_read(
                std::future::ready(Err(failure)),
                std::time::Duration::from_secs(1)
            )
            .await,
            ConsentPermission::Unknown
        );
        assert_eq!(
            bounded_consent_read(std::future::pending(), std::time::Duration::ZERO).await,
            ConsentPermission::Unknown
        );
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
