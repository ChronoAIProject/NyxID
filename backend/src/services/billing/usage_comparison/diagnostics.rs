//! Provider-free, bounded sentences for Integrity.
use super::super::{BillingStartupDiagnostic, DiagnosticSeverity};
use super::*;

pub(crate) async fn diagnostics(db: &Database) -> AppResult<Vec<BillingStartupDiagnostic>> {
    if db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"retired_at":Bson::Null})
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(vec![]);
    }
    let capture = capture_sample(db).await?;
    let rows: Vec<Document> = db
        .collection::<Document>(CONTROL_COLLECTION)
        .aggregate([
            doc! {"$match":{"_id":COVERAGE_ID}},
            doc! {"$set":{"diagnostic_now":clock()}},
        ])
        .await?
        .try_collect()
        .await?;
    let raw = rows.first();
    let now = raw
        .and_then(|r| r.get_datetime("diagnostic_now").ok())
        .map(|n| n.to_chrono())
        .unwrap_or_else(Utc::now);
    let coverage = raw.and_then(|r| bson::from_document::<UsageCoverage>(r.clone()).ok());
    let Some(coverage) = coverage.filter(|c| {
        c.ready
            && c.continuous_since.is_some()
            && c.observed_at <= now
            && (now - c.observed_at).num_milliseconds() < coverage::EVIDENCE_HORIZON_MILLIS
    }) else {
        return Ok(vec![item(
            "billing_usage_coverage",
            DiagnosticSeverity::Info,
            "Usage comparison is warming",
            format!(
                "Usage comparison is waiting for a successful check of recorded receipt-capture support. Earlier results are not current. {capture}"
            ),
            "The next successful comparison pass checks the retained fleet evidence automatically.",
            now,
        )]);
    };
    let current = doc! {"$and":[{"$eq":["$comparison.generation",&coverage.generation]},
    {"$eq":["$comparison.sequence","$acknowledgement_sequence"]},
    {"$eq":["$comparison.uncertainty_revision",{"$ifNull":["$uncertainty_revision",0_i64]}]}]};
    let numeric = doc! {"$and":[&current,{"$eq":["$coverage_verified",true]},
    {"$lte":["$comparison.start_second",now.timestamp()]},{"$gt":["$comparison.end_second",now.timestamp()]}]};
    let result:Vec<Document>=db.collection::<Document>(COLLECTION_NAME).aggregate([
        doc! {"$match":{"retired_at":Bson::Null}},
        doc! {"$set":{"visible_status":{"$cond":[&current,{"$cond":[{"$in":["$comparison.status",["equal","drift"]]},
            {"$cond":[numeric,"$comparison.status","coverage"]},{"$ifNull":["$comparison.status","coverage"]}]},"coverage"]},
            "visible_reason":{"$cond":[&current,{"$ifNull":["$comparison.reason","not_observed"]},"evidence_changed"]}}},
        doc! {"$facet":{
            "counts":[{"$group":{"_id":{"status":"$visible_status","reason":"$visible_reason"},"count":{"$sum":1}}}],
            "drift":[{"$match":{"visible_status":"drift"}},{"$sort":{"_id":1}},{"$limit":5}],
            "unsupported":[{"$match":{"visible_status":"unsupported"}},{"$sort":{"_id":1}},
                {"$group":{"_id":"$visible_reason","count":{"$sum":1},"ids":{"$firstN":{"input":"$subscription_id","n":5}}}}],
        }},
    ]).max_time(std::time::Duration::from_secs(5)).await?.try_collect().await?;
    let Some(result) = result.first() else {
        return Ok(vec![]);
    };
    let mut drift = 0_i64;
    let mut waits = BTreeMap::<String, i64>::new();
    for row in documents(result, "counts") {
        let key = row
            .get_document("_id")
            .map_err(|_| invalid("Usage diagnostic count is malformed"))?;
        let status = key.get_str("status").unwrap_or("coverage");
        let count = count(row);
        if status == "drift" {
            drift += count;
        } else if !matches!(status, "equal" | "unsupported") {
            *waits
                .entry(key.get_str("reason").unwrap_or("not_observed").to_owned())
                .or_default() += count;
        }
    }
    let mut items = Vec::new();
    if drift > 0 {
        let mut detail = format!(
            "{drift} {} from the provider.",
            if drift == 1 {
                "subscription differs"
            } else {
                "subscriptions differ"
            }
        );
        for row in documents(result, "drift").take(5) {
            let Ok(observation) = row.get_document("comparison") else {
                continue;
            };
            let Ok(differences) = observation.get_document("differences") else {
                continue;
            };
            let Some((metric, difference)) = differences.iter().next() else {
                continue;
            };
            let local = observation
                .get_document("local")
                .ok()
                .and_then(|d| d.get_str(metric).ok())
                .unwrap_or("unknown");
            let remote = observation
                .get_document("remote")
                .ok()
                .and_then(|d| d.get_str(metric).ok())
                .unwrap_or("unknown");
            let start = second(observation, "start_second");
            let end = second(observation, "end_second");
            let first = date(observation, "first_observed_at");
            let last = date(observation, "observed_at");
            detail.push_str(&format!(" Subscription {} (owner {}), period {start} to {end}: {} delivered {local}, the provider has {remote}, difference {}, first seen {first}, last checked {last}.",
                id(row,"subscription_id"),id(row,"owner_id"),short(metric),difference.as_str().unwrap_or("unknown")));
            if differences.len() > 1 {
                detail.push_str(&format!(
                    " It also has {} other differing metrics.",
                    differences.len() - 1
                ));
            }
        }
        items.push(item("billing_usage_drift", DiagnosticSeverity::Action,"Delivered usage differs from provider usage",detail,
            "Investigate captured deliveries and provider intake. Comparison never repairs or charges usage.",now));
    }
    let mut unsupported = String::new();
    for row in documents(result, "unsupported") {
        let reason = row.get_str("_id").unwrap_or("unsupported_usage_shape");
        let ids = row
            .get_array("ids")
            .into_iter()
            .flatten()
            .filter_map(Bson::as_str)
            .take(5)
            .map(short)
            .collect::<Vec<_>>()
            .join(", ");
        unsupported.push_str(&format!(
            "{} {}: {}. Example subscription ids: {ids}. ",
            count(row),
            if count(row) == 1 {
                "subscription cannot be compared"
            } else {
                "subscriptions cannot be compared"
            },
            explanation(reason)
        ));
    }
    if !unsupported.is_empty() {
        items.push(item("billing_usage_unsupported", DiagnosticSeverity::Action,"Usage comparison needs attention",unsupported,
        "Check the subscription and provider charge layout. Stable subscription findings are revisited daily.",now));
    }
    if !waits.is_empty() || !capture.is_empty() {
        let mut detail = if waits.contains_key("warming_full_period") {
            "Usage comparison starts with the next full billing period. ".to_owned()
        } else {
            String::new()
        };
        detail.push_str(&format!(
            "Capture has been continuous since {}. ",
            human(coverage.continuous_since.unwrap())
        ));
        for (reason, count) in waits {
            let noun = if count == 1 {
                "subscription is"
            } else {
                "subscriptions are"
            };
            detail.push_str(&format!("{count} {noun} {}. ", wait_explanation(&reason)));
        }
        detail.push_str(&capture);
        items.push(item(
            "billing_usage_coverage",
            DiagnosticSeverity::Info,
            "Usage comparison coverage and waits",
            detail,
            "Comparison resumes automatically when the stated wait ends.",
            now,
        ));
    }
    Ok(items)
}

fn documents<'a>(document: &'a Document, key: &str) -> impl Iterator<Item = &'a Document> {
    document
        .get_array(key)
        .into_iter()
        .flatten()
        .filter_map(Bson::as_document)
}
fn count(row: &Document) -> i64 {
    row.get_i64("count")
        .or_else(|_| row.get_i32("count").map(i64::from))
        .unwrap_or(0)
}
fn short(value: &str) -> String {
    let mut chars = value.chars().filter(|c| !c.is_control());
    let mut s: String = chars.by_ref().take(96).collect();
    if chars.next().is_some() {
        s.push('…');
    }
    s
}
fn id(row: &Document, key: &str) -> String {
    short(row.get_str(key).unwrap_or("unknown"))
}
fn human(at: chrono::DateTime<Utc>) -> String {
    at.format("%-d %b %Y %H:%M UTC").to_string()
}
fn date(row: &Document, key: &str) -> String {
    row.get_datetime(key)
        .map(|d| human(d.to_chrono()))
        .unwrap_or_else(|_| "unknown".into())
}
fn second(row: &Document, key: &str) -> String {
    row.get_i64(key)
        .ok()
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(human)
        .unwrap_or_else(|| "unknown".into())
}
fn explanation(reason: &str) -> String {
    match reason {
        "subscription_unavailable" => "the provider no longer exposes the subscription".into(),
        "provider_request_rejected" => "the provider rejected the usage request".into(),
        "delivered_metric_missing" => "a delivered metric is missing from provider usage".into(),
        "provider_target_mismatch" => "the returned customer or subscription does not match".into(),
        "response_too_large" => "the response exceeds the comparison size limit".into(),
        "invalid_provider_period" => "the provider period is invalid".into(),
        "unsupported_provider_units" => "the quantities cannot be represented exactly".into(),
        "duplicate_metric" => "the provider returned the same metric more than once".into(),
        "missing_metric_code" => "the provider omitted a metric code".into(),
        "missing_customer_usage" => "the provider omitted customer_usage".into(),
        "missing_charges_usage" => {
            "the provider omitted charges_usage or returned it in a non-array form".into()
        }
        "charge_not_object" => "a charges_usage entry is not an object".into(),
        "too_many_charges" => "the provider returned more than 256 charges".into(),
        "breakdown_mismatch" => {
            "a provider usage breakdown does not add up to its charge total".into()
        }
        "unsupported_usage_shape" => {
            "the previous release recorded an unsupported provider usage shape".into()
        }
        reason
            if reason
                .strip_prefix("non_sum_aggregation:")
                .is_some_and(valid_aggregation_name) =>
        {
            let name = reason
                .strip_prefix("non_sum_aggregation:")
                .unwrap_or("other");
            format!("a delivered metric has non-sum provider aggregation '{name}'")
        }
        _ => "the provider returned an unrecognized comparison finding".into(),
    }
}

fn valid_aggregation_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}
fn wait_explanation(reason: &str) -> &'static str {
    match reason {
        "warming_full_period" => "waiting for that full billing period",
        "acknowledgements_not_quiet" => "waiting for ten quiet minutes",
        "unresolved_deliveries_in_period" => "waiting for an unresolved delivery",
        "duplicate_delivery_origin_unverified" => {
            "waiting for a new period because a delivery's origin could not be verified"
        }
        "legacy_capture_uncertainty" | "capture_uncertainty_in_period" => {
            "waiting for a period after the recorded capture uncertainty"
        }
        "receipt_history_outside_retention" => {
            "waiting for a period within retained receipt history"
        }
        "projection_backlog" | "projection_pending" => "catching up on captured deliveries",
        "awaiting_persistent_difference" => "waiting to confirm a persistent difference",
        "provider_try_later" => "waiting because the provider asked to try later",
        "provider_or_projection_failed"
        | "receipt_projection_failed"
        | "capture_evidence_unreadable" => {
            "waiting after the last check failed; earlier findings are stale"
        }
        _ => "waiting for a fresh observation",
    }
}
async fn capture_sample(db: &Database) -> AppResult<String> {
    use crate::models::billing_usage_delivery::COLLECTION_NAME as DELIVERIES;
    let rows: Vec<Document> = db
        .collection(DELIVERIES)
        .find(doc! {"state":{"$in":["prepared","rejected"]}})
        .projection(doc! {"subscription_id":1,"state":1,"code":1,"properties.quantity":1})
        .sort(doc! {"state":1,"last_checked_at":1,"_id":1})
        .limit(1000)
        .batch_size(1000)
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() {
        return Ok(String::new());
    }
    let examples = rows
        .iter()
        .take(5)
        .map(|row| {
            let quantity = row
                .get_document("properties")
                .ok()
                .and_then(|p| p.get("quantity"))
                .and_then(|v| bson::from_bson::<serde_json::Value>(v.clone()).ok())
                .and_then(|v| super::super::lago_client::current_usage::units(&v).ok())
                .map(|q| observation::decimal(q, false))
                .unwrap_or_else(|| "unknown".into());
            format!(
                "subscription {} has {} quantity {quantity} for {}",
                id(row, "subscription_id"),
                id(row, "state"),
                id(row, "code")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Ok(format!(
        "A retained sample contains {} prepared or rejected deliveries, not the complete historical backlog: {examples}.",
        rows.len()
    ))
}
fn item(
    code: &str,
    severity: DiagnosticSeverity,
    summary: &str,
    detail: String,
    remediation: &str,
    detected_at: chrono::DateTime<Utc>,
) -> BillingStartupDiagnostic {
    let mut chars = detail.chars();
    let mut bounded: String = chars.by_ref().take(4095).collect();
    if chars.next().is_some() {
        bounded.push('…');
    }
    BillingStartupDiagnostic {
        code: code.into(),
        severity,
        technical_details: None,
        summary: summary.into(),
        detail: bounded,
        remediation: remediation.into(),
        detected_at,
    }
}
