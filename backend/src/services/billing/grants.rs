use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::billing_ledger::BillingLedgerEventType;
use crate::models::billing_target::{BillingServiceScope, BillingTargetKind};
use crate::models::credit_grant::{
    COLLECTION_NAME as CREDIT_GRANTS, CreditGrant, CreditGrantStatus,
};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};

pub const CREDIT_MICROS: i64 = 1_000_000;
pub const MAX_GRANT_CREDITS: i64 = 1_000_000;
pub const MAX_SELECTED_USERS: usize = 500;
pub const MAX_SCOPED_SERVICES: usize = 100;
pub const MAX_GRANT_REASON_LEN: usize = 2_000;
const EXPIRY_SWEEP_BATCH: i64 = 500;
const MAX_EXPIRATIONS_PER_TICK: usize = 10_000;
const LEDGER_RECOVERY_BATCH: i64 = 500;
pub const INLINE_ISSUANCE_LEDGER_LIMIT: usize = 50;

pub fn available_grant_micros(grant: &CreditGrant) -> i64 {
    grant
        .remaining_micros
        .saturating_sub(grant.reserved_micros)
        .max(0)
}

pub fn service_scope_applies(
    scope: &BillingServiceScope,
    service_id: Option<&str>,
    service_slug: Option<&str>,
) -> bool {
    scope.all_services
        || service_id.is_some_and(|id| scope.service_ids.iter().any(|item| item == id))
        || service_slug.is_some_and(|slug| scope.service_slugs.iter().any(|item| item == slug))
}

#[derive(Clone, Debug)]
pub struct IssueCreditGrantInput {
    pub amount_credits: i64,
    pub target_kind: BillingTargetKind,
    pub target_user_ids: Vec<String>,
    pub all_services: bool,
    pub service_refs: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub granted_by: String,
}

pub async fn issue_grants(
    db: &mongodb::Database,
    input: IssueCreditGrantInput,
) -> AppResult<Vec<CreditGrant>> {
    validate_issue_input(&input)?;
    let recipients = resolve_recipients(db, input.target_kind, &input.target_user_ids).await?;
    if recipients.is_empty() {
        return Err(AppError::ValidationError(
            "credit grant has no eligible user recipients".to_string(),
        ));
    }
    let scope = resolve_service_scope(db, input.all_services, &input.service_refs).await?;
    let now = Utc::now();
    let batch_id = Uuid::new_v4().to_string();
    let amount_micros = input
        .amount_credits
        .checked_mul(CREDIT_MICROS)
        .ok_or_else(|| AppError::ValidationError("credit grant amount is too large".to_string()))?;
    let reason = input
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let mut grants: Vec<CreditGrant> = recipients
        .into_iter()
        .map(|recipient_user_id| CreditGrant {
            id: Uuid::new_v4().to_string(),
            batch_id: batch_id.clone(),
            schedule_origin: None,
            recipient_user_id,
            target_kind: input.target_kind,
            amount_credits: input.amount_credits,
            amount_micros,
            remaining_micros: amount_micros,
            reserved_micros: 0,
            scope: scope.clone(),
            expires_at: input.expires_at,
            reason: reason.clone(),
            granted_by: input.granted_by.clone(),
            status: CreditGrantStatus::Active,
            issued_ledgered_at: None,
            terminal_ledgered_at: None,
            terminal_amount_micros: 0,
            active_settlement: None,
            created_at: now,
            updated_at: now,
            consumed_at: None,
            expired_at: None,
            revoked_at: None,
        })
        .collect();

    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .insert_many(&grants)
        .await?;
    // Hash-chain appends serialize on the ledger head. Keep the admin request
    // bounded for platform-wide batches; remaining grants stay deliberately
    // unspendable until recover_unledgered_events journals them.
    for grant in grants.iter_mut().take(INLINE_ISSUANCE_LEDGER_LIMIT) {
        if super::ledger::record_grant_event(
            db,
            BillingLedgerEventType::GrantIssued,
            &grant.recipient_user_id,
            &grant.id,
            grant.amount_micros,
            None,
            format!("grant-issued:{}", grant.id),
        )
        .await
        {
            match mark_issued_ledgered(db, &grant.id, now).await {
                Ok(()) => grant.issued_ledgered_at = Some(now),
                Err(error) => {
                    tracing::warn!(grant_id = %grant.id, %error, "grant issuance ledger marker will retry");
                }
            }
        }
    }
    Ok(grants)
}

pub async fn list_grants(
    db: &mongodb::Database,
    recipient_user_id: Option<&str>,
    limit: i64,
    skip: u64,
) -> AppResult<(Vec<CreditGrant>, u64)> {
    let filter = recipient_user_id
        .map(|user_id| doc! { "recipient_user_id": user_id })
        .unwrap_or_default();
    let collection = db.collection::<CreditGrant>(CREDIT_GRANTS);
    let total = collection.count_documents(filter.clone()).await?;
    let rows = collection
        .find(filter)
        .sort(doc! { "created_at": -1, "_id": -1 })
        .skip(skip)
        .limit(limit.clamp(1, 500))
        .await?
        .try_collect()
        .await?;
    Ok((rows, total))
}

pub async fn list_active_for_user(
    db: &mongodb::Database,
    user_id: &str,
    now: DateTime<Utc>,
) -> AppResult<Vec<CreditGrant>> {
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! {
            "recipient_user_id": user_id,
            "status": "active",
            "issued_ledgered_at": { "$type": "date" },
            "remaining_micros": { "$gt": 0_i64 },
            "$or": [
                { "expires_at": Bson::Null },
                { "expires_at": { "$exists": false } },
                { "expires_at": { "$gt": bson::DateTime::from_chrono(now) } },
            ],
        })
        .sort(doc! { "created_at": 1 })
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

pub async fn revoke_grant(db: &mongodb::Database, grant_id: &str) -> AppResult<CreditGrant> {
    let now = Utc::now();
    let previous = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one_and_update(
            doc! {
                "_id": grant_id,
                "status": "active",
                "reserved_micros": 0_i64,
                "$or": [
                    { "active_settlement": Bson::Null },
                    { "active_settlement": { "$exists": false } },
                ],
            },
            vec![doc! {
                "$set": {
                    "status": "revoked",
                    "terminal_amount_micros": "$remaining_micros",
                    "remaining_micros": 0_i64,
                    "reserved_micros": 0_i64,
                    "revoked_at": bson::DateTime::from_chrono(now),
                    "updated_at": bson::DateTime::from_chrono(now),
                }
            }],
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::Before)
                .build(),
        )
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(
                "credit grant is not active or has an in-flight settlement".to_string(),
            )
        })?;
    let revoked_micros = previous.remaining_micros.max(0);
    if super::ledger::record_grant_event(
        db,
        BillingLedgerEventType::GrantRevoked,
        &previous.recipient_user_id,
        &previous.id,
        revoked_micros,
        None,
        format!("grant-revoked:{}", previous.id),
    )
    .await
        && let Err(error) = mark_terminal_ledgered(db, &previous.id, Utc::now()).await
    {
        tracing::warn!(grant_id = %previous.id, %error, "grant revocation ledger marker will retry");
    }
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": grant_id })
        .await?
        .ok_or_else(|| AppError::Internal("revoked credit grant disappeared".to_string()))
}

pub async fn expire_due_grants(db: &mongodb::Database, now: DateTime<Utc>) -> AppResult<u64> {
    let mut expired = 0;
    let mut examined = 0;
    while examined < MAX_EXPIRATIONS_PER_TICK {
        let batch_limit = EXPIRY_SWEEP_BATCH.min((MAX_EXPIRATIONS_PER_TICK - examined) as i64);
        let due: Vec<CreditGrant> = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find(doc! {
                "status": "active",
                "reserved_micros": 0_i64,
                "expires_at": { "$lte": bson::DateTime::from_chrono(now) },
                "$or": [
                    { "active_settlement": Bson::Null },
                    { "active_settlement": { "$exists": false } },
                ],
            })
            .sort(doc! { "expires_at": 1, "_id": 1 })
            .limit(batch_limit)
            .await?
            .try_collect()
            .await?;
        if due.is_empty() {
            break;
        }
        examined += due.len();
        let short_batch = due.len() < batch_limit as usize;
        for grant in due {
            let result = db
                .collection::<CreditGrant>(CREDIT_GRANTS)
                .update_one(
                    doc! {
                        "_id": &grant.id,
                        "status": "active",
                        "reserved_micros": 0_i64,
                        "expires_at": { "$lte": bson::DateTime::from_chrono(now) },
                        "$or": [
                            { "active_settlement": Bson::Null },
                            { "active_settlement": { "$exists": false } },
                        ],
                    },
                    vec![doc! { "$set": {
                        "status": "expired",
                        "terminal_amount_micros": "$remaining_micros",
                        "remaining_micros": 0_i64,
                        "reserved_micros": 0_i64,
                        "expired_at": bson::DateTime::from_chrono(now),
                        "updated_at": bson::DateTime::from_chrono(now),
                    } }],
                )
                .await?;
            if result.modified_count == 0 {
                continue;
            }
            expired += 1;
            if super::ledger::record_grant_event(
                db,
                BillingLedgerEventType::GrantExpired,
                &grant.recipient_user_id,
                &grant.id,
                grant.remaining_micros.max(0),
                None,
                format!("grant-expired:{}", grant.id),
            )
            .await
                && let Err(error) = mark_terminal_ledgered(db, &grant.id, now).await
            {
                tracing::warn!(grant_id = %grant.id, %error, "grant expiry ledger marker will retry");
            }
        }
        if short_batch {
            break;
        }
    }
    Ok(expired)
}

/// Retry grant lifecycle journal writes left pending by a process crash or a
/// transient ledger failure. Grants are not spendable until issuance is
/// ledgered; terminal mutations remain observable and retry here until their
/// corresponding immutable entry is confirmed by dedupe key.
pub async fn recover_unledgered_events(
    db: &mongodb::Database,
    now: DateTime<Utc>,
) -> AppResult<u64> {
    let grants: Vec<CreditGrant> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! {
            "$or": [
                { "issued_ledgered_at": Bson::Null },
                { "issued_ledgered_at": { "$exists": false } },
                {
                    "status": { "$in": ["expired", "revoked"] },
                    "$or": [
                        { "terminal_ledgered_at": Bson::Null },
                        { "terminal_ledgered_at": { "$exists": false } },
                    ],
                },
            ],
        })
        .sort(doc! { "created_at": 1, "_id": 1 })
        .limit(LEDGER_RECOVERY_BATCH)
        .await?
        .try_collect()
        .await?;
    let mut recovered = 0;
    for grant in grants {
        if grant.issued_ledgered_at.is_none()
            && super::ledger::record_grant_event(
                db,
                BillingLedgerEventType::GrantIssued,
                &grant.recipient_user_id,
                &grant.id,
                grant.amount_micros,
                None,
                format!("grant-issued:{}", grant.id),
            )
            .await
        {
            mark_issued_ledgered(db, &grant.id, now).await?;
            recovered += 1;
        }

        let terminal = match grant.status {
            CreditGrantStatus::Expired => Some((
                BillingLedgerEventType::GrantExpired,
                grant.terminal_amount_micros.max(0),
                format!("grant-expired:{}", grant.id),
            )),
            CreditGrantStatus::Revoked => Some((
                BillingLedgerEventType::GrantRevoked,
                grant.terminal_amount_micros.max(0),
                format!("grant-revoked:{}", grant.id),
            )),
            CreditGrantStatus::Active | CreditGrantStatus::Consumed => None,
        };
        if grant.terminal_ledgered_at.is_none()
            && let Some((event_type, amount_micros, dedupe_key)) = terminal
            && super::ledger::record_grant_event(
                db,
                event_type,
                &grant.recipient_user_id,
                &grant.id,
                amount_micros,
                None,
                dedupe_key,
            )
            .await
        {
            mark_terminal_ledgered(db, &grant.id, now).await?;
            recovered += 1;
        }
    }
    Ok(recovered)
}

pub(super) async fn mark_issued_ledgered(
    db: &mongodb::Database,
    grant_id: &str,
    now: DateTime<Utc>,
) -> AppResult<()> {
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .update_one(
            doc! { "_id": grant_id },
            doc! { "$set": {
                "issued_ledgered_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .await?;
    Ok(())
}

async fn mark_terminal_ledgered(
    db: &mongodb::Database,
    grant_id: &str,
    now: DateTime<Utc>,
) -> AppResult<()> {
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .update_one(
            doc! { "_id": grant_id },
            doc! { "$set": {
                "terminal_ledgered_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .await?;
    Ok(())
}

fn validate_issue_input(input: &IssueCreditGrantInput) -> AppResult<()> {
    if !(1..=MAX_GRANT_CREDITS).contains(&input.amount_credits) {
        return Err(AppError::ValidationError(format!(
            "amount_credits must be between 1 and {MAX_GRANT_CREDITS}"
        )));
    }
    if input.target_kind == BillingTargetKind::SelectedUsers
        && (input.target_user_ids.is_empty() || input.target_user_ids.len() > MAX_SELECTED_USERS)
    {
        return Err(AppError::ValidationError(format!(
            "selected grants require 1-{MAX_SELECTED_USERS} target users"
        )));
    }
    if input.target_kind == BillingTargetKind::AllUsers && !input.target_user_ids.is_empty() {
        return Err(AppError::ValidationError(
            "all-users grants must not include target_user_ids".to_string(),
        ));
    }
    if input.all_services && !input.service_refs.is_empty() {
        return Err(AppError::ValidationError(
            "all-services grants must not include service_refs".to_string(),
        ));
    }
    if !input.all_services
        && (input.service_refs.is_empty() || input.service_refs.len() > MAX_SCOPED_SERVICES)
    {
        return Err(AppError::ValidationError(format!(
            "service-scoped grants require 1-{MAX_SCOPED_SERVICES} services"
        )));
    }
    if input
        .reason
        .as_ref()
        .is_some_and(|reason| reason.trim().len() > MAX_GRANT_REASON_LEN)
    {
        return Err(AppError::ValidationError(format!(
            "reason must not exceed {MAX_GRANT_REASON_LEN} characters"
        )));
    }
    if input
        .expires_at
        .is_some_and(|expires_at| expires_at <= Utc::now())
    {
        return Err(AppError::ValidationError(
            "expires_at must be in the future".to_string(),
        ));
    }
    Ok(())
}

pub(super) async fn resolve_recipients(
    db: &mongodb::Database,
    target_kind: BillingTargetKind,
    selected: &[String],
) -> AppResult<Vec<String>> {
    // Billing owners are polymorphic user rows: both people and organization
    // accounts may own a wallet and consume a grant.
    let mut filter = doc! { "is_active": true };
    if target_kind == BillingTargetKind::SelectedUsers {
        let unique: std::collections::BTreeSet<&str> = selected
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .collect();
        if unique.len() != selected.len() {
            return Err(AppError::ValidationError(
                "target_user_ids must be unique, non-empty user ids".to_string(),
            ));
        }
        filter.insert(
            "_id",
            doc! { "$in": unique.into_iter().collect::<Vec<_>>() },
        );
    }
    let users: Vec<User> = db
        .collection::<User>(USERS)
        .find(filter)
        .sort(doc! { "_id": 1 })
        .await?
        .try_collect()
        .await?;
    if target_kind == BillingTargetKind::SelectedUsers && users.len() != selected.len() {
        return Err(AppError::ValidationError(
            "one or more target users do not exist or are inactive".to_string(),
        ));
    }
    Ok(users.into_iter().map(|user| user.id).collect())
}

pub(super) async fn resolve_service_scope(
    db: &mongodb::Database,
    all_services: bool,
    references: &[String],
) -> AppResult<BillingServiceScope> {
    if all_services {
        return Ok(BillingServiceScope {
            all_services: true,
            service_ids: Vec::new(),
            service_slugs: Vec::new(),
        });
    }
    let normalized: std::collections::BTreeSet<&str> = references
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect();
    if normalized.len() != references.len() {
        return Err(AppError::ValidationError(
            "service_refs must be unique and non-empty".to_string(),
        ));
    }
    let values: Vec<&str> = normalized.into_iter().collect();
    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! {
            "$or": [
                { "_id": { "$in": &values } },
                { "slug": { "$in": &values } },
            ]
        })
        .await?
        .try_collect()
        .await?;
    if services.len() != references.len() {
        return Err(AppError::ValidationError(
            "one or more scoped services were not found".to_string(),
        ));
    }
    Ok(BillingServiceScope {
        all_services: false,
        service_ids: services.iter().map(|service| service.id.clone()).collect(),
        service_slugs: services.into_iter().map(|service| service.slug).collect(),
    })
}

#[cfg(test)]
mod tests {
    use crate::models::billing_ledger::{BillingLedgerEntry, COLLECTION_NAME as BILLING_LEDGER};
    use crate::models::user::{UserProfileConfig, UserType};
    use crate::test_utils::connect_test_database;

    use super::*;

    fn user(id: &str, user_type: UserType, active: bool) -> User {
        let now = Utc::now();
        User {
            id: id.to_string(),
            email: format!("{id}@example.com"),
            password_hash: user_type.is_person().then(|| "hash".to_string()),
            display_name: Some(id.to_string()),
            slug: user_type.is_org().then(|| id.to_string()),
            avatar_url: None,
            email_verified: true,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: active,
            is_admin: false,
            is_operator: false,
            role_ids: Vec::new(),
            group_ids: Vec::new(),
            invite_code_id: None,
            mfa_enabled: false,
            social_provider: None,
            social_provider_id: None,
            user_type,
            primary_org_id: None,
            created_at: now,
            updated_at: now,
            last_login_at: None,
            profile_config: UserProfileConfig::default(),
        }
    }

    #[test]
    fn issue_validation_enforces_bounds_and_target_shape() {
        let base = IssueCreditGrantInput {
            amount_credits: 10,
            target_kind: BillingTargetKind::SelectedUsers,
            target_user_ids: vec!["user-1".to_string()],
            all_services: true,
            service_refs: Vec::new(),
            expires_at: Some(Utc::now() + chrono::Duration::days(1)),
            reason: Some("Launch credit".to_string()),
            granted_by: "admin-1".to_string(),
        };
        assert!(validate_issue_input(&base).is_ok());
        assert!(
            validate_issue_input(&IssueCreditGrantInput {
                amount_credits: 0,
                ..base.clone()
            })
            .is_err()
        );
        assert!(
            validate_issue_input(&IssueCreditGrantInput {
                target_kind: BillingTargetKind::AllUsers,
                ..base
            })
            .is_err()
        );
    }

    #[tokio::test]
    async fn all_users_grant_snapshots_active_people_and_organizations() {
        let Some(db) = connect_test_database("credit_grant_all_billing_owners").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        db.collection::<User>(USERS)
            .insert_many([
                user("person-1", UserType::Person, true),
                user("org-1", UserType::Org, true),
                user("inactive-1", UserType::Person, false),
            ])
            .await
            .expect("insert billing owners");

        let grants = issue_grants(
            &db,
            IssueCreditGrantInput {
                amount_credits: 25,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: Vec::new(),
                all_services: true,
                service_refs: Vec::new(),
                expires_at: None,
                reason: Some("Launch".to_string()),
                granted_by: "admin-1".to_string(),
            },
        )
        .await
        .expect("issue grants");

        let recipients: std::collections::BTreeSet<&str> = grants
            .iter()
            .map(|grant| grant.recipient_user_id.as_str())
            .collect();
        assert_eq!(
            recipients,
            std::collections::BTreeSet::from(["org-1", "person-1"])
        );
        assert_eq!(
            db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
                .count_documents(doc! { "event_type": "grant_issued" })
                .await
                .expect("count grant ledger entries"),
            2
        );
    }

    #[tokio::test]
    async fn large_batch_defers_activation_until_ledger_recovery() {
        let Some(db) = connect_test_database("credit_grant_deferred_activation").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let users: Vec<User> = (0..=INLINE_ISSUANCE_LEDGER_LIMIT)
            .map(|index| user(&format!("owner-{index:03}"), UserType::Person, true))
            .collect();
        db.collection::<User>(USERS)
            .insert_many(users)
            .await
            .expect("insert billing owners");

        let grants = issue_grants(
            &db,
            IssueCreditGrantInput {
                amount_credits: 10,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: Vec::new(),
                all_services: true,
                service_refs: Vec::new(),
                expires_at: None,
                reason: Some("large launch batch".to_string()),
                granted_by: "admin-1".to_string(),
            },
        )
        .await
        .expect("issue large batch");

        assert_eq!(grants.len(), INLINE_ISSUANCE_LEDGER_LIMIT + 1);
        assert_eq!(
            grants
                .iter()
                .filter(|grant| grant.issued_ledgered_at.is_some())
                .count(),
            INLINE_ISSUANCE_LEDGER_LIMIT
        );
        let pending_owner = grants
            .iter()
            .find(|grant| grant.issued_ledgered_at.is_none())
            .expect("one deferred grant")
            .recipient_user_id
            .clone();
        assert!(
            list_active_for_user(&db, &pending_owner, Utc::now())
                .await
                .expect("list pending owner's grants")
                .is_empty(),
            "a deferred grant must remain unspendable"
        );

        assert_eq!(
            recover_unledgered_events(&db, Utc::now())
                .await
                .expect("recover deferred issuance"),
            1
        );
        assert_eq!(
            list_active_for_user(&db, &pending_owner, Utc::now())
                .await
                .expect("list activated grants")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn expiry_and_revocation_preserve_in_flight_reservations() {
        let Some(db) = connect_test_database("credit_grant_expiry_and_revoke").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let now = Utc::now();
        let due = CreditGrant {
            id: "grant-due".to_string(),
            batch_id: "batch-1".to_string(),
            schedule_origin: None,
            recipient_user_id: "owner-1".to_string(),
            target_kind: BillingTargetKind::SelectedUsers,
            amount_credits: 2,
            amount_micros: 2 * CREDIT_MICROS,
            remaining_micros: 2 * CREDIT_MICROS,
            reserved_micros: 0,
            scope: BillingServiceScope {
                all_services: true,
                service_ids: Vec::new(),
                service_slugs: Vec::new(),
            },
            expires_at: Some(now),
            reason: None,
            granted_by: "admin-1".to_string(),
            status: CreditGrantStatus::Active,
            issued_ledgered_at: Some(now),
            terminal_ledgered_at: None,
            terminal_amount_micros: 0,
            active_settlement: None,
            created_at: now - chrono::Duration::days(1),
            updated_at: now,
            consumed_at: None,
            expired_at: None,
            revoked_at: None,
        };
        let mut reserved = due.clone();
        reserved.id = "grant-reserved".to_string();
        reserved.expires_at = Some(now - chrono::Duration::seconds(1));
        reserved.reserved_micros = CREDIT_MICROS;
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .insert_many([due, reserved])
            .await
            .expect("insert grants");

        assert_eq!(expire_due_grants(&db, now).await.expect("expire grants"), 1);
        assert!(revoke_grant(&db, "grant-reserved").await.is_err());
        let due = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! { "_id": "grant-due" })
            .await
            .expect("find due grant")
            .expect("due grant exists");
        let reserved = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! { "_id": "grant-reserved" })
            .await
            .expect("find reserved grant")
            .expect("reserved grant exists");

        assert_eq!(due.status, CreditGrantStatus::Expired);
        assert_eq!(due.remaining_micros, 0);
        assert_eq!(due.terminal_amount_micros, 2 * CREDIT_MICROS);
        assert_eq!(reserved.status, CreditGrantStatus::Active);
        assert_eq!(reserved.remaining_micros, 2 * CREDIT_MICROS);
    }

    #[tokio::test]
    async fn reconcile_recovers_missing_issue_and_terminal_ledger_entries() {
        let Some(db) = connect_test_database("credit_grant_ledger_recovery").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let now = Utc::now();
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .insert_one(CreditGrant {
                id: "grant-ledger-recovery".to_string(),
                batch_id: "batch-ledger-recovery".to_string(),
                schedule_origin: None,
                recipient_user_id: "owner-ledger-recovery".to_string(),
                target_kind: BillingTargetKind::SelectedUsers,
                amount_credits: 3,
                amount_micros: 3 * CREDIT_MICROS,
                remaining_micros: 0,
                reserved_micros: 0,
                scope: BillingServiceScope {
                    all_services: true,
                    service_ids: Vec::new(),
                    service_slugs: Vec::new(),
                },
                expires_at: None,
                reason: Some("recovery".to_string()),
                granted_by: "admin-1".to_string(),
                status: CreditGrantStatus::Revoked,
                issued_ledgered_at: None,
                terminal_ledgered_at: None,
                terminal_amount_micros: 2_500_000,
                active_settlement: None,
                created_at: now - chrono::Duration::minutes(1),
                updated_at: now,
                consumed_at: None,
                expired_at: None,
                revoked_at: Some(now),
            })
            .await
            .expect("insert recovery grant");

        assert_eq!(
            recover_unledgered_events(&db, now)
                .await
                .expect("recover grant ledger"),
            2
        );
        let recovered = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! { "_id": "grant-ledger-recovery" })
            .await
            .expect("find recovered grant")
            .expect("recovered grant exists");
        assert_eq!(
            recovered
                .issued_ledgered_at
                .map(|value| value.timestamp_millis()),
            Some(now.timestamp_millis())
        );
        assert_eq!(
            recovered
                .terminal_ledgered_at
                .map(|value| value.timestamp_millis()),
            Some(now.timestamp_millis())
        );
        let entries: Vec<BillingLedgerEntry> = db
            .collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .find(doc! { "reference_id": "grant-ledger-recovery" })
            .await
            .expect("find recovery ledger entries")
            .try_collect()
            .await
            .expect("collect recovery ledger entries");
        assert_eq!(entries.len(), 2);
        let revoked = entries
            .iter()
            .find(|entry| entry.event_type == BillingLedgerEventType::GrantRevoked)
            .expect("revocation entry");
        assert_eq!(revoked.amount_micros, Some(2_500_000));
    }
}
