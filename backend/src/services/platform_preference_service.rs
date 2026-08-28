use std::collections::HashSet;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::ReturnDocument;

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::platform_operation::{
    COLLECTION_NAME as PLATFORM_OPERATIONS, OperationBilling, PlatformOperationRow,
};
use crate::models::platform_service_preference::{
    COLLECTION_NAME as PLATFORM_SERVICE_PREFERENCES, CredentialIntent,
    PlatformOperationPreferenceOverride, PlatformServicePreference,
};
use crate::models::platform_spend_usage::{
    COLLECTION_NAME as PLATFORM_SPEND_USAGE, PlatformSpendUsage,
};
use crate::services::org_service;

const MAX_OPERATION_OVERRIDES: usize = 100;
const MAX_CEILING_CREDITS: i64 = 10_000_000_000;
const SPEND_USAGE_RETENTION_DAYS: i64 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreferenceWrite {
    pub platform_enabled: bool,
    pub max_credits_per_call: String,
    pub max_credits_per_day: String,
    pub operation_overrides: Vec<PlatformOperationPreferenceOverride>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectivePlatformPreference {
    pub max_credits_per_call_micros: i64,
    pub max_credits_per_day_micros: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedCredentialIntent {
    pub intent: CredentialIntent,
    pub platform_preference: Option<EffectivePlatformPreference>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformSpendReservation {
    pub owner_id: String,
    pub catalog_service_id: String,
    pub yyyymmdd: String,
    pub reserved_micros: i64,
}

pub async fn list_preferences(
    db: &mongodb::Database,
    actor_user_id: &str,
    owner_id: &str,
) -> AppResult<Vec<PlatformServicePreference>> {
    let access = org_service::resolve_owner_access(db, actor_user_id, owner_id).await?;
    if !access.can_read() {
        return Err(AppError::Forbidden(
            "You are not allowed to view this owner's platform preferences".to_string(),
        ));
    }

    db.collection::<PlatformServicePreference>(PLATFORM_SERVICE_PREFERENCES)
        .find(doc! { "owner_id": owner_id })
        .sort(doc! { "catalog_service_id": 1 })
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

pub async fn upsert_preference(
    db: &mongodb::Database,
    actor_user_id: &str,
    owner_id: &str,
    catalog_service_id: &str,
    mut requested: PreferenceWrite,
) -> AppResult<PlatformServicePreference> {
    ensure_can_manage_owner(db, actor_user_id, owner_id).await?;
    ensure_catalog_service(db, catalog_service_id).await?;
    normalize_preference(db, catalog_service_id, &mut requested).await?;

    let now = Utc::now();
    let overrides = bson::to_bson(&requested.operation_overrides).map_err(|error| {
        AppError::Internal(format!(
            "Failed to serialize platform operation preference overrides: {error}"
        ))
    })?;
    db.collection::<PlatformServicePreference>(PLATFORM_SERVICE_PREFERENCES)
        .find_one_and_update(
            doc! {
                "owner_id": owner_id,
                "catalog_service_id": catalog_service_id,
            },
            doc! {
                "$set": {
                    "platform_enabled": requested.platform_enabled,
                    "max_credits_per_call": &requested.max_credits_per_call,
                    "max_credits_per_day": &requested.max_credits_per_day,
                    "operation_overrides": overrides,
                    "updated_by": actor_user_id,
                    "updated_at": bson::DateTime::from_chrono(now),
                },
                "$setOnInsert": {
                    "_id": uuid::Uuid::new_v4().to_string(),
                    "owner_id": owner_id,
                    "catalog_service_id": catalog_service_id,
                    "created_by": actor_user_id,
                    "created_at": bson::DateTime::from_chrono(now),
                },
            },
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| {
            AppError::Internal("Platform preference upsert returned no document".to_string())
        })
}

pub async fn delete_preference(
    db: &mongodb::Database,
    actor_user_id: &str,
    owner_id: &str,
    catalog_service_id: &str,
) -> AppResult<bool> {
    ensure_can_manage_owner(db, actor_user_id, owner_id).await?;
    Ok(db
        .collection::<PlatformServicePreference>(PLATFORM_SERVICE_PREFERENCES)
        .delete_one(doc! {
            "owner_id": owner_id,
            "catalog_service_id": catalog_service_id,
        })
        .await?
        .deleted_count
        > 0)
}

pub async fn load_preferences_for_owners(
    db: &mongodb::Database,
    owner_ids: &[String],
    catalog_service_ids: &[String],
) -> AppResult<Vec<PlatformServicePreference>> {
    if owner_ids.is_empty() || catalog_service_ids.is_empty() {
        return Ok(Vec::new());
    }
    db.collection::<PlatformServicePreference>(PLATFORM_SERVICE_PREFERENCES)
        .find(doc! {
            "owner_id": { "$in": owner_ids },
            "catalog_service_id": { "$in": catalog_service_ids },
        })
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

pub fn resolve_credential_intent(
    requested: CredentialIntent,
    preference: Option<&PlatformServicePreference>,
    operation_id: &str,
) -> AppResult<ResolvedCredentialIntent> {
    if requested == CredentialIntent::OwnOnly {
        return Ok(ResolvedCredentialIntent {
            intent: CredentialIntent::OwnOnly,
            platform_preference: None,
        });
    }

    let effective = match preference {
        Some(preference) => effective_preference(preference, operation_id)?,
        None => None,
    };
    match (requested, effective) {
        (CredentialIntent::Auto, Some(platform_preference)) => Ok(ResolvedCredentialIntent {
            intent: CredentialIntent::Auto,
            platform_preference: Some(platform_preference),
        }),
        (CredentialIntent::Auto, None) => Ok(ResolvedCredentialIntent {
            intent: CredentialIntent::OwnOnly,
            platform_preference: None,
        }),
        (CredentialIntent::PlatformOnly, Some(platform_preference)) => {
            Ok(ResolvedCredentialIntent {
                intent: CredentialIntent::PlatformOnly,
                platform_preference: Some(platform_preference),
            })
        }
        (CredentialIntent::PlatformOnly, None) => Err(AppError::PlatformOperationUnavailable),
        (CredentialIntent::OwnOnly, _) => unreachable!("own_only returned above"),
    }
}

fn effective_preference(
    preference: &PlatformServicePreference,
    operation_id: &str,
) -> AppResult<Option<EffectivePlatformPreference>> {
    let (enabled, max_call, max_day) = preference
        .operation_overrides
        .iter()
        .find(|entry| entry.operation_id == operation_id)
        .map(|entry| {
            (
                entry.platform_enabled,
                entry.max_credits_per_call.as_str(),
                entry.max_credits_per_day.as_str(),
            )
        })
        .unwrap_or((
            preference.platform_enabled,
            preference.max_credits_per_call.as_str(),
            preference.max_credits_per_day.as_str(),
        ));
    if !enabled {
        return Ok(None);
    }
    Ok(Some(EffectivePlatformPreference {
        max_credits_per_call_micros: parse_stored_credits(max_call)?,
        max_credits_per_day_micros: parse_stored_credits(max_day)?,
    }))
}

pub fn estimated_charge_micros(
    billing: &OperationBilling,
    estimated_usage: &crate::models::service_billing::PlatformUsage,
) -> AppResult<i64> {
    let rate =
        crate::services::billing::lago_client::decimal_credits_to_micros(&billing.price_per_unit)
            .ok_or(AppError::PlatformOperationUnavailable)?;
    let base = match billing.base_fee_per_call.as_deref() {
        Some(value) => crate::services::billing::lago_client::decimal_credits_to_micros(value)
            .ok_or(AppError::PlatformOperationUnavailable)?,
        None => 0,
    };
    let primary_quantity =
        crate::services::billing::meter::platform_quantity(billing.metric, estimated_usage);
    let secondary_micros = match &billing.secondary {
        Some(component) => {
            let rate = crate::services::billing::lago_client::decimal_credits_to_micros(
                &component.price_per_unit,
            )
            .ok_or(AppError::PlatformOperationUnavailable)?;
            i128::from(rate).saturating_mul(i128::from(
                crate::services::billing::meter::platform_quantity(
                    component.metric,
                    estimated_usage,
                ),
            ))
        }
        None => 0,
    };
    Ok(i128::from(rate)
        .saturating_mul(i128::from(primary_quantity))
        .saturating_add(secondary_micros)
        .saturating_add(i128::from(base))
        .min(i128::from(i64::MAX)) as i64)
}

pub async fn reserve_daily_spend(
    db: &mongodb::Database,
    owner_id: &str,
    catalog_service_id: &str,
    yyyymmdd: &str,
    estimated_charge_micros: i64,
    preference: EffectivePlatformPreference,
) -> AppResult<PlatformSpendReservation> {
    let amount = estimated_charge_micros.max(0);
    if amount > preference.max_credits_per_call_micros {
        return Err(AppError::PlatformOperationUnavailable);
    }
    if amount > preference.max_credits_per_day_micros {
        return Err(AppError::PlatformOperationUnavailable);
    }
    if amount == 0 {
        return Ok(PlatformSpendReservation {
            owner_id: owner_id.to_string(),
            catalog_service_id: catalog_service_id.to_string(),
            yyyymmdd: yyyymmdd.to_string(),
            reserved_micros: 0,
        });
    }

    let collection = db.collection::<PlatformSpendUsage>(PLATFORM_SPEND_USAGE);
    let now = Utc::now();
    let expires_at = now + Duration::days(SPEND_USAGE_RETENTION_DAYS);
    let remaining = preference.max_credits_per_day_micros - amount;
    let result = collection
        .find_one_and_update(
            doc! {
                "owner_id": owner_id,
                "catalog_service_id": catalog_service_id,
                "yyyymmdd": yyyymmdd,
                "reserved_micros": { "$lte": remaining },
            },
            doc! {
                "$setOnInsert": {
                    "_id": uuid::Uuid::new_v4().to_string(),
                    "owner_id": owner_id,
                    "catalog_service_id": catalog_service_id,
                    "yyyymmdd": yyyymmdd,
                },
                "$inc": { "reserved_micros": amount },
                "$set": {
                    "updated_at": bson::DateTime::from_chrono(now),
                    "expires_at": bson::DateTime::from_chrono(expires_at),
                },
            },
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await;
    match result {
        Ok(Some(_)) => Ok(PlatformSpendReservation {
            owner_id: owner_id.to_string(),
            catalog_service_id: catalog_service_id.to_string(),
            yyyymmdd: yyyymmdd.to_string(),
            reserved_micros: amount,
        }),
        Ok(None) => Err(AppError::PlatformOperationUnavailable),
        Err(error) if is_duplicate_key_error(&error) => Err(AppError::PlatformOperationUnavailable),
        Err(error) => Err(AppError::DatabaseError(error)),
    }
}

pub async fn release_daily_spend(
    db: &mongodb::Database,
    reservation: &PlatformSpendReservation,
) -> AppResult<()> {
    if reservation.reserved_micros <= 0 {
        return Ok(());
    }
    let collection = db.collection::<PlatformSpendUsage>(PLATFORM_SPEND_USAGE);
    collection
        .update_one(
            doc! {
                "owner_id": &reservation.owner_id,
                "catalog_service_id": &reservation.catalog_service_id,
                "yyyymmdd": &reservation.yyyymmdd,
                "reserved_micros": { "$gte": reservation.reserved_micros },
            },
            doc! {
                "$inc": { "reserved_micros": -reservation.reserved_micros },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .await?;
    collection
        .delete_one(doc! {
            "owner_id": &reservation.owner_id,
            "catalog_service_id": &reservation.catalog_service_id,
            "yyyymmdd": &reservation.yyyymmdd,
            "reserved_micros": { "$lte": 0_i64 },
        })
        .await?;
    Ok(())
}

async fn ensure_can_manage_owner(
    db: &mongodb::Database,
    actor_user_id: &str,
    owner_id: &str,
) -> AppResult<()> {
    let access = org_service::resolve_owner_access(db, actor_user_id, owner_id).await?;
    if !access.can_write() {
        return Err(AppError::Forbidden(
            "Only the owner or an organization admin may change platform spending preferences"
                .to_string(),
        ));
    }
    Ok(())
}

async fn ensure_catalog_service(db: &mongodb::Database, catalog_service_id: &str) -> AppResult<()> {
    let exists = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": catalog_service_id, "is_active": true })
        .await?
        .is_some();
    if !exists {
        return Err(AppError::NotFound("Catalog service not found".to_string()));
    }
    Ok(())
}

async fn normalize_preference(
    db: &mongodb::Database,
    catalog_service_id: &str,
    requested: &mut PreferenceWrite,
) -> AppResult<()> {
    requested.max_credits_per_call =
        normalize_ceiling("max_credits_per_call", &requested.max_credits_per_call)?;
    requested.max_credits_per_day =
        normalize_ceiling("max_credits_per_day", &requested.max_credits_per_day)?;
    if parse_stored_credits(&requested.max_credits_per_day)?
        < parse_stored_credits(&requested.max_credits_per_call)?
    {
        return Err(AppError::ValidationError(
            "max_credits_per_day must be at least max_credits_per_call".to_string(),
        ));
    }
    if requested.operation_overrides.len() > MAX_OPERATION_OVERRIDES {
        return Err(AppError::ValidationError(format!(
            "operation_overrides must contain at most {MAX_OPERATION_OVERRIDES} entries"
        )));
    }

    let mut operation_ids = HashSet::with_capacity(requested.operation_overrides.len());
    for entry in &mut requested.operation_overrides {
        if uuid::Uuid::parse_str(&entry.operation_id).is_err() {
            return Err(AppError::ValidationError(
                "operation_overrides contains an invalid operation_id".to_string(),
            ));
        }
        if !operation_ids.insert(entry.operation_id.clone()) {
            return Err(AppError::ValidationError(
                "operation_overrides contains duplicate operation_id values".to_string(),
            ));
        }
        entry.max_credits_per_call = normalize_ceiling(
            "operation_overrides.max_credits_per_call",
            &entry.max_credits_per_call,
        )?;
        entry.max_credits_per_day = normalize_ceiling(
            "operation_overrides.max_credits_per_day",
            &entry.max_credits_per_day,
        )?;
        if parse_stored_credits(&entry.max_credits_per_day)?
            < parse_stored_credits(&entry.max_credits_per_call)?
        {
            return Err(AppError::ValidationError(
                "each operation override max_credits_per_day must be at least max_credits_per_call"
                    .to_string(),
            ));
        }
    }
    if operation_ids.is_empty() {
        return Ok(());
    }

    let matching = db
        .collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
        .count_documents(doc! {
            "_id": { "$in": operation_ids.into_iter().collect::<Vec<_>>() },
            "catalog_service_id": catalog_service_id,
        })
        .await?;
    if matching != requested.operation_overrides.len() as u64 {
        return Err(AppError::ValidationError(
            "every operation override must reference an operation owned by the catalog service"
                .to_string(),
        ));
    }
    Ok(())
}

fn normalize_ceiling(field: &str, raw: &str) -> AppResult<String> {
    let value = raw.trim();
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.chars().all(|character| character.is_ascii_digit())
        || fraction.is_some_and(|part| {
            part.is_empty()
                || part.len() > 6
                || !part.chars().all(|character| character.is_ascii_digit())
        })
    {
        return Err(AppError::ValidationError(format!(
            "{field} must be a non-negative decimal with at most 6 fractional digits"
        )));
    }
    let micros = crate::services::billing::lago_client::decimal_credits_to_micros(value)
        .ok_or_else(|| {
            AppError::ValidationError(format!("{field} is outside the supported range"))
        })?;
    if micros > MAX_CEILING_CREDITS.saturating_mul(1_000_000) {
        return Err(AppError::ValidationError(format!(
            "{field} must not exceed {MAX_CEILING_CREDITS} credits"
        )));
    }
    Ok(format_micros(micros))
}

fn parse_stored_credits(value: &str) -> AppResult<i64> {
    crate::services::billing::lago_client::decimal_credits_to_micros(value)
        .ok_or(AppError::PlatformOperationUnavailable)
}

fn format_micros(micros: i64) -> String {
    let whole = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    if fraction == 0 {
        return whole.to_string();
    }
    format!("{whole}.{fraction:06}")
        .trim_end_matches('0')
        .to_string()
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Command(command) if command.code == 11000
    ) || matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;
    use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgRole};
    use crate::models::platform_operation::{OperationLimits, PerRequestCaps};
    use crate::models::service_billing::BillingMetric;
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};

    fn preference(enabled: bool) -> PlatformServicePreference {
        let timestamp = chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp");
        PlatformServicePreference {
            id: "preference-id".to_string(),
            owner_id: "owner-id".to_string(),
            catalog_service_id: "catalog-id".to_string(),
            platform_enabled: enabled,
            max_credits_per_call: "5".to_string(),
            max_credits_per_day: "50".to_string(),
            operation_overrides: Vec::new(),
            created_by: "owner-id".to_string(),
            updated_by: "owner-id".to_string(),
            created_at: timestamp,
            updated_at: timestamp,
        }
    }

    fn preference_write() -> PreferenceWrite {
        PreferenceWrite {
            platform_enabled: true,
            max_credits_per_call: "2.500000".to_string(),
            max_credits_per_day: "25.000000".to_string(),
            operation_overrides: Vec::new(),
        }
    }

    async fn insert_catalog_service(db: &mongodb::Database) -> DownstreamService {
        let mut service = dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = format!("preference-provider-{}", &service.id[..8]);
        service.is_active = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert catalog service");
        service
    }

    #[test]
    fn auto_without_opt_in_resolves_to_own_only() {
        let resolved = resolve_credential_intent(CredentialIntent::Auto, None, "operation-id")
            .expect("resolve intent");

        assert_eq!(resolved.intent, CredentialIntent::OwnOnly);
        assert!(resolved.platform_preference.is_none());
    }

    #[test]
    fn operation_override_can_revoke_provider_opt_in() {
        let mut preference = preference(true);
        preference.operation_overrides = vec![PlatformOperationPreferenceOverride {
            operation_id: "operation-id".to_string(),
            platform_enabled: false,
            max_credits_per_call: "1".to_string(),
            max_credits_per_day: "10".to_string(),
        }];

        let resolved =
            resolve_credential_intent(CredentialIntent::Auto, Some(&preference), "operation-id")
                .expect("resolve intent");

        assert_eq!(resolved.intent, CredentialIntent::OwnOnly);
        assert!(resolved.platform_preference.is_none());
    }

    #[test]
    fn explicit_platform_only_still_requires_stored_consent() {
        assert!(matches!(
            resolve_credential_intent(CredentialIntent::PlatformOnly, None, "operation-id"),
            Err(AppError::PlatformOperationUnavailable)
        ));
    }

    #[test]
    fn estimated_charge_includes_base_and_quantity_without_float() {
        let billing = OperationBilling {
            metric: crate::models::service_billing::BillingMetric::Characters,
            price_per_unit: "0.002".to_string(),
            secondary: None,
            base_fee_per_call: Some("1.5".to_string()),
            lago_metric_code: "metric".to_string(),
            sync_status: crate::models::service_billing::PricingSyncStatus::Synced,
            sync_error: None,
        };

        let usage =
            crate::models::service_billing::PlatformUsage::single_request(0).with_characters(250);
        assert_eq!(
            estimated_charge_micros(&billing, &usage).unwrap(),
            2_000_000
        );
    }

    #[test]
    fn ceiling_normalization_is_exact_and_rejects_excess_precision() {
        assert_eq!(normalize_ceiling("cap", " 2.500000 ").unwrap(), "2.5");
        assert!(normalize_ceiling("cap", "0.0000001").is_err());
        assert!(normalize_ceiling("cap", "-1").is_err());
    }

    #[tokio::test]
    async fn preference_writes_enforce_owner_acl_and_operation_ownership() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_preference_owner_acl").await
        else {
            eprintln!("skipping platform preference ACL test: no local MongoDB available");
            return;
        };
        let catalog = insert_catalog_service(&db).await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let direct = upsert_preference(&db, &owner_id, &owner_id, &catalog.id, preference_write())
            .await
            .expect("direct owner writes preference");
        assert_eq!(direct.max_credits_per_call, "2.5");
        assert_eq!(direct.max_credits_per_day, "25");

        let org_id = uuid::Uuid::new_v4().to_string();
        let admin_id = uuid::Uuid::new_v4().to_string();
        let member_id = uuid::Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_many([
                crate::test_utils::test_user(&org_id, UserType::Org),
                crate::test_utils::test_user(&admin_id, UserType::Person),
                crate::test_utils::test_user(&member_id, UserType::Person),
            ])
            .await
            .expect("insert organization actors");
        db.collection(ORG_MEMBERSHIPS)
            .insert_many([
                crate::test_utils::test_membership(&org_id, &admin_id, OrgRole::Admin, None),
                crate::test_utils::test_membership(&org_id, &member_id, OrgRole::Member, None),
            ])
            .await
            .expect("insert organization memberships");

        let org_preference =
            upsert_preference(&db, &admin_id, &org_id, &catalog.id, preference_write())
                .await
                .expect("organization admin writes preference");
        assert_eq!(org_preference.owner_id, org_id);
        assert_eq!(
            list_preferences(&db, &member_id, &org_preference.owner_id)
                .await
                .expect("organization member reads preference")
                .len(),
            1
        );
        assert!(matches!(
            upsert_preference(
                &db,
                &member_id,
                &org_preference.owner_id,
                &catalog.id,
                preference_write(),
            )
            .await,
            Err(AppError::Forbidden(_))
        ));

        let foreign_operation = PlatformOperationRow::new_endpoint(
            uuid::Uuid::new_v4().to_string(),
            "GET".to_string(),
            "/foreign".to_string(),
            "Foreign operation".to_string(),
            None,
            OperationLimits {
                per_request: PerRequestCaps::Endpoint,
                per_user_per_day: Some(10),
            },
            OperationBilling::free(BillingMetric::Requests),
            admin_id.clone(),
        );
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(&foreign_operation)
            .await
            .expect("insert foreign operation");
        let mut invalid = preference_write();
        invalid.operation_overrides = vec![PlatformOperationPreferenceOverride {
            operation_id: foreign_operation.id,
            platform_enabled: true,
            max_credits_per_call: "1".to_string(),
            max_credits_per_day: "10".to_string(),
        }];
        assert!(matches!(
            upsert_preference(&db, &admin_id, &org_preference.owner_id, &catalog.id, invalid)
                .await,
            Err(AppError::ValidationError(message))
                if message.contains("owned by the catalog service")
        ));
    }

    #[tokio::test]
    async fn malformed_stored_ceiling_fails_closed_without_panicking() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_preference_malformed_ceiling").await
        else {
            eprintln!("skipping malformed preference test: no local MongoDB available");
            return;
        };
        let catalog = insert_catalog_service(&db).await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let mut stored = preference(true);
        stored.id = uuid::Uuid::new_v4().to_string();
        stored.owner_id = owner_id.clone();
        stored.catalog_service_id = catalog.id.clone();
        stored.max_credits_per_day = "not-a-credit-amount".to_string();
        db.collection::<PlatformServicePreference>(PLATFORM_SERVICE_PREFERENCES)
            .insert_one(stored)
            .await
            .expect("store malformed preference outside the API write path");

        let loaded = load_preferences_for_owners(
            &db,
            std::slice::from_ref(&owner_id),
            std::slice::from_ref(&catalog.id),
        )
        .await
        .expect("load malformed stored preference");
        assert_eq!(loaded.len(), 1);
        assert!(matches!(
            resolve_credential_intent(CredentialIntent::Auto, loaded.first(), "operation-id"),
            Err(AppError::PlatformOperationUnavailable)
        ));
    }

    #[tokio::test]
    async fn preference_owner_catalog_index_is_unique() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_preference_unique_index").await
        else {
            eprintln!("skipping platform preference index test: no local MongoDB available");
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("create platform preference indexes");

        let mut first = preference(true);
        first.id = uuid::Uuid::new_v4().to_string();
        first.owner_id = uuid::Uuid::new_v4().to_string();
        first.catalog_service_id = uuid::Uuid::new_v4().to_string();
        let mut duplicate = first.clone();
        duplicate.id = uuid::Uuid::new_v4().to_string();
        let collection = db.collection::<PlatformServicePreference>(PLATFORM_SERVICE_PREFERENCES);
        collection
            .insert_one(first)
            .await
            .expect("insert first preference");
        let error = collection
            .insert_one(duplicate)
            .await
            .expect_err("owner and catalog preference must be unique");
        assert!(is_duplicate_key_error(&error));
    }

    #[tokio::test]
    async fn provider_daily_spend_ceiling_is_atomic_and_releasable() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_preference_daily_spend").await
        else {
            eprintln!("skipping platform daily spend test: no local MongoDB available");
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("create platform spend indexes");
        let owner_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = uuid::Uuid::new_v4().to_string();
        let preference = EffectivePlatformPreference {
            max_credits_per_call_micros: 1_000_000,
            max_credits_per_day_micros: 1_000_000,
        };

        let (first, second) = tokio::join!(
            reserve_daily_spend(
                &db,
                &owner_id,
                &catalog_service_id,
                "20260828",
                600_000,
                preference,
            ),
            reserve_daily_spend(
                &db,
                &owner_id,
                &catalog_service_id,
                "20260828",
                600_000,
                preference,
            ),
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        assert!(
            [first.as_ref(), second.as_ref()]
                .into_iter()
                .any(|result| matches!(result, Err(AppError::PlatformOperationUnavailable)))
        );
        let reservation = first.or(second).expect("one reservation succeeds");
        let stored = db
            .collection::<PlatformSpendUsage>(PLATFORM_SPEND_USAGE)
            .find_one(doc! {
                "owner_id": &owner_id,
                "catalog_service_id": &catalog_service_id,
                "yyyymmdd": "20260828",
            })
            .await
            .expect("read daily spend")
            .expect("daily spend row");
        assert_eq!(stored.reserved_micros, 600_000);

        release_daily_spend(&db, &reservation)
            .await
            .expect("release daily spend");
        assert!(
            db.collection::<PlatformSpendUsage>(PLATFORM_SPEND_USAGE)
                .find_one(doc! { "_id": stored.id })
                .await
                .expect("read released row")
                .is_none()
        );
    }
}
