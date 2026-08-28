use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::ReturnDocument;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::platform_operation::{ConstrainedConfig, ConstrainedOp, PlatformOperationKind};
use crate::models::usage_meter::{COLLECTION_NAME as USAGE_METER, DeferredQuantity, UsageMeterRow};
use crate::services::{audit_service, platform_credential_service, platform_operation_service};

const MAX_DEFERRED_BATCH: i64 = 100;
const DEFERRED_CLAIM_LEASE_SECS: i64 = 60;
const DEFERRED_RETRY_BASE_SECS: i64 = 30;
const DEFERRED_RETRY_MAX_SECS: i64 = 30 * 60;
const DEFERRED_TIMEOUT_HOURS: i64 = 24;

#[derive(Clone)]
pub struct PlatformBillingRuntime {
    pub encryption_keys: Arc<EncryptionKeys>,
    pub http_client: reqwest::Client,
}

impl PlatformBillingRuntime {
    pub fn new(encryption_keys: Arc<EncryptionKeys>, http_client: reqwest::Client) -> Self {
        Self {
            encryption_keys,
            http_client,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeferredReconcileStats {
    pub completed: u64,
    pub retried: u64,
    pub timed_out: u64,
}

pub async fn reconcile_due(
    db: &mongodb::Database,
    runtime: &PlatformBillingRuntime,
    now: DateTime<Utc>,
) -> AppResult<DeferredReconcileStats> {
    let rows: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find(doc! {
            "status": "forwarded",
            "released": false,
            "deferred_quantity.type": "twilio_call",
            "$or": [
                { "deferred_next_retry_at": { "$lte": bson::DateTime::from_chrono(now) } },
                { "deferred_next_retry_at": null },
                { "deferred_next_retry_at": { "$exists": false } },
            ],
        })
        .sort(doc! { "deferred_next_retry_at": 1, "created_at": 1 })
        .limit(MAX_DEFERRED_BATCH)
        .await?
        .try_collect()
        .await?;

    let mut stats = DeferredReconcileStats::default();
    for row in rows {
        let Some(claimed) = claim_row(db, &row, now).await? else {
            continue;
        };
        let Some(descriptor) = claimed.deferred_quantity.clone() else {
            continue;
        };

        if claimed.created_at <= now - Duration::hours(DEFERRED_TIMEOUT_HOURS) {
            if finalize_row(db, &claimed, &descriptor, 0, now).await? {
                stats.timed_out += 1;
                if let Err(error) = audit_service::log_system_event(
                    db.clone(),
                    "platform_call_duration_unresolved",
                    Some(serde_json::json!({
                        "usage_row_id": claimed.id,
                        "catalog_service_id": claimed.service_id,
                        "operation": "call_and_say",
                        "reason": "duration_timeout",
                    })),
                )
                .await
                {
                    tracing::warn!(error = %error, "deferred Twilio timeout audit failed");
                }
            }
            continue;
        }

        if !claimed.base_fee_applied
            && let Err(error) =
                super::funding::apply_deferred_base_fee(db, &claimed, &descriptor).await
        {
            tracing::warn!(
                usage_row_id = %claimed.id,
                error = %error,
                "deferred Twilio base-fee application will retry"
            );
            schedule_retry(db, &claimed, &descriptor, now).await?;
            stats.retried += 1;
            continue;
        }

        match poll_twilio(db, runtime, &claimed, &descriptor).await {
            Ok(Some(seconds)) => {
                if finalize_row(db, &claimed, &descriptor, seconds, now).await? {
                    stats.completed += 1;
                }
            }
            Ok(None) => {
                schedule_retry(db, &claimed, &descriptor, now).await?;
                stats.retried += 1;
            }
            Err(error) => {
                tracing::warn!(
                    usage_row_id = %claimed.id,
                    error = %error,
                    "deferred Twilio duration poll will retry"
                );
                schedule_retry(db, &claimed, &descriptor, now).await?;
                stats.retried += 1;
            }
        }
    }
    Ok(stats)
}

async fn claim_row(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    now: DateTime<Utc>,
) -> AppResult<Option<UsageMeterRow>> {
    let Some(descriptor) = row.deferred_quantity.as_ref() else {
        return Ok(None);
    };
    let descriptor = bson::to_bson(descriptor).map_err(|error| {
        AppError::Internal(format!("failed to encode deferred quantity: {error}"))
    })?;
    let attempts_filter = if row.deferred_attempts == 0 {
        doc! { "$or": [
            { "deferred_attempts": 0_i32 },
            { "deferred_attempts": { "$exists": false } },
        ] }
    } else {
        doc! { "deferred_attempts": row.deferred_attempts }
    };
    db.collection::<UsageMeterRow>(USAGE_METER)
        .find_one_and_update(
            doc! {
                "_id": &row.id,
                "status": "forwarded",
                "released": false,
                "deferred_quantity": descriptor,
                "$and": [attempts_filter],
            },
            doc! {
                "$inc": { "deferred_attempts": 1_i32 },
                "$set": {
                    "deferred_next_retry_at": bson::DateTime::from_chrono(
                        now + Duration::seconds(DEFERRED_CLAIM_LEASE_SECS)
                    ),
                    "updated_at": bson::DateTime::from_chrono(now),
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await
        .map_err(Into::into)
}

async fn poll_twilio(
    db: &mongodb::Database,
    runtime: &PlatformBillingRuntime,
    row: &UsageMeterRow,
    descriptor: &DeferredQuantity,
) -> AppResult<Option<i64>> {
    let DeferredQuantity::TwilioCall {
        account_sid,
        call_sid,
    } = descriptor;
    if !platform_operation_service::is_twilio_account_sid(account_sid)
        || !platform_operation_service::is_twilio_call_sid(call_sid)
    {
        return Err(AppError::Internal(
            "stored deferred Twilio identity is invalid".to_string(),
        ));
    }
    let catalog_service_id = row.service_id.as_deref().ok_or_else(|| {
        AppError::Internal("deferred Twilio row has no catalog service id".to_string())
    })?;
    let authorization = platform_credential_service::authorize_constrained(
        db,
        catalog_service_id,
        ConstrainedOp::CallAndSay,
    )
    .await?;
    let operation = authorization.operation();
    let PlatformOperationKind::Constrained {
        config: ConstrainedConfig::CallAndSay(config),
        ..
    } = &operation.kind
    else {
        return Err(AppError::PlatformOperationUnavailable);
    };
    if config.account_sid != *account_sid {
        return Err(AppError::PlatformOperationUnavailable);
    }

    let target = platform_credential_service::materialize_authorized(
        db,
        &runtime.encryption_keys,
        authorization,
    )
    .await?
    .into_proxy_target();
    let (credential_sid, auth_token) = target
        .credential
        .split_once(':')
        .ok_or_else(|| AppError::PlatformOperationUnavailable)?;
    if credential_sid != account_sid || auth_token.is_empty() {
        return Err(AppError::PlatformOperationUnavailable);
    }
    let mut base_url = target.base_url.trim_end_matches('/').to_string();
    base_url.push('/');
    let url = reqwest::Url::parse(&base_url)
        .and_then(|url| {
            url.join(&format!(
                "2010-04-01/Accounts/{account_sid}/Calls/{call_sid}.json"
            ))
        })
        .map_err(|error| AppError::Internal(format!("Twilio poll URL is invalid: {error}")))?;
    let response = runtime
        .http_client
        .get(url)
        .basic_auth(credential_sid, Some(auth_token))
        .send()
        .await
        .map_err(|error| AppError::Internal(format!("Twilio duration poll failed: {error}")))?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let value = platform_operation_service::read_vendor_json(
        crate::models::platform_operation::PlatformOperationName::CallAndSay,
        response,
    )
    .await?;
    if value.get("status").and_then(serde_json::Value::as_str) != Some("completed") {
        return Ok(None);
    }
    let duration = match value.get("duration") {
        Some(serde_json::Value::String(value)) => value.parse::<i64>().ok(),
        Some(serde_json::Value::Number(value)) => value.as_i64(),
        _ => None,
    }
    .filter(|duration| *duration >= 0)
    .ok_or_else(|| AppError::Internal("Twilio completed call has invalid duration".to_string()))?;
    Ok(Some(duration))
}

async fn finalize_row(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    descriptor: &DeferredQuantity,
    seconds: i64,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    let descriptor = bson::to_bson(descriptor).map_err(|error| {
        AppError::Internal(format!("failed to encode deferred quantity: {error}"))
    })?;
    let finalized = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one_and_update(
            doc! {
                "_id": &row.id,
                "status": "forwarded",
                "released": false,
                "deferred_quantity": descriptor,
            },
            doc! {
                "$set": {
                    "status": "finalized",
                    "quantity": seconds.max(0),
                    "finalized_at": bson::DateTime::from_chrono(now),
                    "updated_at": bson::DateTime::from_chrono(now),
                },
                "$unset": {
                    "deferred_quantity": "",
                    "deferred_next_retry_at": "",
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;
    let Some(finalized) = finalized else {
        return Ok(false);
    };
    super::reservation::claim_released_and_settle(db, &finalized).await?;
    Ok(true)
}

async fn schedule_retry(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    descriptor: &DeferredQuantity,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let exponent = u32::try_from(row.deferred_attempts.saturating_sub(1))
        .unwrap_or(0)
        .min(10);
    let seconds = DEFERRED_RETRY_BASE_SECS
        .saturating_mul(1_i64 << exponent)
        .min(DEFERRED_RETRY_MAX_SECS);
    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! {
                "_id": &row.id,
                "status": "forwarded",
                "released": false,
                "deferred_quantity": bson::to_bson(descriptor).map_err(|error| {
                    AppError::Internal(format!("failed to encode deferred quantity: {error}"))
                })?,
                "deferred_attempts": row.deferred_attempts,
            },
            doc! { "$set": {
                "deferred_next_retry_at": bson::DateTime::from_chrono(
                    now + Duration::seconds(seconds)
                ),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{Json, Router, http::StatusCode, routing::get};
    use mongodb::bson::doc;

    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::platform_operation::{
        COLLECTION_NAME as PLATFORM_OPERATIONS, CallAndSayOperationConfig, OperationBilling,
        OperationLimits, PerRequestCaps, PlatformOperationRow,
    };
    use crate::models::service_billing::{BillingMetric, PricingSyncStatus};
    use crate::models::usage_meter::{BillingLayer, CredentialClass, UsageStatus};
    use crate::services::platform_credential_service;
    use crate::test_utils::{connect_test_database, test_encryption_keys};

    use super::*;

    const ACCOUNT_SID: &str = "AC11111111111111111111111111111111";
    const CALL_SID: &str = "CA22222222222222222222222222222222";

    async fn spawn_twilio_stub(
        status: &'static str,
        duration: Option<&'static str>,
    ) -> (String, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_handler = calls.clone();
        let path = format!("/2010-04-01/Accounts/{ACCOUNT_SID}/Calls/{CALL_SID}.json");
        let app = Router::new().route(
            &path,
            get(move || {
                let calls = calls_for_handler.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({
                        "sid": CALL_SID,
                        "status": status,
                        "duration": duration,
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind Twilio stub");
        let address = listener.local_addr().expect("Twilio stub address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve Twilio stub");
        });
        (format!("http://{address}"), calls)
    }

    async fn spawn_transient_twilio_stub() -> (String, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_handler = calls.clone();
        let path = format!("/2010-04-01/Accounts/{ACCOUNT_SID}/Calls/{CALL_SID}.json");
        let app = Router::new().route(
            &path,
            get(move || {
                let calls = calls_for_handler.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind transient Twilio stub");
        let address = listener.local_addr().expect("Twilio stub address");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve transient Twilio stub");
        });
        (format!("http://{address}"), calls)
    }

    async fn insert_twilio_authority(
        db: &mongodb::Database,
        keys: &EncryptionKeys,
        base_url: &str,
    ) -> (DownstreamService, PlatformOperationRow) {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = "api-twilio".to_string();
        service.name = "Twilio".to_string();
        service.base_url = base_url.to_string();
        service.service_type = "http".to_string();
        service.auth_method = "basic".to_string();
        service.auth_key_name = "Authorization".to_string();
        service.credential_encrypted = Vec::new();
        service.is_active = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert Twilio catalog service");
        platform_credential_service::set_credential_for_test(
            db,
            keys,
            &service.id,
            &format!("{ACCOUNT_SID}:auth-token"),
            "admin",
        )
        .await
        .expect("set Twilio platform credential");

        let mut operation = PlatformOperationRow::new_constrained(
            service.id.clone(),
            ConstrainedOp::CallAndSay,
            ConstrainedConfig::CallAndSay(CallAndSayOperationConfig {
                allowed_destination_prefixes: vec!["+65".to_string()],
                voice: "alice".to_string(),
                account_sid: ACCOUNT_SID.to_string(),
                call_from: "+6512345678".to_string(),
            }),
            OperationLimits {
                per_request: PerRequestCaps::CallAndSay {
                    max_message_chars: 500,
                    max_duration_seconds: 600,
                },
                per_user_per_day: Some(10),
            },
            OperationBilling {
                metric: BillingMetric::Seconds,
                price_per_unit: "0.01".to_string(),
                secondary: None,
                base_fee_per_call: Some("1.5".to_string()),
                lago_metric_code: "platform_op_api_twilio_constrained_call_and_say".to_string(),
                sync_status: PricingSyncStatus::Synced,
                sync_error: None,
            },
            "admin".to_string(),
        );
        operation.enabled = true;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(&operation)
            .await
            .expect("insert call-and-say operation");
        (service, operation)
    }

    fn deferred_row(service_id: &str, created_at: DateTime<Utc>) -> UsageMeterRow {
        UsageMeterRow {
            id: uuid::Uuid::new_v4().to_string(),
            transaction_id: uuid::Uuid::new_v4().to_string(),
            billing_request_id: uuid::Uuid::new_v4().to_string(),
            layer: BillingLayer::Platform,
            flush_seq: None,
            billing_owner_id: "owner-1".to_string(),
            wallet_id: None,
            actor_user_id: "owner-1".to_string(),
            api_key_id: None,
            service_id: Some(service_id.to_string()),
            service_slug: Some("api-twilio".to_string()),
            metric: BillingMetric::Seconds,
            lago_metric_code: "platform_op_api_twilio_constrained_call_and_say".to_string(),
            credential_class: CredentialClass::NyxidManagedMaster,
            model: None,
            token_breakdown: None,
            reserved_credits: 0,
            funding: None,
            quantity: None,
            base_fee_micros: Some(1_500_000),
            base_fee_applied: false,
            base_fee_applied_credits: 0,
            deferred_quantity: Some(DeferredQuantity::TwilioCall {
                account_sid: ACCOUNT_SID.to_string(),
                call_sid: CALL_SID.to_string(),
            }),
            deferred_attempts: 0,
            deferred_next_retry_at: Some(created_at),
            pending_resale_quantity: None,
            pending_platform_secondary_quantity: None,
            status: UsageStatus::Forwarded,
            forwarded: true,
            released: false,
            lago_acked: false,
            attempt: 0,
            settlement_attempts: 0,
            settlement_next_retry_at: None,
            created_at,
            updated_at: created_at,
            finalized_at: None,
            expires_at: None,
            last_error: None,
        }
    }

    #[test]
    fn retry_backoff_is_bounded() {
        let row_attempts = [1_i32, 2, 12];
        let delays: Vec<i64> = row_attempts
            .into_iter()
            .map(|attempt| {
                let exponent = u32::try_from(attempt.saturating_sub(1))
                    .unwrap_or(0)
                    .min(10);
                DEFERRED_RETRY_BASE_SECS
                    .saturating_mul(1_i64 << exponent)
                    .min(DEFERRED_RETRY_MAX_SECS)
            })
            .collect();
        assert_eq!(delays, vec![30, 60, 1800]);
    }

    #[tokio::test]
    async fn concurrent_completed_call_reconcile_settles_seconds_once() {
        let Some(db) = connect_test_database("deferred_twilio_completed_once").await else {
            return;
        };
        let (base_url, vendor_calls) = spawn_twilio_stub("completed", Some("37")).await;
        let keys = Arc::new(test_encryption_keys());
        let (service, _) = insert_twilio_authority(&db, &keys, &base_url).await;
        let now = Utc::now();
        let row = deferred_row(&service.id, now - Duration::seconds(1));
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .expect("insert deferred row");
        let runtime = PlatformBillingRuntime::new(keys, reqwest::Client::new());

        let (left, right) = tokio::join!(
            reconcile_due(&db, &runtime, now),
            reconcile_due(&db, &runtime, now)
        );
        let left = left.expect("left reconcile");
        let right = right.expect("right reconcile");
        assert_eq!(left.completed + right.completed, 1);
        assert_eq!(vendor_calls.load(Ordering::SeqCst), 1);

        let saved = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("find deferred row")
            .expect("deferred row exists");
        assert_eq!(saved.quantity, Some(37));
        assert_eq!(saved.status, UsageStatus::Finalized);
        assert!(saved.released);
        assert!(saved.deferred_quantity.is_none());
        assert!(saved.base_fee_applied);
    }

    #[tokio::test]
    async fn nonterminal_then_disabled_authorizer_retries_without_decrypting() {
        let Some(db) = connect_test_database("deferred_twilio_disabled_retry").await else {
            return;
        };
        let (base_url, vendor_calls) = spawn_twilio_stub("queued", None).await;
        let keys = Arc::new(test_encryption_keys());
        let (service, operation) = insert_twilio_authority(&db, &keys, &base_url).await;
        let now = Utc::now();
        let row = deferred_row(&service.id, now - Duration::seconds(1));
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .expect("insert deferred row");
        let runtime = PlatformBillingRuntime::new(keys.clone(), reqwest::Client::new());

        let first = reconcile_due(&db, &runtime, now)
            .await
            .expect("queued reconcile");
        assert_eq!(first.retried, 1);
        assert_eq!(vendor_calls.load(Ordering::SeqCst), 1);

        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .update_one(
                doc! { "_id": &operation.id },
                doc! { "$set": { "enabled": false } },
            )
            .await
            .expect("disable operation");
        db.collection::<UsageMeterRow>(USAGE_METER)
            .update_one(
                doc! { "_id": &row.id },
                doc! { "$set": {
                    "deferred_next_retry_at": bson::DateTime::from_chrono(now - Duration::seconds(1)),
                } },
            )
            .await
            .expect("make deferred retry due");
        let before = keys.decrypt_stats();
        let second = reconcile_due(&db, &runtime, now)
            .await
            .expect("disabled reconcile");
        assert_eq!(second.retried, 1);
        assert_eq!(keys.decrypt_stats(), before);
        assert_eq!(vendor_calls.load(Ordering::SeqCst), 1);

        let saved = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("find deferred row")
            .expect("deferred row exists");
        assert_eq!(saved.status, UsageStatus::Forwarded);
        assert!(saved.deferred_quantity.is_some());
        assert!(!saved.released);
    }

    #[tokio::test]
    async fn transient_vendor_failure_retains_descriptor_and_schedules_retry() {
        let Some(db) = connect_test_database("deferred_twilio_transient_retry").await else {
            return;
        };
        let (base_url, vendor_calls) = spawn_transient_twilio_stub().await;
        let keys = Arc::new(test_encryption_keys());
        let (service, _) = insert_twilio_authority(&db, &keys, &base_url).await;
        let now = Utc::now();
        let row = deferred_row(&service.id, now - Duration::seconds(1));
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .expect("insert deferred row");
        let runtime = PlatformBillingRuntime::new(keys, reqwest::Client::new());

        let stats = reconcile_due(&db, &runtime, now)
            .await
            .expect("transient failure reconcile");
        assert_eq!(stats.retried, 1);
        assert_eq!(vendor_calls.load(Ordering::SeqCst), 1);

        let saved = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("find deferred row")
            .expect("deferred row exists");
        assert_eq!(saved.status, UsageStatus::Forwarded);
        assert_eq!(saved.deferred_attempts, 1);
        assert!(saved.deferred_quantity.is_some());
        assert!(saved.deferred_next_retry_at.is_some_and(|at| at > now));
        assert!(!saved.released);
    }

    #[tokio::test]
    async fn twenty_four_hour_timeout_finalizes_base_only_without_vendor_or_decrypt() {
        let Some(db) = connect_test_database("deferred_twilio_timeout").await else {
            return;
        };
        let (base_url, vendor_calls) = spawn_twilio_stub("completed", Some("99")).await;
        let keys = Arc::new(test_encryption_keys());
        let (service, _) = insert_twilio_authority(&db, &keys, &base_url).await;
        let now = Utc::now();
        let row = deferred_row(&service.id, now - Duration::hours(25));
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .expect("insert deferred row");
        let before = keys.decrypt_stats();
        let runtime = PlatformBillingRuntime::new(keys.clone(), reqwest::Client::new());

        let stats = reconcile_due(&db, &runtime, now)
            .await
            .expect("timeout reconcile");
        assert_eq!(stats.timed_out, 1);
        assert_eq!(vendor_calls.load(Ordering::SeqCst), 0);
        assert_eq!(keys.decrypt_stats(), before);

        let saved = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": &row.id })
            .await
            .expect("find timed-out row")
            .expect("timed-out row exists");
        assert_eq!(saved.quantity, Some(0));
        assert_eq!(saved.status, UsageStatus::Finalized);
        assert!(saved.released);
        assert!(saved.deferred_quantity.is_none());

        let audit = db
            .collection::<crate::models::audit_log::AuditLog>(
                crate::models::audit_log::COLLECTION_NAME,
            )
            .find_one(doc! { "event_type": "platform_call_duration_unresolved" })
            .await
            .expect("find timeout audit")
            .expect("timeout audit exists");
        let encoded = serde_json::to_string(&audit.event_data).expect("encode timeout audit");
        assert!(!encoded.contains(ACCOUNT_SID));
        assert!(!encoded.contains(CALL_SID));
    }
}
