use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, doc};
use mongodb::options::ReturnDocument;
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
    if grant.revoke_requested_at.is_some() {
        return 0;
    }
    grant
        .remaining_micros
        .saturating_sub(super::service_limits::held_micros(grant))
        .max(0)
}

pub fn eligible_available_micros(grant: &CreditGrant, now: DateTime<Utc>) -> i64 {
    if grant.status != CreditGrantStatus::Active
        || grant.issued_ledgered_at.is_none()
        || grant.expires_at.is_some_and(|expiry| expiry <= now)
    {
        return 0;
    }
    super::service_limits::realizable_micros(grant)
}

pub fn has_restrictions(grant: &CreditGrant) -> bool {
    !grant.scope.all_services || !grant.service_limits.is_empty()
}

pub fn used_micros(grant: &CreditGrant) -> i64 {
    grant
        .amount_micros
        .saturating_sub(grant.remaining_micros)
        .saturating_sub(grant.terminal_amount_micros)
        .max(0)
}

pub fn service_scope_applies(
    scope: &BillingServiceScope,
    service_id: Option<&str>,
    service_slug: Option<&str>,
) -> bool {
    if scope.all_services {
        return true;
    }
    // New scopes freeze canonical IDs. Legacy slug-only scopes still work when
    // there is no canonical identity to compare.
    match service_id {
        Some(id) if !scope.service_ids.is_empty() => scope.service_ids.iter().any(|v| v == id),
        _ => service_slug.is_some_and(|slug| scope.service_slugs.iter().any(|v| v == slug)),
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct IssueCreditGrantInput {
    pub amount_credits: i64,
    pub target_kind: BillingTargetKind,
    pub target_user_ids: Vec<String>,
    pub all_services: bool,
    pub service_refs: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub granted_by: String,
    pub service_limits: Vec<crate::models::credit_service_limit::CreditServiceLimitInput>,
    pub request_id: Option<String>,
}

pub async fn issue_grants(
    db: &mongodb::Database,
    input: IssueCreditGrantInput,
) -> AppResult<Vec<CreditGrant>> {
    let payload = serde_json::to_string(&input).map_err(|e| AppError::Internal(e.to_string()))?;
    let previous =
        super::submissions::existing(db, &input.granted_by, input.request_id.as_deref(), &payload)
            .await?;
    let (recipients, scope, service_limits, now, batch_id) = if let Some(row) = previous {
        decode_issue_snapshot(row)?
    } else {
        validate_issue_input(&input)?;
        let recipients = resolve_recipients(db, input.target_kind, &input.target_user_ids).await?;
        if recipients.is_empty() {
            return Err(AppError::ValidationError(
                "Credit allocation has no eligible recipients".into(),
            ));
        }
        let scope = resolve_service_scope(db, input.all_services, &input.service_refs).await?;
        let rules = super::service_limits::resolve(
            db,
            &scope,
            input.amount_credits * CREDIT_MICROS,
            &input.service_limits,
        )
        .await?;
        let now = Utc::now();
        if let Some(key) = input.request_id.as_deref() {
            let snapshot = doc! { "recipients": &recipients,
            "scope": bson::to_bson(&scope).map_err(|e| AppError::Internal(e.to_string()))?,
            "service_limits": bson::to_bson(&rules).map_err(|e| AppError::Internal(e.to_string()))? };
            decode_issue_snapshot(
                super::submissions::freeze(db, &input.granted_by, key, &payload, snapshot).await?,
            )?
        } else {
            (recipients, scope, rules, now, Uuid::new_v4().to_string())
        }
    };
    let amount_micros = input
        .amount_credits
        .checked_mul(CREDIT_MICROS)
        .ok_or_else(|| AppError::ValidationError("Credit allocation amount is too large".into()))?;
    let reason = input
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);

    let mut grants: Vec<CreditGrant> = recipients
        .into_iter()
        .map(|recipient_user_id| CreditGrant {
            id: Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("{batch_id}:{recipient_user_id}").as_bytes(),
            )
            .to_string(),
            batch_id: batch_id.clone(),
            schedule_origin: None,
            recipient_user_id,
            target_kind: input.target_kind,
            amount_credits: input.amount_credits,
            amount_micros,
            remaining_micros: amount_micros,
            reserved_micros: 0,
            scope: scope.clone(),
            service_limits: service_limits.clone(),
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
            revoke_requested_at: None,
            revoke_requested_by: None,
        })
        .collect();

    // Deterministic per-recipient insert-if-absent recovers a partial batch;
    // replay never resets counters, status, activation, or reclaimed money.
    for chunk in grants.chunks(super::schedules::DISBURSEMENT_CHUNK) {
        super::schedules::insert_grant_chunk(db, chunk).await?;
    }
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
    let ids: Vec<&str> = grants.iter().map(|g| g.id.as_str()).collect();
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "_id": { "$in": ids } })
        .sort(doc! { "recipient_user_id": 1 })
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

type IssueSnapshot = (
    Vec<String>,
    BillingServiceScope,
    Vec<crate::models::credit_service_limit::CreditServiceLimit>,
    DateTime<Utc>,
    String,
);
fn decode_issue_snapshot(
    row: crate::models::credit_allocation_request::CreditAllocationRequest,
) -> AppResult<IssueSnapshot> {
    #[derive(serde::Deserialize)]
    struct Snapshot {
        recipients: Vec<String>,
        scope: BillingServiceScope,
        service_limits: Vec<crate::models::credit_service_limit::CreditServiceLimit>,
    }
    let snapshot: Snapshot = bson::from_document(row.snapshot)
        .map_err(|e| AppError::Internal(format!("Allocation snapshot is invalid: {e}")))?;
    Ok((
        snapshot.recipients,
        snapshot.scope,
        snapshot.service_limits,
        row.created_at,
        row.id,
    ))
}

pub async fn list_grants(
    db: &mongodb::Database,
    recipient_user_id: Option<&str>,
    schedule_id: Option<&str>,
    limit: i64,
    skip: u64,
) -> AppResult<(Vec<CreditGrant>, u64)> {
    let mut filter = doc! {};
    if let Some(user_id) = recipient_user_id {
        filter.insert("recipient_user_id", user_id);
    }
    if let Some(schedule_id) = schedule_id {
        filter.insert("schedule_origin.schedule_id", schedule_id);
    }
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
            "revoke_requested_at": Bson::Null,
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

/// Request reclaim immediately; previously admitted holds remain durable until
/// settlement/release. Repeated requests return the same pending/final result.
pub async fn revoke_grant(db: &mongodb::Database, grant_id: &str) -> AppResult<CreditGrant> {
    request_revoke_grant(db, grant_id, None).await
}

pub async fn request_revoke_grant(
    db: &mongodb::Database,
    grant_id: &str,
    actor: Option<&str>,
) -> AppResult<CreditGrant> {
    let now = Utc::now();
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .update_one(
            doc! { "_id": grant_id, "status": "active", "revoke_requested_at": Bson::Null },
            doc! { "$set": { "revoke_requested_at": bson::DateTime::from_chrono(now),
            "revoke_requested_by": actor, "updated_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;
    finalize_reclaim(db, grant_id).await?;
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": grant_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Credit allocation not found".into()))
}

pub async fn finalize_reclaim(db: &mongodb::Database, grant_id: &str) -> AppResult<()> {
    let now = Utc::now();
    let previous = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one_and_update(
            doc! { "_id": grant_id, "status": "active", "revoke_requested_at": { "$type": "date" },
            "reserved_micros": 0_i64, "active_settlement": Bson::Null },
            vec![
                doc! { "$set": { "status": "revoked", "terminal_amount_micros": "$remaining_micros",
                "remaining_micros": 0_i64, "revoked_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now) } },
            ],
        )
        .return_document(ReturnDocument::Before)
        .await?;
    if let Some(grant) = previous {
        if super::ledger::record_grant_event(
            db,
            BillingLedgerEventType::GrantRevoked,
            &grant.recipient_user_id,
            &grant.id,
            grant.remaining_micros.max(0),
            None,
            format!("grant-revoked:{}", grant.id),
        )
        .await
        {
            mark_terminal_ledgered(db, grant_id, now).await?;
        }
    }
    Ok(())
}

pub async fn finalize_pending_reclaims(db: &mongodb::Database) -> AppResult<u64> {
    let mut cursor = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! {
            "status": "active", "revoke_requested_at": { "$type": "date" },
            "reserved_micros": 0_i64, "active_settlement": Bson::Null,
        })
        .limit(500)
        .await?;
    let mut count = 0;
    while let Some(grant) = cursor.try_next().await? {
        finalize_reclaim(db, &grant.id).await?;
        count += 1;
    }
    Ok(count)
}

pub async fn expire_due_grants(db: &mongodb::Database, now: DateTime<Utc>) -> AppResult<u64> {
    finalize_pending_reclaims(db).await?;
    let mut expired = 0;
    let mut examined = 0;
    while examined < MAX_EXPIRATIONS_PER_TICK {
        let batch_limit = EXPIRY_SWEEP_BATCH.min((MAX_EXPIRATIONS_PER_TICK - examined) as i64);
        let due: Vec<CreditGrant> = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find(doc! {
                "status": "active",
                "reserved_micros": 0_i64,
                "revoke_requested_at": Bson::Null,
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
                .find_one_and_update(
                    doc! {
                            "_id": &grant.id,
                            "status": "active",
                            "reserved_micros": 0_i64,
                            "revoke_requested_at": Bson::Null,
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
                .return_document(ReturnDocument::Before)
                .await?;
            let Some(grant) = result else {
                continue;
            };
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
        if !references.is_empty() {
            return Err(AppError::ValidationError(
                "All-services scope must not include service_refs".into(),
            ));
        }
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
            service_limits: Vec::new(),
            request_id: None,

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
                service_limits: Vec::new(),
                request_id: None,

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
                service_limits: Vec::new(),
                request_id: None,

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
            service_limits: Vec::new(),
            revoke_requested_at: None,
            revoke_requested_by: None,

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
        let pending = revoke_grant(&db, "grant-reserved")
            .await
            .expect("accept pending reclaim");
        assert!(pending.revoke_requested_at.is_some());
        assert_eq!(pending.reserved_micros, CREDIT_MICROS);
        assert_eq!(available_grant_micros(&pending), 0);
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
    async fn expiry_sweep_processes_more_than_one_legacy_batch() {
        let Some(db) = connect_test_database("credit_grant_expiry_budget_loop").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let now = Utc::now();
        let grant_count = EXPIRY_SWEEP_BATCH as usize + 1;
        let grants: Vec<CreditGrant> = (0..grant_count)
            .map(|index| CreditGrant {
                service_limits: Vec::new(),
                revoke_requested_at: None,
                revoke_requested_by: None,

                id: format!("grant-expiry-budget-{index:04}"),
                batch_id: "batch-expiry-budget".to_string(),
                schedule_origin: None,
                recipient_user_id: format!("owner-expiry-budget-{index:04}"),
                target_kind: BillingTargetKind::SelectedUsers,
                amount_credits: 1,
                amount_micros: CREDIT_MICROS,
                remaining_micros: CREDIT_MICROS,
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
            })
            .collect();
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .insert_many(grants)
            .await
            .expect("insert due grants across expiry batches");

        assert_eq!(
            expire_due_grants(&db, now)
                .await
                .expect("expire all due grants in one tick"),
            grant_count as u64
        );
        assert_eq!(
            db.collection::<CreditGrant>(CREDIT_GRANTS)
                .count_documents(doc! { "status": "expired" })
                .await
                .expect("count expired grants"),
            grant_count as u64
        );
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
                service_limits: Vec::new(),
                revoke_requested_at: None,
                revoke_requested_by: None,

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
