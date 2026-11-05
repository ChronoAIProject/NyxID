//! Issuance reporting from the immutable journal, grouped by financial owner.
//! No usage, recipient expansion, or membership joins participate in the sum.
use super::{AdminUsageQuery, QUERY_TIMEOUT, UsageWindow, query_error};
use crate::{
    errors::{AppError, AppResult},
    models::credits::Credits,
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Bson, Document, doc},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub period: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub owners: Option<String>,
    pub page: Option<u64>,
}
#[derive(Serialize, ToSchema)]
pub struct Account {
    pub owner_id: String,
    pub label: String,
    pub account_kind: String,
    pub issued: Credits,
    pub grants: i64,
}
#[derive(Serialize, ToSchema)]
pub struct Response {
    pub window: UsageWindow,
    pub generated_at: DateTime<Utc>,
    pub issued: Credits,
    pub grants: i64,
    pub total_accounts: i64,
    pub accounts: Vec<Account>,
    pub page: u64,
    pub per_page: u64,
}
pub async fn report(db: &Database, q: Query, now: DateTime<Utc>) -> AppResult<Response> {
    let page = q.page.unwrap_or(1);
    if !(1..=100_000).contains(&page) {
        return Err(AppError::BadRequest(
            "page must be between 1 and 100000".into(),
        ));
    }
    let params = AdminUsageQuery {
        period: q.period,
        from: q.from,
        to: q.to,
        owners: q.owners,
        ..Default::default()
    }
    .validate(now)?;
    tokio::time::timeout(QUERY_TIMEOUT + std::time::Duration::from_secs(2), async {
        let mut filter = doc! {
            "created_at": {"$gte": bson::DateTime::from_chrono(params.window.from), "$lt": bson::DateTime::from_chrono(params.window.to)},
            "$or": [{"event_type": "grant_issued"}, {"event_type": "accounting_v2", "movement": "grant_issued"}],
        };
        if !params.selection.owners.is_empty() {
            filter.insert("owner_id", doc! {"$in": &params.selection.owners});
        }
        let amount = doc! {"$cond": [
            {"$eq": ["$event_type", "accounting_v2"]},
            {"$reduce": {"input": {"$filter": {"input": "$postings", "as": "p", "cond": {"$and": [
                {"$eq": ["$$p.account", "platform:promotions"]}, {"$eq": ["$$p.side", "credit"]}
            ]}}}, "initialValue": Credits::ZERO, "in": {"$add": ["$$value", "$$this.amount"]}}},
            {"$ifNull": [{"$divide": [{"$toDecimal": "$amount_micros"}, 1_000_000]}, {"$toDecimal": "$amount_credits"}]}
        ]};
        let rows: Vec<Document> = db.collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
            .aggregate(vec![
                doc! {"$match": filter},
                doc! {"$group": {"_id": "$owner_id", "issued": {"$sum": amount}, "grants": {"$sum": 1_i64}}},
                doc! {"$facet": {
                    "totals": [{"$group": {"_id": Bson::Null, "issued": {"$sum": "$issued"}, "grants": {"$sum": "$grants"}, "accounts": {"$sum": 1_i64}}}],
                    "accounts": [{"$sort": {"issued": -1, "_id": 1}}, {"$skip": ((page-1)*50) as i64}, {"$limit": 50}]
                }},
            ]).max_time(QUERY_TIMEOUT).allow_disk_use(true)
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .await.map_err(query_error)?.try_collect().await.map_err(query_error)?;
        let root = rows.first().ok_or_else(|| AppError::Internal("Missing grant report".into()))?;
        let totals = root.get_array("totals").ok().and_then(|v| v.first()).and_then(Bson::as_document);
        let accounts = root.get_array("accounts").map_err(|_| AppError::Internal("Invalid grant report".into()))?;
        let ids: Vec<String> = accounts.iter().filter_map(Bson::as_document)
            .filter_map(|d| d.get_str("_id").ok().map(str::to_owned)).collect();
        let identities = crate::services::reporting_identity_service::resolve(db, &ids, QUERY_TIMEOUT).await.map_err(query_error)?;
        let mut output = Vec::new();
        for row in accounts.iter().filter_map(Bson::as_document) {
            let owner_id = row.get_str("_id").map_err(|_| AppError::Internal("Invalid grant owner".into()))?.to_owned();
            let identity = identities.get(&owner_id);
            output.push(Account {
                label: identity.map_or("Deleted account", |i| i.label()).into(),
                account_kind: identity.map_or("unknown", |i| i.user_type.as_str()).into(),
                owner_id, issued: money(row, "issued")?, grants: count(row, "grants"),
            });
        }
        Ok(Response {
            window: params.window, generated_at: now,
            issued: totals.map(|d| money(d, "issued")).transpose()?.unwrap_or(Credits::ZERO),
            grants: totals.map_or(0, |d| count(d, "grants")),
            total_accounts: totals.map_or(0, |d| count(d, "accounts")),
            accounts: output, page, per_page: 50,
        })
    }).await.map_err(|_| AppError::AdminUsageQueryTimeout)?
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
        assert_eq!(p.accounts[0].account_kind, "person");
        let o = report(&db, query(1, Some(org)), now).await.unwrap();
        assert_eq!(o.issued, c);
        assert_eq!(o.accounts[0].account_kind, "org");
        assert!(report(&db, query(0, None), now).await.is_err());
    }
}
