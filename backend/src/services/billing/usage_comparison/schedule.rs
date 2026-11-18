//! Due times, discovery and read budgets are durable across process changes.
use super::*;
use crate::models::billing_wallet::COLLECTION_NAME as WALLETS;
use std::collections::BTreeSet;

pub(super) const SUBSCRIPTIONS_PER_PASS: usize = 20;
const WALLET_DISCOVERY_LIMIT: i64 = 100;

pub(super) async fn discover(db: &Database) -> AppResult<Vec<AppError>> {
    let control = db
        .collection::<Document>(CONTROL_COLLECTION)
        .find_one(doc! {"_id":SCHEDULE_ID})
        .await?;
    let cursor = control
        .as_ref()
        .and_then(|r| r.get_str("wallet_cursor").ok())
        .unwrap_or("");
    let wallets = db.collection::<Document>(WALLETS);
    let mut rows: Vec<Document> = wallets
        .find(doc! {"_id":{"$gt":cursor}})
        .projection(doc! {"_id":1,"owner_id":1,"lago_customer_id":1,"lago_subscription_id":1})
        .sort(doc! {"_id":1})
        .limit(WALLET_DISCOVERY_LIMIT)
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() && !cursor.is_empty() {
        rows = wallets
            .find(doc! {})
            .projection(doc! {"_id":1,"owner_id":1,"lago_customer_id":1,"lago_subscription_id":1})
            .sort(doc! {"_id":1})
            .limit(WALLET_DISCOVERY_LIMIT)
            .await?
            .try_collect()
            .await?;
    }
    let mut errors = Vec::new();
    let mut candidates = Vec::new();
    for wallet in &rows {
        let Some(subscription) = wallet
            .get_str("lago_subscription_id")
            .ok()
            .filter(|s| !s.trim().is_empty())
        else {
            continue;
        };
        let Some((owner, customer, id)) = wallet
            .get_str("owner_id")
            .ok()
            .zip(wallet.get_str("lago_customer_id").ok())
            .zip(wallet.get_str("_id").ok())
            .map(|((o, c), id)| (o, c, id))
            .filter(|(o, c, _)| !o.trim().is_empty() && !c.trim().is_empty())
        else {
            errors.push(invalid("Wallet usage target is incomplete"));
            continue;
        };
        candidates.push((
            owner.to_owned(),
            customer.to_owned(),
            subscription.to_owned(),
            id.to_owned(),
        ));
    }
    let mut existing = BTreeSet::new();
    if !candidates.is_empty() {
        let matches: Vec<_> = candidates
            .iter()
            .map(|(o, c, s, _)| doc! {"owner_id":o,"customer_id":c,"subscription_id":s})
            .collect();
        let targets: Vec<Document> = db.collection(COLLECTION_NAME).find(doc! {"$or":matches})
            .projection(doc! {"owner_id":1,"customer_id":1,"subscription_id":1,"wallet_id":1,"retired_at":1})
            .limit(WALLET_DISCOVERY_LIMIT).await?.try_collect().await?;
        for target in targets {
            if let (Ok(owner), Ok(customer), Ok(subscription)) = (
                target.get_str("owner_id"),
                target.get_str("customer_id"),
                target.get_str("subscription_id"),
            ) {
                existing.insert((
                    owner.to_owned(),
                    customer.to_owned(),
                    subscription.to_owned(),
                ));
                if let Some((_, _, _, wallet)) = candidates
                    .iter()
                    .find(|(o, c, s, _)| o == owner && c == customer && s == subscription)
                    && (target.get_str("wallet_id").ok() != Some(wallet.as_str())
                        || target.get_datetime("retired_at").is_ok())
                {
                    db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":target.get("_id")},
                        doc! {"$set":{"wallet_id":wallet,"retired_at":Bson::Null,"next_visit_at":Bson::Null}}).await?;
                }
            }
        }
    }
    let mut inserted = false;
    for (owner, customer, subscription, wallet) in candidates {
        if existing.contains(&(owner.clone(), customer.clone(), subscription.clone())) {
            continue;
        }
        if !super::super::reconcile::continue_reconcile() {
            break;
        }
        let result = db.collection::<Document>(COLLECTION_NAME).update_one(
            doc! {"owner_id":&owner,"customer_id":&customer,"subscription_id":&subscription},
            doc! {"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"owner_id":owner,"wallet_id":wallet,
                "customer_id":customer,"subscription_id":subscription,"acknowledgement_sequence":0_i64,
                "coverage_verified":false,"warming_since":bson::DateTime::now()}},
        ).upsert(true).await;
        match result {
            Ok(result) => inserted |= result.upserted_id.is_some(),
            Err(error) => errors.push(error.into()),
        }
    }
    let last = rows
        .last()
        .and_then(|r| r.get_str("_id").ok())
        .unwrap_or("");
    if last != cursor
        && (rows.len() == WALLET_DISCOVERY_LIMIT as usize
            || inserted
            || rows.is_empty()
            || !cursor.is_empty())
    {
        db.collection::<Document>(CONTROL_COLLECTION)
            .update_one(
                doc! {"_id":SCHEDULE_ID},
                doc! {"$set":{"wallet_cursor":last,"kind":"schedule"}},
            )
            .upsert(true)
            .await?;
    }
    Ok(errors)
}

pub(super) async fn select(db: &Database, now: chrono::DateTime<Utc>) -> AppResult<Vec<Document>> {
    let mut rows: Vec<Document> = db.collection(COLLECTION_NAME).find(doc! {"retired_at":Bson::Null,
        "$or":[{"next_visit_at":Bson::Null},{"next_visit_at":{"$lte":bson::DateTime::from_chrono(now)}}]})
        .sort(doc! {"next_visit_at":1,"_id":1}).limit(SUBSCRIPTIONS_PER_PASS as i64)
        .await?.try_collect().await?;
    let remaining = SUBSCRIPTIONS_PER_PASS - rows.len();
    if remaining > 0 {
        let selected: Vec<_> = rows
            .iter()
            .filter_map(|row| row.get("_id").cloned())
            .collect();
        let legacy: Result<Vec<Document>, mongodb::error::Error> = async { db.collection(COLLECTION_NAME)
            .find(doc! {"retired_at":Bson::Null,"comparison.reason":"unsupported_usage_shape","next_visit_at":{"$gt":bson::DateTime::from_chrono(now)},"_id":{"$nin":selected}})
            .sort(doc! {"next_visit_at":1,"_id":1})
            .hint(mongodb::options::Hint::Name(super::super::usage_delivery::LEGACY_USAGE_SHAPE_INDEX.into()))
            .limit(remaining as i64)
            .await?.try_collect().await }.await;
        match legacy {
            Ok(legacy) => rows.extend(legacy),
            Err(error) => {
                if let Some(code) = missing_legacy_index_code(&error) {
                    tracing::warn!(
                        database_code = code,
                        "Legacy usage lookup skipped because its index is unavailable"
                    );
                } else {
                    return Err(error.into());
                }
            }
        }
    }
    Ok(rows)
}

pub(super) async fn live_wallets(
    db: &Database,
    targets: &[BillingUsageComparison],
) -> AppResult<BTreeSet<(String, String, String, String)>> {
    if targets.is_empty() {
        return Ok(BTreeSet::new());
    }
    let ids: Vec<_> = targets.iter().map(|t| t.wallet_id.as_str()).collect();
    let rows: Vec<Document> = db
        .collection(WALLETS)
        .find(doc! {"_id":{"$in":ids}})
        .projection(doc! {"owner_id":1,"lago_customer_id":1,"lago_subscription_id":1})
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some((
                r.get_str("_id").ok()?.to_owned(),
                r.get_str("owner_id").ok()?.to_owned(),
                r.get_str("lago_customer_id").ok()?.to_owned(),
                r.get_str("lago_subscription_id").ok()?.to_owned(),
            ))
        })
        .collect())
}

pub(super) async fn reserve(db: &Database, requested: usize) -> AppResult<usize> {
    if requested == 0 {
        return Ok(0);
    }
    let now = clock();
    let day = doc! {"$dateTrunc":{"date":&now,"unit":"day","timezone":"UTC"}};
    let row=db.collection::<Document>(CONTROL_COLLECTION).find_one_and_update(doc! {"_id":SCHEDULE_ID},vec![
        doc! {"$set":{"kind":"schedule","clock":&now,
            "attempts":{"$filter":{"input":{"$ifNull":["$attempts",[]]},"as":"attempt","cond":{"$gt":["$$attempt",{"$subtract":[&now,300_000_i64]}]}}},
            "day_reads":{"$cond":[{"$gte":["$day",&day]},{"$ifNull":["$day_reads",0_i64]},0_i64]},"day":{"$max":["$day",&day]}}},
        doc! {"$set":{"reserved":{"$max":[0_i64,{"$min":[requested as i64,{"$subtract":[20_i64,{"$size":"$attempts"}]},{"$subtract":[5_760_i64,"$day_reads"]}]}]}}},
        doc! {"$set":{"day_reads":{"$add":["$day_reads","$reserved"]},
            "attempts":{"$concatArrays":["$attempts",{"$map":{"input":{"$range":[0,"$reserved"]},"as":"slot","in":&now}}]}}},
    ]).upsert(true).return_document(ReturnDocument::After).await?.ok_or_else(||invalid("Usage read reservation is unavailable"))?;
    usize::try_from(
        row.get_i64("reserved")
            .map_err(|_| invalid("Usage read reservation is malformed"))?,
    )
    .map_err(|_| invalid("Usage read reservation is negative"))
}

fn missing_legacy_index_code(error: &mongodb::error::Error) -> Option<i32> {
    match error.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command)
            if command.code == 27
                || (command.code == 2
                    && command
                        .message
                        .contains("hint provided does not correspond to an existing index")) =>
        {
            Some(command.code)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_comparison_legacy_query_error_classification_is_narrow() {
        for (code, message, expected) in [
            (
                2,
                "planner: hint provided does not correspond to an existing index",
                Some(2),
            ),
            (27, "index not found", Some(27)),
            (2, "hint must be a string or an object", None),
            (2, "invalid query", None),
            (13, "unauthorized", None),
            (26, "namespace not found", None),
            (50, "query timed out", None),
            (175, "query plan killed", None),
        ] {
            let command: mongodb::error::CommandError = bson::from_document(doc! {
                "code":code, "codeName":"fixture", "errmsg":message,
            })
            .unwrap();
            let error = mongodb::error::ErrorKind::Command(command).into();
            assert_eq!(missing_legacy_index_code(&error), expected);
        }
    }
}
