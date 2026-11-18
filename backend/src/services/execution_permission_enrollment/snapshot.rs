//! Bounded local rollout hints, never an authorization cache. Unknown means legacy.
use std::{
    collections::HashMap,
    future::IntoFuture,
    sync::{Arc, LazyLock, Mutex, RwLock, Weak},
    time::{Duration, Instant},
};

use bson::doc;
use futures::{StreamExt, TryStreamExt};
use mongodb::Database;
use tokio::sync::Notify;

use super::*;

const MAX_ROWS: usize = 10_000;
const POLL: Duration = Duration::from_secs(5);
const MAX_AGE: Duration = Duration::from_secs(15);

#[derive(Default, Clone)]
pub struct Cache(Arc<RwLock<Option<Data>>>);
// Database-scoped weak handles let db-only authentication and group services use
// the AppState-owned process snapshot without changing their read budget. Only
// server-created AppStates register handles; requests never populate this map.
type Snapshot = RwLock<Option<Data>>;
static CACHES: LazyLock<Mutex<HashMap<String, Weak<Snapshot>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn candidate(db: &Database, person: &User, known: &[(String, OrgRole)]) -> bool {
    let cache = CACHES
        .lock()
        .ok()
        .and_then(|map| map.get(db.name()).and_then(Weak::upgrade));
    Cache(cache.unwrap_or_default()).candidate(person, known)
}

/// Server channel ingress has a verified owner ID, but no loaded person row.
/// This deliberately conservative hint infers only global/personal enrollment;
/// it never infers org membership. The live admission checks person eligibility.
pub(crate) fn channel_candidate(db: &Database, actor: &str) -> bool {
    let cache = CACHES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(db.name())
        .and_then(Weak::upgrade);
    let Some(cache) = cache else {
        return false;
    };
    let Ok(data) = cache.read() else {
        return false;
    };
    let Some(data) = data.as_ref().filter(|d| d.at.elapsed() <= MAX_AGE) else {
        return false;
    };
    data.users.get(actor).map_or_else(
        || data.global.as_ref().is_some_and(|r| r.enabled),
        |r| r.enabled,
    )
}

struct Data {
    at: Instant,
    global: Option<FeatureFlagOverride>,
    users: HashMap<String, FeatureFlagOverride>,
    orgs: HashMap<String, Vec<FeatureFlagOverride>>,
}
impl Data {
    fn new(rows: Vec<FeatureFlagOverride>) -> Option<Self> {
        if rows.len() > MAX_ROWS {
            return None;
        }
        let mut data = Self {
            at: Instant::now(),
            global: None,
            users: HashMap::new(),
            orgs: HashMap::new(),
        };
        for row in rows
            .into_iter()
            .filter(|r| r.flag_key == FLAG && r.definition_id.is_none())
        {
            if let Some(org) = &row.org_user_id {
                data.orgs.entry(org.clone()).or_default().push(row);
            } else if row.target_kind == FlagTargetKind::Global {
                data.global = Some(row);
            } else if row.target_kind == FlagTargetKind::User
                && let Some(user) = &row.target_key
            {
                data.users.insert(user.clone(), row);
            }
        }
        Some(data)
    }
}
impl Cache {
    pub(crate) fn for_database(db: &Database) -> Self {
        let mut caches = CACHES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = caches.get(db.name()).and_then(Weak::upgrade) {
            return Self(existing);
        }
        caches.retain(|_, weak| weak.strong_count() != 0);
        let cache = Self::default();
        // Production has one database; also bound independently isolated fixtures.
        if caches.len() < 128 {
            caches.insert(db.name().to_owned(), Arc::downgrade(&cache.0));
        }
        cache
    }

    /// Known scope may be incomplete: absence of a cached deny only creates a
    /// candidate. Admission MUST resolve all live memberships before marking work.
    pub fn candidate(&self, person: &User, known_memberships: &[(String, OrgRole)]) -> bool {
        if !person.is_active || !person.user_type.is_person() {
            return false;
        }
        if person.execution_permission_enrollment == Some(1) {
            return true;
        }
        if known_memberships.len() > MAX_MEMBERSHIPS {
            return false;
        }
        let Ok(snapshot) = self.0.read() else {
            return false;
        };
        let Some(data) = snapshot.as_ref().filter(|d| d.at.elapsed() <= MAX_AGE) else {
            return false;
        };
        if !data.global.as_ref().is_some_and(|r| r.enabled) {
            return false;
        }
        // No inference from org enables without memberships; even with a known
        // scope, the snapshot arm requires global=true by contract.
        let mut rows: Vec<_> = data
            .global
            .iter()
            .chain(data.users.get(&person.id))
            .cloned()
            .collect();
        for (org, _) in known_memberships {
            if let Some(matching) = data.orgs.get(org) {
                rows.extend(matching.iter().cloned());
            }
        }
        resolve(&rows, known_memberships, &person.id, true)
    }

    pub(super) fn replace(&self, rows: Option<Vec<FeatureFlagOverride>>) {
        let value = rows.and_then(Data::new);
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = value;
    }

    pub(crate) async fn refresh(&self, db: &Database) -> AppResult<()> {
        let load = async {
            db.collection::<FeatureFlagOverride>(OVERRIDES)
                .find(doc! {"flag_key":FLAG,"definition_id":null})
                .limit(MAX_ROWS as i64 + 1)
                .max_time(READ_TIMEOUT)
                .await?
                .try_collect::<Vec<_>>()
                .await
        };
        match tokio::time::timeout(READ_TIMEOUT, load).await {
            Ok(Ok(rows)) => {
                let overflow = rows.len() > MAX_ROWS;
                self.replace(Some(rows));
                if overflow {
                    tracing::warn!(
                        reason = "row_limit",
                        "Execution enrollment snapshot unavailable"
                    );
                }
                Ok(())
            }
            Ok(Err(e)) => {
                self.replace(None);
                Err(e.into())
            }
            Err(_) => {
                self.replace(None);
                Err(AppError::Internal(
                    "Execution enrollment snapshot timed out".into(),
                ))
            }
        }
    }

    pub async fn run(&self, db: Database) {
        let wake = Arc::new(Notify::new());
        let mut workers = tokio::task::JoinSet::new();
        let watch_db = db.clone();
        let notify = wake.clone();
        crate::services::shutdown::process().spawn_worker_joined(&mut workers, async move {
            loop {
                // Reopening before refresh plus periodic full reload covers lost
                // resume tokens, deletes (no post-image), and startup races.
                let collection = watch_db.collection::<Document>(OVERRIDES);
                if let Ok(Ok(mut stream)) = tokio::time::timeout(
                    READ_TIMEOUT,
                    collection.watch().max_await_time(POLL).into_future(),
                )
                .await
                {
                    notify.notify_one();
                    while let Some(event) = stream.next().await {
                        if event.is_err() {
                            break;
                        }
                        notify.notify_one();
                    }
                }
                tokio::time::sleep(POLL).await;
            }
        });
        let mut interval = tokio::time::interval(POLL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! { _ = interval.tick() => {}, _ = wake.notified() => {
                // Coalesce bursts without keeping a queue of row contents.
                tokio::time::sleep(Duration::from_millis(250)).await;
            } }
            if self.refresh(&db).await.is_err() {
                tracing::warn!(
                    reason = "refresh_failed",
                    "Execution enrollment snapshot unavailable"
                );
            }
        }
    }
}
