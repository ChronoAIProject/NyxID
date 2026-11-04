//! Finite CMA receipts use ordinary service prices and the shared funding ledger.
use chrono::{Duration, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use uuid::Uuid;

use super::BillingService;
use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
        credits::Credits,
        sandbox_billing::*,
        service_billing::{BillingMetric, PricingSyncStatus},
        usage_meter::{COLLECTION_NAME as USAGE, UsageMeterRow},
    },
};

mod evidence;
mod prices;
mod windows;

#[cfg(test)]
mod tests;

pub(super) use evidence::verify_usage_provider_rate;
pub use evidence::{SandboxEvidence, SandboxIntervalEvidence};
pub use windows::ReserveSandboxWindow;

pub const WRITE_SCOPE: &str = "billing:sandbox:write";
pub const METRICS: [BillingMetric; 3] = [
    BillingMetric::SandboxCpuMillisSeconds,
    BillingMetric::SandboxMemoryMibSeconds,
    BillingMetric::SandboxDiskGibSeconds,
];

fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}
fn conflict(message: &str) -> AppError {
    AppError::Conflict(message.into())
}
fn unavailable(message: &str) -> AppError {
    AppError::BillingProviderUnavailable(message.into())
}

fn uuid(value: &str) -> AppResult<()> {
    let parsed =
        Uuid::parse_str(value).map_err(|_| invalid("Sandbox billing identity must be UUID v4"))?;
    if parsed.get_version_num() != 4 || parsed.to_string() != value {
        return Err(invalid(
            "Sandbox billing identity must be canonical UUID v4",
        ));
    }
    Ok(())
}

fn sandbox_id(value: &str) -> AppResult<()> {
    if value.strip_prefix("sbx_").is_none_or(|id| {
        id.len() != 32
            || !id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }) {
        return Err(invalid("Invalid CMA Sandbox identity"));
    }
    Ok(())
}

fn units(allocation: &SandboxAllocation) -> [i64; 3] {
    [
        allocation.cpu_millis,
        allocation.memory_mib,
        allocation.disk_gib,
    ]
}

fn quantities(quantity: &SandboxUsageQuantity) -> [i64; 3] {
    [
        quantity.cpu_millis_seconds,
        quantity.memory_mib_seconds,
        quantity.disk_gib_seconds,
    ]
}

fn digest<T: serde::Serialize>(value: &T) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(value).map_err(|_| invalid("Invalid Sandbox billing input"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// Created only after the verified AuthUser extractor has established an SA JWT.
/// Live scopes and credential generation are fenced again with money writes.
pub struct ProducerAuthority {
    id: String,
    token_jti: String,
    credential_generation: i64,
}

pub async fn authorize_producer(
    db: &Database,
    configured_id: Option<&str>,
    actor_id: &str,
    scopes: &str,
    token_jti: &str,
) -> AppResult<ProducerAuthority> {
    use crate::models::service_account::{COLLECTION_NAME, ServiceAccount, ServiceAccountPurpose};
    if configured_id != Some(actor_id)
        || !scopes.split_whitespace().any(|s| s == WRITE_SCOPE)
        || token_jti.is_empty()
    {
        return Err(AppError::Forbidden(
            "Sandbox billing requires the configured producer".into(),
        ));
    }
    let account = db
        .collection::<ServiceAccount>(COLLECTION_NAME)
        .find_one(doc! {"_id":actor_id,"is_active":true})
        .await?
        .ok_or_else(|| AppError::Forbidden("Sandbox billing producer is inactive".into()))?;
    if account.purpose != ServiceAccountPurpose::General
        || !account
            .allowed_scopes
            .split_whitespace()
            .any(|s| s == WRITE_SCOPE)
    {
        return Err(AppError::Forbidden(
            "Sandbox billing producer scope is unavailable".into(),
        ));
    }
    Ok(ProducerAuthority {
        id: actor_id.into(),
        token_jti: token_jti.into(),
        credential_generation: account.credential_generation,
    })
}

async fn fence_producer(
    db: &Database,
    session: &mut ClientSession,
    producer: &ProducerAuthority,
) -> AppResult<()> {
    use crate::models::service_account::{COLLECTION_NAME, ServiceAccount, ServiceAccountPurpose};
    let account = db
        .collection::<ServiceAccount>(COLLECTION_NAME)
        .find_one(doc! {"_id":&producer.id,"is_active":true,})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Forbidden("Sandbox billing producer changed".into()))?;
    if account.credential_generation != producer.credential_generation
        || account.purpose != ServiceAccountPurpose::General
        || !account
            .allowed_scopes
            .split_whitespace()
            .any(|s| s == WRITE_SCOPE)
    {
        return Err(AppError::Forbidden(
            "Sandbox billing producer scope was removed".into(),
        ));
    }
    let changed = db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&producer.id,"is_active":true,},
            doc! {"$inc":{"sandbox_billing_revision":1_i64}},
        )
        .session(&mut *session)
        .await?;
    if changed.matched_count != 1 {
        return Err(conflict("Sandbox producer admission raced"));
    }
    // AuthUser verified this persisted JWT. Touch it so revocation conflicts
    // with the same transaction, rather than admitting from a stale extractor.
    let token = db.collection::<crate::models::service_account_token::ServiceAccountToken>(crate::models::service_account_token::COLLECTION_NAME)
        .find_one(doc! {"jti":&producer.token_jti,"service_account_id":&producer.id,"revoked":false,"expires_at":{"$gt":bson::DateTime::from_chrono(Utc::now())}})
        .session(&mut *session).await?
        .ok_or_else(|| AppError::Forbidden("Sandbox producer token is no longer active".into()))?;
    if token.credential_generation != producer.credential_generation
        || !token
            .scope
            .split_whitespace()
            .any(|scope| scope == WRITE_SCOPE)
    {
        return Err(AppError::Forbidden(
            "Sandbox producer token authority changed".into(),
        ));
    }
    let changed = db
        .collection::<Document>(crate::models::service_account_token::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&token.id,"revoked":false},
            doc! {"$inc":{"sandbox_billing_revision":1_i64}},
        )
        .session(session)
        .await?;
    if changed.matched_count != 1 {
        return Err(AppError::Forbidden(
            "Sandbox producer token is no longer active".into(),
        ));
    }
    Ok(())
}

async fn fence_owner(db: &Database, session: &mut ClientSession, owner: &str) -> AppResult<()> {
    let changed = db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id":owner,"is_active":true,"deletion_requested_at":bson::Bson::Null,"$or":[{"user_type":"person"},{"user_type":{"$exists":false}}]},doc! {"$inc":{"sandbox_billing_revision":1_i64}})
        .session(session).await?;
    if changed.matched_count != 1 {
        return Err(AppError::Forbidden(
            "Sandbox billing requires an active personal owner".into(),
        ));
    }
    Ok(())
}

pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    for (collection, keys) in [
        (WINDOW_COLLECTION, doc! {"producer_id":1,"request_id":1}),
        (CURSOR_COLLECTION, doc! {"producer_id":1,"sandbox_id":1}),
        (
            EVIDENCE_COLLECTION,
            doc! {"producer_id":1,"sandbox_id":1,"revision":1},
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
    }
    db.collection::<Document>(QUOTE_COLLECTION)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"producer_id":1,"accepted_operation_id":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(
                            doc! {"accepted_operation_id":{"$type":"string"}},
                        )
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(QUOTE_COLLECTION)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"service_id":1,"accepted_at":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(WINDOW_COLLECTION)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"producer_id":1,"sandbox_id":1,"finalized_at":1})
                .build(),
        )
        .await?;
    Ok(())
}
