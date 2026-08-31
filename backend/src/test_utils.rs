use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use uuid::Uuid;

use crate::AppState;
use crate::config::AppConfig;
use crate::crypto::aes::EncryptionKeys;
use crate::crypto::jwks::JwksCache;
use crate::crypto::jwt::JwtKeys;
use crate::models::audit_log::AuditLog;
use crate::models::mcp_session::McpSessionStore;
use crate::models::node_pending_credential::NodePendingCredential;
use crate::models::org_membership::{MemberScopeSource, OrgMembership, OrgRole};
use crate::models::user::{User, UserType};
use crate::models::user_endpoint::UserEndpoint;
use crate::models::user_service::UserService;
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::platform_settings_service::BrokerPolicy;
use crate::services::provider_token_exchange_service::TokenExchangeCache;
use crate::services::ssh_service::SshSessionManager;

const TEST_DB_NAME_PREFIX: &str = "nyxid_test_";
const TEST_DB_MAX_NAME_LEN: usize = 63;
const TEST_DB_CREATED_AT_HEX_LEN: usize = 16;
const TEST_DB_RUN_ID_HEX_LEN: usize = 16;
const TEST_DB_U32_HEX_LEN: usize = 8;
const TEST_DB_MANAGED_SEPARATOR_COUNT: usize = 3;
const TEST_DB_MANAGED_FIELDS_LEN: usize = TEST_DB_CREATED_AT_HEX_LEN
    + TEST_DB_RUN_ID_HEX_LEN
    + TEST_DB_U32_HEX_LEN
    + TEST_DB_MANAGED_SEPARATOR_COUNT;
const MAX_TEST_DB_PREFIX_LEN: usize =
    TEST_DB_MAX_NAME_LEN - TEST_DB_NAME_PREFIX.len() - TEST_DB_MANAGED_FIELDS_LEN;
const MAX_LEGACY_TEST_DB_PREFIX_LEN: usize =
    TEST_DB_MAX_NAME_LEN - TEST_DB_NAME_PREFIX.len() - 1 - 36;
const TEST_DATABASE_URL_ENV: &str = "NYXID_TEST_DATABASE_URL";
const TEST_DB_RUN_LEASE: Duration = Duration::from_secs(15 * 60);
const TEST_DB_RUN_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const MANAGED_TEST_DB_MIN_AGE: Duration = Duration::from_secs(60 * 60);
const LEGACY_TEST_DB_QUARANTINE: Duration = Duration::from_secs(24 * 60 * 60);
const STALE_TEST_DB_LIST_TIMEOUT: Duration = Duration::from_secs(5);
const STALE_TEST_DB_METADATA_TIMEOUT: Duration = Duration::from_secs(5);
const STALE_TEST_DB_DROP_TIMEOUT: Duration = Duration::from_secs(3);
// A stale-drop claim fences producer renewal across the final lease check and
// bounded dropDatabase call. The lease is deliberately much longer than the
// drop timeout so ordinary scheduler stalls cannot reopen that race; expiry is
// only crash recovery for a sweeper that disappears while holding the claim.
const STALE_TEST_DB_DROP_CLAIM_LEASE: Duration = Duration::from_secs(30);
const STALE_TEST_DB_SWEEP_BUDGET: Duration = Duration::from_secs(45);
const STALE_TEST_DB_SWEEP_LEASE: Duration = Duration::from_secs(90);
const STALE_TEST_DB_SWEEP_COOLDOWN: Duration = Duration::from_secs(30);
const TEST_DB_EXIT_CLEANUP_BUDGET: Duration = Duration::from_secs(30);
const TEST_DB_CLEANUP_CLIENT_PARSE_TIMEOUT: Duration = Duration::from_secs(3);
const TEST_DB_CLIENT_HEARTBEAT_FREQUENCY: Duration = Duration::from_secs(1);
const TEST_DB_DROP_QUEUE_CAPACITY: usize = 256;
const TEST_DB_DROP_WORKER_COUNT: usize = 8;
const TEST_DB_DROP_RETRY_INTERVAL: Duration = Duration::from_millis(50);
const TEST_DB_LIFECYCLE_DROP_MAX_ATTEMPTS: usize = 3;
const TEST_DB_LIFECYCLE_DROP_RETRY_BACKOFF: Duration = Duration::from_millis(100);
const TEST_DB_RUNS_COLLECTION: &str = "__test_db_runs";
const TEST_DB_SWEEP_LEASES_COLLECTION: &str = "__test_db_sweep_leases";
const TEST_DB_LEGACY_CANDIDATES_COLLECTION: &str = "__test_db_legacy_candidates";
const TEST_DB_SWEEP_LEASE_ID: &str = "stale-test-database-sweep";

static TEST_DB_NAME_SEQUENCE: AtomicUsize = AtomicUsize::new(0);
static NEXT_PROCESS_LOCAL_STALE_SWEEP_AT_SECS: AtomicU64 = AtomicU64::new(0);
static TEST_DB_PROCESS: OnceLock<TestDbProcess> = OnceLock::new();
static TEST_DB_PINNED_URI: OnceLock<String> = OnceLock::new();
static TEST_DB_HEARTBEAT: std::sync::Mutex<Option<TestDbHeartbeat>> = std::sync::Mutex::new(None);

struct TestDbProcess {
    run_id: String,
    started_at_secs: u64,
}

struct TestDbHeartbeat {
    stop: SyncSender<()>,
    thread: std::thread::JoinHandle<()>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManagedTestDbName {
    created_at_secs: u64,
    run_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum StaleTestDbKind {
    Managed,
    Legacy,
}

/// Single shared database used by every probe to check write-readiness. Reusing
/// one fixed name (instead of a per-call UUID database) keeps the probe from
/// creating one throwaway database per `connect_test_database` call — historically
/// a major source of the leaked-`nyxid_test_*`-database pile that hammered
/// WiredTiger's file count. It stays within the `nyxid_test_` prefix (so a manual
/// "drop all nyxid_test_*" sweep still catches it) and is intentionally left in
/// place rather than dropped at exit: it is a single harmless database, and not
/// dropping it avoids a cross-process drop/insert race under per-test-process
/// runners such as cargo-nextest.
const TEST_DB_PROBE_NAME: &str = "nyxid_test_probe";

/// Connect to a fresh per-test MongoDB database.
///
/// `NYXID_TEST_DATABASE_URL` is authoritative when set. Otherwise this probes the
/// dev docker-compose mongod on `127.0.0.1:27018` first, then the CI-style mongod
/// on `127.0.0.1:27017`. Default candidates are gated by a fast TCP reachability
/// check, so a port with no listener is skipped in milliseconds instead of
/// stalling on the driver's server-selection timeout. Returns `None` when neither
/// default candidate is reachable so non-transactional integration tests retain
/// their existing optional-Mongo behavior. A configured override fails loudly
/// when unusable instead of silently falling back to a different database.
///
/// Deliberately NOT cached: a per-test client is required for correct llvm-cov
/// coverage measurement — a shared client broke under the runtime-per-test
/// harness (see #864). The TCP pre-check keeps per-test connects cheap.
///
/// Each per-test client's SDAM handler retains a database drop guard. After the
/// driver finishes tearing down the client (which can outlive the final external
/// Client/Database/Collection handle), the guard queues a drop on a
/// runtime-independent worker. A renewable process lease, process-exit retry,
/// and the cross-process stale sweep remain crash recovery.
pub(crate) async fn connect_test_database(prefix: &str) -> Option<mongodb::Database> {
    let db_name = new_test_db_name(prefix);
    let client = probe_test_mongo_client(&db_name, None).await?;

    Some(client.database(&db_name))
}

/// Connect to a fresh test database through a client with MongoDB command
/// monitoring enabled. Tests use this to prove read-only handler boundaries;
/// setup commands are observable too, so callers should clear their recorder
/// immediately before invoking the behavior under test.
pub(crate) async fn connect_test_database_with_command_handler(
    prefix: &str,
    handler: mongodb::event::EventHandler<mongodb::event::command::CommandEvent>,
) -> Option<mongodb::Database> {
    let db_name = new_test_db_name(prefix);
    let client = probe_test_mongo_client(&db_name, Some(handler)).await?;
    Some(client.database(&db_name))
}

/// Connect to a fresh database and prove that it supports multi-document
/// transactions. Transaction-dependent tests must use this helper instead of
/// conditionally returning when `connect_test_database` yields `None`.
///
/// The topology check catches a standalone mongod with a clear diagnostic. The
/// write-and-abort probe then verifies the actual session/transaction path, so a
/// misleading connection string or partially initialized replica set cannot
/// make an atomicity test pass without exercising transactions.
pub(crate) async fn connect_transaction_test_database(prefix: &str) -> mongodb::Database {
    let db = connect_test_database(prefix).await.unwrap_or_else(|| {
        panic!(
            "transaction test requires MongoDB; set {TEST_DATABASE_URL_ENV} to a writable replica-set or mongos URI"
        )
    });

    assert_transaction_test_topology(&db).await;
    db
}

async fn assert_transaction_test_topology(db: &mongodb::Database) {
    let hello = db
        .run_command(doc! { "hello": 1 })
        .await
        .unwrap_or_else(|error| {
            panic!(
                "transaction test could not inspect MongoDB topology via hello: {error}; set {TEST_DATABASE_URL_ENV} to a writable replica-set or mongos URI"
            )
        });
    let is_replica_set = hello.get_str("setName").is_ok();
    let is_mongos = hello.get_str("msg").is_ok_and(|msg| msg == "isdbgrid");
    assert!(
        is_replica_set || is_mongos,
        "transaction tests require a MongoDB replica set or mongos, but the configured server is standalone; set {TEST_DATABASE_URL_ENV} to a transaction-capable URI"
    );
    assert!(
        hello.get("logicalSessionTimeoutMinutes").is_some(),
        "transaction tests require MongoDB logical sessions; set {TEST_DATABASE_URL_ENV} to a transaction-capable URI"
    );

    let mut session = db.client().start_session().await.unwrap_or_else(|error| {
        panic!("transaction test could not start a MongoDB session: {error}")
    });
    session
        .start_transaction()
        .await
        .unwrap_or_else(|error| panic!("transaction test could not start a transaction: {error}"));
    let probe_id = Uuid::new_v4().to_string();
    let transaction_probe = db.collection::<mongodb::bson::Document>("__transaction_probe");
    if let Err(error) = transaction_probe
        .insert_one(doc! { "_id": probe_id })
        .session(&mut session)
        .await
    {
        let _ = session.abort_transaction().await;
        panic!(
            "transaction test MongoDB rejected a transactional write: {error}; set {TEST_DATABASE_URL_ENV} to a writable, initialized replica-set or mongos URI"
        );
    }
    session.abort_transaction().await.unwrap_or_else(|error| {
        panic!("transaction test could not abort its topology probe: {error}")
    });
}

/// Returns `true` when a TCP connection to `addr` succeeds quickly. A closed
/// local port returns `ECONNREFUSED` almost immediately, so this rejects a dead
/// probe candidate in ~milliseconds rather than paying the mongo server-selection
/// timeout. The timeout is only an upper bound for the pathological case of a
/// port that neither accepts nor refuses (not expected on loopback).
async fn test_mongo_port_reachable(addr: &str) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_millis(500),
            tokio::net::TcpStream::connect(addr),
        )
        .await,
        Ok(Ok(_))
    )
}

/// Probe both candidate mongods and return a connected client plus the URI that
/// won, so callers can register a fresh-client teardown that survives the test's
/// own tokio runtime being torn down. `command_event_handler` is independent of
/// the SDAM slot used to retain the per-database cleanup guard.
async fn probe_test_mongo_client(
    db_name: &str,
    command_event_handler: Option<
        mongodb::event::EventHandler<mongodb::event::command::CommandEvent>,
    >,
) -> Option<mongodb::Client> {
    if let Some(configured_uri) = std::env::var_os(TEST_DATABASE_URL_ENV) {
        let configured_uri = configured_uri
            .into_string()
            .unwrap_or_else(|_| panic!("{TEST_DATABASE_URL_ENV} must contain valid Unicode"));
        assert!(
            !configured_uri.trim().is_empty(),
            "{TEST_DATABASE_URL_ENV} must not be empty"
        );
        assert!(
            TEST_DB_PINNED_URI
                .get()
                .is_none_or(|pinned_uri| pinned_uri == &configured_uri),
            "{TEST_DATABASE_URL_ENV} cannot change after this test process has selected MongoDB"
        );
        let client = probe_test_mongo_uri(
            &configured_uri,
            db_name,
            command_event_handler.clone(),
        )
        .await
        .unwrap_or_else(|| {
            panic!(
                "{TEST_DATABASE_URL_ENV} is configured but MongoDB is not reachable and writable; refusing to fall back to a different test database"
            )
        });
        return Some(client);
    }

    // Heartbeat, stale-sweep metadata, lifecycle drops, and exit recovery must
    // all address the same server for the lifetime of the process. Once one
    // default candidate wins, fail closed if it disappears instead of silently
    // moving later databases to the other local mongod.
    if let Some(pinned_uri) = TEST_DB_PINNED_URI.get() {
        return probe_test_mongo_uri(pinned_uri, db_name, command_event_handler).await;
    }

    // (tcp address, client URI). 27018 is the dev docker-compose port; 27017 is
    // the CI service-container port. Probe order is no longer load-bearing — the
    // TCP pre-check below skips whichever candidate has no listener. Every probe
    // shares one fixed `TEST_DB_PROBE_NAME` database (unique doc id per call
    // below) instead of a per-call throwaway database.
    let candidates = [
        (
            "127.0.0.1:27018",
            format!(
                "mongodb://nyxid:nyxid_dev_password@127.0.0.1:27018/{TEST_DB_PROBE_NAME}?authSource=admin&directConnection=true"
            ),
        ),
        (
            "127.0.0.1:27017",
            format!("mongodb://127.0.0.1:27017/{TEST_DB_PROBE_NAME}?directConnection=true"),
        ),
    ];

    for (addr, uri) in candidates {
        // Fast-fail a port with no listener instead of blocking on server
        // selection before falling over to the next candidate.
        if !test_mongo_port_reachable(addr).await {
            continue;
        }

        if let Some(client) =
            probe_test_mongo_uri(&uri, db_name, command_event_handler.clone()).await
        {
            return Some(client);
        }
    }

    // A concurrent probe may have selected the other candidate after this
    // call passed the initial pinned-URI check. If this call lost that race,
    // retry the winner once instead of misreporting MongoDB as unavailable.
    if let Some(pinned_uri) = TEST_DB_PINNED_URI.get() {
        return probe_test_mongo_uri(pinned_uri, db_name, command_event_handler).await;
    }

    None
}

fn pin_test_db_uri_in(cell: &OnceLock<String>, uri: &str) -> bool {
    if let Some(pinned_uri) = cell.get() {
        return pinned_uri == uri;
    }
    let _ = cell.set(uri.to_string());
    cell.get().is_some_and(|pinned_uri| pinned_uri == uri)
}

fn pin_test_db_uri(uri: &str) -> bool {
    pin_test_db_uri_in(&TEST_DB_PINNED_URI, uri)
}

async fn probe_test_mongo_uri(
    uri: &str,
    db_name: &str,
    command_event_handler: Option<
        mongodb::event::EventHandler<mongodb::event::command::CommandEvent>,
    >,
) -> Option<mongodb::Client> {
    let Ok(mut options) = mongodb::options::ClientOptions::parse(uri).await else {
        return None;
    };
    // The TCP pre-check guards default candidates against dead-port stalls in
    // milliseconds. These more generous driver timeouts cover a real mongod and
    // remote explicit overrides. Under cargo llvm-cov, argon2 plus instrumentation
    // can starve the heartbeat monitor long enough to otherwise clear the pool.
    options.server_selection_timeout = Some(Duration::from_secs(30));
    options.connect_timeout = Some(Duration::from_secs(20));
    // The cleanup guard is released only after the driver's SDAM monitor exits.
    // Keep test clients near the driver's 500 ms minimum so teardown does not
    // inherit the production-default 10 second heartbeat delay under load.
    options.heartbeat_freq = Some(TEST_DB_CLIENT_HEARTBEAT_FREQUENCY);
    options.max_pool_size = Some(4);
    options.command_event_handler = command_event_handler;
    let drop_guard = Arc::new(TestDbDropGuard::new(uri, db_name));
    let guard_keepalive = Arc::clone(&drop_guard);
    options.sdam_event_handler = Some(mongodb::event::EventHandler::callback(move |_event| {
        let _ = Arc::strong_count(&guard_keepalive);
    }));
    let Ok(client) = mongodb::Client::with_options(options) else {
        return None;
    };
    let db = client.database(TEST_DB_PROBE_NAME);
    if db.run_command(doc! { "ping": 1 }).await.is_err() {
        return None;
    }

    // Unique doc id per call so concurrent probes against the shared probe
    // database don't collide on a duplicate `_id`.
    let probe = db.collection::<mongodb::bson::Document>("__probe");
    let probe_id = Uuid::new_v4().to_string();
    let write_ready = tokio::time::timeout(
        Duration::from_secs(5),
        probe.insert_one(doc! { "_id": probe_id.clone() }),
    )
    .await;
    if !matches!(write_ready, Ok(Ok(_))) {
        return None;
    }
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        probe.delete_one(doc! { "_id": probe_id }),
    )
    .await;

    // Pin only after this candidate proves writable, but before publishing a
    // run lease or registering a destructive cleanup target. A concurrent
    // probe that loses the race may retry the URI that won; it must never
    // register state on a second server.
    if !pin_test_db_uri(uri) {
        return None;
    }

    // Register before sweeping. A process that cannot publish its own live
    // lease must never decide that another process's databases are abandoned.
    let process = test_db_process();
    if !matches!(
        tokio::time::timeout(
            STALE_TEST_DB_METADATA_TIMEOUT,
            renew_test_db_run_record(
                &client,
                &process.run_id,
                process.started_at_secs,
                unix_time_secs(),
            ),
        )
        .await,
        Ok(Ok(true))
    ) {
        return None;
    }
    ensure_test_db_heartbeat(uri);
    sweep_stale_test_databases_once(&client, unix_time_secs()).await;
    register_test_db_for_cleanup(uri, db_name);
    drop_guard.arm();
    Some(client)
}

fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test database cleanup requires a system clock after the Unix epoch")
        .as_secs()
}

fn test_db_process() -> &'static TestDbProcess {
    TEST_DB_PROCESS.get_or_init(|| TestDbProcess {
        run_id: format!("{:016x}", Uuid::new_v4().as_u128() as u64),
        started_at_secs: unix_time_secs(),
    })
}

fn new_test_db_name(prefix: &str) -> String {
    let created_at_secs = unix_time_secs();
    let sequence = TEST_DB_NAME_SEQUENCE.fetch_add(1, Ordering::Relaxed) as u32;
    let process = test_db_process();

    format_test_db_name(prefix, created_at_secs, &process.run_id, sequence)
}

fn format_test_db_name(prefix: &str, created_at_secs: u64, run_id: &str, sequence: u32) -> String {
    debug_assert_eq!(run_id.len(), TEST_DB_RUN_ID_HEX_LEN);
    debug_assert!(
        run_id
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
    let name = format!(
        "{TEST_DB_NAME_PREFIX}{created_at_secs:016x}_{run_id}_{sequence:08x}_{}",
        sanitize_test_db_prefix(prefix)
    );
    debug_assert!(name.len() <= TEST_DB_MAX_NAME_LEN);
    name
}

fn parse_managed_test_db_name(name: &str) -> Option<ManagedTestDbName> {
    let suffix = name.strip_prefix(TEST_DB_NAME_PREFIX)?;
    let mut fields = suffix.splitn(4, '_');
    let created_at = fields.next()?;
    let run_id = fields.next()?;
    let sequence = fields.next()?;
    let prefix = fields.next()?;

    if created_at.len() != TEST_DB_CREATED_AT_HEX_LEN
        || run_id.len() != TEST_DB_RUN_ID_HEX_LEN
        || sequence.len() != TEST_DB_U32_HEX_LEN
        || prefix.is_empty()
        || prefix.len() > MAX_TEST_DB_PREFIX_LEN
        || !run_id
            .chars()
            .all(|character| character.is_ascii_hexdigit())
        || !prefix
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }

    let created_at_secs = u64::from_str_radix(created_at, 16).ok()?;
    u32::from_str_radix(sequence, 16).ok()?;
    Some(ManagedTestDbName {
        created_at_secs,
        run_id: run_id.to_ascii_lowercase(),
    })
}

fn is_legacy_test_db_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(TEST_DB_NAME_PREFIX) else {
        return false;
    };
    let Some((prefix, uuid)) = suffix.rsplit_once('_') else {
        return false;
    };
    let Ok(parsed_uuid) = Uuid::parse_str(uuid) else {
        return false;
    };

    !prefix.is_empty()
        && prefix.len() <= MAX_LEGACY_TEST_DB_PREFIX_LEN
        && sanitize_test_db_prefix_with_limit(prefix, MAX_LEGACY_TEST_DB_PREFIX_LEN) == prefix
        && uuid.len() == 36
        && parsed_uuid.get_version() == Some(uuid::Version::Random)
        && parsed_uuid.hyphenated().to_string() == uuid
}

fn managed_test_db_is_eligible(
    metadata: &ManagedTestDbName,
    now_secs: u64,
    active_run_ids: &HashSet<String>,
) -> bool {
    now_secs
        .checked_sub(metadata.created_at_secs)
        .is_some_and(|age_secs| age_secs >= MANAGED_TEST_DB_MIN_AGE.as_secs())
        && !active_run_ids.contains(&metadata.run_id)
}

fn legacy_test_db_is_eligible(first_seen_at_secs: u64, now_secs: u64) -> bool {
    now_secs
        .checked_sub(first_seen_at_secs)
        .is_some_and(|age_secs| age_secs >= LEGACY_TEST_DB_QUARANTINE.as_secs())
}

fn test_db_metadata_collection(
    client: &mongodb::Client,
    collection_name: &str,
) -> mongodb::Collection<Document> {
    client
        .database(TEST_DB_PROBE_NAME)
        .collection(collection_name)
}

fn bson_secs(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

async fn renew_test_db_run_record(
    client: &mongodb::Client,
    run_id: &str,
    started_at_secs: u64,
    now_secs: u64,
) -> Result<bool, mongodb::error::Error> {
    renew_test_db_run_record_in_collection(
        &test_db_metadata_collection(client, TEST_DB_RUNS_COLLECTION),
        run_id,
        started_at_secs,
        now_secs,
    )
    .await
}

async fn renew_test_db_run_record_in_collection(
    run_records: &mongodb::Collection<Document>,
    run_id: &str,
    started_at_secs: u64,
    now_secs: u64,
) -> Result<bool, mongodb::error::Error> {
    let lease_until_secs = now_secs.saturating_add(TEST_DB_RUN_LEASE.as_secs());
    let initial_lease = run_records
        .update_one(
            doc! { "_id": run_id },
            doc! {
                "$setOnInsert": {
                    "started_at_secs": bson_secs(started_at_secs),
                    "process_id": i64::from(std::process::id()),
                    "heartbeat_at_secs": bson_secs(now_secs),
                    "lease_until_secs": bson_secs(lease_until_secs),
                },
            },
        )
        .upsert(true)
        .await;
    match initial_lease {
        Ok(result) if result.upserted_id.is_some() => return Ok(true),
        Ok(_) => {}
        Err(error) if is_duplicate_key_error(&error) => {}
        Err(error) => return Err(error),
    }

    // Renewal and stale-drop claiming use mutually exclusive predicates on the
    // same producer row. No upsert is allowed here: a losing renewal must return
    // false instead of racing another first writer into a duplicate-key error.
    let result = run_records
        .update_one(
            doc! {
                "_id": run_id,
                "$or": [
                    {
                        "cleanup_claim_id": { "$exists": false },
                        "cleanup_claim_until_secs": { "$exists": false },
                    },
                    {
                        "cleanup_claim_id": { "$type": "string" },
                        "cleanup_claim_until_secs": {
                            "$type": "long",
                            "$lte": bson_secs(now_secs),
                        },
                    },
                ],
            },
            doc! {
                "$set": {
                    "heartbeat_at_secs": bson_secs(now_secs),
                    "lease_until_secs": bson_secs(lease_until_secs),
                },
                "$unset": {
                    "cleanup_claim_id": "",
                    "cleanup_claim_until_secs": "",
                },
            },
        )
        .await?;
    Ok(result.matched_count == 1)
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    match error.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command) => command.code == 11000,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) => {
            write.code == 11000
        }
        _ => false,
    }
}

fn ensure_test_db_heartbeat(uri: &str) {
    let mut heartbeat = TEST_DB_HEARTBEAT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if heartbeat
        .as_ref()
        .is_some_and(|current| !current.thread.is_finished())
    {
        return;
    }
    if let Some(finished) = heartbeat.take() {
        let _ = finished.thread.join();
    }

    let (stop, receiver) = sync_channel(1);
    let uri = uri.to_string();
    let process = test_db_process();
    let run_id = process.run_id.clone();
    let started_at_secs = process.started_at_secs;
    let Ok(thread) = std::thread::Builder::new()
        .name("nyxid-test-db-heartbeat".to_string())
        .spawn(move || test_db_heartbeat_loop(uri, run_id, started_at_secs, receiver))
    else {
        return;
    };
    *heartbeat = Some(TestDbHeartbeat { stop, thread });
}

fn stop_test_db_heartbeat() {
    let heartbeat = TEST_DB_HEARTBEAT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(heartbeat) = heartbeat {
        let _ = heartbeat.stop.try_send(());
        let _ = heartbeat.thread.join();
    }
}

fn test_db_heartbeat_loop(uri: String, run_id: String, started_at_secs: u64, stop: Receiver<()>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let Some(client) = runtime.block_on(test_db_cleanup_client(&uri)) else {
        return;
    };

    loop {
        match stop.recv_timeout(TEST_DB_RUN_HEARTBEAT_INTERVAL) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = runtime.block_on(tokio::time::timeout(
                    STALE_TEST_DB_METADATA_TIMEOUT,
                    renew_test_db_run_record(&client, &run_id, started_at_secs, unix_time_secs()),
                ));
            }
        }
    }
}

async fn acquire_test_db_sweep_lease(
    collection: &mongodb::Collection<Document>,
    owner_id: &str,
    now_secs: u64,
) -> bool {
    if collection
        .update_one(
            doc! { "_id": TEST_DB_SWEEP_LEASE_ID },
            doc! {
                "$setOnInsert": {
                    "owner_id": "",
                    "lease_until_secs": 0_i64,
                },
            },
        )
        .upsert(true)
        .await
        .is_err()
    {
        return false;
    }

    let lease_until_secs = now_secs.saturating_add(STALE_TEST_DB_SWEEP_LEASE.as_secs());
    collection
        .find_one_and_update(
            doc! {
                "_id": TEST_DB_SWEEP_LEASE_ID,
                "$or": [
                    { "lease_until_secs": { "$lte": bson_secs(now_secs) } },
                    { "lease_until_secs": { "$exists": false } },
                ],
            },
            doc! {
                "$set": {
                    "owner_id": owner_id,
                    "lease_until_secs": bson_secs(lease_until_secs),
                    "started_at_secs": bson_secs(now_secs),
                },
            },
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await
        .ok()
        .flatten()
        .and_then(|document| document.get_str("owner_id").ok().map(str::to_string))
        .is_some_and(|claimed_owner| claimed_owner == owner_id)
}

async fn release_test_db_sweep_lease(
    collection: &mongodb::Collection<Document>,
    owner_id: &str,
    now_secs: u64,
) {
    let next_sweep_at_secs = now_secs.saturating_add(STALE_TEST_DB_SWEEP_COOLDOWN.as_secs());
    let _ = tokio::time::timeout(
        STALE_TEST_DB_METADATA_TIMEOUT,
        collection.update_one(
            doc! {
                "_id": TEST_DB_SWEEP_LEASE_ID,
                "owner_id": owner_id,
            },
            doc! {
                "$set": {
                    "owner_id": "",
                    "lease_until_secs": bson_secs(next_sweep_at_secs),
                    "completed_at_secs": bson_secs(now_secs),
                },
            },
        ),
    )
    .await;
}

async fn active_test_db_run_ids(
    client: &mongodb::Client,
    run_ids: &HashSet<String>,
    now_secs: u64,
) -> Option<HashSet<String>> {
    if run_ids.is_empty() {
        return Some(HashSet::new());
    }
    let run_ids: Vec<String> = run_ids.iter().cloned().collect();
    let query = async {
        let cursor = test_db_metadata_collection(client, TEST_DB_RUNS_COLLECTION)
            .find(doc! {
                "_id": { "$in": run_ids },
                "lease_until_secs": { "$gt": bson_secs(now_secs) },
            })
            .await?;
        let documents: Vec<Document> = cursor.try_collect().await?;
        Ok::<HashSet<String>, mongodb::error::Error>(
            documents
                .into_iter()
                .filter_map(|document| document.get_str("_id").ok().map(str::to_string))
                .collect(),
        )
    };
    match tokio::time::timeout(STALE_TEST_DB_METADATA_TIMEOUT, query).await {
        Ok(Ok(active)) => Some(active),
        _ => None,
    }
}

async fn claim_expired_test_db_run(
    run_records: &mongodb::Collection<Document>,
    run_id: &str,
    claim_id: &str,
    now_secs: u64,
) -> bool {
    let claim_until_secs = now_secs.saturating_add(STALE_TEST_DB_DROP_CLAIM_LEASE.as_secs());
    let claim = run_records
        .find_one_and_update(
            doc! {
                "_id": run_id,
                "lease_until_secs": {
                    "$type": "long",
                    "$lte": bson_secs(now_secs),
                },
                "$or": [
                    {
                        "cleanup_claim_id": { "$exists": false },
                        "cleanup_claim_until_secs": { "$exists": false },
                    },
                    {
                        "cleanup_claim_id": { "$type": "string" },
                        "cleanup_claim_until_secs": {
                            "$type": "long",
                            "$lte": bson_secs(now_secs),
                        },
                    },
                ],
            },
            doc! {
                "$set": {
                    "cleanup_claim_id": claim_id,
                    "cleanup_claim_until_secs": bson_secs(claim_until_secs),
                },
            },
        )
        .return_document(mongodb::options::ReturnDocument::After);
    matches!(
        tokio::time::timeout(STALE_TEST_DB_METADATA_TIMEOUT, claim).await,
        Ok(Ok(Some(document)))
            if document.get_str("cleanup_claim_id").ok() == Some(claim_id)
    )
}

async fn release_test_db_drop_claim(
    run_records: &mongodb::Collection<Document>,
    run_id: &str,
    claim_id: &str,
) -> bool {
    matches!(
        tokio::time::timeout(
            STALE_TEST_DB_METADATA_TIMEOUT,
            run_records.update_one(
                doc! {
                    "_id": run_id,
                    "cleanup_claim_id": claim_id,
                },
                doc! {
                    "$unset": {
                        "cleanup_claim_id": "",
                        "cleanup_claim_until_secs": "",
                    },
                },
            ),
        )
        .await,
        Ok(Ok(result)) if result.modified_count == 1
    )
}

async fn release_test_db_drop_claim_after_confirmed_drop(
    run_records: &mongodb::Collection<Document>,
    run_id: &str,
    claim_id: &str,
    dropped: bool,
) -> bool {
    if !dropped {
        return false;
    }

    release_test_db_drop_claim(run_records, run_id, claim_id).await
}

async fn legacy_test_db_first_seen_at(
    client: &mongodb::Client,
    database_name: &str,
    now_secs: u64,
) -> Option<u64> {
    let collection = test_db_metadata_collection(client, TEST_DB_LEGACY_CANDIDATES_COLLECTION);
    let update = collection
        .find_one_and_update(
            doc! { "_id": database_name },
            doc! {
                "$set": { "last_seen_at_secs": bson_secs(now_secs) },
                "$setOnInsert": { "first_seen_at_secs": bson_secs(now_secs) },
            },
        )
        .upsert(true)
        .return_document(mongodb::options::ReturnDocument::After);
    match tokio::time::timeout(STALE_TEST_DB_METADATA_TIMEOUT, update).await {
        Ok(Ok(Some(document))) => document
            .get_i64("first_seen_at_secs")
            .ok()
            .and_then(|value| u64::try_from(value).ok()),
        _ => None,
    }
}

async fn prune_test_db_metadata_collections(
    run_records: &mongodb::Collection<Document>,
    legacy_candidates: &mongodb::Collection<Document>,
    now_secs: u64,
    referenced_run_ids: &HashSet<String>,
    present_legacy_database_names: &HashSet<String>,
) {
    let referenced_run_ids: Vec<String> = referenced_run_ids.iter().cloned().collect();
    let present_legacy_database_names: Vec<String> =
        present_legacy_database_names.iter().cloned().collect();

    // Both deletes fail closed: a run record must carry an explicitly expired
    // lease and must have no managed database referent; a legacy quarantine row
    // survives for as long as its exact database name is still present.
    let _ = tokio::join!(
        tokio::time::timeout(
            STALE_TEST_DB_METADATA_TIMEOUT,
            run_records.delete_many(doc! {
                "_id": {
                    "$type": "string",
                    "$nin": referenced_run_ids,
                },
                "lease_until_secs": {
                    "$type": "long",
                    "$lte": bson_secs(now_secs),
                },
            }),
        ),
        tokio::time::timeout(
            STALE_TEST_DB_METADATA_TIMEOUT,
            legacy_candidates.delete_many(doc! {
                "_id": {
                    "$type": "string",
                    "$nin": present_legacy_database_names,
                },
            }),
        ),
    );
}

async fn sweep_stale_test_databases_once(client: &mongodb::Client, now_secs: u64) {
    if !claim_process_local_stale_sweep_attempt(&NEXT_PROCESS_LOCAL_STALE_SWEEP_AT_SECS, now_secs) {
        return;
    }
    let owner_id = test_db_process().run_id.clone();
    let lease_collection = test_db_metadata_collection(client, TEST_DB_SWEEP_LEASES_COLLECTION);
    let acquired = matches!(
        tokio::time::timeout(
            STALE_TEST_DB_METADATA_TIMEOUT,
            acquire_test_db_sweep_lease(&lease_collection, &owner_id, now_secs),
        )
        .await,
        Ok(true)
    );
    if !acquired {
        return;
    }

    sweep_stale_test_databases_under_lease(client, now_secs).await;
    release_test_db_sweep_lease(&lease_collection, &owner_id, unix_time_secs()).await;
}

fn claim_process_local_stale_sweep_attempt(next_attempt_at: &AtomicU64, now_secs: u64) -> bool {
    let next = now_secs.saturating_add(STALE_TEST_DB_SWEEP_COOLDOWN.as_secs());
    let mut observed = next_attempt_at.load(Ordering::Relaxed);
    loop {
        if now_secs < observed {
            return false;
        }
        match next_attempt_at.compare_exchange_weak(
            observed,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(actual) => observed = actual,
        }
    }
}

/// Recover logical test databases left by killed or crashed test processes.
///
/// Recovery deliberately starts only after a writable MongoDB probe succeeds.
/// It cannot make an already-unstartable mongod recover from file-descriptor or
/// dbpath exhaustion, and dropping logical databases does not guarantee that
/// WiredTiger files are reclaimed. An isolated test dbpath that has reached
/// that state still requires out-of-process container/dbpath recreation.
async fn sweep_stale_test_databases_under_lease(client: &mongodb::Client, now_secs: u64) {
    let deadline = tokio::time::Instant::now() + STALE_TEST_DB_SWEEP_BUDGET;
    let Ok(Ok(database_names)) =
        tokio::time::timeout(STALE_TEST_DB_LIST_TIMEOUT, client.list_database_names()).await
    else {
        return;
    };

    let mut managed_databases = Vec::new();
    let mut legacy_databases = Vec::new();
    let mut managed_run_ids = HashSet::new();
    let mut managed_run_ref_counts = HashMap::new();
    let mut present_legacy_database_names = HashSet::new();
    for name in database_names {
        if let Some(metadata) = parse_managed_test_db_name(&name) {
            managed_run_ids.insert(metadata.run_id.clone());
            *managed_run_ref_counts
                .entry(metadata.run_id.clone())
                .or_insert(0_usize) += 1;
            managed_databases.push((metadata, name));
        } else if is_legacy_test_db_name(&name) {
            present_legacy_database_names.insert(name.clone());
            legacy_databases.push(name);
        }
    }

    // Fail closed for managed names if the authoritative live-run query fails.
    let active_run_ids = active_test_db_run_ids(client, &managed_run_ids, now_secs).await;
    let mut stale_databases: Vec<(u64, String, StaleTestDbKind, Option<String>)> = active_run_ids
        .map(|active_run_ids| {
            managed_databases
                .into_iter()
                .filter(|(metadata, _)| {
                    managed_test_db_is_eligible(metadata, now_secs, &active_run_ids)
                })
                .map(|(metadata, name)| {
                    (
                        metadata.created_at_secs,
                        name,
                        StaleTestDbKind::Managed,
                        Some(metadata.run_id),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    // Legacy names have no producer timestamp or run lease. Quarantine the
    // exact historical `<prefix>_<uuid>` shape from first observation before
    // considering it stale; arbitrary `nyxid_test_*` names are never adopted.
    for name in legacy_databases {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        let Some(first_seen_at_secs) = legacy_test_db_first_seen_at(client, &name, now_secs).await
        else {
            continue;
        };
        if legacy_test_db_is_eligible(first_seen_at_secs, now_secs) {
            stale_databases.push((first_seen_at_secs, name, StaleTestDbKind::Legacy, None));
        }
    }

    stale_databases.sort_unstable();
    let run_records = test_db_metadata_collection(client, TEST_DB_RUNS_COLLECTION);
    for (_, name, kind, run_id) in stale_databases {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        let cleanup_claim = if kind == StaleTestDbKind::Managed {
            let Some(run_id) = run_id.as_deref() else {
                continue;
            };
            // The active-run set above is only a batching snapshot. Atomically
            // claim an explicitly expired producer lease before dropDatabase;
            // producer renewal uses the same record predicate and therefore
            // cannot slip between this final check and the destructive call.
            // Missing, malformed, claimed, or unreadable state fails closed.
            let claim_id = Uuid::new_v4().to_string();
            if !claim_expired_test_db_run(&run_records, run_id, &claim_id, unix_time_secs()).await {
                continue;
            }
            Some((run_id.to_string(), claim_id))
        } else {
            None
        };
        let dropped = matches!(
            tokio::time::timeout(STALE_TEST_DB_DROP_TIMEOUT, client.database(&name).drop(),).await,
            Ok(Ok(()))
        );
        if let Some((run_id, claim_id)) = cleanup_claim.as_ref() {
            let _ = release_test_db_drop_claim_after_confirmed_drop(
                &run_records,
                run_id,
                claim_id,
                dropped,
            )
            .await;
        }
        if !dropped {
            continue;
        }
        match kind {
            StaleTestDbKind::Managed => {
                let Some(run_id) = run_id else {
                    continue;
                };
                if let Some(reference_count) = managed_run_ref_counts.get_mut(&run_id) {
                    *reference_count = reference_count.saturating_sub(1);
                    if *reference_count == 0 {
                        managed_run_ref_counts.remove(&run_id);
                    }
                }
            }
            StaleTestDbKind::Legacy => {
                present_legacy_database_names.remove(&name);
                let _ = tokio::time::timeout(
                    STALE_TEST_DB_METADATA_TIMEOUT,
                    test_db_metadata_collection(client, TEST_DB_LEGACY_CANDIDATES_COLLECTION)
                        .delete_one(doc! { "_id": name }),
                )
                .await;
            }
        }
    }

    let referenced_run_ids: HashSet<String> = managed_run_ref_counts.into_keys().collect();
    let legacy_candidates =
        test_db_metadata_collection(client, TEST_DB_LEGACY_CANDIDATES_COLLECTION);
    let prune_metadata = prune_test_db_metadata_collections(
        &run_records,
        &legacy_candidates,
        now_secs,
        &referenced_run_ids,
        &present_legacy_database_names,
    );
    let _ = tokio::time::timeout_at(deadline, prune_metadata).await;
}

async fn test_db_cleanup_client(uri: &str) -> Option<mongodb::Client> {
    let parse = async { mongodb::options::ClientOptions::parse(uri).await };
    let Some(Ok(mut options)) =
        test_db_cleanup_operation_with_timeout(TEST_DB_CLEANUP_CLIENT_PARSE_TIMEOUT, parse).await
    else {
        return None;
    };
    options.server_selection_timeout = Some(Duration::from_secs(3));
    options.connect_timeout = Some(Duration::from_secs(3));
    options.max_pool_size = Some(2);
    mongodb::Client::with_options(options).ok()
}

async fn test_db_cleanup_operation_with_timeout<T>(
    timeout: Duration,
    operation: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::time::timeout(timeout, operation).await.ok()
}

/// Per-process recovery registry for databases that have not yet been dropped
/// by their per-client lifecycle guard. Process exit retries the remaining set.
///
static TEST_DB_CLEANUP: OnceLock<std::sync::Mutex<TestDbCleanup>> = OnceLock::new();
static TEST_DB_DROP_WORKERS: OnceLock<Option<TestDbDropWorkers>> = OnceLock::new();
static TEST_DB_DROP_OBSERVERS: OnceLock<std::sync::Mutex<HashMap<String, SyncSender<bool>>>> =
    OnceLock::new();

struct TestDbCleanup {
    /// Connection URI (with credentials) of the mongod that owns the test
    /// databases, captured from the first successful probe. Every test database
    /// in a run lives on the same server, so one URI is enough to reconnect a
    /// fresh client at exit.
    uri: Option<String>,
    /// Databases created this run that still need a successful drop.
    db_names: HashSet<String>,
    /// Guards one-time `atexit` registration.
    hook_installed: bool,
}

impl TestDbCleanup {
    fn new() -> Self {
        Self {
            uri: None,
            db_names: HashSet::new(),
            hook_installed: false,
        }
    }

    fn register(&mut self, uri: &str, db_name: &str) {
        if self.uri.is_none() {
            self.uri = Some(uri.to_string());
        }
        self.db_names.insert(db_name.to_string());
    }

    fn complete_lifecycle_drop(&mut self, db_name: &str, dropped: bool) {
        if dropped {
            self.db_names.remove(db_name);
        }
    }

    fn is_registered(&self, db_name: &str) -> bool {
        self.db_names.contains(db_name)
    }
}

struct TestDbDropGuard {
    uri: String,
    db_name: String,
    armed: AtomicBool,
}

impl TestDbDropGuard {
    fn new(uri: &str, db_name: &str) -> Self {
        Self {
            uri: uri.to_string(),
            db_name: db_name.to_string(),
            armed: AtomicBool::new(false),
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }
}

impl Drop for TestDbDropGuard {
    fn drop(&mut self) {
        if !self.armed.swap(false, Ordering::AcqRel) {
            return;
        }
        let uri = self.uri.clone();
        let db_name = self.db_name.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            enqueue_test_db_lifecycle_drop(uri, db_name);
        }));
    }
}

#[derive(Clone)]
struct TestDbDropJob {
    uri: String,
    db_name: String,
}

struct TestDbDropWorkers {
    job_sender: SyncSender<TestDbDropJob>,
    retry: Option<TestDbDropRetry>,
}

struct TestDbDropRetry {
    wake_sender: SyncSender<()>,
    jobs: Arc<std::sync::Mutex<HashMap<String, TestDbDropJob>>>,
}

fn enqueue_test_db_lifecycle_drop(uri: String, db_name: String) {
    let workers = TEST_DB_DROP_WORKERS.get_or_init(start_test_db_drop_workers);
    let queued = workers.as_ref().is_some_and(|workers| {
        enqueue_test_db_drop_job(
            workers,
            TestDbDropJob {
                uri,
                db_name: db_name.clone(),
            },
        )
    });
    if !queued {
        notify_test_db_drop_observer(&db_name, false);
    }
}

fn enqueue_test_db_drop_job(workers: &TestDbDropWorkers, job: TestDbDropJob) -> bool {
    match workers.job_sender.try_send(job) {
        Ok(()) => true,
        Err(TrySendError::Full(job)) => workers
            .retry
            .as_ref()
            .is_some_and(|retry| defer_test_db_drop_job(retry, job)),
        Err(TrySendError::Disconnected(_)) => false,
    }
}

fn defer_test_db_drop_job(retry: &TestDbDropRetry, job: TestDbDropJob) -> bool {
    let db_name = job.db_name.clone();
    retry
        .jobs
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(db_name.clone(), job);

    match retry.wake_sender.try_send(()) {
        Ok(()) | Err(TrySendError::Full(())) => true,
        Err(TrySendError::Disconnected(())) => {
            retry
                .jobs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&db_name);
            false
        }
    }
}

fn start_test_db_drop_workers() -> Option<TestDbDropWorkers> {
    let (sender, receiver) = sync_channel(TEST_DB_DROP_QUEUE_CAPACITY);
    let receiver = Arc::new(std::sync::Mutex::new(receiver));
    let mut started = 0_usize;

    for worker_index in 0..TEST_DB_DROP_WORKER_COUNT {
        let receiver = Arc::clone(&receiver);
        if std::thread::Builder::new()
            .name(format!("nyxid-test-db-drop-{worker_index}"))
            .spawn(move || test_db_drop_worker(receiver))
            .is_ok()
        {
            started += 1;
        }
    }

    if started == 0 {
        return None;
    }

    let retry = start_test_db_drop_retry_coordinator(sender.clone());
    Some(TestDbDropWorkers {
        job_sender: sender,
        retry,
    })
}

fn start_test_db_drop_retry_coordinator(
    job_sender: SyncSender<TestDbDropJob>,
) -> Option<TestDbDropRetry> {
    let jobs = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let coordinator_jobs = Arc::clone(&jobs);
    let (wake_sender, wake_receiver) = sync_channel(1);
    let started = std::thread::Builder::new()
        .name("nyxid-test-db-drop-retry".to_string())
        .spawn(move || test_db_drop_retry_coordinator(job_sender, coordinator_jobs, wake_receiver))
        .is_ok();

    started.then_some(TestDbDropRetry { wake_sender, jobs })
}

fn test_db_drop_retry_coordinator(
    job_sender: SyncSender<TestDbDropJob>,
    jobs: Arc<std::sync::Mutex<HashMap<String, TestDbDropJob>>>,
    wake_receiver: Receiver<()>,
) {
    while wake_receiver.recv().is_ok() {
        loop {
            let retry_result = {
                let mut jobs = jobs.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let Some(db_name) = jobs.keys().next().cloned() else {
                    break;
                };
                let job = jobs
                    .get(&db_name)
                    .expect("retry job selected from the same registry")
                    .clone();
                match job_sender.try_send(job) {
                    Ok(()) => {
                        jobs.remove(&db_name);
                        Ok(true)
                    }
                    Err(TrySendError::Full(_)) => Ok(false),
                    Err(TrySendError::Disconnected(_)) => Err(()),
                }
            };

            match retry_result {
                Ok(true) => continue,
                Ok(false) => std::thread::sleep(TEST_DB_DROP_RETRY_INTERVAL),
                Err(()) => return,
            }
        }
    }
}

fn receive_test_db_drop_job(
    receiver: &std::sync::Mutex<Receiver<TestDbDropJob>>,
) -> Option<TestDbDropJob> {
    receiver
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .recv()
        .ok()
}

fn test_db_drop_worker(receiver: Arc<std::sync::Mutex<Receiver<TestDbDropJob>>>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let mut clients = HashMap::<String, mongodb::Client>::new();
    while let Some(job) = receive_test_db_drop_job(&receiver) {
        let client = if let Some(client) = clients.get(&job.uri) {
            Some(client.clone())
        } else {
            let client = runtime.block_on(test_db_cleanup_client(&job.uri));
            if let Some(client) = client.as_ref() {
                clients.insert(job.uri.clone(), client.clone());
            }
            client
        };
        let dropped = client.is_some_and(|client| {
            runtime.block_on(retry_test_db_lifecycle_drop(
                TEST_DB_LIFECYCLE_DROP_RETRY_BACKOFF,
                || {
                    let client = client.clone();
                    let db_name = job.db_name.clone();
                    async move {
                        matches!(
                            tokio::time::timeout(
                                STALE_TEST_DB_DROP_TIMEOUT,
                                client.database(&db_name).drop(),
                            )
                            .await,
                            Ok(Ok(()))
                        )
                    }
                },
            ))
        });
        complete_test_db_lifecycle_drop(&job.db_name, dropped);
        notify_test_db_drop_observer(&job.db_name, dropped);
    }
}

async fn retry_test_db_lifecycle_drop<F, Fut>(backoff: Duration, mut attempt: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for attempt_index in 0..TEST_DB_LIFECYCLE_DROP_MAX_ATTEMPTS {
        if attempt().await {
            return true;
        }
        if attempt_index + 1 < TEST_DB_LIFECYCLE_DROP_MAX_ATTEMPTS {
            tokio::time::sleep(backoff).await;
        }
    }
    false
}

fn register_test_db_for_cleanup(uri: &str, db_name: &str) {
    TEST_DB_DROP_WORKERS.get_or_init(start_test_db_drop_workers);
    let cell = TEST_DB_CLEANUP.get_or_init(|| std::sync::Mutex::new(TestDbCleanup::new()));
    let mut guard = cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.register(uri, db_name);
    if !guard.hook_installed {
        guard.hook_installed = true;
        // SAFETY: `atexit` registers a C callback invoked once at normal process
        // termination (the Rust test harness exits via `process::exit`). The
        // callback only reads this global registry and performs best-effort,
        // panic-guarded, time-bounded cleanup; it never unwinds across the FFI
        // boundary and touches no other process state.
        unsafe {
            libc::atexit(drop_test_databases_at_exit);
        }
    }
}

fn complete_test_db_lifecycle_drop(db_name: &str, dropped: bool) {
    let Some(cell) = TEST_DB_CLEANUP.get() else {
        return;
    };
    cell.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .complete_lifecycle_drop(db_name, dropped);
}

fn test_db_is_registered_for_cleanup(db_name: &str) -> bool {
    TEST_DB_CLEANUP.get().is_some_and(|cell| {
        cell.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_registered(db_name)
    })
}

fn observe_test_db_drop(db_name: &str) -> Receiver<bool> {
    let (sender, receiver) = sync_channel(1);
    let previous = TEST_DB_DROP_OBSERVERS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(db_name.to_string(), sender);
    assert!(previous.is_none(), "duplicate test DB drop observer");
    receiver
}

fn notify_test_db_drop_observer(db_name: &str, dropped: bool) {
    let Some(observers) = TEST_DB_DROP_OBSERVERS.get() else {
        return;
    };
    let observer = observers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(db_name);
    if let Some(observer) = observer {
        let _ = observer.try_send(dropped);
    }
}

fn test_db_exit_cleanup_can_delete_run_record(drop_pass_result: Option<bool>) -> bool {
    matches!(drop_pass_result, Some(true))
}

/// Drop every database this test process created. Best-effort: connection or
/// drop failures are swallowed so a flaky or absent mongod never aborts process
/// exit. The entire drop pass is bounded; leftovers remain associated with an
/// expired run lease for a later cross-process sweep.
extern "C" fn drop_test_databases_at_exit() {
    // Never unwind across the FFI boundary — a panic escaping an `atexit`
    // callback would abort the process.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stop_test_db_heartbeat();
        let Some(cell) = TEST_DB_CLEANUP.get() else {
            return;
        };
        let (uri, db_names) = {
            let mut guard = cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            (guard.uri.clone(), std::mem::take(&mut guard.db_names))
        };
        let Some(uri) = uri else {
            return;
        };
        // Run the teardown on a freshly spawned OS thread and join it. `atexit`
        // fires *after* the main thread's thread-local storage has been
        // destroyed, so building or driving a tokio runtime on the main thread
        // panics inside `std::thread::current()`. A new thread has intact
        // thread-local state; joining keeps the process alive until cleanup
        // finishes (bounded by the per-drop timeouts below).
        let _ = std::thread::spawn(move || {
            // A fresh single-threaded runtime + client, independent of whatever
            // runtime the tests used (already torn down by now).
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let Some(client) = test_db_cleanup_client(&uri).await else {
                    return;
                };
                let process = test_db_process();
                let runs = test_db_metadata_collection(&client, TEST_DB_RUNS_COLLECTION);
                let _ = tokio::time::timeout(
                    STALE_TEST_DB_METADATA_TIMEOUT,
                    runs.update_one(
                        doc! { "_id": &process.run_id },
                        doc! {
                            "$set": {
                                "heartbeat_at_secs": bson_secs(unix_time_secs()),
                                "lease_until_secs": 0_i64,
                            },
                        },
                    ),
                )
                .await;

                let cleanup = async {
                    let mut all_dropped = true;
                    for name in db_names {
                        all_dropped &= matches!(
                            tokio::time::timeout(
                                STALE_TEST_DB_DROP_TIMEOUT,
                                client.database(&name).drop(),
                            )
                            .await,
                            Ok(Ok(()))
                        );
                    }
                    all_dropped
                };
                let drop_pass_result = tokio::time::timeout(TEST_DB_EXIT_CLEANUP_BUDGET, cleanup)
                    .await
                    .ok();
                if test_db_exit_cleanup_can_delete_run_record(drop_pass_result) {
                    let _ = tokio::time::timeout(
                        STALE_TEST_DB_METADATA_TIMEOUT,
                        runs.delete_one(doc! { "_id": &process.run_id }),
                    )
                    .await;
                }
            });
        })
        .join();
    }));
}

fn sanitize_test_db_prefix(prefix: &str) -> String {
    sanitize_test_db_prefix_with_limit(prefix, MAX_TEST_DB_PREFIX_LEN)
}

fn sanitize_test_db_prefix_with_limit(prefix: &str, max_len: usize) -> String {
    let sanitized: String = prefix
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .take(max_len)
        .collect();
    let sanitized = sanitized.trim_matches('_');

    if sanitized.is_empty() {
        "db".to_string()
    } else {
        sanitized.to_string()
    }
}

/// Build an `AppConfig` suitable for unit tests that need access to the
/// config's side-effect-free fields (encryption key, limits, feature flags).
pub(crate) fn test_app_config() -> AppConfig {
    AppConfig {
        port: 3001,
        base_url: "http://localhost:3001".to_string(),
        frontend_url: "http://localhost:3000".to_string(),
        cors_allowed_origins: vec![],
        csrf_trusted_origins: vec![],
        database_url: "mongodb://ignored-for-test".to_string(),
        database_max_connections: 10,
        environment: "test".to_string(),
        jwt_private_key_path: "keys/private.pem".to_string(),
        jwt_public_key_path: "keys/public.pem".to_string(),
        jwt_issuer: "nyxid".to_string(),
        jwt_access_ttl_secs: 900,
        jwt_relay_reply_ttl_secs: 1800,
        jwt_relay_callback_ttl_secs: 300,
        jwt_relay_access_ttl_secs: 300,
        jwt_assistant_forward_ttl_secs: 300,
        jwt_refresh_ttl_secs: 604800,
        release_integrity_manifest_url: None,
        credential_accept_dist_dir: "frontend/dist/credential-accept".to_string(),
        google_client_id: None,
        google_client_secret: None,
        github_client_id: None,
        github_client_secret: None,
        apple_client_id: None,
        apple_team_id: None,
        apple_key_id: None,
        apple_private_key_path: None,
        smtp_host: None,
        smtp_port: None,
        smtp_username: None,
        smtp_password: None,
        smtp_from_address: None,
        encryption_key: Some("11".repeat(32)),
        encryption_key_previous: None,
        rate_limit_per_second: 10,
        rate_limit_burst: 30,
        platform_service_rate_limit_per_second: 2,
        platform_service_rate_limit_burst: 10,
        trusted_proxy_ips: vec![],
        mtls_client_cert_header: None,
        broker_require_sender_constraint: false,
        broker_require_admin_capability: false,
        cli_pairing_hmac_key: None,
        audit_chain_hmac_key: None,
        billing_ledger_hmac_key: None,
        chain_verify_interval_secs: 0,
        sa_token_ttl_secs: 3600,
        cookie_domain: None,
        telegram_bot_token: None,
        telegram_webhook_secret: None,
        telegram_webhook_url: None,
        telegram_bot_username: None,
        approval_expiry_interval_secs: 5,
        connect_link_expiry_sweep_interval_secs: 60,
        oauth_refresh_sweep_interval_secs: 600,
        oauth_refresh_sweep_window_secs: 900,
        connection_expiry_notifications: true,
        fcm_service_account_path: None,
        fcm_project_id: None,
        apns_key_path: None,
        apns_key_id: None,
        apns_team_id: None,
        apns_topic: None,
        apns_sandbox: true,
        key_provider: "local".to_string(),
        aws_kms_key_arn: None,
        aws_kms_key_arn_previous: None,
        gcp_kms_key_name: None,
        gcp_kms_key_name_previous: None,
        instance_name: "test-backend".to_string(),
        internal_bind_addr: "127.0.0.1:3002".to_string(),
        internal_advertise_url: "http://127.0.0.1:3002".to_string(),
        internal_dispatch_hmac_key: None,
        internal_auth_max_skew_secs: 30,
        internal_nonce_ttl_secs: 120,
        internal_duplex_handshake_timeout_secs: 5,
        node_owner_lease_ttl_secs: 90,
        node_owner_lease_renew_secs: 30,
        cluster_lease_ttl_secs: 30,
        cluster_lease_renew_secs: 10,
        cluster_slot_ttl_secs: 30,
        cluster_slot_renew_secs: 10,
        mcp_notification_poll_interval_ms: 250,
        mcp_notification_ttl_secs: 86_400,
        node_heartbeat_interval_secs: 30,
        node_heartbeat_timeout_secs: 90,
        node_proxy_timeout_secs: 30,
        node_registration_token_ttl_secs: 3600,
        node_pending_credential_ttl_secs: 86_400,
        node_max_per_user: 10,
        node_max_ws_connections: 100,
        node_max_stream_duration_secs: 300,
        node_hmac_signing_enabled: true,
        proxy_max_body_size: 100 * 1024 * 1024,
        llm_max_body_size: 10 * 1024 * 1024,
        proxy_stream_idle_timeout_secs: 60,
        ssh_max_sessions_per_user: 4,
        ssh_connect_timeout_secs: 10,
        ssh_max_tunnel_duration_secs: 3600,
        ws_passthrough_max_connections: 200,
        public_proxy_max_body_size:
            crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_PROXY_MAX_BODY_SIZE,
        public_proxy_rate_limit_per_minute:
            crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_PROXY_RATE_LIMIT_PER_MINUTE,
        public_mcp_rate_limit_per_minute:
            crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_MCP_RATE_LIMIT_PER_MINUTE,
        channel_relay_callback_timeout_secs: 30,
        channel_relay_max_bots_per_user: 5,
        channel_relay_message_ttl_days: 30,
        channel_relay_edit_rate_limit_per_second: 10,
        channel_relay_edit_rate_limit_burst: 20,
        channel_event_rate_limit_per_second: 100,
        channel_event_rate_limit_burst: 200,
        channel_event_dedup_ttl_secs: 300,
        trigger_rate_limit_per_second: 10,
        trigger_rate_limit_burst: 20,
        trigger_payload_max_bytes: 256 * 1024,
        trigger_delivery_retention_hours: 72,
        oracle_task_retention_days: 30,
        cloud_response_cache_ttl_secs: 0,
        cloud_response_cache_max_entry_bytes: 1024 * 1024,
        cloud_response_cache_max_entries: 256,
        billing_enabled: false,
        lago_api_url: None,
        lago_api_key: None,
        lago_plan_code: "starter".to_string(),
        lago_payment_provider_code: None,
        lago_webhook_secret: None,
        billing_reconcile_interval_secs: 300,
        billing_rate_cache_ttl_secs: 900,
        billing_reservation_abandon_secs: 600,
        billing_default_overdraft_cap_credits: 0,
        billing_fail_closed: false,
        billing_resale_enabled: false,
        invite_code_required: false,
        email_auth_enabled: false,
        auto_verify_email: false,
        telemetry_dsn: None,
        telemetry_host: None,
        share_analytics: false,
    }
}

pub(crate) fn test_encryption_keys() -> EncryptionKeys {
    EncryptionKeys::from_config(&test_app_config())
}

/// A throwaway 2048-bit RSA private key (PKCS#8 PEM) for GCP service-account
/// mint tests. A mock token server never verifies the signature, so this
/// only needs to be a parseable, signable key.
pub(crate) const TEST_GCP_SA_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCxOfGio6jS5FhN\nxWOq8diF22dhiHhJ+IHxHM7NP40+ljQri6sRnfFzbEZoS2JcXgX7vuWBwjopYgR0\nawMK+fjhOzuy1bEltJ940ZyFgtVIMxgAVosI9fz38faLd1hqc1X/S2KADLYFdt2I\nTucnPg3W5eLlwXrggCBR5TuGBkSGO2uX4H48pZ54vEVrT4APz3GF6kn378lM/04G\nXKfuR3VBCQtQ1N1t+uSDHVEZCOXqnOm1KDgBuBvGCwn+nDAo8X7vSUZ53CvzIsgX\nmHCf7u2cHdYw9LRYlZdMeNuuIRX/2pH5chuIGoVKgywG3svb3/STJG6jT2oUpM7c\nCYu0p7SVAgMBAAECggEAFcPQdZFUy+WIJLDvnxBJb5L03MkGQMtYpfRMP2+lGIEY\n0ho6fZTgkLTE5s0PPNm9MWANzoQ8YVWsx2FXA9OUKZD9MWbF9SP8C7nuV4UsTUwd\nD/mQ5J5VHVwlU5ZqENSuRIaNB73H4t7osPNDtxGLYI9l8KJ0xTpm/bfBuiFt6/AO\nvJoCT12m5gZzF7cLHk3Gb8a9YSlj86rM3eJJF+L0UZ0gpob//RDqnX58SaqeW3sM\nIRXPL9ZHUsKZ9i2Ke68DMox9ACi3gFmnsyaiB0yhBjOiBvTpIjgAT7ucmdFKP15D\ndPgphTxM6cnxGLRE37PiSqW6GDzA7itly8zPRi2OpwKBgQDls4wyeaMNQtgSvXWM\nvSImzgyk7/KagmWtniSYw8Kh8pAL0vHUz38XL8PpVDBTCp8N7pSHnu8brxXOwwYU\n/a9kJzgmncYHogkrcsDXskx4czUx6BO7p8qMBSYh2dCI9iHIJejl3Be+tWmWdEPk\nXn7WCOzq3mzJVfubdMuAqGTpEwKBgQDFhGAJHSMIsEInEHMDCmHy7cX5pDOJoX9K\nB2SjQTpHXmTS6LjrpAFSodyuM3lr/M/coVk8FAwwGfNAlViaEotQBlbkU/HkekkM\n+iNvlMKm8YL2fMpCHQNDI/S9sjiI0Yi7unPFnlbmpCY7NDCWGJsm0x5IsDs4sKfF\nQ8ISheGItwKBgQDgOu3ZODSbdW1InfpqcRctmmdte27wtepcGczP9AnD3e4QHNRG\nUmhWUiKFW9HwvqWWDBiia9wuwjQfqvH8+8iDlGWUDOCMAvnAmDz4Uu2jh5OeLFdX\nEO0A0uXulZqkmOFRaPB5sujbGm0Amm7MOBLJDd15SbgYsv7zOoiOB9S6UQKBgCDZ\nx288nVsQlbARmE9lJq1Uxpyipr+5UIZrfF16t8qu9G3vrvHiMSYhLab7gLJpNdko\nLMNFQlGtvzt6m2Xkt67znvgSziSGAihaYhJo14cUnAeK8cjVMnm0PTxfq+91ihxP\nAnpXv3RU0Nb/8yTDqupmKp9EUFU5bG3uuxSBl+U5AoGBAL+NOw9adup24YiPJ/Gc\nMC3YWJLHTMmWthhQl2zoST3B2qyF59herT0OapF9uvSA/3R7l2/hjY7Y62qHdvlp\nyvwM98ObxwlT/Cip3pDK1E/cek9QwqxyAsRDdy/Tr1PnISowhaNRtv/6yjpjDMRq\n36i//64vyzDNvwtlnvGWhsCs\n-----END PRIVATE KEY-----\n";

/// Build a Google service-account key JSON whose `token_uri` points at a
/// (test) token endpoint. Used by GCP service-account mint/handler tests.
pub(crate) fn test_gcp_sa_json(token_uri: &str) -> String {
    serde_json::json!({
        "type": "service_account",
        "project_id": "test-project",
        "private_key_id": "abc123",
        "private_key": TEST_GCP_SA_PRIVATE_KEY,
        "client_email": "svc@test-project.iam.gserviceaccount.com",
        "client_id": "1234567890",
        "token_uri": token_uri,
    })
    .to_string()
}

/// Spawn a one-route mock OAuth token endpoint on localhost. Returns the
/// `/token` URL (usable as a service-account `token_uri` under `cfg(test)`)
/// and the server task handle.
pub(crate) async fn spawn_mock_token_server(
    response: serde_json::Value,
    status: axum::http::StatusCode,
) -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/token",
        axum::routing::post(move || {
            let resp = response.clone();
            async move { (status, axum::Json(resp)) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/token"), handle)
}

/// Returns a process-wide RSA `JwtKeys` for tests, generated lazily once.
///
/// Generating an RSA keypair via the pure-Rust `rsa` crate is the dominant
/// cost in many test paths (tens of seconds per call, even at 2048 bits, in
/// debug profiles). Tests don't need production-grade key sizes or unique
/// keys per test, so we share one 2048-bit pair across the entire test
/// binary and clone the cheap `JwtKeys` handle for each caller.
pub(crate) fn cached_test_jwt_keys() -> JwtKeys {
    static CACHED: OnceLock<JwtKeys> = OnceLock::new();
    CACHED.get_or_init(generate_test_jwt_keys).clone()
}

fn generate_test_jwt_keys() -> JwtKeys {
    use jsonwebtoken::{DecodingKey, EncodingKey};
    use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey};
    use rsa::traits::PublicKeyParts;
    use sha2::{Digest, Sha256};

    let mut rng = rand::thread_rng();
    let private_key =
        rsa::RsaPrivateKey::new(&mut rng, 2048).expect("generate test RSA private key");
    let public_key = private_key.to_public_key();

    let private_pem = private_key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .expect("encode test RSA private PEM");
    let public_pem = public_key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .expect("encode test RSA public PEM");

    let n_bytes = public_key.n().to_bytes_be();
    let kid = hex::encode(&Sha256::digest(&n_bytes)[..8]);

    JwtKeys {
        encoding: EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .expect("build test RSA encoding key"),
        decoding: DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .expect("build test RSA decoding key"),
        kid,
    }
}

/// Build a minimal `AppState` for handler tests.
pub(crate) fn test_app_state(db: mongodb::Database) -> AppState {
    let config = test_app_config();
    test_app_state_with_config(db, config)
}

/// Build an `AppState` with a caller-provided config for pure handler tests.
pub(crate) fn test_app_state_with_config(db: mongodb::Database, config: AppConfig) -> AppState {
    let http_client = reqwest::Client::new();
    let jwt_keys = cached_test_jwt_keys();
    let billing = Arc::new(crate::services::billing::BillingService::new(
        db.clone(),
        Arc::new(config.clone()),
    ));
    let encryption_keys = Arc::new(test_encryption_keys());
    let node_ws_manager = Arc::new(NodeWsManager::new(
        config.node_proxy_timeout_secs,
        config.node_max_ws_connections,
    ));
    let replica_identity = Arc::new(crate::services::node_owner_service::ReplicaIdentity::new(
        config.instance_name.clone(),
        config.internal_advertise_url.clone(),
    ));
    let internal_auth = crate::services::internal_auth::InternalAuth::new(
        db.clone(),
        crate::services::internal_auth::derive_key(
            config.internal_dispatch_hmac_key.as_deref(),
            config.encryption_key.as_deref().map(str::as_bytes),
            &[],
        ),
        std::time::Duration::from_secs(config.internal_auth_max_skew_secs),
        std::time::Duration::from_secs(config.internal_nonce_ttl_secs),
    );
    let node_dispatch = Arc::new(crate::services::node_dispatch::NodeDispatch::new(
        db.clone(),
        node_ws_manager.clone(),
        replica_identity.clone(),
        http_client.clone(),
        internal_auth,
        crate::services::node_ws_manager::node_proxy_ws_message_size_limit(
            config.proxy_max_body_size,
        ),
        std::time::Duration::from_secs(config.internal_duplex_handshake_timeout_secs),
    ));
    node_ws_manager.attach_cluster_dispatch(Arc::downgrade(&node_dispatch));
    let developer_webhook_dispatcher = Arc::new(
        crate::services::developer_webhook_service::DeveloperWebhookDispatcher::new(
            http_client.clone(),
            encryption_keys.clone(),
        ),
    );
    let cluster_slot_manager = crate::services::cluster_slot_service::RenewableSlotManager::new(
        db.clone(),
        replica_identity.coordination_holder(),
        std::time::Duration::from_secs(config.cluster_slot_ttl_secs),
        std::time::Duration::from_secs(config.cluster_slot_renew_secs),
    );

    AppState {
        db: db.clone(),
        config: config.clone(),
        jwt_keys,
        http_client: http_client.clone(),
        jwk_json: serde_json::json!({}),
        mcp_sessions: Arc::new(McpSessionStore::new()),
        jwks_cache: Arc::new(JwksCache::new(http_client.clone())),
        fcm_auth: None,
        apns_auth: None,
        connection_expiry_notifier: Arc::new(
            crate::services::connection_expiry_service::ConnectionExpiryNotifier::new(
                Arc::new(config.clone()),
                http_client.clone(),
                None,
                None,
                Some(developer_webhook_dispatcher.clone()),
            ),
        ),
        developer_webhook_dispatcher,
        encryption_keys,
        node_ws_manager,
        replica_identity,
        node_dispatch,
        ssh_session_manager: Arc::new(SshSessionManager::new(
            cluster_slot_manager.clone(),
            config.ssh_max_sessions_per_user,
        )),
        cluster_slot_manager: cluster_slot_manager.clone(),
        per_agent_limiter: Arc::new(crate::mw::rate_limit::PerAgentRateLimiter::with_db(
            db.clone(),
            "agent",
        )),
        platform_user_rate_limit: crate::mw::rate_limit::PlatformUserRateLimitPolicy::new(
            config.platform_service_rate_limit_per_second,
            config.platform_service_rate_limit_burst,
        ),
        direct_chat_limiter: crate::mw::rate_limit::create_direct_chat_rate_limiter(
            db.clone(),
            cluster_slot_manager,
        ),
        device_code_pubkey_limiter: crate::mw::rate_limit::create_per_pubkey_rate_limiter(
            db.clone(),
            "device_code_pubkey",
        ),
        device_code_ip_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "device_code_ip",
            5,
            60,
        ),
        auth_device_request_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_device_request",
            5,
            60,
        ),
        auth_device_poll_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_device_poll",
            60,
            60,
        ),
        auth_device_approve_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_device_approve",
            10,
            60,
        ),
        auth_device_approve_per_user_limiter: crate::mw::rate_limit::create_per_key_rate_limiter(
            db.clone(),
            "auth_device_approve_user",
            10,
            300,
        ),
        auth_device_preview_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_device_preview",
            30,
            60,
        ),
        connect_link_create_limiter: crate::mw::rate_limit::create_per_key_rate_limiter(
            db.clone(),
            "connect_link_create",
            10,
            60,
        ),
        connect_link_preview_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "connect_link_preview",
            30,
            60,
        ),
        connect_link_complete_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "connect_link_complete",
            30,
            60,
        ),
        public_proxy_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "public_proxy",
            config.public_proxy_rate_limit_per_minute,
            60,
        ),
        public_mcp_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "public_mcp",
            config.public_mcp_rate_limit_per_minute,
            60,
        ),
        broker_policy: Arc::new(std::sync::RwLock::new(BrokerPolicy::from_config(&config))),
        // Production default from backend/src/main.rs — 5 claims per
        // 60s per IP; mirror here so claim-rate-limit tests see the
        // same shape.
        cli_pairing_claim_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "cli_pairing_claim",
            5,
            60,
        ),
        // Tests don't exercise pairing-code HMAC verification; a
        // zero-filled key is deterministic and never touches prod data.
        cli_pairing_hmac_key: Arc::new(zeroize::Zeroizing::new([0u8; 32])),
        // Tests don't exercise auth-device HMAC verification through AppState yet;
        // service-level tests pass their own explicit HMAC key.
        auth_device_hmac_key: Arc::new(zeroize::Zeroizing::new([1u8; 32])),
        audit_chain_hmac_key: Arc::new(zeroize::Zeroizing::new([2u8; 32])),
        billing_ledger_hmac_key: Arc::new(zeroize::Zeroizing::new([3u8; 32])),
        per_channel_event_limiter: Arc::new(
            crate::mw::rate_limit::PerChannelEventLimiter::with_db(
                db.clone(),
                "channel_event",
                config.channel_event_rate_limit_per_second,
                config.channel_event_rate_limit_burst,
            ),
        ),
        per_message_edit_limiter: Arc::new(
            crate::mw::rate_limit::PerMessageEditRateLimiter::with_db(
                db.clone(),
                "channel_message_edit",
                config.channel_relay_edit_rate_limit_per_second,
                config.channel_relay_edit_rate_limit_burst,
            ),
        ),
        per_trigger_limiter: Arc::new(crate::mw::rate_limit::PerChannelEventLimiter::with_db(
            db,
            "trigger_ingress",
            config.trigger_rate_limit_per_second,
            config.trigger_rate_limit_burst,
        )),
        token_exchange_cache: Arc::new(TokenExchangeCache::new()),
        cloud_response_cache: Arc::new(
            crate::services::cloud_response_cache::CloudResponseCache::new(0),
        ),
        billing,
        telemetry: None,
    }
}

/// Build an `AppState` for tests that never perform MongoDB operations.
pub(crate) async fn test_app_state_no_db() -> AppState {
    let client = mongodb::Client::with_uri_str("mongodb://localhost:27017")
        .await
        .expect("build inert test MongoDB client");
    test_app_state(client.database("nyxid_unit_unused"))
}

/// Build a permissive session-auth `AuthUser` for handler tests.
pub(crate) fn test_auth_user(user_id: &str) -> AuthUser {
    AuthUser {
        user_id: Uuid::parse_str(user_id).expect("valid uuid user id"),
        session_id: None,
        scope: String::new(),
        acting_client_id: None,
        oauth_client_id: None,
        token_jti: None,
        approval_owner_user_id: None,
        auth_method: AuthMethod::Session,
        allow_all_services: true,
        allow_all_nodes: true,
        allowed_service_ids: vec![],
        resource_uris: None,
        allowed_node_ids: vec![],
        api_key_id: None,
        api_key_name: None,
        api_key_purpose: crate::models::api_key::ApiKeyPurpose::General,
        rate_limit_per_second: None,
        rate_limit_burst: None,
        ip_address: None,
        user_agent: None,
    }
}

fn sorted_strings(values: &[&str]) -> Vec<String> {
    let mut values: Vec<String> = values.iter().map(|value| value.to_string()).collect();
    values.sort();
    values
}

fn b64url_fixture(byte: u8, len: usize) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![byte; len])
}

/// Assert an RCI audit row is metadata-only and has exactly the expected keys.
pub(crate) fn assert_rci_audit_row(
    entry: &AuditLog,
    expected_event_type: &str,
    pending: &NodePendingCredential,
    expected_remote_state: Option<&str>,
    extra_keys: &[&str],
) {
    assert_eq!(entry.event_type, expected_event_type);
    assert_eq!(
        entry.user_id.as_deref(),
        Some(pending.owner_user_id.as_str())
    );

    let event_data = entry.event_data.as_ref().expect("audit event data");
    let object = event_data.as_object().expect("audit event data object");
    let mut expected_keys = vec![
        "event_at",
        "flow",
        "node_id",
        "owner_user_id",
        "pending_created_at",
        "pending_credential_id",
        "pending_expires_at",
        "routed_via",
        "service_slug",
    ];
    if expected_remote_state.is_some() {
        expected_keys.push("remote_state");
    }
    expected_keys.extend(extra_keys.iter().copied());

    let mut actual_keys: Vec<String> = object.keys().cloned().collect();
    actual_keys.sort();
    assert_eq!(actual_keys, sorted_strings(&expected_keys));

    assert_eq!(object["flow"], "remote_credential_injection");
    assert_eq!(object["routed_via"], "node");
    assert_eq!(object["node_id"], pending.node_id);
    assert_eq!(object["pending_credential_id"], pending.id);
    assert_eq!(object["service_slug"], pending.service_slug);
    assert_eq!(object["owner_user_id"], pending.owner_user_id);
    assert_eq!(
        object["pending_created_at"],
        pending.created_at.to_rfc3339()
    );
    assert_eq!(
        object["pending_expires_at"],
        pending.expires_at.to_rfc3339()
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(object["event_at"].as_str().expect("event_at string"))
            .is_ok()
    );

    if let Some(remote_state) = expected_remote_state {
        assert_eq!(object["remote_state"], remote_state);
    } else {
        assert!(object.get("remote_state").is_none());
    }

    if let Some(queued_at) = object.get("ciphertext_queued_at") {
        assert_eq!(
            queued_at.as_str().expect("ciphertext_queued_at string"),
            pending
                .ciphertext_queued_at
                .expect("pending has queued timestamp")
                .to_rfc3339()
        );
    }
    if let Some(expires_at) = object.get("ciphertext_expires_at") {
        assert_eq!(
            expires_at.as_str().expect("ciphertext_expires_at string"),
            pending
                .ciphertext_expires_at
                .expect("pending has ciphertext expiry")
                .to_rfc3339()
        );
    }

    for forbidden_key in [
        "plaintext",
        "secret",
        "ciphertext",
        "nonce",
        "node_pubkey",
        "admin_pubkey",
        "sealed_privkey",
        "private_key",
        "hash",
        "fingerprint",
        "length",
        "bytes",
        "target_url",
        "field_name",
        "injection_method",
        "raw_version",
        "raw_status",
        "raw_node_error",
        "raw_decrypt_error",
        "raw_decline_reason",
        "decrypt_error",
        "queue_count",
        "queued_pending_ids",
    ] {
        assert!(
            !object.contains_key(forbidden_key),
            "{expected_event_type}: {forbidden_key}"
        );
    }

    let event_json = event_data.to_string();
    let forbidden_values = [
        b64url_fixture(5, 32),
        b64url_fixture(6, 32),
        b64url_fixture(7, 24),
        b64url_fixture(8, 32),
        b64url_fixture(9, 31),
        b64url_fixture(10, 32),
        b64url_fixture(11, 24),
        b64url_fixture(12, 32),
        b64url_fixture(13, 32),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3]),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3, 4]),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([42]),
        "super-secret-plaintext-fixture".to_string(),
        "secret-value-fixture".to_string(),
        "raw-node-error-fixture".to_string(),
        "decline-reason-fixture".to_string(),
        "raw-decline-reason-fixture".to_string(),
    ];
    for forbidden_value in forbidden_values {
        assert!(!event_json.contains(&forbidden_value), "{forbidden_value}");
    }
}

pub(crate) fn test_user(user_id: &str, user_type: UserType) -> User {
    let now = chrono::Utc::now();
    User {
        id: user_id.to_string(),
        email: format!("{user_id}@example.com"),
        password_hash: None,
        display_name: Some(match user_type {
            UserType::Person => "Test User".to_string(),
            UserType::Org => "Test Org".to_string(),
        }),
        // Derive a unique slug from the user_id so tests that create multiple
        // org fixtures and call `ensure_indexes` (which builds the partial
        // unique slug index) don't collide on a shared "test-org" value.
        slug: match user_type {
            UserType::Person => None,
            UserType::Org => Some(format!(
                "test-org-{}",
                user_id.replace('-', "").chars().take(8).collect::<String>()
            )),
        },
        avatar_url: None,
        email_verified: true,
        email_verification_token: None,
        password_reset_token: None,
        password_reset_expires_at: None,
        is_active: true,
        is_admin: false,
        is_operator: false,
        role_ids: vec![],
        group_ids: vec![],
        invite_code_id: None,
        mfa_enabled: false,
        social_provider: None,
        social_provider_id: None,
        user_type,
        primary_org_id: None,
        created_at: now,
        updated_at: now,
        last_login_at: None,
        profile_config: Default::default(),
    }
}

pub(crate) fn test_membership(
    org_user_id: &str,
    member_user_id: &str,
    role: OrgRole,
    allowed_service_ids: Option<Vec<String>>,
) -> OrgMembership {
    OrgMembership {
        id: Uuid::new_v4().to_string(),
        org_user_id: org_user_id.to_string(),
        member_user_id: member_user_id.to_string(),
        role,
        scope_source: MemberScopeSource::Override,
        allowed_service_ids,
        created_at: chrono::Utc::now(),
        revoked_at: None,
    }
}

pub(crate) fn test_user_endpoint(
    endpoint_id: &str,
    user_id: &str,
    label: &str,
    url: &str,
    openapi_spec_url: Option<&str>,
    catalog_service_id: Option<&str>,
) -> UserEndpoint {
    UserEndpoint {
        id: endpoint_id.to_string(),
        user_id: user_id.to_string(),
        label: label.to_string(),
        url: url.to_string(),
        catalog_service_id: catalog_service_id.map(str::to_string),
        openapi_spec_url: openapi_spec_url.map(str::to_string),
        recommended_skills: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

pub(crate) fn test_user_service(
    service_id: &str,
    user_id: &str,
    slug: &str,
    endpoint_id: &str,
    catalog_service_id: Option<&str>,
    node_id: Option<&str>,
) -> UserService {
    UserService {
        id: service_id.to_string(),
        user_id: user_id.to_string(),
        slug: slug.to_string(),
        endpoint_id: endpoint_id.to_string(),
        api_key_id: None,
        auth_method: "none".to_string(),
        auth_key_name: String::new(),
        catalog_service_id: catalog_service_id.map(str::to_string),
        node_id: node_id.map(str::to_string),
        node_priority: 0,
        service_type: "http".to_string(),
        ssh_auth_mode: crate::models::ssh_auth_mode::SshAuthMode::ProxyOnly,
        admin_only: false,
        ssh_node_keys_stale: false,
        identity_propagation_mode: "none".to_string(),
        identity_include_user_id: false,
        identity_include_email: false,
        identity_include_name: false,
        identity_jwt_audience: None,
        forward_access_token: false,
        inject_delegation_token: false,
        delegation_token_scope: "llm:proxy".to_string(),
        custom_user_agent: None,
        default_request_headers: None,
        ws_frame_injections: Vec::new(),
        is_active: true,
        source: None,
        source_id: None,
        source_app_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        state_version: 1,
        rotation_predecessor_id: None,
    }
}

/// Mirror of the Aevatar assistant-action evidence reader's recursive
/// secret-shape scan (`RejectSecretBearingRead`): forbidden property names,
/// plus any string value matching `Bearer\s+\S+` or a long `nyxid_` prefix.
///
/// Returns the offending property or value instead of panicking so tests can
/// assert both directions — that an evidence projection is clean, and that
/// the full detail document it projects from is not.
pub(crate) fn aevatar_secret_free_violation(value: &serde_json::Value) -> Option<String> {
    const FORBIDDEN_NAMES: &[&str] = &[
        "apikey",
        "fullkey",
        "keyhash",
        "credential",
        "credentials",
        "accesstoken",
        "refreshtoken",
        "authorization",
        "cookie",
        "cookies",
        "secret",
        "secrets",
        "clientsecret",
        "password",
        "token",
        "passphrase",
        "usercode",
        "devicecode",
        "rawbody",
        "rawupstreambody",
    ];
    let secret_value =
        regex::Regex::new(r"(?i)(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})").unwrap();

    fn visit(value: &serde_json::Value, secret_value: &regex::Regex) -> Option<String> {
        match value {
            serde_json::Value::Object(properties) => {
                for (name, nested) in properties {
                    let normalized: String = name
                        .chars()
                        .filter(char::is_ascii_alphanumeric)
                        .map(|character| character.to_ascii_lowercase())
                        .collect();
                    if FORBIDDEN_NAMES.contains(&normalized.as_str()) {
                        return Some(format!("secret-bearing response property: {name}"));
                    }
                    if let Some(violation) = visit(nested, secret_value) {
                        return Some(violation);
                    }
                }
                None
            }
            serde_json::Value::Array(items) => {
                items.iter().find_map(|item| visit(item, secret_value))
            }
            serde_json::Value::String(text) => secret_value
                .is_match(text)
                .then(|| "secret-shaped response value at serialization boundary".to_string()),
            _ => None,
        }
    }

    visit(value, &secret_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn process_local_stale_sweep_cooldown_skips_repeated_attempts() {
        let next_attempt_at = AtomicU64::new(0);
        let now_secs = 10_000;
        assert!(claim_process_local_stale_sweep_attempt(
            &next_attempt_at,
            now_secs
        ));
        assert!(!claim_process_local_stale_sweep_attempt(
            &next_attempt_at,
            now_secs + STALE_TEST_DB_SWEEP_COOLDOWN.as_secs() - 1,
        ));
        assert!(claim_process_local_stale_sweep_attempt(
            &next_attempt_at,
            now_secs + STALE_TEST_DB_SWEEP_COOLDOWN.as_secs(),
        ));
    }

    #[test]
    fn concurrent_process_local_stale_sweep_admission_has_one_winner() {
        let next_attempt_at = Arc::new(AtomicU64::new(0));
        let winners = std::thread::scope(|scope| {
            let attempts = (0..16)
                .map(|_| {
                    let next_attempt_at = next_attempt_at.clone();
                    scope.spawn(move || {
                        claim_process_local_stale_sweep_attempt(&next_attempt_at, 20_000)
                    })
                })
                .collect::<Vec<_>>();
            attempts
                .into_iter()
                .map(|attempt| attempt.join().expect("cooldown participant"))
                .filter(|won| *won)
                .count()
        });
        assert_eq!(winners, 1);
    }

    #[test]
    fn managed_test_db_name_round_trips_run_lease_identity_and_fits_mongodb_limit() {
        let created_at_secs = 0x0123_4567_89ab_cdef;
        let run_id = "1020304050607080";
        let name = format_test_db_name(
            "approval drift/with a prefix that is much too long",
            created_at_secs,
            run_id,
            0x5060_7080,
        );

        assert_eq!(
            parse_managed_test_db_name(&name),
            Some(ManagedTestDbName {
                created_at_secs,
                run_id: run_id.to_string(),
            })
        );
        assert!(name.len() <= TEST_DB_MAX_NAME_LEN);
        assert!(name.ends_with("approval"));
    }

    #[test]
    fn managed_test_db_parser_rejects_probe_legacy_and_malformed_names() {
        let valid = format_test_db_name("parser", 123, "0000000000000001", 2);
        assert_eq!(
            parse_managed_test_db_name(&valid),
            Some(ManagedTestDbName {
                created_at_secs: 123,
                run_id: "0000000000000001".to_string(),
            })
        );

        let malformed = [
            TEST_DB_PROBE_NAME.to_string(),
            format!("{TEST_DB_NAME_PREFIX}legacy_{}", Uuid::nil()),
            valid.replacen("000000000000007b", "00000000000000zz", 1),
            valid.replacen("0000000000000001", "1", 1),
            format!("{TEST_DB_NAME_PREFIX}000000000000007b_0000000000000001_00000002_"),
            format!("{TEST_DB_NAME_PREFIX}000000000000007b_0000000000000001_00000002_bad-prefix"),
        ];

        for name in malformed {
            assert_eq!(
                parse_managed_test_db_name(&name),
                None,
                "unexpectedly accepted {name}"
            );
        }
    }

    #[test]
    fn managed_test_db_eligibility_requires_age_and_an_expired_run_lease() {
        let now_secs = 2_000_000;
        let stale_after_secs = MANAGED_TEST_DB_MIN_AGE.as_secs();
        let exactly_stale = ManagedTestDbName {
            created_at_secs: now_secs - stale_after_secs,
            run_id: "0000000000000001".to_string(),
        };
        let one_second_too_young = ManagedTestDbName {
            created_at_secs: now_secs - stale_after_secs + 1,
            run_id: "0000000000000002".to_string(),
        };
        let future = ManagedTestDbName {
            created_at_secs: now_secs + 1,
            run_id: "0000000000000003".to_string(),
        };
        let active = HashSet::from([exactly_stale.run_id.clone()]);

        assert!(!managed_test_db_is_eligible(
            &exactly_stale,
            now_secs,
            &active
        ));
        assert!(managed_test_db_is_eligible(
            &exactly_stale,
            now_secs,
            &HashSet::new()
        ));
        assert!(!managed_test_db_is_eligible(
            &one_second_too_young,
            now_secs,
            &HashSet::new()
        ));
        assert!(!managed_test_db_is_eligible(
            &future,
            now_secs,
            &HashSet::new()
        ));
    }

    #[test]
    fn legacy_test_db_adoption_is_exact_and_quarantined_from_first_observation() {
        let canonical_v4 = "01234567-89ab-4def-8123-456789abcdef";
        let legacy = format!("{TEST_DB_NAME_PREFIX}approval_drift_{canonical_v4}");
        assert!(is_legacy_test_db_name(&legacy));
        assert!(!is_legacy_test_db_name(TEST_DB_PROBE_NAME));
        assert!(!is_legacy_test_db_name(&format!(
            "{TEST_DB_NAME_PREFIX}arbitrary_user_database"
        )));
        assert!(!is_legacy_test_db_name(&format!(
            "{TEST_DB_NAME_PREFIX}bad-prefix_{}",
            canonical_v4
        )));
        for noncanonical in [
            canonical_v4.replace('-', ""),
            format!("{{{canonical_v4}}}"),
            canonical_v4.to_uppercase(),
            Uuid::nil().to_string(),
            format!("urn:uuid:{canonical_v4}"),
        ] {
            assert!(
                !is_legacy_test_db_name(&format!(
                    "{TEST_DB_NAME_PREFIX}approval_drift_{noncanonical}"
                )),
                "unexpectedly adopted noncanonical UUID form {noncanonical}"
            );
        }
        assert!(!is_legacy_test_db_name(&format!(
            "{TEST_DB_NAME_PREFIX}_approval_{canonical_v4}"
        )));
        assert!(!is_legacy_test_db_name(&format!(
            "{TEST_DB_NAME_PREFIX}approval__{canonical_v4}"
        )));

        let now_secs = 2_000_000;
        let quarantine_secs = LEGACY_TEST_DB_QUARANTINE.as_secs();
        assert!(legacy_test_db_is_eligible(
            now_secs - quarantine_secs,
            now_secs
        ));
        assert!(!legacy_test_db_is_eligible(
            now_secs - quarantine_secs + 1,
            now_secs
        ));
        assert!(!legacy_test_db_is_eligible(now_secs + 1, now_secs));
    }

    #[test]
    fn first_writable_test_database_uri_is_pinned_for_the_process() {
        let pinned_uri = OnceLock::new();
        assert!(pin_test_db_uri_in(&pinned_uri, "mongodb://127.0.0.1:27018"));
        assert!(pin_test_db_uri_in(&pinned_uri, "mongodb://127.0.0.1:27018"));
        assert!(!pin_test_db_uri_in(
            &pinned_uri,
            "mongodb://127.0.0.1:27017"
        ));
        assert_eq!(
            pinned_uri.get().map(String::as_str),
            Some("mongodb://127.0.0.1:27018")
        );
    }

    #[test]
    fn losing_test_database_uri_race_converges_on_the_winner() {
        let pinned_uri = Arc::new(OnceLock::new());
        let start = Arc::new(std::sync::Barrier::new(2));
        let contenders = [
            "mongodb://127.0.0.1:27018".to_string(),
            "mongodb://127.0.0.1:27017".to_string(),
        ];
        let contenders: Vec<_> = contenders
            .into_iter()
            .map(|candidate| {
                let pinned_uri = Arc::clone(&pinned_uri);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    let won = pin_test_db_uri_in(&pinned_uri, &candidate);
                    let retry_uri = pinned_uri
                        .get()
                        .expect("one contender must publish the winner")
                        .clone();
                    (won, retry_uri)
                })
            })
            .collect();
        let outcomes: Vec<_> = contenders
            .into_iter()
            .map(|contender| contender.join().expect("join URI contender"))
            .collect();

        assert_eq!(outcomes.iter().filter(|(won, _)| *won).count(), 1);
        let winner = pinned_uri.get().expect("URI winner");
        assert!(
            outcomes.iter().all(|(_, retry_uri)| retry_uri == winner),
            "the losing probe must retry the process-wide winner"
        );
    }

    #[test]
    fn exit_cleanup_deletes_run_record_only_after_a_complete_successful_drop_pass() {
        assert!(test_db_exit_cleanup_can_delete_run_record(Some(true)));
        assert!(!test_db_exit_cleanup_can_delete_run_record(Some(false)));
        assert!(!test_db_exit_cleanup_can_delete_run_record(None));
    }

    #[test]
    fn failed_lifecycle_drop_remains_registered_for_exit_recovery() {
        let mut cleanup = TestDbCleanup::new();
        cleanup.register("mongodb://test.invalid", "nyxid_test_failed_drop");

        cleanup.complete_lifecycle_drop("nyxid_test_failed_drop", false);
        assert!(cleanup.is_registered("nyxid_test_failed_drop"));

        cleanup.complete_lifecycle_drop("nyxid_test_failed_drop", true);
        assert!(!cleanup.is_registered("nyxid_test_failed_drop"));
    }

    #[tokio::test]
    async fn lifecycle_drop_retries_transient_failures_and_stays_bounded() {
        let transient_attempts = Arc::new(AtomicUsize::new(0));
        let attempts = Arc::clone(&transient_attempts);
        assert!(
            retry_test_db_lifecycle_drop(Duration::ZERO, move || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                std::future::ready(attempt == TEST_DB_LIFECYCLE_DROP_MAX_ATTEMPTS)
            })
            .await
        );
        assert_eq!(
            transient_attempts.load(Ordering::SeqCst),
            TEST_DB_LIFECYCLE_DROP_MAX_ATTEMPTS
        );

        let failed_attempts = Arc::new(AtomicUsize::new(0));
        let attempts = Arc::clone(&failed_attempts);
        assert!(
            !retry_test_db_lifecycle_drop(Duration::ZERO, move || {
                attempts.fetch_add(1, Ordering::SeqCst);
                std::future::ready(false)
            })
            .await
        );
        assert_eq!(
            failed_attempts.load(Ordering::SeqCst),
            TEST_DB_LIFECYCLE_DROP_MAX_ATTEMPTS
        );
    }

    #[test]
    fn drop_worker_pool_claims_a_bounded_backlog_concurrently() {
        let (sender, receiver) = sync_channel(TEST_DB_DROP_QUEUE_CAPACITY);
        for job_index in 0..TEST_DB_DROP_QUEUE_CAPACITY {
            sender
                .send(TestDbDropJob {
                    uri: "mongodb://test.invalid".to_string(),
                    db_name: format!("nyxid_test_backlog_{job_index}"),
                })
                .expect("seed bounded drop backlog");
        }

        let receiver = Arc::new(std::sync::Mutex::new(receiver));
        let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let (started_sender, started_receiver) = sync_channel(TEST_DB_DROP_WORKER_COUNT);
        let workers: Vec<_> = (0..TEST_DB_DROP_WORKER_COUNT)
            .map(|_| {
                let receiver = Arc::clone(&receiver);
                let release = Arc::clone(&release);
                let started_sender = started_sender.clone();
                std::thread::spawn(move || {
                    let job = receive_test_db_drop_job(&receiver).expect("claim drop job");
                    started_sender
                        .send(job.db_name)
                        .expect("report claimed drop job");

                    let (released, wake) = &*release;
                    let mut released = released
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    while !*released {
                        released = wake
                            .wait(released)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                })
            })
            .collect();
        drop(started_sender);

        let mut claimed_names = HashSet::new();
        let mut all_workers_started = true;
        for _ in 0..TEST_DB_DROP_WORKER_COUNT {
            match started_receiver.recv_timeout(Duration::from_secs(5)) {
                Ok(db_name) => {
                    claimed_names.insert(db_name);
                }
                Err(_) => {
                    all_workers_started = false;
                    break;
                }
            }
        }

        let (released, wake) = &*release;
        *released
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        wake.notify_all();
        for worker in workers {
            worker.join().expect("join drop backlog worker");
        }

        assert!(
            all_workers_started,
            "every drop worker must claim backlog without waiting for another worker to finish"
        );
        assert_eq!(claimed_names.len(), TEST_DB_DROP_WORKER_COUNT);
    }

    #[test]
    fn full_drop_queue_defers_to_one_retry_coordinator_until_capacity_returns() {
        let (job_sender, job_receiver) = sync_channel(1);
        job_sender
            .send(TestDbDropJob {
                uri: "mongodb://test.invalid".to_string(),
                db_name: "nyxid_test_queue_seed".to_string(),
            })
            .expect("fill the bounded queue");
        let retry = start_test_db_drop_retry_coordinator(job_sender.clone())
            .expect("start fixed retry coordinator");
        let workers = TestDbDropWorkers {
            job_sender,
            retry: Some(retry),
        };

        assert!(enqueue_test_db_drop_job(
            &workers,
            TestDbDropJob {
                uri: "mongodb://test.invalid".to_string(),
                db_name: "nyxid_test_queue_deferred".to_string(),
            },
        ));
        assert_eq!(
            workers
                .retry
                .as_ref()
                .expect("retry coordinator")
                .jobs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1,
            "queue admission overflow must remain tracked"
        );

        assert_eq!(
            job_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("read queue seed")
                .db_name,
            "nyxid_test_queue_seed"
        );
        assert_eq!(
            job_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("retry coordinator queues the deferred job")
                .db_name,
            "nyxid_test_queue_deferred"
        );
        assert!(
            workers
                .retry
                .as_ref()
                .expect("retry coordinator")
                .jobs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
        );
    }

    #[tokio::test]
    async fn cleanup_client_operation_timeout_fails_closed() {
        let result = test_db_cleanup_operation_with_timeout(
            Duration::ZERO,
            std::future::pending::<Result<(), ()>>(),
        )
        .await;

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn per_test_client_release_drops_database_before_process_exit() {
        let db = connect_transaction_test_database("lifecycle_release").await;
        let db_name = db.name().to_string();
        db.collection::<Document>("fixture")
            .insert_one(doc! { "_id": "materialized" })
            .await
            .expect("materialize lifecycle test database");
        assert!(test_db_is_registered_for_cleanup(&db_name));
        let observer = observe_test_db_drop(&db_name);

        drop(db);

        let dropped =
            tokio::task::spawn_blocking(move || observer.recv_timeout(Duration::from_secs(20)))
                .await
                .expect("join lifecycle drop observer")
                .expect("driver client teardown should release the database before process exit");
        assert!(
            dropped,
            "the lifecycle worker must report a successful drop"
        );
        assert!(!test_db_is_registered_for_cleanup(&db_name));
    }

    #[tokio::test]
    async fn database_clone_keeps_lifecycle_cleanup_from_running_early() {
        let db = connect_transaction_test_database("lifecycle_clone").await;
        let db_name = db.name().to_string();
        db.collection::<Document>("fixture")
            .insert_one(doc! { "_id": "clone-still-live" })
            .await
            .expect("materialize clone lifecycle database");
        let surviving_clone = db.clone();
        let observer = observe_test_db_drop(&db_name);
        drop(db);

        let mut drop_wait =
            tokio::task::spawn_blocking(move || observer.recv_timeout(Duration::from_secs(20)));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut drop_wait)
                .await
                .is_err(),
            "cleanup must not run while a Database clone still owns the client"
        );
        assert!(test_db_is_registered_for_cleanup(&db_name));
        assert_eq!(
            surviving_clone
                .collection::<Document>("fixture")
                .count_documents(doc! {})
                .await
                .expect("use the surviving database clone"),
            1
        );

        drop(surviving_clone);

        let dropped = drop_wait
            .await
            .expect("join clone lifecycle drop observer")
            .expect("final clone release should enqueue cleanup");
        assert!(
            dropped,
            "the lifecycle worker must report a successful drop"
        );
        assert!(!test_db_is_registered_for_cleanup(&db_name));
    }

    #[tokio::test]
    async fn mongo_sweep_lease_is_cross_process_exclusive_and_owner_checked() {
        let db = connect_transaction_test_database("sweep_lease").await;
        let collection = db.collection::<Document>("sweep_lease_test");
        let now_secs = unix_time_secs();

        assert!(acquire_test_db_sweep_lease(&collection, "owner-a", now_secs).await);
        assert!(!acquire_test_db_sweep_lease(&collection, "owner-b", now_secs).await);

        release_test_db_sweep_lease(&collection, "owner-b", now_secs).await;
        assert!(!acquire_test_db_sweep_lease(&collection, "owner-b", now_secs).await);

        release_test_db_sweep_lease(&collection, "owner-a", now_secs).await;
        assert!(
            !acquire_test_db_sweep_lease(
                &collection,
                "owner-b",
                now_secs + STALE_TEST_DB_SWEEP_COOLDOWN.as_secs() - 1,
            )
            .await
        );
        assert!(
            acquire_test_db_sweep_lease(
                &collection,
                "owner-b",
                now_secs + STALE_TEST_DB_SWEEP_COOLDOWN.as_secs(),
            )
            .await
        );
    }

    #[tokio::test]
    async fn mongo_metadata_pruning_keeps_live_referenced_and_ambiguous_records() {
        let db = connect_transaction_test_database("sweep_metadata").await;
        let run_records = db.collection::<Document>("run_records");
        let legacy_candidates = db.collection::<Document>("legacy_candidates");
        let now_secs = 2_000_000;

        run_records
            .insert_many([
                doc! { "_id": "expired-orphan", "lease_until_secs": bson_secs(now_secs - 1) },
                doc! { "_id": "expired-referenced", "lease_until_secs": bson_secs(now_secs - 1) },
                doc! { "_id": "live-orphan", "lease_until_secs": bson_secs(now_secs + 1) },
                doc! { "_id": "missing-expiry" },
            ])
            .await
            .expect("insert run metadata fixtures");
        legacy_candidates
            .insert_many([
                doc! { "_id": "nyxid_test_present_00000000-0000-0000-0000-000000000000" },
                doc! { "_id": "nyxid_test_missing_00000000-0000-0000-0000-000000000000" },
            ])
            .await
            .expect("insert legacy metadata fixtures");

        prune_test_db_metadata_collections(
            &run_records,
            &legacy_candidates,
            now_secs,
            &HashSet::from(["expired-referenced".to_string()]),
            &HashSet::from(["nyxid_test_present_00000000-0000-0000-0000-000000000000".to_string()]),
        )
        .await;

        let remaining_runs: HashSet<String> = run_records
            .find(doc! {})
            .await
            .expect("query remaining run records")
            .try_collect::<Vec<Document>>()
            .await
            .expect("read remaining run records")
            .into_iter()
            .map(|record| record.get_str("_id").expect("string run id").to_string())
            .collect();
        assert_eq!(
            remaining_runs,
            HashSet::from([
                "expired-referenced".to_string(),
                "live-orphan".to_string(),
                "missing-expiry".to_string(),
            ])
        );

        let remaining_legacy_candidates: HashSet<String> = legacy_candidates
            .find(doc! {})
            .await
            .expect("query remaining legacy candidates")
            .try_collect::<Vec<Document>>()
            .await
            .expect("read remaining legacy candidates")
            .into_iter()
            .map(|record| {
                record
                    .get_str("_id")
                    .expect("string legacy candidate id")
                    .to_string()
            })
            .collect();
        assert_eq!(
            remaining_legacy_candidates,
            HashSet::from(["nyxid_test_present_00000000-0000-0000-0000-000000000000".to_string(),])
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_first_run_renewals_publish_one_live_lease() {
        let Some(db) = connect_test_database("concurrent_run_renewal").await else {
            return;
        };
        let run_records = db.collection::<Document>("run_records");
        let run_id = Uuid::new_v4().to_string();
        let now_secs = 2_000_000;
        let participants = 16;
        let barrier = Arc::new(tokio::sync::Barrier::new(participants));
        let mut renewals = Vec::with_capacity(participants);

        for _ in 0..participants {
            let run_records = run_records.clone();
            let run_id = run_id.clone();
            let barrier = Arc::clone(&barrier);
            renewals.push(tokio::spawn(async move {
                barrier.wait().await;
                renew_test_db_run_record_in_collection(
                    &run_records,
                    &run_id,
                    now_secs - 100,
                    now_secs,
                )
                .await
            }));
        }

        for renewal in futures::future::join_all(renewals).await {
            assert!(
                matches!(renewal, Ok(Ok(true))),
                "every concurrent first renewal must observe a live lease: {renewal:?}"
            );
        }
        assert_eq!(
            run_records
                .count_documents(doc! { "_id": &run_id })
                .await
                .expect("count run records"),
            1
        );
        let lease = run_records
            .find_one(doc! { "_id": &run_id })
            .await
            .expect("read run lease")
            .expect("run lease exists");
        assert_eq!(
            lease.get_i64("lease_until_secs"),
            Ok(bson_secs(
                now_secs.saturating_add(TEST_DB_RUN_LEASE.as_secs())
            ))
        );
    }

    #[tokio::test]
    async fn mongo_managed_drop_claim_fences_renewal_and_is_owner_checked() {
        let db = connect_transaction_test_database("sweep_drop_claim").await;
        let run_records = db.collection::<Document>("run_records");
        let now_secs = 2_000_000;
        let started_at_secs = now_secs - 100;

        run_records
            .insert_many([
                doc! { "_id": "renewed-first", "lease_until_secs": bson_secs(now_secs - 1) },
                doc! { "_id": "claimed-first", "lease_until_secs": bson_secs(now_secs - 1) },
                doc! {
                    "_id": "expired-claim",
                    "lease_until_secs": bson_secs(now_secs - 1),
                    "cleanup_claim_id": "crashed-sweeper",
                    "cleanup_claim_until_secs": bson_secs(now_secs - 1),
                },
                doc! {
                    "_id": "malformed-claim",
                    "lease_until_secs": bson_secs(now_secs - 1),
                    "cleanup_claim_id": "missing-expiry",
                },
            ])
            .await
            .expect("insert run lease fixtures");

        renew_test_db_run_record_in_collection(
            &run_records,
            "renewed-first",
            started_at_secs,
            now_secs,
        )
        .await
        .expect("producer renews before stale-drop claim");
        assert!(
            !claim_expired_test_db_run(
                &run_records,
                "renewed-first",
                "sweeper-after-renewal",
                now_secs,
            )
            .await,
            "a producer renewal that wins the race must block stale cleanup"
        );

        assert!(
            claim_expired_test_db_run(&run_records, "claimed-first", "sweeper-owner", now_secs,)
                .await,
            "an explicitly expired producer lease can be claimed"
        );
        assert!(
            !claim_expired_test_db_run(
                &run_records,
                "claimed-first",
                "competing-sweeper",
                now_secs,
            )
            .await,
            "an active cleanup claim is exclusive"
        );
        assert!(
            matches!(
                renew_test_db_run_record_in_collection(
                    &run_records,
                    "claimed-first",
                    started_at_secs,
                    now_secs,
                )
                .await,
                Ok(false)
            ),
            "a producer renewal that loses the atomic claim race must fail closed"
        );
        assert!(
            !release_test_db_drop_claim(&run_records, "claimed-first", "wrong-owner").await,
            "a different sweeper must not release the fencing claim"
        );
        let claimed = run_records
            .find_one(doc! { "_id": "claimed-first" })
            .await
            .expect("read claimed run")
            .expect("claimed run exists");
        assert_eq!(
            claimed
                .get_str("cleanup_claim_id")
                .expect("string cleanup claim owner"),
            "sweeper-owner"
        );
        assert!(
            !release_test_db_drop_claim_after_confirmed_drop(
                &run_records,
                "claimed-first",
                "sweeper-owner",
                false,
            )
            .await,
            "an ambiguous or failed drop must retain its fencing claim"
        );
        assert!(matches!(
            renew_test_db_run_record_in_collection(
                &run_records,
                "claimed-first",
                started_at_secs,
                now_secs,
            )
            .await,
            Ok(false)
        ));
        let retained = run_records
            .find_one(doc! { "_id": "claimed-first" })
            .await
            .expect("read retained claim")
            .expect("retained claim exists");
        assert_eq!(retained.get_str("cleanup_claim_id"), Ok("sweeper-owner"));
        assert!(
            release_test_db_drop_claim_after_confirmed_drop(
                &run_records,
                "claimed-first",
                "sweeper-owner",
                true,
            )
            .await,
            "only a confirmed drop may release its fencing claim"
        );
        renew_test_db_run_record_in_collection(
            &run_records,
            "claimed-first",
            started_at_secs,
            now_secs,
        )
        .await
        .expect("producer can renew after the bounded drop claim is released");
        let renewed = run_records
            .find_one(doc! { "_id": "claimed-first" })
            .await
            .expect("read renewed run")
            .expect("renewed run exists");
        assert_eq!(
            renewed.get_i64("lease_until_secs"),
            Ok(bson_secs(
                now_secs.saturating_add(TEST_DB_RUN_LEASE.as_secs())
            ))
        );
        assert!(!renewed.contains_key("cleanup_claim_id"));
        assert!(!renewed.contains_key("cleanup_claim_until_secs"));

        assert!(
            claim_expired_test_db_run(
                &run_records,
                "expired-claim",
                "replacement-sweeper",
                now_secs,
            )
            .await
        );
        assert!(
            !claim_expired_test_db_run(&run_records, "malformed-claim", "sweeper", now_secs,).await,
            "malformed claim state must block destructive recovery"
        );
        assert!(
            matches!(
                renew_test_db_run_record_in_collection(
                    &run_records,
                    "malformed-claim",
                    started_at_secs,
                    now_secs,
                )
                .await,
                Ok(false)
            ),
            "malformed claim state must also fail producer renewal closed"
        );
    }

    /// Guards the per-test mongo probe against the CI slowdown regression: a
    /// candidate port with no listener must fail over in ~milliseconds, not stall
    /// on the driver's server-selection timeout. (In CI only 27017 is published,
    /// so the 27018 candidate is dead — without the TCP pre-check every DB test
    /// paid ~10s here.) Uses a freshly-closed ephemeral port so the assertion is
    /// deterministic and needs no running mongod.
    #[tokio::test]
    async fn closed_port_probe_fails_fast() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local addr").to_string();
        drop(listener); // port now has no listener -> connect refused immediately

        let start = Instant::now();
        let reachable = test_mongo_port_reachable(&addr).await;
        let elapsed = start.elapsed();

        assert!(!reachable, "a closed port must be reported unreachable");
        assert!(
            elapsed < Duration::from_secs(2),
            "closed-port probe must fail fast (got {elapsed:?}); a dead candidate \
             must not block on the mongo server-selection timeout"
        );
    }

    /// CI configures a single-node replica set. Keeping this as a mandatory test
    /// makes a regression to standalone MongoDB fail at setup with an actionable
    /// message instead of letting transaction-dependent tests silently return.
    #[tokio::test]
    async fn transaction_test_database_supports_atomic_writes() {
        let _db = connect_transaction_test_database("transaction_topology").await;
    }
}
