//! Immutable grant issuance and consumption, grouped by financial owner.
//! Shared grants have no service issuance allocation; service rows report usage only.
use super::{AdminUsageQuery, QUERY_TIMEOUT, UsageWindow, query_error};
use crate::{
    errors::{AppError, AppResult},
    models::credits::Credits,
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Bson, Document, doc},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

const PER_PAGE: u64 = 50;

#[derive(Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub period: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub owners: Option<String>,
    pub page: Option<u64>,
    pub service_page: Option<u64>,
}
#[derive(Serialize, ToSchema)]
pub struct Account {
    pub owner_id: String,
    pub label: String,
    pub account_kind: String,
    pub issued: Credits,
    pub used: Credits,
    pub grants: i64,
}
#[derive(Serialize, ToSchema)]
pub struct Service {
    /// None identifies historical consumption without a recorded service.
    pub service_slug: Option<String>,
    pub used: Credits,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Interval {
    Day,
}
#[derive(Serialize, ToSchema)]
pub struct TrendBucket {
    /// UTC day bounds clipped to the selected inclusive-start, exclusive-end window.
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub issued: Credits,
    pub used: Credits,
}
#[derive(Serialize, ToSchema)]
pub struct Response {
    pub window: UsageWindow,
    pub generated_at: DateTime<Utc>,
    pub issued: Credits,
    pub used: Credits,
    pub grants: i64,
    pub total_accounts: i64,
    pub accounts: Vec<Account>,
    pub page: u64,
    pub per_page: u64,
    pub total_services: i64,
    pub services: Vec<Service>,
    pub service_page: u64,
    pub interval: Interval,
    pub trend: Vec<TrendBucket>,
}
fn page(value: Option<u64>, name: &str) -> AppResult<u64> {
    let value = value.unwrap_or(1);
    if !(1..=100_000).contains(&value) {
        return Err(AppError::BadRequest(format!(
            "{name} must be between 1 and 100000"
        )));
    }
    Ok(value)
}

pub async fn report(db: &Database, q: Query, now: DateTime<Utc>) -> AppResult<Response> {
    let page = page(q.page, "page")?;
    let service_page = self::page(q.service_page, "service_page")?;
    let params = AdminUsageQuery {
        period: q.period,
        from: q.from,
        to: q.to,
        owners: q.owners,
        ..Default::default()
    }
    .validate(now)?;
    tokio::time::timeout(QUERY_TIMEOUT + std::time::Duration::from_secs(2), async {
        // Each OR branch uses an existing event_type/created_at index. Do not
        // derive this report from mutable grants or acting-user memberships.
        let mut filter = doc! {
            "created_at": {"$gte": bson::DateTime::from_chrono(params.window.from), "$lt": bson::DateTime::from_chrono(params.window.to)},
            "$or": [
                {"event_type": {"$in": ["grant_issued", "grant_consumed"]}},
                {"event_type": "accounting_v2", "movement": {"$in": ["grant_issued", "grant_consumed"]}}
            ],
        };
        if !params.selection.owners.is_empty() {
            filter.insert("owner_id", doc! {"$in": &params.selection.owners});
        }
        // A v2 transfer has two balanced legs. Count only its matching financial
        // source: promotions for issuance, grant:{reference_id} for consumption.
        let amount = doc! {"$cond": [
            {"$eq": ["$event_type", "accounting_v2"]},
            {"$reduce": {"input": {"$filter": {"input": "$postings", "as": "p", "cond": {"$and": [
                {"$eq": ["$$p.account", {"$cond": [
                    {"$eq": ["$movement", "grant_issued"]}, "platform:promotions", {"$concat": ["grant:", "$reference_id"]}
                ]}]}, {"$eq": ["$$p.side", "credit"]}
            ]}}}, "initialValue": Credits::ZERO, "in": {"$add": ["$$value", "$$this.amount"]}}},
            {"$ifNull": [{"$divide": [{"$toDecimal": "$amount_micros"}, 1_000_000]}, {"$toDecimal": "$amount_credits"}]}
        ]};
        let account_group = doc! {"$group": {"_id": "$owner_id", "issued": {"$sum": "$issued"}, "used": {"$sum": "$used"}, "grants": {"$sum": "$grants"}}};
        let service_group = doc! {"$group": {"_id": "$service_slug", "used": {"$sum": "$used"}}};
        let pipeline = vec![
                doc! {"$match": filter},
                doc! {"$project": {
                    "owner_id": 1, "service_slug": {"$ifNull": ["$service_slug", Bson::Null]},
                    "day": {"$dateTrunc": {"date": "$created_at", "unit": "day", "timezone": "UTC"}},
                    "is_issuance": {"$eq": [{"$cond": [{"$eq": ["$event_type", "accounting_v2"]}, "$movement", "$event_type"]}, "grant_issued"]},
                    "amount": amount,
                }},
                doc! {"$project": {
                    "owner_id": 1, "service_slug": 1, "day": 1, "is_issuance": 1,
                    "issued": {"$cond": ["$is_issuance", "$amount", Credits::ZERO]},
                    "used": {"$cond": ["$is_issuance", Credits::ZERO, "$amount"]},
                    "grants": {"$cond": ["$is_issuance", 1_i64, 0_i64]},
                }},
                doc! {"$facet": {
                    "totals": [account_group.clone(), {"$group": {"_id": Bson::Null, "issued": {"$sum": "$issued"}, "used": {"$sum": "$used"}, "grants": {"$sum": "$grants"}, "accounts": {"$sum": 1_i64}}}],
                    "accounts": [account_group, {"$sort": {"issued": -1, "used": -1, "_id": 1}}, {"$skip": ((page-1)*PER_PAGE) as i64}, {"$limit": PER_PAGE as i64}],
                    "services": [{"$match": {"is_issuance": false}}, service_group.clone(), {"$sort": {"used": -1, "_id": 1}}, {"$skip": ((service_page-1)*PER_PAGE) as i64}, {"$limit": PER_PAGE as i64}],
                    "service_count": [{"$match": {"is_issuance": false}}, service_group, {"$count": "count"}],
                    "trend": [{"$group": {"_id": "$day", "issued": {"$sum": "$issued"}, "used": {"$sum": "$used"}}}, {"$sort": {"_id": 1}}],
                }},
        ];
        let rows = retry_snapshot_read(|| async {
            db.collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
                .aggregate(pipeline.clone()).max_time(QUERY_TIMEOUT).allow_disk_use(true)
                .read_concern(mongodb::options::ReadConcern::snapshot())
                .await?.try_collect().await
        }).await?;
        let root = rows.first().ok_or_else(|| AppError::Internal("Missing grant report".into()))?;
        let totals = documents(root, "totals")?.next();
        let accounts: Vec<&Document> = documents(root, "accounts")?.collect();
        let ids: Vec<String> = accounts.iter()
            .filter_map(|d| d.get_str("_id").ok().map(str::to_owned)).collect();
        let identities = crate::services::reporting_identity_service::resolve(db, &ids, QUERY_TIMEOUT).await.map_err(query_error)?;
        let mut output = Vec::new();
        for row in accounts {
            let owner_id = row.get_str("_id").map_err(|_| AppError::Internal("Invalid grant owner".into()))?.to_owned();
            let identity = identities.get(&owner_id);
            output.push(Account {
                label: identity.map_or("Deleted account", |i| i.label()).into(),
                account_kind: identity.map_or("unknown", |i| i.user_type.as_str()).into(),
                owner_id, issued: money(row, "issued")?, used: money(row, "used")?, grants: count(row, "grants"),
            });
        }
        let services = documents(root, "services")?.map(|row| Ok(Service {
            service_slug: row.get_str("_id").ok().map(str::to_owned),
            used: money(row, "used")?,
        })).collect::<AppResult<Vec<_>>>()?;
        let mut days = HashMap::new();
        for row in documents(root, "trend")? {
            let day = row.get_datetime("_id").map_err(|_| AppError::Internal("Invalid grant trend".into()))?;
            days.insert(day.timestamp_millis(), (money(row, "issued")?, money(row, "used")?));
        }
        let mut trend = Vec::new();
        let mut day = params.window.from.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
        // The shared validator caps the window at 31 days (at most 32 UTC buckets).
        while day < params.window.to {
            let end = day + Duration::days(1);
            let (issued, used) = days.remove(&day.timestamp_millis()).unwrap_or((Credits::ZERO, Credits::ZERO));
            trend.push(TrendBucket {start: day.max(params.window.from), end: end.min(params.window.to), issued, used});
            day = end;
        }
        Ok(Response {
            window: params.window, generated_at: now,
            issued: totals.map(|d| money(d, "issued")).transpose()?.unwrap_or(Credits::ZERO),
            used: totals.map(|d| money(d, "used")).transpose()?.unwrap_or(Credits::ZERO),
            grants: totals.map_or(0, |d| count(d, "grants")),
            total_accounts: totals.map_or(0, |d| count(d, "accounts")),
            accounts: output, page, per_page: PER_PAGE,
            total_services: documents(root, "service_count")?.next().map_or(0, |d| count(d, "count")),
            services, service_page, interval: Interval::Day, trend,
        })
    }).await.map_err(|_| AppError::AdminUsageQueryTimeout)?
}
// A snapshot can expire even between aggregate admission and its first read.
// Restart the entire facet/cursor so every amount and page shares one snapshot.
async fn retry_snapshot_read<F, Fut>(mut read: F) -> AppResult<Vec<Document>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = mongodb::error::Result<Vec<Document>>>,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        match read().await {
            Err(error)
                if attempt < super::MAX_READ_ATTEMPTS
                    && matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if matches!(command.code, 239 | 246)) =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            result => return result.map_err(query_error),
        }
    }
}

fn documents<'a>(row: &'a Document, key: &str) -> AppResult<impl Iterator<Item = &'a Document>> {
    Ok(row
        .get_array(key)
        .map_err(|_| AppError::Internal("Invalid grant report".into()))?
        .iter()
        .filter_map(Bson::as_document))
}
fn count(row: &Document, key: &str) -> i64 {
    row.get_i64(key)
        .unwrap_or_else(|_| i64::from(row.get_i32(key).unwrap_or(0)))
}
fn money(row: &Document, key: &str) -> AppResult<Credits> {
    Credits::from_bson(
        row.get(key).cloned().unwrap_or(Bson::Null),
        crate::models::credits::SCALE,
    )
    .map_err(|_| AppError::Internal("Invalid grant report amount".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn command_error(code: i32) -> mongodb::error::Error {
        mongodb::error::ErrorKind::Command(bson::from_document(doc! {"code": code}).unwrap()).into()
    }

    #[tokio::test]
    async fn free_grants_snapshot_read_recovers_from_expired_and_unavailable_snapshots() {
        let expected = vec![doc! {"totals": [{"issued": "1.000000000001"}]}];
        let mut results = std::collections::VecDeque::from([
            Err(command_error(239)),
            Err(command_error(246)),
            Ok(expected.clone()),
        ]);
        let rows = retry_snapshot_read(|| std::future::ready(results.pop_front().unwrap()))
            .await
            .unwrap();
        assert_eq!(rows, expected);
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn free_grants_snapshot_read_stops_at_the_retry_budget() {
        let mut attempts = 0;
        let error = retry_snapshot_read(|| {
            attempts += 1;
            std::future::ready(Err(command_error(239)))
        })
        .await
        .unwrap_err();
        assert_eq!(attempts, super::super::MAX_READ_ATTEMPTS);
        assert!(matches!(error, AppError::DatabaseError(error)
            if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 239)));
    }

    #[tokio::test]
    async fn free_grants_snapshot_read_preserves_nonretryable_errors_and_timeouts() {
        for code in [2, 50] {
            let mut attempts = 0;
            let error = retry_snapshot_read(|| {
                attempts += 1;
                std::future::ready(Err(command_error(code)))
            })
            .await
            .unwrap_err();
            assert_eq!(attempts, 1);
            match code {
                50 => assert!(matches!(error, AppError::AdminUsageQueryTimeout)),
                _ => assert!(matches!(error, AppError::DatabaseError(error)
                    if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == code))),
            }
        }
    }

    #[test]
    fn free_grants_amount_reads_decimal128_and_whole_credit_integers() {
        let exact: Credits = "55.500000000053".parse().unwrap();
        assert_eq!(money(&doc! {"issued": exact}, "issued").unwrap(), exact);
        assert_eq!(
            money(&doc! {"issued": 2_i64}, "issued")
                .unwrap()
                .to_string(),
            "2"
        );
        assert_eq!(
            money(&doc! {"issued": 0_i32}, "issued").unwrap(),
            Credits::ZERO
        );
        assert!(money(&doc! {"issued": Bson::Null}, "issued").is_err());
        assert!(money(&doc! {"issued": 1.5_f64}, "issued").is_err());
    }

    #[tokio::test]
    async fn free_grants_report_counts_issuance_exactly_across_accounts_and_pages() {
        let db = crate::test_utils::connect_transaction_test_database("free-grants-report").await;
        let now = Utc::now();
        let from = now - chrono::Duration::days(1);
        let person = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        for (id, kind) in [
            (&person, crate::models::user::UserType::Person),
            (&org, crate::models::user::UserType::Org),
        ] {
            db.collection("users")
                .insert_one(crate::test_utils::test_user(id, kind))
                .await
                .unwrap();
        }
        let c: Credits = "1.000000000001".parse().unwrap();
        let mut rows = Vec::new();
        for i in 0..53 {
            let owner = if i == 0 {
                person.clone()
            } else if i == 1 {
                org.clone()
            } else {
                Uuid::new_v4().to_string()
            };
            rows.push(doc! {"_id": Uuid::new_v4().to_string(), "event_type": "accounting_v2", "movement": "grant_issued", "owner_id": owner,
                "created_at": bson::DateTime::from_chrono(from), "reference_id": format!("scheduled-{i}"),
                "postings": [{"account": "platform:promotions", "side": "credit", "amount": c}, {"account": format!("grant:{i}"), "side": "debit", "amount": c}]});
        }
        rows.push(doc! {"event_type": "grant_issued", "owner_id": &person, "created_at": bson::DateTime::from_chrono(from), "amount_micros": 2_500_000_i64});
        rows.push(doc! {"event_type": "grant_consumed", "owner_id": &person, "created_at": bson::DateTime::from_chrono(from), "amount_micros": 100_000_000_i64});
        rows.push(doc! {"event_type": "grant_issued", "owner_id": &org, "created_at": bson::DateTime::from_chrono(now), "amount_credits": 100_i64});
        db.collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
            .insert_many(rows)
            .await
            .unwrap();
        let query = |page, owners| Query {
            from: Some(from.to_rfc3339()),
            to: Some(now.to_rfc3339()),
            page: Some(page),
            owners,
            ..Default::default()
        };
        let first = report(&db, query(1, None), now).await.unwrap();
        let second = report(&db, query(2, None), now).await.unwrap();
        assert_eq!(first.issued.to_string(), "55.500000000053");
        assert_eq!(second.issued, first.issued);
        assert_eq!(first.used.to_string(), "100");
        assert_eq!(second.used, first.used);
        assert_eq!((first.grants, first.total_accounts), (54, 53));
        assert_eq!((first.accounts.len(), second.accounts.len()), (50, 3));
        assert!(
            !first
                .accounts
                .iter()
                .any(|a| second.accounts.iter().any(|b| a.owner_id == b.owner_id))
        );
        let p = report(&db, query(1, Some(person.clone())), now)
            .await
            .unwrap();
        assert_eq!(p.issued.to_string(), "3.500000000001");
        assert_eq!(p.used.to_string(), "100");
        assert_eq!(p.accounts[0].account_kind, "person");
        let o = report(&db, query(1, Some(org)), now).await.unwrap();
        assert_eq!(o.issued, c);
        assert_eq!(o.accounts[0].account_kind, "org");
        assert!(report(&db, query(0, None), now).await.is_err());
    }
    #[tokio::test]
    async fn free_grants_report_tracks_exact_movements_trend_and_independent_pages() {
        let db =
            crate::test_utils::connect_transaction_test_database("grant-credit-movements").await;
        let from = DateTime::parse_from_rfc3339("2026-10-01T06:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = from + Duration::days(3);
        let later = from + Duration::days(2);
        let person = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        let usage_only = Uuid::new_v4().to_string();
        let issue_only = Uuid::new_v4().to_string();
        for (id, kind) in [
            (&person, crate::models::user::UserType::Person),
            (&org, crate::models::user::UserType::Org),
        ] {
            db.collection("users")
                .insert_one(crate::test_utils::test_user(id, kind))
                .await
                .unwrap();
        }
        let exact: Credits = "1.000000000001".parse().unwrap();
        let entry = |owner: &str,
                     movement: &str,
                     grant: &str,
                     service: Option<&str>,
                     amount: Credits,
                     at| {
            let source = if movement == "grant_issued" {
                "platform:promotions".into()
            } else {
                format!("grant:{grant}")
            };
            let destination = if movement == "grant_issued" {
                format!("grant:{grant}")
            } else {
                "usage:row".into()
            };
            doc! {"_id": Uuid::new_v4().to_string(), "event_type": "accounting_v2", "movement": movement, "owner_id": owner,
            "reference_id": grant, "service_slug": service, "created_at": bson::DateTime::from_chrono(at),
            "postings": [{"account": source, "side": "credit", "amount": amount}, {"account": destination, "side": "debit", "amount": amount}]}
        };
        let mut rows = Vec::new();
        for i in 0..53 {
            let owner = match i {
                0 => person.clone(),
                1 => org.clone(),
                _ => Uuid::new_v4().to_string(),
            };
            rows.push(entry(
                &owner,
                "grant_consumed",
                &format!("old-shared-{i}"),
                Some(&format!("service-{i:03}")),
                exact,
                later,
            ));
        }
        // Shared issuance has no per-service allocation.
        rows.push(entry(
            &person,
            "grant_issued",
            "shared",
            None,
            "2.000000000001".parse().unwrap(),
            from,
        ));
        rows.push(entry(
            &person,
            "grant_consumed",
            "shared",
            Some("service-000"),
            "2.125000000001".parse().unwrap(),
            later,
        ));
        rows.push(doc! {"event_type": "grant_issued", "owner_id": &person, "amount_micros": 750_000_i64, "created_at": bson::DateTime::from_chrono(from)});
        rows.push(doc! {"event_type": "grant_issued", "owner_id": &issue_only, "amount_credits": 1_i64, "created_at": bson::DateTime::from_chrono(from)});
        rows.push(doc! {"event_type": "grant_consumed", "owner_id": &person, "amount_credits": 3_i64, "created_at": bson::DateTime::from_chrono(later)});
        rows.push(doc! {"event_type": "grant_consumed", "owner_id": &person, "service_slug": "legacy", "amount_micros": 2_500_000_i64, "created_at": bson::DateTime::from_chrono(later)});
        // Usage from older grants can exceed issuance within this report's window.
        rows.push(entry(
            &usage_only,
            "grant_issued",
            "older",
            None,
            "7".parse().unwrap(),
            from - Duration::milliseconds(1),
        ));
        rows.push(entry(
            &usage_only,
            "grant_consumed",
            "older",
            None,
            "7".parse().unwrap(),
            later,
        ));
        rows.push(entry(
            &person,
            "grant_consumed",
            "shared",
            Some("excluded"),
            "500".parse().unwrap(),
            to,
        ));
        for movement in [
            "grant_expired",
            "grant_revoked",
            "usage_settled",
            "topup_paid",
            "allowance_consumed",
        ] {
            rows.push(entry(
                &person,
                movement,
                "shared",
                Some("excluded"),
                "500".parse().unwrap(),
                later,
            ));
            rows.push(doc! {"event_type": movement, "owner_id": &person, "amount_credits": 500_i64, "created_at": bson::DateTime::from_chrono(later)});
        }
        db.collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
            .insert_many(rows)
            .await
            .unwrap();
        let query = |page, service_page, owners| Query {
            from: Some(from.to_rfc3339()),
            to: Some(to.to_rfc3339()),
            page: Some(page),
            service_page: Some(service_page),
            owners,
            ..Default::default()
        };
        let first = report(&db, query(1, 1, None), to).await.unwrap();
        let second = report(&db, query(2, 2, None), to).await.unwrap();
        assert_eq!(first.issued.to_string(), "3.750000000001");
        assert_eq!(first.used.to_string(), "67.625000000054");
        assert_eq!(
            (first.grants, first.total_accounts, first.total_services),
            (3, 55, 55)
        );
        assert_eq!(
            (
                first.accounts.len(),
                second.accounts.len(),
                first.services.len(),
                second.services.len()
            ),
            (50, 5, 50, 5)
        );
        assert_eq!((first.issued, first.used), (second.issued, second.used));
        assert_eq!(first.trend.len(), 4);
        assert_eq!(first.trend[0].start, from);
        assert_eq!(first.trend[0].issued, first.issued);
        assert_eq!(first.trend[1].issued, Credits::ZERO);
        assert_eq!(first.trend[1].used, Credits::ZERO);
        assert_eq!(first.trend[2].used, first.used);
        assert_eq!(first.trend[3].end, to);
        let accounts: Vec<_> = first.accounts.iter().chain(&second.accounts).collect();
        assert_eq!(
            accounts
                .iter()
                .map(|a| &a.owner_id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            55
        );
        let usage = accounts.iter().find(|a| a.owner_id == usage_only).unwrap();
        assert_eq!(
            (usage.issued, usage.used, usage.grants),
            (Credits::ZERO, "7".parse().unwrap(), 0)
        );
        let org_account = accounts.iter().find(|a| a.owner_id == org).unwrap();
        assert_eq!(org_account.account_kind, "org");
        assert_eq!(org_account.issued, Credits::ZERO);
        let services: Vec<_> = first.services.iter().chain(&second.services).collect();
        assert_eq!(
            services
                .iter()
                .map(|s| &s.service_slug)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            55
        );
        assert_eq!(
            services
                .iter()
                .find(|s| s.service_slug.is_none())
                .unwrap()
                .used
                .to_string(),
            "10"
        );
        assert_eq!(
            services
                .iter()
                .find(|s| s.service_slug.as_deref() == Some("service-000"))
                .unwrap()
                .used
                .to_string(),
            "3.125000000002"
        );
        let repeated = report(&db, query(1, 1, None), to).await.unwrap();
        assert_eq!(
            first
                .accounts
                .iter()
                .map(|a| &a.owner_id)
                .collect::<Vec<_>>(),
            repeated
                .accounts
                .iter()
                .map(|a| &a.owner_id)
                .collect::<Vec<_>>()
        );
        let p = report(&db, query(1, 2, Some(person.clone())), to)
            .await
            .unwrap();
        assert_eq!((p.total_accounts, p.total_services), (1, 3));
        assert_eq!(p.issued.to_string(), "2.750000000001");
        assert_eq!(p.used.to_string(), "8.625000000002");
        assert!(p.services.is_empty());
        assert_eq!(p.accounts[0].account_kind, "person");
        let mut late = query(1, 1, Some(person));
        late.from = Some(later.to_rfc3339());
        let late = report(&db, late, to).await.unwrap();
        assert_eq!(late.issued, Credits::ZERO);
        assert_eq!(late.used, p.used);
        let empty = report(&db, query(1, 1, Some(Uuid::new_v4().to_string())), to)
            .await
            .unwrap();
        assert_eq!(
            (
                empty.issued,
                empty.used,
                empty.total_accounts,
                empty.total_services
            ),
            (Credits::ZERO, Credits::ZERO, 0, 0)
        );
        assert_eq!(empty.trend.len(), 4);
        assert!(
            empty
                .trend
                .iter()
                .all(|b| b.used == Credits::ZERO && b.issued == Credits::ZERO)
        );
        for (page, service_page) in [(0, 1), (1, 0), (100_001, 1), (1, 100_001)] {
            assert!(matches!(
                report(&db, query(page, service_page, None), to).await,
                Err(AppError::BadRequest(_))
            ));
        }
        let mut invalid = query(1, 1, None);
        invalid.owners = Some("not-an-id".into());
        assert!(matches!(
            report(&db, invalid, to).await,
            Err(AppError::ValidationError(_))
        ));
        let mut invalid = query(1, 1, None);
        invalid.from = Some((to - Duration::days(32)).to_rfc3339());
        assert!(matches!(
            report(&db, invalid, to).await,
            Err(AppError::ValidationError(_))
        ));
        assert!(serde_json::from_str::<Query>(r#"{"interval":"month"}"#).is_err());
        assert!(serde_json::from_str::<Query>(r#"{"service_page":-1}"#).is_err());
        db.drop().await.unwrap();
    }
}
