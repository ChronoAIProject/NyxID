//! Admin-authored producer authority for the existing CMA catalog service.
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Document, doc},
};

use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        sandbox_billing::WINDOW_COLLECTION,
        sandbox_billing_settings::{COLLECTION_NAME as SETTINGS, SandboxBillingSettings},
        service_account::{COLLECTION_NAME as ACCOUNTS, ServiceAccount, ServiceAccountPurpose},
    },
    services::audit_service::{self, AuditActor},
};

pub const CMA_SLUG: &str = "cma";

pub async fn resolve_service(db: &Database) -> AppResult<DownstreamService> {
    let mut services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(SERVICES)
        .find(doc! {"slug": CMA_SLUG, "is_active": true})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    if services.len() != 1 {
        return Err(AppError::BillingProviderUnavailable(
            "CMA catalog service is missing or ambiguous".into(),
        ));
    }
    let service = services.pop().unwrap();
    super::sandbox_runtime::uuid(&service.id)?;
    Ok(service)
}

pub async fn load(db: &Database, service_id: &str) -> AppResult<Option<SandboxBillingSettings>> {
    Ok(db
        .collection::<SandboxBillingSettings>(SETTINGS)
        .find_one(doc! {"_id": service_id})
        .await?)
}

pub async fn for_producer(db: &Database, producer_id: &str) -> AppResult<SandboxBillingSettings> {
    let mut settings: Vec<SandboxBillingSettings> = db
        .collection::<SandboxBillingSettings>(SETTINGS)
        .find(doc! {"authorized_producer_service_account_id": producer_id})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    if settings.len() != 1 {
        return Err(AppError::Forbidden(
            "Sandbox billing requires an authorized producer".into(),
        ));
    }
    Ok(settings.pop().unwrap())
}

pub async fn update(
    db: &Database,
    service_id: &str,
    producer_id: Option<&str>,
    expected_revision: i64,
    actor: &AuditActor,
    key: &[u8],
) -> AppResult<SandboxBillingSettings> {
    super::sandbox_runtime::uuid(service_id)?;
    if let Some(id) = producer_id {
        super::sandbox_runtime::uuid(id)?;
    }
    let service = resolve_service(db).await?;
    if service.id != service_id {
        return Err(AppError::ValidationError(
            "Sandbox billing settings belong to the active CMA service".into(),
        ));
    }
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let settings = db.collection::<SandboxBillingSettings>(SETTINGS);
    let current = settings
        .find_one(doc! {"_id": service_id})
        .session(&mut session)
        .await?;
    let revision = current.as_ref().map_or(0, |s| s.revision);
    if revision != expected_revision {
        return Err(AppError::Conflict(
            "Sandbox settings changed; reload before saving".into(),
        ));
    }
    let previous = current
        .as_ref()
        .and_then(|s| s.authorized_producer_service_account_id.as_deref());
    if let Some(current) = current.as_ref().filter(|_| previous == producer_id) {
        return Ok(current.clone());
    }
    // Both publication and admission touch this row. Changing the pin cannot
    // race a newly funded window and strand its retained evidence.
    let changed = db.collection::<Document>(SERVICES).update_one(
        doc! {"_id": service_id, "slug": CMA_SLUG, "is_active": true, "updated_at": bson::DateTime::from_chrono(service.updated_at)},
        doc! {"$inc": {"sandbox_billing_revision": 1_i64}},
    ).session(&mut session).await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict(
            "CMA service changed; reload before saving".into(),
        ));
    }
    if let Some(old) = previous
        && db
            .collection::<Document>(WINDOW_COLLECTION)
            .find_one(doc! {"producer_id": old, "finalized_at": bson::Bson::Null})
            .session(&mut session)
            .await?
            .is_some()
    {
        return Err(AppError::Conflict("Submit evidence for funded Sandbox windows before changing the producer; rotate the existing account's credentials for continuity".into()));
    }
    if let Some(id) = producer_id {
        let account = db
            .collection::<ServiceAccount>(ACCOUNTS)
            .find_one(doc! {"_id": id, "is_active": true})
            .session(&mut session)
            .await?
            .ok_or_else(|| {
                AppError::ValidationError(
                    "Sandbox producer must be an active service account".into(),
                )
            })?;
        if account.purpose != ServiceAccountPurpose::General
            || !account
                .allowed_scopes
                .split_whitespace()
                .any(|s| s == super::sandbox_runtime::WRITE_SCOPE)
        {
            return Err(AppError::ValidationError(
                "Sandbox producer requires General purpose and billing:sandbox:write scope".into(),
            ));
        }
        if settings
            .find_one(
                doc! {"_id": {"$ne": service_id}, "authorized_producer_service_account_id": id},
            )
            .session(&mut session)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict(
                "Sandbox producer already belongs to another service".into(),
            ));
        }
        // The pin grants arbitrary-owner producer authority. Only platform
        // admins may subsequently manage this account, including its secret.
        db.collection::<Document>(ACCOUNTS).update_one(doc! {"_id": id}, doc! {"$set": {"platform_protected": true}, "$inc": {"sandbox_billing_revision": 1_i64}})
            .session(&mut session).await?;
    }
    let next = SandboxBillingSettings {
        service_id: service_id.into(),
        authorized_producer_service_account_id: producer_id.map(str::to_owned),
        revision: revision
            .checked_add(1)
            .ok_or_else(|| AppError::Conflict("Sandbox settings revision exhausted".into()))?,
        updated_at: Utc::now(),
    };
    settings
        .replace_one(doc! {"_id": service_id}, &next)
        .upsert(true)
        .session(&mut session)
        .await?;
    audit_service::log_actor_event_in_session(db, &mut session, key, actor, "sandbox_billing_producer_updated", serde_json::json!({
        "service_id": service_id, "before": previous, "after": producer_id, "revision": next.revision,
    })).await?;
    session.commit_transaction().await?;
    Ok(next)
}
