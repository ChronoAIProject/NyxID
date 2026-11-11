//! Install identity constraints before retiring exactly recognized v1 indexes.
//! Native writers require the durable completion marker; legacy authority keeps
//! its existing fields, HMAC inputs and collections throughout the migration.
use crate::errors::{AppError, AppResult};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{Bson, Document, doc},
    options::IndexOptions,
};

const MIGRATIONS: &str = "platform_entry_migrations";
const VERSION: &str = "native-entry-v2";

#[derive(Clone)]
struct Replacement {
    collection: &'static str,
    old_name: &'static str,
    keys: Document,
    old_partial: Option<Document>,
    new_name: &'static str,
    new_partial: Document,
}

fn replacements() -> Vec<Replacement> {
    vec![
        Replacement {
            collection: "platform_bot_configs",
            old_name: "manager_bot_id_1",
            keys: doc! {"manager_bot_id":1},
            old_partial: None,
            new_name: "platform_telegram_manager_unique_v2",
            new_partial: doc! {"native_entry":Bson::Null},
        },
        Replacement {
            collection: "platform_manager_ingress_receipts",
            old_name: "manager_bot_id_1_telegram_bot_id_1_sender_id_1_update_id_1",
            keys: doc! {"manager_bot_id":1,"telegram_bot_id":1,"sender_id":1,"update_id":1},
            old_partial: None,
            new_name: "platform_telegram_ingress_unique_v2",
            new_partial: doc! {"telegram_bot_id":{"$type":["int","long"]}},
        },
        Replacement {
            collection: "platform_personal_bindings",
            old_name: "personal_sender_active",
            keys: doc! {"platform_config_id":1,"manager_authority_epoch":1,"telegram_sender_id":1},
            old_partial: Some(doc! {"state":"active"}),
            new_name: "platform_telegram_personal_active_v2",
            new_partial: doc! {"state":"active","telegram_bot_id":{"$type":["int","long"]}},
        },
        Replacement {
            collection: "platform_personal_bindings",
            old_name: "personal_sender_generation",
            keys: doc! {"platform_config_id":1,"manager_authority_epoch":1,"telegram_sender_id":1,"generation":-1},
            old_partial: None,
            new_name: "platform_telegram_personal_generation_v2",
            new_partial: doc! {"telegram_bot_id":{"$type":["int","long"]}},
        },
        Replacement {
            collection: "platform_personal_link_intents",
            old_name: "platform_config_id_1_manager_authority_epoch_1_telegram_sender_id_1_initiation_ref_1",
            keys: doc! {"platform_config_id":1,"manager_authority_epoch":1,"telegram_sender_id":1,"initiation_ref":1},
            old_partial: None,
            new_name: "platform_telegram_intent_initiation_v2",
            new_partial: doc! {"telegram_bot_id":{"$type":["int","long"]}},
        },
        Replacement {
            collection: "platform_public_sessions",
            old_name: "public_sender_live_unique",
            keys: doc! {"profile_id":1,"manager_bot_id":1,"telegram_bot_id":1,"sender_id":1,"chat_id":1},
            old_partial: Some(doc! {"revoked_at":Bson::Null}),
            new_name: "platform_telegram_public_live_v2",
            new_partial: doc! {"revoked_at":Bson::Null,"telegram_bot_id":{"$type":["int","long"]}},
        },
    ]
}

fn native_indexes() -> Vec<(&'static str, &'static str, Document, Document, bool)> {
    let native = doc! {"platform_entry.version":2};
    vec![
        (
            "platform_bot_configs",
            "platform_entry_channel_unique_v2",
            doc! {"channel_bot_id":1},
            doc! {"channel_bot_id":{"$type":"string"}},
            true,
        ),
        (
            "platform_bot_configs",
            "platform_native_namespace_unique_v2",
            doc! {"native_entry.identity.platform":1,"channel_bot_id":1,"native_entry.identity.platform_bot_id":1},
            doc! {"identity_version":2},
            true,
        ),
        (
            "platform_manager_ingress_receipts",
            "platform_native_ingress_unique_v2",
            doc! {"profile_id":1,"manager_authority_epoch":1,"platform_entry.platform":1,"platform_entry.channel_bot_id":1,"platform_entry.platform_bot_id":1,"platform_sender.sender_id":1,"platform_event_id":1},
            native.clone(),
            true,
        ),
        (
            "platform_personal_bindings",
            "platform_native_personal_active_v2",
            doc! {"platform_config_id":1,"manager_authority_epoch":1,"platform_sender.sender_id":1},
            doc! {"platform_entry.version":2,"state":"active"},
            true,
        ),
        (
            "platform_personal_bindings",
            "platform_native_personal_generation_v2",
            doc! {"platform_config_id":1,"manager_authority_epoch":1,"platform_sender.sender_id":1,"generation":-1},
            native.clone(),
            true,
        ),
        (
            "platform_personal_link_intents",
            "platform_native_intent_initiation_v2",
            doc! {"platform_config_id":1,"manager_authority_epoch":1,"platform_sender.sender_id":1,"initiation_ref":1},
            native.clone(),
            true,
        ),
        (
            "platform_personal_link_intents",
            "platform_native_intent_pending_v2",
            doc! {"platform_config_id":1,"manager_authority_epoch":1,"platform_sender.sender_id":1,"state":1},
            native.clone(),
            false,
        ),
        (
            "platform_personal_link_challenges",
            "platform_native_challenge_pending_v2",
            doc! {"platform_config_id":1,"manager_authority_epoch":1,"platform_sender.sender_id":1,"state":1},
            native.clone(),
            false,
        ),
        (
            "platform_public_sessions",
            "platform_native_public_live_v2",
            doc! {"profile_id":1,"platform_entry.platform":1,"platform_entry.channel_bot_id":1,"platform_entry.platform_bot_id":1,"platform_sender.sender_id":1,"platform_sender.private_conversation_id":1},
            doc! {"platform_entry.version":2,"revoked_at":Bson::Null},
            true,
        ),
        (
            "platform_native_entry_receipts",
            "platform_native_receipt_unique_v2",
            doc! {"profile_id":1,"manager_authority_epoch":1,"configuration.transport_generation":1,"configuration.identity.platform":1,"configuration.identity.channel_bot_id":1,"configuration.identity.platform_bot_id":1,"sender.sender_id":1,"event_id":1},
            doc! {"configuration.identity.version":2},
            true,
        ),
        (
            "platform_manager_deliveries",
            "platform_native_delivery_sender_v2",
            doc! {"profile_id":1,"platform_entry.platform":1,"platform_entry.channel_bot_id":1,"platform_sender.sender_id":1,"state":1},
            native,
            false,
        ),
    ]
}

fn conflict() -> AppError {
    AppError::Conflict(
        "Platform identity migration requires index review before native activation".into(),
    )
}

pub(crate) async fn native_writes_ready(db: &Database) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(MIGRATIONS)
        .find_one(doc! {"_id":VERSION,"state":"ready","schema_version":2})
        .await?
        .is_some())
}

pub(crate) async fn ensure(db: &Database) -> AppResult<()> {
    ensure_until(db, false).await
}

const LEASE_SECONDS: i64 = 120;

#[cfg(test)]
tokio::task_local! {
    static TEST_LEASE_DURATION: std::time::Duration;
    static TEST_INITIAL_OPERATION_STALL: std::sync::Arc<OperationStall>;
    static TEST_READY_PUBLICATION_STALL: std::sync::Arc<OperationStall>;
    static TEST_RELEASE_STALL: std::sync::Arc<OperationStall>;
    static TEST_MAINTENANCE_TIMEOUT: std::time::Duration;
    static TEST_LEASE_NOW: std::cell::Cell<chrono::DateTime<chrono::Utc>>;
    static TEST_HEARTBEAT_TICK: std::sync::Arc<tokio::sync::Notify>;
}

#[cfg(test)]
struct OperationStall {
    entered: std::sync::atomic::AtomicBool,
    cancelled: std::sync::atomic::AtomicBool,
    duration: std::time::Duration,
    release: Option<std::sync::Arc<tokio::sync::Notify>>,
}

#[cfg(test)]
struct ReadyPublicationObservation(std::sync::Arc<OperationStall>);

#[cfg(test)]
impl Drop for ReadyPublicationObservation {
    fn drop(&mut self) {
        self.0
            .cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn lease_duration() -> std::time::Duration {
    let duration = std::time::Duration::from_secs(LEASE_SECONDS as u64);
    #[cfg(test)]
    let duration = TEST_LEASE_DURATION
        .try_with(|duration| *duration)
        .unwrap_or(duration);
    duration
}

fn maintenance_timeout() -> std::time::Duration {
    let duration = std::time::Duration::from_secs(5);
    #[cfg(test)]
    let duration = TEST_MAINTENANCE_TIMEOUT
        .try_with(|duration| *duration)
        .unwrap_or(duration);
    duration
}

#[cfg(test)]
async fn operation_stall(stall: std::sync::Arc<OperationStall>) {
    struct Cancellation(std::sync::Arc<OperationStall>);
    impl Drop for Cancellation {
        fn drop(&mut self) {
            self.0
                .cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let _cancelled = Cancellation(stall.clone());
    stall
        .entered
        .store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(release) = &stall.release {
        release.notified().await;
    } else {
        tokio::time::sleep(stall.duration).await;
    }
}

fn lease_now() -> chrono::DateTime<chrono::Utc> {
    #[cfg(test)]
    if let Ok(now) = TEST_LEASE_NOW.try_with(std::cell::Cell::get) {
        return now;
    }
    chrono::Utc::now()
}

async fn heartbeat_tick(period: std::time::Duration) {
    #[cfg(test)]
    if let Ok(tick) = TEST_HEARTBEAT_TICK.try_with(std::sync::Arc::clone) {
        tick.notified().await;
        return;
    }
    tokio::time::sleep(period).await;
}

fn lease_until(now: chrono::DateTime<chrono::Utc>) -> AppResult<bson::DateTime> {
    Ok(bson::DateTime::from_chrono(
        now + chrono::Duration::from_std(lease_duration()).map_err(|_| conflict())?,
    ))
}

/// Poll the owner operation and lease renewal in this same future. Returning,
/// cancellation or loss drops both futures; no unjoined task can renew a lease
/// after its owner has stopped. Every subsequent mutation also renews/fences.
async fn while_owned<T>(
    db: &Database,
    lease: &str,
    operation: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    let heartbeat = async {
        let period = lease_duration() / 3;
        loop {
            heartbeat_tick(period).await;
            // A stuck Mongo renewal cannot leave mutation work running beyond
            // the lease. Losing either storage or ownership cancels the work.
            match tokio::time::timeout(period, renew(db, lease)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => break Err::<(), _>(error),
                Err(_) => break Err(conflict()),
            }
        }
    };
    tokio::pin!(operation, heartbeat);
    tokio::select! {
        biased;
        result = &mut heartbeat => { result?; Err(conflict()) }
        result = &mut operation => result,
    }
}

async fn renew(db: &Database, lease: &str) -> AppResult<()> {
    let now = lease_now();
    let changed = db
        .collection::<Document>(MIGRATIONS)
        .update_one(
            doc! {"_id":VERSION,"schema_version":2,"state":"preparing","lease_id":lease,
            "lease_until":{"$gt":bson::DateTime::from_chrono(now)}},
            doc! {"$set":{"lease_until":lease_until(now)?}},
        )
        .await?;
    if changed.matched_count != 1 {
        return Err(conflict());
    }
    Ok(())
}

/// Existing database startup APIs return Mongo errors; keep diagnostics generic
/// because index/data values can contain private platform identifiers.
pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    ensure(db).await.map_err(|_| {
        mongodb::error::Error::custom("Platform entry identity indexes require review")
    })
}

async fn ensure_until(db: &Database, stop_before_retire: bool) -> AppResult<()> {
    // Completed startup never resets readiness, including while other replicas
    // are admitting native messages. An incomplete migration is resumable.
    if native_writes_ready(db).await? {
        return Ok(());
    }
    let markers = db.collection::<Document>(MIGRATIONS);
    markers
        .update_one(
            doc! {"_id":VERSION},
            doc! {"$setOnInsert":{
                "state":"preparing","schema_version":2,"created_at":bson::DateTime::now()
            }},
        )
        .upsert(true)
        .await?;
    let lease = uuid::Uuid::new_v4().to_string();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(125);
    loop {
        if native_writes_ready(db).await? {
            return Ok(());
        }
        let now = lease_now();
        let acquired = markers.find_one_and_update(doc! {
            "_id":VERSION,"schema_version":2,"state":"preparing",
            "$or":[{"lease_until":Bson::Null},{"lease_until":{"$lte":bson::DateTime::from_chrono(now)}}]
        },doc! {"$set":{"lease_id":&lease,"lease_until":lease_until(now)?}})
            .await?.is_some();
        if acquired {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(conflict());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let result = while_owned(db, &lease, async {
        prepare_and_retire(db, &lease, stop_before_retire).await?;
        renew(db, &lease).await?;
        let now = lease_now();
        // Publication can become visible to renewal before this await resolves.
        #[cfg(test)]
        let publication = TEST_READY_PUBLICATION_STALL
            .try_with(std::sync::Arc::clone)
            .ok()
            .map(|stall| {
                stall
                    .entered
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                ReadyPublicationObservation(stall)
            });
        let completed = markers
            .update_one(
                doc! {"_id":VERSION,"schema_version":2,"state":"preparing",
                "lease_id":&lease,"lease_until":{"$gt":bson::DateTime::from_chrono(now)}},
                doc! {"$set":{"state":"ready","completed_at":bson::DateTime::from_chrono(now)},
                "$unset":{"lease_id":"","lease_until":""}},
            )
            .await?;
        if completed.matched_count != 1 {
            return Err(conflict());
        }
        #[cfg(test)]
        if let Some(publication) = &publication {
            tokio::time::sleep(publication.0.duration).await;
        }
        Ok(())
    })
    .await;
    if result.is_err() {
        // Readiness is authoritative. Publication can commit before its await
        // resolves, while renewal sees the now-completed owner and stops. A
        // bounded read converges that overlap without resetting readiness.
        if matches!(
            tokio::time::timeout(maintenance_timeout(), native_writes_ready(db)).await,
            Ok(Ok(true))
        ) {
            return Ok(());
        }
        // Storage loss must not turn bounded heartbeat failure into an unbounded
        // release wait. Failure preserves the leased/recoverable marker; a later
        // owner can resume after expiry. Never release a replacement owner's row.
        let _ = tokio::time::timeout(maintenance_timeout(), async {
            #[cfg(test)]
            if let Ok(stall) = TEST_RELEASE_STALL.try_with(std::sync::Arc::clone) {
                operation_stall(stall).await;
            }
            markers
                .update_one(
                    doc! {"_id":VERSION,"state":"preparing","lease_id":&lease},
                    doc! {"$unset":{"lease_id":"","lease_until":""}},
                )
                .await
        })
        .await;
    }
    result
}

async fn prepare_and_retire(db: &Database, lease: &str, stop_before_retire: bool) -> AppResult<()> {
    renew(db, lease).await?;
    #[cfg(test)]
    if let Ok(stall) = TEST_INITIAL_OPERATION_STALL.try_with(std::sync::Arc::clone) {
        operation_stall(stall).await;
    }
    renew(db, lease).await?;
    // A small additive marker on profiles scopes the replacement legacy index.
    // Signed consent/binding/ingress rows are never rewritten by migration.
    db.collection::<Document>("platform_bot_configs")
        .update_many(
            doc! {"native_entry":Bson::Null,"identity_version":{"$exists":false}},
            doc! {"$set":{"identity_version":1}},
        )
        .await?;
    let replacements = replacements();
    for replacement in &replacements {
        renew(db, lease).await?;
        db.collection::<Document>(replacement.collection)
            .create_index(
                IndexModel::builder()
                    .keys(replacement.keys.clone())
                    .options(
                        IndexOptions::builder()
                            .name(replacement.new_name.to_owned())
                            .unique(true)
                            .partial_filter_expression(replacement.new_partial.clone())
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    for (collection, name, keys, partial, unique) in native_indexes() {
        renew(db, lease).await?;
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_owned())
                            .unique(unique)
                            .partial_filter_expression(partial)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    // A deterministic interrupted-state fixture stops only after EVERY new
    // constraint exists and before ANY old constraint is retired.
    if stop_before_retire {
        return Err(conflict());
    }
    for replacement in &replacements {
        renew(db, lease).await?;
        let collection = db.collection::<Document>(replacement.collection);
        let indexes = collection
            .list_indexes()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        if let Some(old) = indexes.iter().find(|index| {
            index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                == Some(replacement.old_name)
        }) {
            let options = old.options.as_ref().ok_or_else(conflict)?;
            if old.keys != replacement.keys
                || options.unique != Some(true)
                || options.partial_filter_expression != replacement.old_partial
                || options.sparse == Some(true)
                || options.collation.is_some()
            {
                return Err(conflict());
            }
            renew(db, lease).await?;
            if let Err(error) = collection.drop_index(replacement.old_name).await {
                // A prior expired lease may have completed this exact retirement.
                // Only a verified absent index is an idempotent successful drop.
                let missing = matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 27);
                if !missing
                    || collection
                        .list_indexes()
                        .await?
                        .try_collect::<Vec<_>>()
                        .await?
                        .iter()
                        .any(|index| {
                            index
                                .options
                                .as_ref()
                                .and_then(|options| options.name.as_deref())
                                == Some(replacement.old_name)
                        })
                {
                    return Err(error.into());
                }
            }
        }
        // An unrecognized unrestricted unique index can still collapse all
        // missing legacy fields for native rows. Do not guess or drop it.
        let remaining = collection
            .list_indexes()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        if remaining.iter().any(|index| {
            index.keys == replacement.keys
                && index.options.as_ref().is_some_and(|options| {
                    options.unique == Some(true)
                        && options.partial_filter_expression
                            != Some(replacement.new_partial.clone())
                })
        }) {
            return Err(conflict());
        }
    }
    renew(db, lease).await?;
    super::platform_entry_gateway_service::ensure_indexes(db).await?;
    super::platform_entry_discord_installation::ensure_indexes(db).await?;
    renew(db, lease).await?;
    db.collection::<Document>("platform_native_reply_handles")
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"receipt_id":1})
                .options(
                    IndexOptions::builder()
                        .name("platform_native_reply_receipt_v2".to_owned())
                        .unique(true)
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .name("platform_native_reply_expiry_v2".to_owned())
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        ])
        .await?;
    renew(db, lease).await?;
    db.collection::<Document>("platform_native_entry_receipts")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .name("platform_native_receipt_expiry_v2".to_owned())
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn old_indexes_are_replaced_after_constraints_and_interrupted_migration_resumes() {
        let db =
            crate::test_utils::connect_transaction_test_database("native_entry_index_migration")
                .await;
        for replacement in replacements() {
            let mut options = IndexOptions::builder()
                .name(replacement.old_name.to_owned())
                .unique(true)
                .build();
            options.partial_filter_expression = replacement.old_partial;
            db.collection::<Document>(replacement.collection)
                .create_index(
                    IndexModel::builder()
                        .keys(replacement.keys)
                        .options(options)
                        .build(),
                )
                .await
                .unwrap();
        }
        db.collection::<Document>("platform_bot_configs")
            .insert_one(
                doc! {"_id":"old-profile","channel_bot_id":"old-channel","manager_bot_id":"99"},
            )
            .await
            .unwrap();
        assert!(ensure_until(&db, true).await.is_err());
        assert!(!native_writes_ready(&db).await.unwrap());
        for replacement in replacements() {
            let indexes = db
                .collection::<Document>(replacement.collection)
                .list_indexes()
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            assert!(indexes.iter().any(|index| {
                index.options.as_ref().and_then(|o| o.name.as_deref()) == Some(replacement.old_name)
            }));
            assert!(indexes.iter().any(|index| {
                index.options.as_ref().and_then(|o| o.name.as_deref()) == Some(replacement.new_name)
            }));
        }
        ensure(&db).await.unwrap();
        ensure(&db).await.unwrap();
        assert!(native_writes_ready(&db).await.unwrap());
        let legacy = db
            .collection::<Document>("platform_bot_configs")
            .find_one(doc! {"_id":"old-profile"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(legacy.get_str("manager_bot_id").unwrap(), "99");
        assert_eq!(legacy.get_i32("identity_version").unwrap(), 1);
        for (platform, channel) in [
            ("slack", "channel-one"),
            ("discord", "channel-two"),
            ("slack", "channel-three"),
        ] {
            db.collection::<Document>("platform_bot_configs").insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"channel_bot_id":channel,"manager_bot_id":"same-opaque","identity_version":2,
                "native_entry":{"identity":{"version":2,"platform":platform,"platform_bot_id":"same-opaque","channel_bot_id":channel}}}).await.unwrap();
        }
    }
    #[tokio::test]
    async fn concurrent_startups_converge_and_completed_marker_never_returns_to_preparing() {
        let db = crate::test_utils::connect_transaction_test_database(
            "native_entry_concurrent_migration",
        )
        .await;
        for replacement in replacements() {
            let mut options = IndexOptions::builder()
                .name(replacement.old_name.to_owned())
                .unique(true)
                .build();
            options.partial_filter_expression = replacement.old_partial;
            db.collection::<Document>(replacement.collection)
                .create_index(
                    IndexModel::builder()
                        .keys(replacement.keys)
                        .options(options)
                        .build(),
                )
                .await
                .unwrap();
        }
        let (a, b) = tokio::join!(ensure(&db), ensure(&db));
        a.unwrap();
        b.unwrap();
        let ready = db
            .collection::<Document>(MIGRATIONS)
            .find_one(doc! {"_id":VERSION})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready.get_str("state").unwrap(), "ready");
        let monitor = async {
            for _ in 0..100 {
                assert!(native_writes_ready(&db).await.unwrap());
                tokio::task::yield_now().await;
            }
        };
        let (a, b, ()) = tokio::join!(ensure(&db), ensure(&db), monitor);
        a.unwrap();
        b.unwrap();
        assert_eq!(
            db.collection::<Document>(MIGRATIONS)
                .find_one(doc! {"_id":VERSION})
                .await
                .unwrap()
                .unwrap(),
            ready
        );
        for replacement in replacements() {
            let indexes = db
                .collection::<Document>(replacement.collection)
                .list_indexes()
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            assert!(!indexes.iter().any(|index| {
                index.options.as_ref().and_then(|o| o.name.as_deref()) == Some(replacement.old_name)
            }));
        }
        let receipts = db
            .collection::<Document>("platform_native_entry_receipts")
            .list_indexes()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        let ttl = receipts
            .iter()
            .find(|index| {
                index.options.as_ref().and_then(|o| o.name.as_deref())
                    == Some("platform_native_receipt_expiry_v2")
            })
            .unwrap();
        assert_eq!(ttl.keys, doc! {"expires_at":1});
        assert_eq!(
            ttl.options.as_ref().unwrap().expire_after,
            Some(std::time::Duration::ZERO)
        );
    }

    fn stall(duration: std::time::Duration) -> std::sync::Arc<OperationStall> {
        std::sync::Arc::new(OperationStall {
            entered: std::sync::atomic::AtomicBool::new(false),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            duration,
            release: None,
        })
    }

    async fn entered(stall: &OperationStall) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !stall.entered.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("migration entered owned operation");
    }

    #[tokio::test]
    async fn owned_lease_is_renewed_during_an_operation_longer_than_its_initial_lifetime() {
        let db =
            crate::test_utils::connect_transaction_test_database("native_entry_lease_heartbeat")
                .await;
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let ticks = std::sync::Arc::new(tokio::sync::Notify::new());
        let mut stalled = stall(std::time::Duration::ZERO);
        std::sync::Arc::get_mut(&mut stalled).unwrap().release = Some(release.clone());
        // Keep production's lease duration and renewal timeout. The test owns
        // the clock and tick delivery, so runner stalls cannot expire the lease
        // and the operation cannot finish before its observer gets scheduled.
        let start = chrono::Utc::now();
        TEST_LEASE_NOW
            .scope(std::cell::Cell::new(start), async {
                let run = TEST_HEARTBEAT_TICK.scope(
                    ticks.clone(),
                    TEST_INITIAL_OPERATION_STALL.scope(stalled.clone(), ensure(&db)),
                );
                let observe = async {
                    entered(&stalled).await;
                    let markers = db.collection::<Document>(MIGRATIONS);
                    let first = markers
                        .find_one(doc! {"_id": VERSION})
                        .await
                        .unwrap()
                        .unwrap();
                    let lease = first.get_str("lease_id").unwrap().to_owned();
                    let initial_expiry = *first.get_datetime("lease_until").unwrap();
                    // Four real renewals take logical time past the initial lease.
                    for step in 1..=4 {
                        let now = start + chrono::Duration::seconds(step * LEASE_SECONDS / 3);
                        TEST_LEASE_NOW.with(|clock| clock.set(now));
                        ticks.notify_one();
                        let expected = lease_until(now).unwrap();
                        tokio::time::timeout(std::time::Duration::from_secs(5), async {
                            loop {
                                let current = markers
                                    .find_one(doc! {"_id": VERSION})
                                    .await
                                    .unwrap()
                                    .unwrap();
                                assert_eq!(current.get_str("lease_id").unwrap(), lease);
                                assert_eq!(current.get_str("state").unwrap(), "preparing");
                                if current.get_datetime("lease_until").unwrap() == &expected {
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                            }
                        })
                        .await
                        .expect("owned operation must renew at every heartbeat");
                        assert!(expected > initial_expiry);
                        assert!(expected.to_chrono() > now);
                        assert!(!native_writes_ready(&db).await.unwrap());
                    }
                    assert!(lease_now() > initial_expiry.to_chrono());
                    release.notify_one();
                    lease
                };
                let (result, lease) =
                    tokio::time::timeout(std::time::Duration::from_secs(15), async {
                        tokio::join!(run, observe)
                    })
                    .await
                    .unwrap();
                result.unwrap();
                assert!(native_writes_ready(&db).await.unwrap());
                let markers = db.collection::<Document>(MIGRATIONS);
                let ready = markers
                    .find_one(doc! {"_id": VERSION})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(!ready.contains_key("lease_id"));
                assert!(!ready.contains_key("lease_until"));
                // A late renewal from the completed owner must not change readiness.
                assert!(renew(&db, &lease).await.is_err());
                assert_eq!(
                    markers
                        .find_one(doc! {"_id": VERSION})
                        .await
                        .unwrap()
                        .unwrap(),
                    ready
                );
            })
            .await;
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn publication_heartbeat_overlap_converges_on_committed_ready() {
        let db = crate::test_utils::connect_transaction_test_database("native_entry_ready_overlap")
            .await;
        let publication = stall(std::time::Duration::from_secs(10));
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            TEST_LEASE_DURATION.scope(
                std::time::Duration::from_millis(900),
                TEST_READY_PUBLICATION_STALL.scope(publication.clone(), ensure(&db)),
            ),
        )
        .await
        .unwrap();
        result.unwrap();
        assert!(
            publication
                .entered
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        assert!(
            publication
                .cancelled
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        assert!(native_writes_ready(&db).await.unwrap());
        let ready = db
            .collection::<Document>(MIGRATIONS)
            .find_one(doc! {"_id":VERSION})
            .await
            .unwrap()
            .unwrap();
        ensure(&db).await.unwrap();
        assert_eq!(
            db.collection::<Document>(MIGRATIONS)
                .find_one(doc! {"_id":VERSION})
                .await
                .unwrap()
                .unwrap(),
            ready
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn ownership_loss_cancels_inflight_operation_and_cannot_retire_or_publish_ready() {
        let db =
            crate::test_utils::connect_transaction_test_database("native_entry_lost_lease").await;
        let old = replacements().remove(0);
        db.collection::<Document>(old.collection)
            .create_index(
                IndexModel::builder()
                    .keys(old.keys)
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .name(old.old_name.to_owned())
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        let release = stall(std::time::Duration::from_secs(10));
        let stall = stall(std::time::Duration::from_secs(10));
        let run = TEST_MAINTENANCE_TIMEOUT.scope(
            std::time::Duration::from_millis(150),
            TEST_RELEASE_STALL.scope(
                release.clone(),
                TEST_LEASE_DURATION.scope(
                    std::time::Duration::from_millis(900),
                    TEST_INITIAL_OPERATION_STALL.scope(stall.clone(), ensure(&db)),
                ),
            ),
        );
        let steal = async {
            entered(&stall).await;
            let changed = db.collection::<Document>(MIGRATIONS).update_one(doc! {"_id":VERSION,"state":"preparing"},
                doc! {"$set":{"lease_id":"replacement-owner","lease_until":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::seconds(30))}}).await.unwrap();
            assert_eq!(changed.matched_count, 1);
        };
        let started = tokio::time::Instant::now();
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(run, steal)
        })
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
        assert!(stall.cancelled.load(std::sync::atomic::Ordering::SeqCst));
        assert!(release.entered.load(std::sync::atomic::Ordering::SeqCst));
        assert!(release.cancelled.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!native_writes_ready(&db).await.unwrap());
        let marker = db
            .collection::<Document>(MIGRATIONS)
            .find_one(doc! {"_id":VERSION})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker.get_str("lease_id").unwrap(), "replacement-owner");
        let indexes = db
            .collection::<Document>(old.collection)
            .list_indexes()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert!(indexes.iter().any(|index| {
            index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                == Some(old.old_name)
        }));
        assert!(!indexes.iter().any(|index| {
            index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                == Some(old.new_name)
        }));
        // No detached renewal survives cancellation to overwrite the new owner.
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert_eq!(
            db.collection::<Document>(MIGRATIONS)
                .find_one(doc! {"_id":VERSION})
                .await
                .unwrap()
                .unwrap(),
            marker
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn expired_preparation_lease_resumes_without_native_admission() {
        let db =
            crate::test_utils::connect_transaction_test_database("native_entry_expired_migration")
                .await;
        db.collection::<Document>(MIGRATIONS).insert_one(doc! {"_id":VERSION,"schema_version":2,"state":"preparing",
            "lease_id":"crashed-replica","lease_until":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}).await.unwrap();
        assert!(!native_writes_ready(&db).await.unwrap());
        let (a, b) = tokio::join!(ensure(&db), ensure(&db));
        a.unwrap();
        b.unwrap();
        assert!(native_writes_ready(&db).await.unwrap());
    }
}
