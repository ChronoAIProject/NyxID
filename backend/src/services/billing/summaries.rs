use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Bson, Document, doc};

use crate::errors::{AppError, AppResult};
use crate::models::billing_wallet::BillingWallet;
use crate::models::credit_grant::{COLLECTION_NAME as CREDIT_GRANTS, CreditGrant};
use crate::models::usage_allowance::UsageAllowance;
use crate::models::usage_allowance_period::UsageAllowancePeriod;

pub const CREDIT_MICROS: i128 = 1_000_000;

#[derive(Clone, Debug)]
pub struct AdminCreditSummaryData {
    pub outstanding_micros: i128,
    pub available_micros: i128,
    pub unavailable_micros: i128,
    pub reserved_micros: i128,
    pub pending_activation_micros: i128,
    pub expiring_micros: i128,
    pub restricted_micros: i128,
    pub expired_awaiting_cleanup_micros: i128,
    pub active_grant_count: u64,
    pub pending_activation_count: u64,
    pub recipient_count: u64,
}

#[derive(Clone, Debug)]
pub struct UserCreditSummaryData {
    pub owner_id: String,
    pub wallet: Option<BillingWallet>,
    pub grants: Vec<CreditGrant>,
    pub allowances: Vec<(UsageAllowance, UsageAllowancePeriod)>,
    pub available_grant_micros: i128,
    pub reserved_grant_micros: i128,
    pub pending_activation_micros: i128,
    pub restricted_grant_micros: i128,
    pub restricted_grant_count: u64,
    /// Exact wallet availability after holds (negative values are retained for
    /// diagnostics) and the combined spendable total after suspension rules.
    pub wallet_available_micros: i128,
    pub wallet_available_credits: i128,
    /// Wallet credits that can contribute to the combined spendable total.
    /// Suspension and the zero lower bound are applied in this service so
    /// handlers only serialize the result.
    pub wallet_spendable_credits: i128,
    pub available_micros: i128,
}

#[derive(Debug)]
struct GrantSummaryRow {
    remaining_micros: i128,
    reserved_micros: i128,
    expires_at: Option<DateTime<Utc>>,
    issued_ledgered: bool,
    restricted: bool,
    realizable_micros: i128,
    reclaiming: bool,
}

/// Stream only active grants and only the fields needed for the summary. This
/// keeps historical grant volume from becoming an in-memory admin-page cost.
pub async fn admin_summary(
    db: &mongodb::Database,
    now: DateTime<Utc>,
) -> AppResult<AdminCreditSummaryData> {
    let expiring_before = now + Duration::days(30);
    let mut cursor = db
        .collection::<Document>(CREDIT_GRANTS)
        .find(doc! { "status": "active" })
        .projection(doc! {
            "recipient_user_id": 1,
            "remaining_micros": 1,
            "reserved_micros": 1,
            "expires_at": 1,
            "issued_ledgered_at": 1,
            "scope": 1,
            "service_limits": 1,
            "revoke_requested_at": 1,
            "active_settlement": 1,
        })
        .await?;
    let mut summary = AdminCreditSummaryData {
        outstanding_micros: 0,
        available_micros: 0,
        unavailable_micros: 0,
        reserved_micros: 0,
        pending_activation_micros: 0,
        expiring_micros: 0,
        restricted_micros: 0,
        expired_awaiting_cleanup_micros: 0,
        active_grant_count: 0,
        pending_activation_count: 0,
        recipient_count: 0,
    };

    while let Some(document) = cursor.try_next().await? {
        let grant = parse_summary_row(document)?;
        summary.active_grant_count = checked_add_u64(summary.active_grant_count, 1)?;
        let reserved = grant.reserved_micros.min(grant.remaining_micros);
        summary.reserved_micros = checked_add(summary.reserved_micros, reserved)?;
        let available = grant.remaining_micros - reserved;
        if grant.expires_at.is_some_and(|expiry| expiry <= now) {
            // Eligibility expires at the wall-clock boundary. A reservation
            // admitted before that boundary remains outstanding until release;
            // the rest is explicitly shown as awaiting expiry cleanup.
            summary.outstanding_micros = checked_add(summary.outstanding_micros, reserved)?;
            summary.expired_awaiting_cleanup_micros =
                checked_add(summary.expired_awaiting_cleanup_micros, available)?;
            continue;
        }

        summary.outstanding_micros =
            checked_add(summary.outstanding_micros, grant.remaining_micros)?;
        if !grant.issued_ledgered {
            summary.pending_activation_count =
                checked_add_u64(summary.pending_activation_count, 1)?;
            summary.pending_activation_micros =
                checked_add(summary.pending_activation_micros, grant.remaining_micros)?;
            continue;
        }
        let realizable = if grant.reclaiming {
            0
        } else {
            grant.realizable_micros
        };
        summary.unavailable_micros =
            checked_add(summary.unavailable_micros, available - realizable)?;
        summary.available_micros = checked_add(summary.available_micros, realizable)?;
        if grant.restricted {
            summary.restricted_micros = checked_add(summary.restricted_micros, realizable)?;
        }
        if grant
            .expires_at
            .is_some_and(|expiry| expiry <= expiring_before)
        {
            summary.expiring_micros = checked_add(summary.expiring_micros, realizable)?;
        }
    }
    let mut recipient_cursor = db
        .collection::<Document>(CREDIT_GRANTS)
        .aggregate(vec![
            doc! { "$match": { "status": "active" } },
            doc! { "$group": { "_id": "$recipient_user_id" } },
            doc! { "$count": "recipient_count" },
        ])
        .allow_disk_use(true)
        .await?;
    summary.recipient_count = match recipient_cursor.try_next().await? {
        Some(document) => parse_u64(document.get("recipient_count").ok_or_else(|| {
            AppError::Internal("recipient count aggregation returned no count".to_string())
        })?)?,
        None => 0,
    };
    Ok(summary)
}

pub async fn user_summary(
    db: &mongodb::Database,
    owner_id: &str,
    now: DateTime<Utc>,
) -> AppResult<UserCreditSummaryData> {
    let wallet = db
        .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
        .find_one(doc! { "owner_id": owner_id })
        .await?;
    // Keep expired-held and reclaiming sources in detail. Eligibility is
    // calculated independently; their holds still belong to this account.
    let grants: Vec<CreditGrant> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! {
            "recipient_user_id": owner_id,
            "$or": [
                { "status": "active", "issued_ledgered_at": { "$type": "date" } },
                { "reserved_micros": { "$gt": 0_i64 } },
                { "active_settlement": { "$type": "object" } },
            ],
        })
        .sort(doc! { "created_at": 1 })
        .await?
        .try_collect()
        .await?;
    let allowances =
        crate::services::billing::allowances::list_current_for_user_read_only(db, owner_id, now)
            .await?;

    // Match list_active_for_user's wall-clock expiry and ledger gate. A
    // malformed BSON amount is an internal data error, never an apparent zero.
    let mut pending_cursor = db
        .collection::<Document>(CREDIT_GRANTS)
        .find(doc! {
            "recipient_user_id": owner_id,
            "status": "active",
            "revoke_requested_at": Bson::Null,
            "$and": [
                { "$or": [
                    { "issued_ledgered_at": Bson::Null },
                    { "issued_ledgered_at": { "$exists": false } },
                ] },
                { "$or": [
                    { "expires_at": Bson::Null },
                    { "expires_at": { "$exists": false } },
                    { "expires_at": { "$gt": mongodb::bson::DateTime::from_chrono(now) } },
                ] },
            ],
        })
        .projection(doc! { "remaining_micros": 1 })
        .await?;
    let mut pending_activation_micros = 0_i128;
    while let Some(document) = pending_cursor.try_next().await? {
        let amount = parse_i128(document.get("remaining_micros").ok_or_else(|| {
            AppError::Internal("pending credit grant is missing remaining_micros".to_string())
        })?)?;
        if amount < 0 {
            return Err(AppError::Internal(
                "pending credit grant has a negative remaining amount".to_string(),
            ));
        }
        pending_activation_micros = checked_add(pending_activation_micros, amount)?;
    }

    let mut available_grant_micros = 0_i128;
    let mut reserved_grant_micros = 0_i128;
    let mut restricted_grant_micros = 0_i128;
    let mut restricted_grant_count = 0_u64;
    for grant in &grants {
        let remaining = checked_nonnegative(grant.remaining_micros, "remaining_micros")?;
        checked_nonnegative(grant.reserved_micros, "reserved_micros")?;
        let reserved = i128::from(super::service_limits::held_micros(grant)).min(remaining);
        let available = i128::from(super::grants::eligible_available_micros(grant, now));
        available_grant_micros = checked_add(available_grant_micros, available)?;
        reserved_grant_micros = checked_add(reserved_grant_micros, reserved)?;
        if super::grants::has_restrictions(grant) {
            restricted_grant_micros = checked_add(restricted_grant_micros, available)?;
            restricted_grant_count = checked_add_u64(restricted_grant_count, 1)?;
        }
    }

    let wallet_available_credits = wallet
        .as_ref()
        .map(available_wallet_credits)
        .unwrap_or_default();
    let wallet_available_micros = wallet_available_credits
        .checked_mul(CREDIT_MICROS)
        .ok_or_else(|| AppError::Internal("wallet amount overflow".to_string()))?;
    let wallet_spendable_credits = wallet
        .as_ref()
        .map(compute_wallet_spendable_credits)
        .unwrap_or_default();
    let wallet_spendable_micros = wallet_spendable_credits
        .checked_mul(CREDIT_MICROS)
        .ok_or_else(|| AppError::Internal("wallet spendable amount overflow".to_string()))?;
    let available_micros = if wallet.as_ref().is_some_and(BillingWallet::is_suspended) {
        0
    } else {
        checked_add(wallet_spendable_micros, available_grant_micros)?
    };

    Ok(UserCreditSummaryData {
        owner_id: owner_id.to_string(),
        wallet,
        grants,
        allowances,
        available_grant_micros,
        reserved_grant_micros,
        pending_activation_micros,
        restricted_grant_micros,
        restricted_grant_count,
        wallet_available_micros,
        wallet_available_credits,
        wallet_spendable_credits,
        available_micros,
    })
}

pub fn available_wallet_credits(wallet: &BillingWallet) -> i128 {
    i128::from(wallet.balance_credits)
        - i128::from(wallet.reserved_credits)
        - i128::from(wallet.pending_lago_debits)
        - i128::from(wallet.pending_topup_expiry_credits)
}

pub fn compute_wallet_spendable_credits(wallet: &BillingWallet) -> i128 {
    if wallet.is_suspended() {
        0
    } else {
        available_wallet_credits(wallet).max(0)
    }
}

fn parse_summary_row(document: Document) -> AppResult<GrantSummaryRow> {
    document
        .get_str("recipient_user_id")
        .map_err(|_| AppError::Internal("active credit grant has no recipient".to_string()))?;
    let remaining_micros = parse_i128(document.get("remaining_micros").ok_or_else(|| {
        AppError::Internal("active credit grant is missing remaining_micros".to_string())
    })?)?;
    let mut reserved_micros = match document.get("reserved_micros") {
        Some(value) => parse_i128(value)?,
        None => 0,
    };
    if remaining_micros < 0 || reserved_micros < 0 {
        return Err(AppError::Internal(
            "active credit grant has a negative amount".to_string(),
        ));
    }
    let expires_at = match document.get("expires_at") {
        None | Some(Bson::Null) => None,
        Some(Bson::DateTime(value)) => Some(value.to_chrono()),
        Some(_) => {
            return Err(AppError::Internal(
                "credit grant expiry has an invalid BSON type".to_string(),
            ));
        }
    };
    let issued_ledgered = matches!(document.get("issued_ledgered_at"), Some(Bson::DateTime(_)));
    let scope: crate::models::billing_target::BillingServiceScope = mongodb::bson::from_bson(
        document
            .get("scope")
            .cloned()
            .ok_or_else(|| AppError::Internal("Credit scope missing".into()))?,
    )
    .map_err(|e| AppError::Internal(format!("Credit scope invalid: {e}")))?;
    let mut rules: Vec<crate::models::credit_service_limit::CreditServiceLimit> =
        mongodb::bson::from_bson(
            document
                .get("service_limits")
                .cloned()
                .unwrap_or(Bson::Array(Vec::new())),
        )
        .map_err(|e| AppError::Internal(format!("Credit limits invalid: {e}")))?;
    if rules
        .iter()
        .any(|r| r.limit_micros < 0 || r.consumed_micros < 0 || r.reserved_micros < 0)
    {
        return Err(AppError::Internal(
            "Credit limit counters are negative".into(),
        ));
    }
    let lock: Option<crate::models::credit_grant::CreditGrantSettlementLock> =
        bson_lock(document.get("active_settlement"))?;
    reserved_micros = checked_add(
        reserved_micros,
        i128::from(super::service_limits::unapplied_extra_micros(lock.as_ref())),
    )?
    .min(remaining_micros);
    super::service_limits::apply_commitment_to_rules(&mut rules, lock.as_ref());
    let restricted = !scope.all_services || !rules.is_empty();
    let realizable_micros = super::service_limits::realizable_for_policy(
        &scope,
        &rules,
        (remaining_micros - reserved_micros).max(0),
    );
    let reclaiming = matches!(document.get("revoke_requested_at"), Some(Bson::DateTime(_)));
    Ok(GrantSummaryRow {
        remaining_micros,
        reserved_micros,
        expires_at,
        issued_ledgered,
        restricted,
        realizable_micros,
        reclaiming,
    })
}

fn bson_lock(
    value: Option<&Bson>,
) -> AppResult<Option<crate::models::credit_grant::CreditGrantSettlementLock>> {
    match value {
        None | Some(Bson::Null) => Ok(None),
        Some(value) => mongodb::bson::from_bson(value.clone())
            .map_err(|e| AppError::Internal(format!("Credit settlement invalid: {e}"))),
    }
}

fn parse_i128(value: &Bson) -> AppResult<i128> {
    match value {
        Bson::Int32(value) => Ok(i128::from(*value)),
        Bson::Int64(value) => Ok(i128::from(*value)),
        _ => Err(AppError::Internal(
            "credit amount has an invalid BSON numeric type".to_string(),
        )),
    }
}

fn parse_u64(value: &Bson) -> AppResult<u64> {
    match value {
        Bson::Int32(value) if *value >= 0 => Ok(*value as u64),
        Bson::Int64(value) if *value >= 0 => u64::try_from(*value)
            .map_err(|_| AppError::Internal("recipient count exceeded u64".to_string())),
        _ => Err(AppError::Internal(
            "recipient count aggregation returned an invalid value".to_string(),
        )),
    }
}

fn checked_nonnegative(value: i64, field: &str) -> AppResult<i128> {
    if value < 0 {
        return Err(AppError::Internal(format!(
            "credit grant has a negative {field}"
        )));
    }
    Ok(i128::from(value))
}

fn checked_add(left: i128, right: i128) -> AppResult<i128> {
    left.checked_add(right)
        .ok_or_else(|| AppError::Internal("credit summary amount overflow".to_string()))
}

fn checked_add_u64(left: u64, right: u64) -> AppResult<u64> {
    left.checked_add(right)
        .ok_or_else(|| AppError::Internal("credit summary count overflow".to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use mongodb::bson::doc;

    use crate::models::billing_target::{BillingServiceScope, BillingTargetKind};
    use crate::models::billing_wallet::{BillingWallet, CollectionState, PlanKind};
    use crate::models::credit_grant::{CreditGrant, CreditGrantStatus};
    use crate::test_utils::connect_test_database;

    use super::{admin_summary, user_summary};

    #[tokio::test]
    async fn admin_summary_streams_active_rows_and_excludes_expired_pending_and_future_value() {
        let db = connect_test_database("credit_summary_admin_contract")
            .await
            .expect("summary integration test requires NYXID_TEST_DATABASE_URL");
        let now = Utc::now();
        let rows = (0..60)
            .map(|index| {
                doc! {
                    "_id": format!("grant-{index}"),
                    "status": "active",
                    "recipient_user_id": format!("owner-{}", index % 3),
                    "remaining_micros": 1_000_001_i64,
                    "reserved_micros": 1_i64,
                    "issued_ledgered_at": mongodb::bson::DateTime::from_chrono(now),
                    "scope": { "all_services": index % 2 == 0 },
                }
            })
            .collect::<Vec<_>>();
        db.collection::<mongodb::bson::Document>(crate::models::credit_grant::COLLECTION_NAME)
            .insert_many(rows)
            .await
            .expect("insert active rows");
        db.collection::<mongodb::bson::Document>(crate::models::credit_grant::COLLECTION_NAME)
            .insert_many([
                doc! {
                    "_id": "expired",
                    "status": "active",
                    "recipient_user_id": "owner-expired",
                    "remaining_micros": 500_i64,
                    "reserved_micros": 100_i64,
                    "expires_at": mongodb::bson::DateTime::from_chrono(now - Duration::seconds(1)),
                    "issued_ledgered_at": mongodb::bson::DateTime::from_chrono(now),
                    "scope": { "all_services": true },
                },
                doc! {
                    "_id": "pending-expired",
                    "status": "active",
                    "recipient_user_id": "owner-pending",
                    "remaining_micros": 700_i64,
                    "expires_at": mongodb::bson::DateTime::from_chrono(now - Duration::seconds(1)),
                    "scope": { "all_services": true },
                },
                doc! {
                    "_id": "pending-live",
                    "status": "active",
                    "recipient_user_id": "owner-pending-live",
                    "remaining_micros": 700_i64,
                    "expires_at": mongodb::bson::DateTime::from_chrono(now + Duration::days(2)),
                    "scope": { "all_services": true },
                },
                doc! {
                    "_id": "revoked-row",
                    "status": "revoked",
                    "recipient_user_id": "owner-revoked",
                    "remaining_micros": 9_000_i64,
                    "scope": { "all_services": true },
                },
                doc! {
                    "_id": "consumed-row",
                    "status": "consumed",
                    "recipient_user_id": "owner-consumed",
                    "remaining_micros": 9_000_i64,
                    "scope": { "all_services": true },
                },
            ])
            .await
            .expect("insert expiry rows");
        db.collection::<mongodb::bson::Document>("credit_schedules")
            .insert_one(doc! {
                "_id": "future-policy",
                "amount_micros": 99_000_000_i64,
                "is_active": true,
                "next_period_start": mongodb::bson::DateTime::from_chrono(now + Duration::days(10)),
            })
            .await
            .expect("insert future policy");

        let summary = admin_summary(&db, now).await.expect("admin summary");
        assert_eq!(summary.active_grant_count, 63);
        assert_eq!(summary.pending_activation_count, 1);
        assert_eq!(summary.pending_activation_micros, 700);
        assert_eq!(summary.outstanding_micros, 60 * 1_000_001 + 100 + 700);
        assert_eq!(summary.expired_awaiting_cleanup_micros, 1_100);
        assert_eq!(summary.recipient_count, 6);
        assert_eq!(summary.available_micros, 60 * 1_000_000);
        assert_eq!(summary.restricted_micros, 30 * 1_000_000);
    }

    #[tokio::test]
    async fn user_summary_preserves_large_microcredit_totals() {
        let db = connect_test_database("credit_summary_precision_contract")
            .await
            .expect("summary integration test requires NYXID_TEST_DATABASE_URL");
        let now = Utc::now();
        let grant = CreditGrant {
            service_limits: Vec::new(),
            revoke_requested_at: None,
            revoke_requested_by: None,

            id: "large-grant".to_string(),
            batch_id: "batch".to_string(),
            schedule_origin: None,
            recipient_user_id: "owner-large".to_string(),
            target_kind: BillingTargetKind::SelectedUsers,
            amount_credits: i64::MAX,
            amount_micros: i64::MAX,
            remaining_micros: i64::MAX,
            reserved_micros: 0,
            scope: BillingServiceScope {
                all_services: true,
                service_ids: vec![],
                service_slugs: vec![],
            },
            expires_at: None,
            reason: None,
            granted_by: "admin".to_string(),
            status: CreditGrantStatus::Active,
            issued_ledgered_at: Some(now),
            terminal_ledgered_at: None,
            terminal_amount_micros: 0,
            active_settlement: None,
            created_at: now,
            updated_at: now,
            consumed_at: None,
            expired_at: None,
            revoked_at: None,
        };
        let mut second_grant = grant.clone();
        second_grant.id = "large-grant-2".to_string();
        db.collection::<CreditGrant>(crate::models::credit_grant::COLLECTION_NAME)
            .insert_many([grant, second_grant])
            .await
            .expect("insert large grant");
        let summary = user_summary(&db, "owner-large", now)
            .await
            .expect("user summary");
        assert_eq!(summary.available_grant_micros, i128::from(i64::MAX) * 2);
        assert_eq!(summary.pending_activation_micros, 0);
    }

    #[tokio::test]
    async fn user_summary_keeps_grants_spendable_when_wallet_is_negative_and_blocks_on_suspend() {
        let db = connect_test_database("credit_summary_wallet_rules")
            .await
            .expect("summary integration test requires NYXID_TEST_DATABASE_URL");
        let now = Utc::now();
        let owner_id = "owner-wallet-rules";
        let grant = CreditGrant {
            service_limits: Vec::new(),
            revoke_requested_at: None,
            revoke_requested_by: None,

            id: "wallet-rules-grant".to_string(),
            batch_id: "batch".to_string(),
            schedule_origin: None,
            recipient_user_id: owner_id.to_string(),
            target_kind: BillingTargetKind::SelectedUsers,
            amount_credits: 10,
            amount_micros: 10_000_000,
            remaining_micros: 10_000_000,
            reserved_micros: 0,
            scope: BillingServiceScope {
                all_services: true,
                service_ids: vec![],
                service_slugs: vec![],
            },
            expires_at: None,
            reason: None,
            granted_by: "admin".to_string(),
            status: CreditGrantStatus::Active,
            issued_ledgered_at: Some(now),
            terminal_ledgered_at: None,
            terminal_amount_micros: 0,
            active_settlement: None,
            created_at: now,
            updated_at: now,
            consumed_at: None,
            expired_at: None,
            revoked_at: None,
        };
        db.collection::<CreditGrant>(crate::models::credit_grant::COLLECTION_NAME)
            .insert_one(&grant)
            .await
            .expect("insert grant");
        let wallet = BillingWallet {
            id: "wallet-rules".to_string(),
            owner_id: owner_id.to_string(),
            lago_customer_id: owner_id.to_string(),
            lago_wallet_id: None,
            lago_subscription_id: None,
            plan_kind: PlanKind::Prepaid,
            balance_credits: 1,
            reserved_credits: 0,
            pending_lago_debits: 5,
            pending_topup_expiry_credits: 0,
            has_payment_instrument: false,
            overdraft_cap_credits: 0,
            suspended: false,
            collection_state: CollectionState::Good,
            topup_expiry_checked_at: None,
            active_topup_expiry: None,
            balance_synced_at: now,
            created_at: now,
            updated_at: now,
        };
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(&wallet)
            .await
            .expect("insert wallet");

        let summary = user_summary(&db, owner_id, now)
            .await
            .expect("negative wallet summary");
        assert_eq!(summary.wallet_available_micros, -4_000_000);
        assert_eq!(summary.wallet_spendable_credits, 0);
        assert_eq!(summary.available_micros, 10_000_000);

        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .update_one(
                doc! { "_id": "wallet-rules" },
                doc! { "$set": { "suspended": true } },
            )
            .await
            .expect("suspend wallet");
        let suspended = user_summary(&db, owner_id, now)
            .await
            .expect("suspended wallet summary");
        assert_eq!(suspended.available_micros, 0);
        assert_eq!(suspended.wallet_spendable_credits, 0);
        assert_eq!(suspended.available_grant_micros, 10_000_000);
    }

    #[tokio::test]
    async fn admin_summary_rejects_malformed_numeric_amounts() {
        let db = connect_test_database("credit_summary_malformed_amount")
            .await
            .expect("summary integration test requires NYXID_TEST_DATABASE_URL");
        db.collection::<mongodb::bson::Document>(crate::models::credit_grant::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": "malformed-amount",
                "status": "active",
                "recipient_user_id": "owner-malformed",
                "remaining_micros": "1000000",
                "scope": { "all_services": true },
            })
            .await
            .expect("insert malformed row");
        assert!(admin_summary(&db, Utc::now()).await.is_err());
    }
}
