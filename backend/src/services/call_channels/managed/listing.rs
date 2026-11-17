//! Batched read projections for the merchant and admin line listings: a page of
//! claims costs one query per projection instead of one per claimed row.
use super::unavailable;
use crate::{
    errors::AppResult,
    models::{channel_adapter::BINDINGS, managed_comms::*},
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use std::collections::HashMap;

/// Paid coverage of one claim. Renewals are contiguous, so coverage runs from
/// the period covering now (if any) to the end of the latest funded period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub current_from: Option<DateTime<Utc>>,
    pub latest_from: DateTime<Utc>,
    pub latest_until: DateTime<Utc>,
    pub period_index: i64,
}
impl Coverage {
    pub fn covered_from(&self) -> DateTime<Utc> {
        self.current_from.unwrap_or(self.latest_from)
    }
}

/// One listing page decoded row by row. A malformed document never fails the
/// whole listing: it is skipped with a metadata-only warning (collection and
/// row ID only) and reported in `unreadable`, and the cursor still advances
/// past it because it is taken from the raw page.
pub struct Page<T> {
    pub rows: Vec<T>,
    pub next_after_id: Option<String>,
    pub unreadable: Vec<String>,
}

pub async fn page<T: serde::de::DeserializeOwned>(
    db: &Database,
    collection: &'static str,
    filter: Document,
    limit: usize,
) -> AppResult<Page<T>> {
    let mut raw: Vec<Document> = db
        .collection::<Document>(collection)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(i64::try_from(limit + 1).map_err(|_| unavailable())?)
        .await?
        .try_collect()
        .await?;
    let next_after_id = if raw.len() > limit {
        raw.truncate(limit);
        raw.iter()
            .rev()
            .find_map(|d| d.get_str("_id").ok())
            .map(str::to_owned)
    } else {
        None
    };
    let (rows, unreadable) = decode(collection, raw);
    Ok(Page {
        rows,
        next_after_id,
        unreadable,
    })
}

/// Typed conversion of already-read rows, skipping (and naming) unreadable ones.
pub fn decode<T: serde::de::DeserializeOwned>(
    collection: &'static str,
    raw: Vec<Document>,
) -> (Vec<T>, Vec<String>) {
    let mut rows = Vec::with_capacity(raw.len());
    let mut unreadable = Vec::new();
    for document in raw {
        let id = row_id(&document);
        match bson::from_document(document) {
            Ok(row) => rows.push(row),
            Err(_) => {
                skipped(collection, &id);
                unreadable.push(id);
            }
        }
    }
    (rows, unreadable)
}

/// Batched rows of one collection, decoded row by row.
pub async fn rows<T: serde::de::DeserializeOwned>(
    db: &Database,
    collection: &'static str,
    filter: Document,
) -> AppResult<(Vec<T>, Vec<String>)> {
    let raw: Vec<Document> = db
        .collection::<Document>(collection)
        .find(filter)
        .await?
        .try_collect()
        .await?;
    Ok(decode(collection, raw))
}

fn row_id(document: &Document) -> String {
    document.get("_id").map(bson_id).unwrap_or_default()
}
fn bson_id(id: &bson::Bson) -> String {
    match id {
        bson::Bson::String(id) => id.clone(),
        other => other.to_string(),
    }
}

// Never log field values or decoding errors: rows can hold contact details.
fn skipped(collection: &str, id: &str) {
    tracing::warn!(collection, row_id = %id, "Unreadable managed listing row skipped");
}

fn int(d: &Document, key: &str) -> AppResult<i64> {
    d.get_i64(key)
        .or_else(|_| d.get_i32(key).map(i64::from))
        .map_err(|_| unavailable())
}

/// Uses the unique `{claim_id, period_index}` index for the per-claim sort.
pub async fn coverage(
    db: &Database,
    claims: &[String],
    now: DateTime<Utc>,
    unreadable: &mut Vec<String>,
) -> AppResult<HashMap<String, Coverage>> {
    if claims.is_empty() {
        return Ok(HashMap::new());
    }
    let now = bson::DateTime::from_chrono(now);
    let rows: Vec<Document> = db
        .collection::<Document>(ManagedLinePeriod::COLLECTION_NAME)
        .aggregate(vec![
            doc! {"$match":{"claim_id":{"$in":claims},"funded":true}},
            doc! {"$sort":{"claim_id":1,"period_index":-1}},
            doc! {"$group":{
                "_id":"$claim_id",
                "latest_id":{"$first":"$_id"},
                "latest_from":{"$first":"$covered_from"},
                "latest_until":{"$first":"$covered_until"},
                "period_index":{"$first":"$period_index"},
                // $max ignores the nulls produced for non-current periods.
                "current_from":{"$max":{"$cond":[
                    {"$and":[{"$lte":["$covered_from",now]},{"$gt":["$covered_until",now]}]},
                    "$covered_from",
                    bson::Bson::Null,
                ]}},
            }},
        ])
        .await?
        .try_collect()
        .await?;
    let mut out = HashMap::new();
    for d in rows {
        let entry = || -> AppResult<(String, Coverage)> {
            Ok((
                d.get_str("_id").map_err(|_| unavailable())?.to_owned(),
                Coverage {
                    current_from: d.get_datetime("current_from").ok().map(|v| v.to_chrono()),
                    latest_from: d
                        .get_datetime("latest_from")
                        .map_err(|_| unavailable())?
                        .to_chrono(),
                    latest_until: d
                        .get_datetime("latest_until")
                        .map_err(|_| unavailable())?
                        .to_chrono(),
                    period_index: int(&d, "period_index")?,
                },
            ))
        };
        match entry() {
            Ok((claim, coverage)) => {
                out.insert(claim, coverage);
            }
            // The claim is listed without coverage; name its newest period.
            Err(_) => {
                let id = d.get("latest_id").map(bson_id).unwrap_or_default();
                skipped(ManagedLinePeriod::COLLECTION_NAME, &id);
                unreadable.push(id);
            }
        }
    }
    Ok(out)
}

/// Admitted calls per claim still holding a slot (prepared, live or awaiting
/// confirmed termination), from the indexed admission receipts.
pub async fn live_calls(db: &Database, claims: &[String]) -> AppResult<HashMap<String, u64>> {
    // Counted with `$sum`, so every group is numeric; a non-string claim key
    // cannot match the string claim IDs it was filtered by.
    if claims.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<Document> = db
        .collection::<Document>(ManagedCallAdmission::COLLECTION_NAME)
        .aggregate(vec![
            doc! {"$match":{"managed_claim_id":{"$in":claims},"slot_held":true}},
            doc! {"$group":{"_id":"$managed_claim_id","count":{"$sum":1_i64}}},
        ])
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|d| {
            Some((
                d.get_str("_id").ok()?.to_owned(),
                u64::try_from(int(&d, "count").ok()?).ok()?,
            ))
        })
        .collect())
}

/// The newest unaccepted, uncancelled, unexpired quote per claim of the owner.
pub async fn open_quotes(
    db: &Database,
    owner: &str,
    claims: &[String],
    now: DateTime<Utc>,
    unreadable: &mut Vec<String>,
) -> AppResult<HashMap<String, ManagedLineQuote>> {
    if claims.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<Document> = db
        .collection::<Document>(ManagedLineQuote::COLLECTION_NAME)
        .aggregate(vec![
            doc! {"$match":{"user_id":owner,"kind.claim_id":{"$in":claims},
            "accepted_at":bson::Bson::Null,"cancelled_at":bson::Bson::Null,
            "accept_before":{"$gt":bson::DateTime::from_chrono(now)}}},
            doc! {"$sort":{"created_at":-1}},
            doc! {"$group":{"_id":"$kind.claim_id","quote":{"$first":"$$ROOT"}}},
        ])
        .await?
        .try_collect()
        .await?;
    let mut out = HashMap::new();
    for d in rows {
        let (Ok(claim), Ok(raw)) = (d.get_str("_id"), d.get_document("quote")) else {
            continue;
        };
        let (mut quotes, mut bad) =
            decode::<ManagedLineQuote>(ManagedLineQuote::COLLECTION_NAME, vec![raw.clone()]);
        // An unreadable newest quote leaves the claim without an open quote.
        unreadable.append(&mut bad);
        if let Some(quote) = quotes.pop() {
            out.insert(claim.to_owned(), quote);
        }
    }
    Ok(out)
}

/// Current revisions of the owner's bindings, projected without credentials.
pub async fn binding_revisions(
    db: &Database,
    owner: &str,
    bindings: &[String],
) -> AppResult<HashMap<String, i64>> {
    if bindings.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<Document> = db
        .collection::<Document>(BINDINGS)
        .find(doc! {"_id":{"$in":bindings},"user_id":owner})
        .projection(doc! {"revision":1})
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|d| match (d.get_str("_id"), int(&d, "revision")) {
            (Ok(id), Ok(revision)) => Some((id.to_owned(), revision)),
            _ => {
                skipped(BINDINGS, &row_id(&d));
                None
            }
        })
        .collect())
}
