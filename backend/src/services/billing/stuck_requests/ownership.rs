//! Read-only process ownership, using the existing readiness observation registry.
use super::*;
use mongodb::ClientSession;
use std::collections::BTreeMap;

// Readiness expires after 45 s. Two extra minutes cover twelve missed 10 s
// heartbeats. This is an ownership policy, not proof a paused process cannot
// resume; the terminal meter transaction still arbitrates its late finalizer.
pub(crate) const OWNER_GRACE_MILLIS: i64 = 120_000;
pub(crate) const LEGACY_MARGIN_MILLIS: i64 = 3_600_000;
const MAX_GENERATIONS: usize = 1024;

#[derive(Clone)]
pub(crate) struct Registry {
    pub now: bson::DateTime,
    live: BTreeMap<String, Option<bson::DateTime>>,
}

pub(crate) async fn registry(
    db: &mongodb::Database,
    session: &mut ClientSession,
) -> AppResult<Registry> {
    let horizon =
        crate::services::security_readiness_service::MAX_AGE_SECS * 1000 + OWNER_GRACE_MILLIS;
    let mut cursor = db
        .collection::<Document>("security_replicas")
        .aggregate(vec![
            doc! {"$match":{"$or":[
                {"observed_at":{"$not":{"$type":"date"}}},
                {"$expr":{"$gte":["$observed_at",{"$subtract":["$$NOW",horizon]}]}}
            ]}},
            doc! {"$limit":(MAX_GENERATIONS+1) as i64},
            doc! {"$project":{"_id":1,"generation_id":1,"observed_at":1,"started_at":1}},
            doc! {"$group":{"_id":bson::Bson::Null,"rows":{"$push":"$$ROOT"}}},
            doc! {"$project":{"rows":1,"now":"$$NOW"}},
        ])
        .max_time(std::time::Duration::from_secs(2))
        .session(&mut *session)
        .await?;
    let Some(value) = next_document(&mut cursor, session).await? else {
        return Err(unavailable());
    };
    let now = *value.get_datetime("now").map_err(|_| unavailable())?;
    let rows = value.get_array("rows").map_err(|_| unavailable())?;
    if rows.is_empty() || rows.len() > MAX_GENERATIONS {
        return Err(unavailable());
    }
    let mut live = BTreeMap::new();
    for value in rows {
        let row = value.as_document().ok_or_else(unavailable)?;
        let id = row.get_str("generation_id").map_err(|_| unavailable())?;
        let observed = *row.get_datetime("observed_at").map_err(|_| unavailable())?;
        if id.is_empty()
            || row.get_str("_id").ok() != Some(id)
            || observed.timestamp_millis() > now.timestamp_millis() + 5_000
        {
            return Err(unavailable());
        }
        let started = match row.get("started_at") {
            None | Some(Bson::Null) => None,
            Some(Bson::DateTime(at)) if *at <= observed => Some(*at),
            _ => return Err(unavailable()),
        };
        if live.insert(id.to_owned(), started).is_some() {
            return Err(unavailable());
        }
    }
    Ok(Registry { now, live })
}

impl Registry {
    pub(super) fn generation_is_live(&self, generation: &str) -> bool {
        self.live.contains_key(generation)
    }
    pub(crate) fn ownership(&self, row: &Document) -> AppResult<&'static str> {
        let horizon =
            crate::services::security_readiness_service::MAX_AGE_SECS * 1000 + OWNER_GRACE_MILLIS;
        match row.get("forwarded_owner") {
            Some(Bson::Document(owner)) => {
                let id = owner.get_str("generation_id").map_err(|_| unavailable())?;
                let marked = owner.get_datetime("marked_at").map_err(|_| unavailable())?;
                if id.is_empty() || *marked > self.now {
                    return Err(unavailable());
                }
                if self.live.contains_key(id) {
                    return Ok("owned_by_live_generation");
                }
                if marked.timestamp_millis() > self.now.timestamp_millis() - horizon {
                    return Ok("owner_expiry_margin");
                }
                Ok("eligible_now")
            }
            None | Some(Bson::Null) => {
                let created = row.get_datetime("created_at").map_err(|_| unavailable())?;
                // Missing start times on any still-live old binary prevent a
                // legacy death inference. Registration is before serving; the
                // extra hour also guards meter timestamps made by wall clocks.
                let Some(starts) = self.live.values().copied().collect::<Option<Vec<_>>>() else {
                    return Ok("legacy_registry_start_unknown");
                };
                let Some(earliest) = starts.into_iter().min() else {
                    return Err(unavailable());
                };
                let cutoff = earliest.timestamp_millis().min(self.now.timestamp_millis())
                    - LEGACY_MARGIN_MILLIS;
                Ok(if created.timestamp_millis() < cutoff {
                    "eligible_now"
                } else {
                    "legacy_may_be_live"
                })
            }
            _ => Err(unavailable()),
        }
    }
}
