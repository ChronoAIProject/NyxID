//! Snapshot hold verification and excess repair.
pub(super) mod backlog;
mod benefits;
use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Bson, Document, doc},
    options::Hint,
};
use sha2::{Digest, Sha256};

use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
        credit_grant::{COLLECTION_NAME as GRANTS, CreditGrant},
        credits::Credits,
        usage_allowance_period::{COLLECTION_NAME as PERIODS, UsageAllowancePeriod},
        usage_meter::{COLLECTION_NAME as METERS, UsageHold},
    },
};

pub(super) const OBSERVATIONS: &str = "billing_hold_observations";
// Bound memory and snapshot duration even for an unusually busy owner. An
// incomplete observation resets persistence; it can never authorize repair.
const MAX_ROWS: i64 = 10_000;
// Drain can start a successor pass immediately. Two rapid visits must not turn
// a transient old-style release into an immediate correction.
const MIN_STABLE_MILLIS: i64 = 300_000;

pub(super) fn live_filter(collection: &str, id: &str) -> Document {
    match collection {
        WALLETS => {
            doc! {"wallet_id": id, "released": false, "reserved_credits":{"$ne":Credits::ZERO}}
        }
        GRANTS => {
            doc! {"funding.grant_reservations.grant_id": {"$eq":id,"$exists":true}, "funding.settled": {"$ne": true}}
        }
        _ => {
            doc! {"funding.allowance_reservations.period_id": {"$eq":id,"$exists":true}, "funding.settled": {"$ne": true}}
        }
    }
}

fn live_index(collection: &str) -> Document {
    match collection {
        WALLETS => doc! {"wallet_id":1, "released":1, "_id":1},
        GRANTS => doc! {"funding.grant_reservations.grant_id":1, "funding.settled":1, "_id":1},
        _ => doc! {"funding.allowance_reservations.period_id":1, "funding.settled":1, "_id":1},
    }
}

fn decode<T: serde::de::DeserializeOwned>(row: Document) -> AppResult<T> {
    bson::from_document(row).map_err(|e| AppError::Internal(e.to_string()))
}

fn amount(row: &Document, field: &str) -> AppResult<Credits> {
    Credits::from_bson(
        row.get(field)
            .cloned()
            .ok_or(crate::models::credits::CreditsError)?,
        crate::models::credits::SCALE,
    )
    .map_err(Into::into)
}

fn add(totals: &mut BTreeMap<String, Credits>, key: &str, value: Credits) -> AppResult<()> {
    let current = totals.entry(key.to_owned()).or_default();
    *current = current.checked_add(value)?;
    Ok(())
}

struct Totals {
    stored: BTreeMap<String, Credits>,
    expected: BTreeMap<String, Credits>,
    exact: bool,
}

async fn totals(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    resource: &Document,
    rows: &[UsageHold],
) -> AppResult<Totals> {
    let mut stored = BTreeMap::new();
    let mut expected = BTreeMap::from([("total".into(), Credits::ZERO)]);
    let mut exact = true;
    match collection {
        WALLETS => {
            let wallet: BillingWallet = decode(resource.clone())?;
            let owner = wallet.owner_id;
            stored.insert("total".into(), wallet.reserved_credits);
            for row in rows {
                exact &= row.billing_owner_id == owner && row.reserved_credits > Credits::ZERO;
                add(&mut expected, "total", row.reserved_credits)?;
            }
            if let Some(lock) = resource
                .get("active_settlement")
                .filter(|v| **v != Bson::Null)
            {
                if let Some(lock) = lock.as_document() {
                    if let (Ok(row_id), Ok(applied), Ok(held)) = (
                        lock.get_str("row_id"),
                        lock.get_bool("applied"),
                        amount(lock, "reserved_credits"),
                    ) {
                        exact &= held >= Credits::ZERO;
                        if let Some(row) = rows.iter().find(|row| row.id == row_id) {
                            exact &= row.reserved_credits == held;
                            if applied {
                                add(&mut expected, "total", -held)?;
                            }
                        } else {
                            // Applied locks can outlive the meter's release bit.
                            // Prove that handoff; a missing/corrupt row is not an excess.
                            let mut filter = doc! {"_id":row_id,"wallet_id":&wallet.id};
                            if held == Credits::ZERO {
                                // Unreserved overage has no positive hold to
                                // subtract, including before its row is released.
                                filter.insert("reserved_credits", Credits::ZERO);
                            } else {
                                filter.insert("released", true);
                            }
                            exact &= db
                                .collection::<Document>(METERS)
                                .find_one(filter)
                                .session(&mut *session)
                                .await?
                                .is_some();
                        }
                    } else {
                        exact = false;
                    }
                } else {
                    exact = false;
                }
            }
        }
        GRANTS => {
            let grant: CreditGrant = decode(resource.clone())?;
            stored.insert("total".into(), grant.reserved);
            for rule in &grant.service_limits {
                stored.insert(format!("rule:{}", rule.key), rule.reserved);
                expected.insert(format!("rule:{}", rule.key), Credits::ZERO);
            }
            for row in rows {
                let Some(funding) = &row.funding else {
                    continue;
                };
                let settle = format!("{}:grant:{}", row.id, grant.id);
                let release = format!("{}:grant-release:{}", row.id, grant.id);
                let recorded = funding
                    .grant_consumptions
                    .iter()
                    .any(|op| op.operation_id == settle || op.operation_id == release);
                for reservation in funding
                    .grant_reservations
                    .iter()
                    .filter(|r| r.grant_id == grant.id)
                {
                    if recorded {
                        continue;
                    }
                    let applied = grant.active_settlement.as_ref().filter(|lock| {
                        lock.applied
                            && lock.usage_row_id == row.id
                            && (lock.operation_id == settle || lock.operation_id == release)
                    });
                    if let Some(lock) = applied {
                        exact &= lock.reserved == reservation.amount
                            && lock.service_limit_key == reservation.service_limit_key;
                    } else {
                        add(&mut expected, "total", reservation.amount)?;
                        if let Some(key) = &reservation.service_limit_key {
                            add(&mut expected, &format!("rule:{key}"), reservation.amount)?;
                        }
                    }
                }
            }
        }
        PERIODS => {
            let period: UsageAllowancePeriod = decode(resource.clone())?;
            stored.insert(
                "total".into(),
                Credits::from_whole(period.reserved_quantity),
            );
            for row in rows {
                let Some(funding) = &row.funding else {
                    continue;
                };
                let settle = format!("{}:allowance:{}", row.id, period.id);
                let release = format!("{}:allowance-release:{}", row.id, period.id);
                let recorded = funding
                    .allowance_consumptions
                    .iter()
                    .any(|op| op.operation_id == settle || op.operation_id == release);
                for reservation in funding
                    .allowance_reservations
                    .iter()
                    .filter(|r| r.period_id == period.id)
                {
                    if recorded {
                        continue;
                    }
                    let applied = period.active_settlement.as_ref().filter(|lock| {
                        lock.applied
                            && lock.usage_row_id == row.id
                            && (lock.operation_id == settle || lock.operation_id == release)
                    });
                    if let Some(lock) = applied {
                        exact &= lock.reserved_quantity == reservation.quantity;
                    } else {
                        add(
                            &mut expected,
                            "total",
                            Credits::from_whole(reservation.quantity),
                        )?;
                    }
                }
            }
        }
        _ => return Err(AppError::Internal("unknown hold resource".into())),
    }
    exact &= expected.values().all(|v| *v >= Credits::ZERO);
    Ok(Totals {
        stored,
        expected,
        exact,
    })
}

pub(super) struct Repair {
    pub collection: String,
    pub resource_id: String,
    pub pass: String,
}

fn repair_identity(resource: &Document) -> Document {
    // Routine balance refresh and expiry polling update clocks/generations even
    // when funds do not move. Those must not starve historical hold repair.
    let mut identity = Document::new();
    for field in [
        "_id",
        "owner_id",
        "lago_customer_id",
        "lago_wallet_id",
        "lago_subscription_id",
        "plan_kind",
        "balance_credits",
        "reserved_credits",
        "pending_lago_debits",
        "pending_topup_expiry_credits",
        "overdraft_cap_credits",
        "has_payment_instrument",
        "suspended",
        "collection_state",
        "active_settlement",
        "active_topup_expiry",
        "exact_accounting_version",
        "created_at",
    ] {
        if let Some(value) = resource.get(field) {
            identity.insert(field, value.clone());
        }
    }
    identity
}

impl Totals {
    fn excesses(&self) -> AppResult<Option<BTreeMap<String, Credits>>> {
        if !self.exact || !self.stored.keys().eq(self.expected.keys()) {
            return Ok(None);
        }
        let mut excesses = BTreeMap::new();
        for (key, stored) in &self.stored {
            let expected = self.expected[key];
            if *stored < Credits::ZERO || expected < Credits::ZERO {
                return Ok(None);
            }
            let excess = stored.checked_sub(expected)?;
            if excess < Credits::ZERO {
                return Ok(None);
            }
            if excess > Credits::ZERO {
                excesses.insert(key.clone(), excess);
            }
        }
        Ok((!excesses.is_empty()).then_some(excesses))
    }
}

struct Snapshot {
    totals: Totals,
    exact: bool,
    fingerprint: String,
    repair_fingerprint: Option<String>,
    difference: Credits,
}

async fn read_snapshot(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    resource: &Document,
    id: &str,
) -> AppResult<Snapshot> {
    // Zero-wallet-share admission need not write the wallet. Exclude its zero
    // rows: they contribute no liability and cannot fence a wallet repair.
    // Wallet liability depends only on membership/identity and the exact hold.
    // Forwarding and funding bookkeeping can change without a wallet write;
    // neither changes this sum and neither belongs in its fingerprint.
    let projection = if collection == WALLETS {
        doc! {"_id":1,"billing_owner_id":1,"wallet_id":1,"reserved_credits":1,"released":1}
    } else {
        doc! {"_id":1,"billing_owner_id":1,"reserved_credits":1,"funding":1,"service_id":1,"user_service_id":1,"service_slug":1}
    };
    let mut cursor = db
        .collection::<Document>(METERS)
        .find(live_filter(collection, id))
        .hint(Hint::Keys(live_index(collection)))
        .projection(projection)
        .max_time(std::time::Duration::from_secs(5))
        .batch_size((MAX_ROWS + 1) as u32)
        .sort(doc! {"_id":1})
        .limit(MAX_ROWS + 1)
        .session(&mut *session)
        .await?;
    let rows: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
    let bounded = rows.len() <= MAX_ROWS as usize;
    let identity = if collection == WALLETS {
        repair_identity(resource)
    } else {
        resource.clone()
    };
    let identity = if collection == WALLETS {
        identity
    } else {
        match benefits::canonical(Bson::Document(identity)) {
            Bson::Document(value) => value,
            _ => unreachable!(),
        }
    };
    let identity = bson::to_vec(&identity).map_err(|e| AppError::Internal(e.to_string()))?;
    let mut repair_fingerprint = Sha256::new();
    repair_fingerprint.update(&identity);
    let benefits_valid = collection == WALLETS
        || (bounded && benefits::valid(db, session, collection, resource, &rows).await?);
    let mut live_fingerprint = Sha256::new();
    for row in &rows {
        let canonical = if collection == WALLETS {
            Bson::Document(row.clone())
        } else {
            benefits::canonical(Bson::Document(row.clone()))
        };
        let bytes = bson::to_vec(&canonical).map_err(|e| AppError::Internal(e.to_string()))?;
        repair_fingerprint.update(&bytes);
        live_fingerprint.update(&bytes);
    }
    let typed: Vec<UsageHold> = rows.into_iter().map(decode).collect::<AppResult<_>>()?;
    let totals = totals(db, session, collection, resource, &typed).await?;
    let exact = bounded && totals.exact && benefits_valid;
    let difference = totals.stored["total"].checked_sub(totals.expected["total"])?;
    // Financial activity can delay repair, but must not hide a persistent hold
    // mismatch. Report on counter/liability equality; repair additionally
    // requires the unchanged financial/identity/lock projection above.
    let mut fingerprint = Sha256::new();
    fingerprint
        .update(bson::to_vec(&totals.stored).map_err(|e| AppError::Internal(e.to_string()))?);
    fingerprint
        .update(bson::to_vec(&totals.expected).map_err(|e| AppError::Internal(e.to_string()))?);
    fingerprint.update(live_fingerprint.finalize());
    if collection != WALLETS {
        fingerprint.update(identity);
    }
    Ok(Snapshot {
        totals,
        exact,
        fingerprint: hex::encode(fingerprint.finalize()),
        repair_fingerprint: Some(hex::encode(repair_fingerprint.finalize())),
        difference,
    })
}

#[derive(Default)]
pub(super) struct Page {
    previous: BTreeMap<String, Document>,
    present: BTreeSet<String>,
}

pub(super) fn presence_pipeline(collection: &str, ids: &[String]) -> Vec<Document> {
    if collection == WALLETS {
        return vec![
            doc! {"$match":{"wallet_id":{"$in":ids},"released":false}},
            doc! {"$group":{"_id":"$wallet_id"}},
        ];
    }
    let path = if collection == GRANTS {
        "funding.grant_reservations.grant_id"
    } else {
        "funding.allowance_reservations.period_id"
    };
    vec![
        doc! {"$match":{path:{"$in":ids,"$exists":true},"funding.settled":{"$ne":true}}},
        doc! {"$project":{"_id":0,"ids":format!("${path}")}},
        doc! {"$unwind":"$ids"},
        doc! {"$match":{"ids":{"$in":ids}}},
        doc! {"$group":{"_id":"$ids"}},
    ]
}

pub(super) async fn load_page(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    resources: &[Document],
) -> AppResult<Page> {
    let ids: Vec<String> = resources
        .iter()
        .filter(|row| {
            collection != WALLETS || row.get_i32("exact_accounting_version").ok() == Some(2)
        })
        .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
        .collect();
    if ids.is_empty() {
        return Ok(Page::default());
    }
    let keys: Vec<_> = ids.iter().map(|id| format!("{collection}:{id}")).collect();
    let mut cursor = db
        .collection::<Document>(OBSERVATIONS)
        .find(doc! {"_id":{"$in":keys}})
        .projection(doc! {
            "pass":1,"fingerprint":1,"repair_fingerprint":1,"exact":1,
            "stable_since":1,"persistent":1,"repairable":1,
            "old_enough":{"$and":[
                {"$eq":[{"$type":"$stable_since"},"date"]},
                {"$lte":["$stable_since",{"$subtract":["$$NOW",MIN_STABLE_MILLIS]}]},
            ]},
        })
        .batch_size((ids.len() + 1) as u32)
        .session(&mut *session)
        .await?;
    let observations: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
    let previous = observations
        .into_iter()
        .filter_map(|row| {
            row.get_str("_id")
                .ok()
                .map(str::to_owned)
                .map(|id| (id, row))
        })
        .collect();
    let mut cursor = db
        .collection::<Document>(METERS)
        .aggregate(presence_pipeline(collection, &ids))
        .hint(Hint::Keys(live_index(collection)))
        .batch_size((ids.len() + 1) as u32)
        .max_time(std::time::Duration::from_secs(5))
        .session(&mut *session)
        .await?;
    let rows: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
    let present = rows
        .iter()
        .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
        .collect();
    Ok(Page { previous, present })
}

fn nonzero(row: &Document, field: &str, legacy: Option<&str>) -> bool {
    let parsed = if let Some(value) = row.get(field) {
        Credits::from_bson(value.clone(), crate::models::credits::SCALE)
    } else if let Some(value) = legacy.and_then(|name| row.get(name)) {
        Credits::from_bson(value.clone(), 1_000_000)
    } else {
        return false;
    };
    parsed != Ok(Credits::ZERO)
}

fn unlocked(resource: &Document) -> bool {
    ["active_settlement", "active_topup_expiry"]
        .into_iter()
        .all(|key| resource.get(key).is_none_or(|value| *value == Bson::Null))
}

impl Page {
    pub(super) fn needs_detail(&self, collection: &str, resource: &Document) -> bool {
        if collection == WALLETS && resource.get_i32("exact_accounting_version").ok() != Some(2) {
            return false;
        }
        let id = resource.get_str("_id").unwrap_or("");
        if self.present.contains(id)
            || self.previous.contains_key(&format!("{collection}:{id}"))
            || !unlocked(resource)
        {
            return true;
        }
        match collection {
            WALLETS => nonzero(resource, "reserved_credits", None),
            GRANTS => {
                nonzero(resource, "reserved", Some("reserved_micros"))
                    || match resource.get("service_limits") {
                        None => false,
                        Some(Bson::Array(rules)) => rules.iter().any(|rule| {
                            rule.as_document().is_none_or(|rule| {
                                nonzero(rule, "reserved", Some("reserved_micros"))
                            })
                        }),
                        Some(_) => true,
                    }
            }
            _ => resource
                .get("reserved_quantity")
                .is_some_and(|v| v.as_i64().or_else(|| v.as_i32().map(i64::from)) != Some(0)),
        }
    }
    pub(super) fn previous(&self, collection: &str, resource: &Document) -> Option<&Document> {
        self.previous.get(&format!(
            "{collection}:{}",
            resource.get_str("_id").unwrap_or("")
        ))
    }
}

async fn checked_snapshot(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    resource: &Document,
    id: &str,
) -> AppResult<Option<Snapshot>> {
    match read_snapshot(db, session, collection, resource, id).await {
        Ok(snapshot) => Ok(Some(snapshot)),
        Err(error @ AppError::DatabaseError(_)) => Err(error),
        // A malformed amount/model or overflow is local to this resource.
        // Never log its raw data or let it stop asset verification.
        Err(_) => Ok(None),
    }
}

pub(super) async fn verify(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    resource: &Document,
    previous: Option<&Document>,
    pass: &str,
    previous_pass: &str,
) -> AppResult<Option<Repair>> {
    let id = resource
        .get_str("_id")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let observation_id = format!("{collection}:{id}");
    let observations = db.collection::<Document>(OBSERVATIONS);
    let snapshot = checked_snapshot(db, session, collection, resource, id).await?;
    let exact = snapshot.as_ref().is_some_and(|s| s.exact);
    if snapshot
        .as_ref()
        .is_some_and(|s| s.exact && s.totals.stored == s.totals.expected)
    {
        if previous.is_some() {
            observations
                .delete_one(doc! {"_id": &observation_id})
                .session(&mut *session)
                .await?;
        }
        return Ok(None);
    }
    let fingerprint = if let Some(snapshot) = &snapshot {
        snapshot.fingerprint.clone()
    } else {
        hex::encode(Sha256::digest(
            bson::to_vec(resource).unwrap_or_else(|_| observation_id.as_bytes().to_vec()),
        ))
    };
    let repair_fingerprint = snapshot
        .as_ref()
        .and_then(|s| s.repair_fingerprint.as_deref());
    let excess = snapshot.as_ref().filter(|s| s.exact).map(|s| s.difference);
    let excesses = snapshot
        .as_ref()
        .filter(|s| s.exact)
        .map(|s| s.totals.excesses())
        .transpose()?
        .flatten();
    let stored_excesses = excesses
        .as_ref()
        .map(bson::to_document)
        .transpose()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let same = previous.is_some_and(|old| {
        old.get_str("fingerprint").ok() == Some(fingerprint.as_str())
            && old.get_bool("exact").unwrap_or(false) == exact
    });
    if same && previous.is_some_and(|old| old.get_str("pass").ok() == Some(pass)) {
        // A held asset cursor revisits this pass. Preserve its confirmation,
        // including eligibility established by two distinct observations.
        let repairable = exact
            && excesses.is_some()
            && unlocked(resource)
            && previous.is_some_and(|old| {
                old.get_bool("repairable").unwrap_or(false)
                    && old.get_str("repair_fingerprint").ok() == repair_fingerprint
            });
        return Ok(repairable.then(|| Repair {
            collection: collection.to_owned(),
            resource_id: id.to_owned(),
            pass: pass.to_owned(),
        }));
    }
    let unchanged = exact
        && same
        && previous.is_some_and(|old| old.get_str("pass").ok() == Some(previous_pass));
    let persistent =
        unchanged && previous.is_some_and(|old| old.get_bool("old_enough").unwrap_or(false));
    let stable_since = if unchanged {
        previous
            .and_then(|old| old.get_datetime("stable_since").ok())
            .copied()
            .map(Bson::DateTime)
            .unwrap_or_else(|| Bson::String("$$NOW".into()))
    } else {
        Bson::String("$$NOW".into())
    };
    let owner_field = match collection {
        WALLETS => "owner_id",
        GRANTS => "recipient_user_id",
        _ => "owner_user_id",
    };
    let owner = resource.get_str(owner_field).ok();
    let wallet_id = if collection == WALLETS {
        Some(id.to_owned())
    } else if let Some(owner) = owner {
        db.collection::<Document>(WALLETS)
            .find_one(doc! {"owner_id":owner})
            .projection(doc! {"_id":1})
            .session(&mut *session)
            .await?
            .and_then(|row| row.get_str("_id").ok().map(str::to_owned))
    } else {
        None
    };
    let repairable = persistent
        && unlocked(resource)
        && excesses.is_some()
        && previous.is_some_and(|old| old.get_str("repair_fingerprint").ok() == repair_fingerprint);
    let kind = match collection {
        WALLETS => match excess {
            Some(value) if value < Credits::ZERO => "wallet_deficit",
            Some(_) => "wallet_excess",
            None => "wallet",
        },
        GRANTS => "grant",
        _ => "allowance",
    };
    let difference = excess.map(|value| {
        if value > Credits::ZERO {
            format!("+{value}")
        } else {
            value.to_string()
        }
    });
    observations.update_one(doc! {"_id": &observation_id}, vec![doc! {"$set": {
        "wallet_id":wallet_id,"kind":kind,"difference":difference,
        "pass":pass,"previous_pass":previous_pass,"fingerprint":fingerprint,
        "repair_fingerprint":repair_fingerprint,"repairable":repairable,"excesses":stored_excesses,
        "exact":exact,"persistent":persistent,"stable_since":stable_since,"updated_at":"$$NOW",
    }}]).upsert(true).session(&mut *session).await?;
    Ok(repairable.then(|| Repair {
        collection: collection.to_owned(),
        resource_id: id.to_owned(),
        pass: pass.to_owned(),
    }))
}

/// Recheck a committed observation in a small snapshot transaction. The caller
/// writes the exact live account-runner lease before entering this function.
pub(super) async fn repair(
    db: &Database,
    session: &mut ClientSession,
    candidate: &Repair,
) -> AppResult<()> {
    let id = &candidate.resource_id;
    let collection = candidate.collection.as_str();
    let Some(observation) = db
        .collection::<Document>(OBSERVATIONS)
        .find_one(doc! {
            "_id":format!("{collection}:{id}"), "pass":&candidate.pass,
            "stable_since":{"$type":"date"},
            "$expr":{"$and":[{"$lte":["$stable_since",{"$subtract":["$$NOW",MIN_STABLE_MILLIS]}]}, {"$ne":["$pass","$previous_pass"]}]},
            "exact":true, "persistent":true, "repairable":true,
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    let mut filter =
        doc! {"_id":id,"active_settlement":Bson::Null,"active_topup_expiry":Bson::Null};
    if collection == WALLETS {
        filter.insert("exact_accounting_version", 2);
    }
    let Some(resource) = db
        .collection::<Document>(collection)
        .find_one(filter)
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    let Some(Snapshot {
        totals,
        exact,
        repair_fingerprint,
        ..
    }) = checked_snapshot(db, session, collection, &resource, id).await?
    else {
        return Ok(());
    };
    if !exact || observation.get_str("repair_fingerprint").ok() != repair_fingerprint.as_deref() {
        return Ok(());
    }
    let Some(excesses) = totals.excesses()? else {
        return Ok(());
    };
    let observed_excesses =
        bson::to_document(&excesses).map_err(|e| AppError::Internal(e.to_string()))?;
    if collection != WALLETS
        && observation.get_document("excesses").ok() != Some(&observed_excesses)
    {
        return Ok(());
    }

    let pass = &candidate.pass;
    let previous_pass = observation
        .get_str("previous_pass")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    #[cfg(test)]
    crate::billing_integration_tests::hold_release::point(
        crate::billing_integration_tests::hold_release::Phase::BeforeHoldRepair,
    )
    .await;
    // Every current admission or hold handoff writes this resource. Its write
    // conflicts with any movement since the snapshot, including a new row.
    let (filter, update) = if collection == WALLETS {
        (
            doc! {"_id":id,"reserved_credits":totals.stored["total"],"active_settlement":Bson::Null,"active_topup_expiry":Bson::Null},
            doc! {"$inc":{"reserved_credits": -excesses["total"]},"$set":{"updated_at":bson::DateTime::now()}},
        )
    } else {
        benefits::correction(collection, &resource, &totals.expected, &excesses)?
    };
    let changed = db
        .collection::<Document>(collection)
        .update_one(filter, update)
        .session(&mut *session)
        .await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict(
            "Resource changed during hold repair".into(),
        ));
    }
    #[cfg(test)]
    crate::billing_integration_tests::hold_release::point(
        crate::billing_integration_tests::hold_release::Phase::AfterHoldRepairWallet,
    )
    .await;
    let key = crate::services::audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("audit key unavailable for hold repair".into()))?;
    let (event, metadata) = if collection == WALLETS {
        (
            "billing_wallet_hold_excess_repaired",
            serde_json::json!({"wallet_id":id,"amount":excesses["total"].to_string(),"previous_pass":previous_pass,"pass":pass}),
        )
    } else {
        (
            if collection == GRANTS {
                "billing_grant_hold_excess_repaired"
            } else {
                "billing_allowance_hold_excess_repaired"
            },
            serde_json::json!({"resource_id":id,"wallet_id":observation.get_str("wallet_id").ok(),"amounts":excesses,"previous_pass":previous_pass,"pass":pass}),
        )
    };
    crate::services::audit_service::log_system_event_in_session(db, session, key, event, metadata)
        .await?;
    #[cfg(test)]
    crate::billing_integration_tests::hold_release::point(
        crate::billing_integration_tests::hold_release::Phase::AfterHoldRepairAudit,
    )
    .await;

    let removed = db
        .collection::<Document>(OBSERVATIONS)
        .delete_one(doc! {"_id":format!("{collection}:{id}"),"pass":pass})
        .session(&mut *session)
        .await?;
    if removed.deleted_count != 1 {
        return Err(AppError::Conflict(
            "Hold observation changed during repair".into(),
        ));
    }
    #[cfg(test)]
    crate::billing_integration_tests::hold_release::point(
        crate::billing_integration_tests::hold_release::Phase::AfterHoldObservationDelete,
    )
    .await;
    Ok(())
}

pub(super) async fn diagnostics(db: &Database) -> AppResult<Vec<super::BillingStartupDiagnostic>> {
    let specifications = [
        (
            "wallet_excess",
            true,
            "Automatic excess repair waits for unchanged wallet state, no lock and a clean asset result.",
        ),
        (
            "wallet_deficit",
            true,
            "Deficits are reported; reserved credits are not changed.",
        ),
        (
            "grant",
            true,
            "Grant excess repair waits for stable exact counters, no deficit or lock and a clean asset result.",
        ),
        (
            "allowance",
            true,
            "Allowance excess repair waits for stable exact units, no deficit or lock and a clean asset result.",
        ),
        (
            "wallet",
            false,
            "An incomplete wallet check never authorizes repair; it repeats on the account cadence.",
        ),
        (
            "grant",
            false,
            "An incomplete grant check never changes counters; it repeats on the account cadence.",
        ),
        (
            "allowance",
            false,
            "An incomplete allowance check never changes counters; it repeats on the account cadence.",
        ),
    ];
    let mut facets = Document::new();
    for (index, (kind, exact, _)) in specifications.iter().enumerate() {
        let mut filter = doc! {"kind":*kind,"exact":*exact};
        if *exact {
            filter.insert("persistent", true);
        }
        let pipeline = vec![
            doc! {"$match":filter},
            doc! {"$sort":{"_id":1}},
            // A wallet groups its resources; an owner with no wallet still
            // has a distinct resource key. Samples retain one resource's
            // signed difference, never add unrelated allowance units.
            doc! {"$group":{"_id":{"$ifNull":["$wallet_id","$_id"]},
            "difference":{"$first":"$difference"},
            "resources":{"$sum":1},
            "has_wallet":{"$first":{"$ne":[{"$ifNull":["$wallet_id",Bson::Null]},Bson::Null]}}}},
        ];
        let mut count = pipeline.clone();
        count.push(doc! {"$group":{"_id":Bson::Null,
            "resources":{"$sum":"$resources"},
            "wallets":{"$sum":{"$cond":["$has_wallet",1,0]}},
            "without_wallet":{"$sum":{"$cond":["$has_wallet",0,"$resources"]}},
        }});
        facets.insert(format!("count_{index}"), count);
        let mut samples = pipeline;
        samples.extend([doc! {"$sort":{"_id":1}}, doc! {"$limit":5}]);
        facets.insert(format!("samples_{index}"), samples);
    }
    let rows = db
        .collection::<Document>(OBSERVATIONS)
        .aggregate(vec![doc! {"$facet":facets}])
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let Some(row) = rows.first() else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    for exact in [true, false] {
        let mut details = Vec::new();
        let mut remediations = Vec::new();
        for (index, (kind, group_exact, remediation)) in specifications.iter().enumerate() {
            if *group_exact != exact {
                continue;
            }
            let counts = row
                .get_array(format!("count_{index}"))
                .ok()
                .and_then(|v| v.first())
                .and_then(Bson::as_document);
            let count = |key: &str| {
                counts
                    .and_then(|v| v.get(key))
                    .and_then(|v| v.as_i64().or_else(|| v.as_i32().map(i64::from)))
                    .unwrap_or(0)
            };
            let resources = count("resources");
            if resources == 0 {
                continue;
            }
            let wallets = count("wallets");
            let without_wallet = count("without_wallet");
            let samples: Vec<_> = row
                .get_array(format!("samples_{index}"))
                .into_iter()
                .flatten()
                .filter_map(Bson::as_document)
                .filter_map(|v| {
                    v.get_str("_id")
                        .ok()
                        .map(|id| format!("{id}: {}", v.get_str("difference").unwrap_or("unknown")))
                })
                .collect();
            details.push(format!("{kind}: {resources} affected resources; {wallets} wallets; {without_wallet} resources without a wallet; sample differences (one resource per wallet, or unbound resource): {}",samples.join(", ")));
            remediations.push(format!("{kind}: {remediation}"));
        }
        if !details.is_empty() {
            output.push(super::BillingStartupDiagnostic {
                severity: super::DiagnosticSeverity::Action,
                technical_details: None,
                code: if exact {
                    "billing_hold_conservation"
                } else {
                    "billing_hold_conservation_incomplete"
                }
                .into(),
                summary: if exact {
                    "Persistent billing hold mismatches"
                } else {
                    "Billing hold checks incomplete"
                }
                .into(),
                detail: details.join("\n"),
                remediation: remediations.join("\n"),
                detected_at: Utc::now(),
            });
        }
    }
    output.extend(backlog::diagnostics(db).await?);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(value: &Bson, key: &str) -> Option<i64> {
        match value {
            Bson::Document(doc) => doc
                .get(key)
                .and_then(|v| v.as_i64().or_else(|| v.as_i32().map(i64::from)))
                .or_else(|| doc.values().find_map(|v| stat(v, key))),
            Bson::Array(values) => values.iter().find_map(|v| stat(v, key)),
            _ => None,
        }
    }

    #[tokio::test]
    async fn hold_diagnostics_reports_resources_and_unfinished_rows() {
        let db = crate::test_utils::connect_transaction_test_database("hold_diagnostics_red").await;
        db.collection::<Document>(METERS)
            .create_indexes(crate::db::billing_hold_indexes())
            .await
            .unwrap();
        for (id, wallet) in [("g1", Some("w")), ("g2", Some("w")), ("g3", None)] {
            db.collection::<Document>(OBSERVATIONS)
                .insert_one(doc! {
                    "_id":format!("{GRANTS}:{id}"),"wallet_id":wallet,"kind":"grant",
                    "exact":true,"persistent":true,"difference":"+0.0000008",
                })
                .await
                .unwrap();
        }
        db.collection::<Document>(GRANTS)
            .insert_one(doc! {
                "_id":"g1","recipient_user_id":"owner","active_settlement":Bson::Null,
            })
            .await
            .unwrap();
        db.collection::<Document>(METERS).insert_one(doc! {
            "_id":"unfinished","billing_owner_id":"owner","wallet_id":"w",
            "status":"forwarded","forwarded":true,"released":false,"quantity":Bson::Null,
            "metric":"tokens","created_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::days(8)),
            "funding":{"settled":false,"grant_reservations":[{"grant_id":"g1","amount":"0.0000008".parse::<Credits>().unwrap()}]},
        }).await.unwrap();
        let result = super::super::startup_diagnostics(&db).await.unwrap();
        db.drop().await.unwrap();
        let conservation = result
            .iter()
            .find(|d| d.code == "billing_hold_conservation")
            .unwrap();
        assert!(
            conservation
                .detail
                .contains("grant: 3 affected resources; 1 wallets; 1 resources without a wallet"),
            "{}",
            conservation.detail
        );
        let backlog = result
            .iter()
            .find(|d| d.code == "billing_hold_backlog")
            .expect("unfinished rows must be visible");
        assert!(backlog.technical_details.as_ref().unwrap().contains("grant status=forwarded age=over_7d unit=credits rows=1 held=0.0000008 resources=1 wallets=1 owners=1"),"{}",backlog.technical_details.as_ref().unwrap());
    }

    #[tokio::test]
    async fn hold_round3_presence_and_detail_use_partial_indexes() {
        let db = crate::test_utils::connect_transaction_test_database("hold_partial_plans").await;
        let meters = db.collection::<Document>(METERS);
        meters
            .create_indexes(crate::db::billing_hold_indexes())
            .await
            .unwrap();
        let mut rows: Vec<_> = (0..3000)
            .map(|i| {
                doc! {
                    "_id":format!("history-{i}"),"wallet_id":"w","released":true,
                }
            })
            .collect();
        for i in 0..3 {
            rows.push(
                doc! {"_id":format!("live-{i}"),"wallet_id":"w","released":false,
                "reserved_credits":Credits::from_whole(1),"funding":{
                    "grant_reservations":[{"grant_id":"g"}],
                    "allowance_reservations":[{"period_id":"p"}],
                }},
            );
        }
        meters.insert_many(rows).await.unwrap();
        for (collection, id) in [(WALLETS, "w"), (GRANTS, "g"), (PERIODS, "p")] {
            for presence in [true, false] {
                let command = if presence {
                    doc! {"aggregate":METERS,"pipeline":presence_pipeline(collection,&[id.to_owned()]),
                    "hint":live_index(collection),"cursor":{}}
                } else {
                    doc! {"find":METERS,"filter":live_filter(collection,id),
                    "hint":live_index(collection),"sort":{"_id":1},"limit":MAX_ROWS+1}
                };
                let explain = db
                    .run_command(doc! {"explain":command,"verbosity":"executionStats"})
                    .await
                    .unwrap();
                let text = format!("{explain:?}");
                assert!(!text.contains("COLLSCAN"), "{text}");
                assert!(
                    text.contains("IXSCAN") || text.contains("DISTINCT_SCAN"),
                    "{text}"
                );
                assert!(text.contains("isPartial\": Boolean(true)"), "{text}");
                let value = Bson::Document(explain);
                let docs = stat(&value, "totalDocsExamined").unwrap();
                let keys = stat(&value, "totalKeysExamined").unwrap();
                if collection == WALLETS && presence {
                    assert_eq!(docs, 0, "wallet presence must be index-only");
                }
                assert!(
                    keys <= 4,
                    "history must not dominate the live lookup: {text}"
                );
                println!(
                    "HOLD_R3_EXPLAIN collection={collection} presence={presence} keys={keys} docs={docs} {value:?}"
                );
            }
        }
        db.drop().await.unwrap();
    }
}
