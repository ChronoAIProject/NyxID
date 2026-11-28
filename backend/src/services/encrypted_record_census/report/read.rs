use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};

#[derive(Serialize)]
pub(crate) struct Status {
    pub generation: String,
    schema_version: i32,
    consistency_method: &'static str,
    keys: Vec<key_lifecycle::Key>,
    phase: Phase,
    pub complete: bool,
    certification_enabled: bool,
    blockers: Vec<&'static str>,
    baseline: Timestamp,
    as_of: Option<Timestamp>,
    registry_digest: String,
    scope_revision: i64,
    restore_epoch: String,
    deployment_revision: String,
    contextual_writers: bool,
    legacy_readers: bool,
    families_staged: usize,
    pub totals: Option<Totals>,
}

impl Status {
    pub(super) fn from_state(
        state: &Checkpoint,
        current: bool,
        invalid: Option<&'static str>,
    ) -> Self {
        let published = state.phase == Phase::Published;
        let mut blockers = vec![if state.ring.revision == 0 {
            "isolated_load_gate_unmet"
        } else {
            "retirement_evaluation_required"
        }];
        if !current {
            blockers.push("registry_digest_mismatch");
        }
        if state.ring.revision == 0 {
            blockers.push("unenrolled");
        }
        if state.totals.unresolved > 0 {
            blockers.push("unresolved_rows");
        }
        if let Some(reason) = invalid {
            blockers.push(reason);
        }
        Self {
            generation: state.start.generation.clone(),
            schema_version: 1,
            consistency_method: "snapshot_and_witnessed_change_stream",
            keys: state
                .ring
                .proofs
                .iter()
                .map(|proof| proof.key.clone())
                .collect(),
            phase: state.phase,
            complete: published && current && invalid.is_none(),
            certification_enabled: false,
            blockers,
            baseline: state.start.time,
            as_of: published
                .then(|| state.end.as_ref().map(|c| c.time))
                .flatten(),
            registry_digest: state.scope.registry_digest.clone(),
            scope_revision: state.ring.revision,
            restore_epoch: state.ring.restore_epoch.clone(),
            deployment_revision: state.ring.deployment_revision.clone(),
            contextual_writers: state.writers,
            legacy_readers: state.legacy,
            families_staged: state.published.len(),
            totals: published.then(|| state.totals.clone()),
        }
    }
}

pub(crate) async fn status(db: &Database, keys: &EncryptionKeys, id: &str) -> AppResult<Status> {
    let db = transaction_service::durable_database(db);
    let store = Store {
        db: &db,
        keys,
        generation: id,
    };
    let (_, state) = store.load().await?;
    let reason = if state.phase != Phase::Published {
        let journal = db
            .collection::<Document>(LOG)
            .find_one(
                doc! {"_id":id,"invalid":{"$exists":false},"$expr":{"$gt":["$expires_at","$$NOW"]}},
            )
            .projection(doc! {"_id":1})
            .await?;
        journal.is_none().then_some("generation_unavailable")
    } else {
        None
    };
    Ok(Status::from_state(
        &state,
        contract().is_ok_and(|(digest, _)| digest == state.scope.registry_digest),
        reason,
    ))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct Query {
    pub after: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Cursor {
    family: usize,
    link: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Page {
    pub items: Vec<Value>,
    pub next: Option<String>,
}

async fn page(
    db: &Database,
    keys: &EncryptionKeys,
    id: &str,
    query: Query,
    kind: &str,
) -> AppResult<Page> {
    let db = transaction_service::durable_database(db);
    let store = Store {
        db: &db,
        keys,
        generation: id,
    };
    let (_, state) = store.load().await?;
    if state.phase != Phase::Published {
        return Err(refused("Census report awaits publication"));
    }
    let mut pos = match query.after {
        None => Cursor::default(),
        Some(token) if token.len() <= 8192 => {
            let bytes = URL_SAFE_NO_PAD.decode(token).map_err(|_| malformed())?;
            store.open(kind, &bytes).await?
        }
        Some(_) => return Err(malformed()),
    };
    let mut items = vec![];
    for _ in 0..8 {
        let Some(hash) = state.published.get(pos.family) else {
            break;
        };
        let family: FamilyReport = store.get("family", hash).await?;
        if kind == "families" {
            items.push(json!({"family":family.family,"collection":family.collection,"totals":family.totals,"buckets":family.buckets}));
        } else if let Some(link) = pos.link.take().or(family.head) {
            let part: Partition = store.get("rows", &link).await?;
            for row in part.rows {
                for finding in row.findings {
                    items.push(json!({"family":family.family,"collection":family.collection,"field":finding.field,"reason":finding.reason,"record_token":row.token,"state":"unresolved"}));
                }
            }
            pos.link = part.previous;
            if pos.link.is_none() {
                pos.family += 1;
            }
            if !items.is_empty() {
                break;
            }
            continue;
        }
        pos.family += 1;
    }
    let next = if pos.family < state.published.len() {
        Some(URL_SAFE_NO_PAD.encode(store.seal(kind, &pos).await?.bytes))
    } else {
        None
    };
    Ok(Page { items, next })
}

pub(crate) async fn families(
    db: &Database,
    keys: &EncryptionKeys,
    id: &str,
    query: Query,
) -> AppResult<Page> {
    page(db, keys, id, query, "families").await
}
pub(crate) async fn findings(
    db: &Database,
    keys: &EncryptionKeys,
    id: &str,
    query: Query,
) -> AppResult<Page> {
    page(db, keys, id, query, "findings").await
}
