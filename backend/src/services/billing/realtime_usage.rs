//! Durable Realtime session prefixes. Only terminal publication sets quantities.
mod deadline;
mod recovery;

pub(crate) use deadline::BridgeDeadline;
pub(crate) use recovery::recover_stale;

use chrono::Utc;
use mongodb::bson::{self, doc};
use std::time::Duration;

use super::{MeteredProxyContext, meter};
use crate::{
    errors::{AppError, AppResult},
    models::{
        realtime_session::RealtimeUsagePrefix,
        service_billing::{BillingMetric, PlatformUsage},
        usage_meter::{BillingLayer, COLLECTION_NAME as METERS, UsageMeterRow},
    },
    services::{
        llm_usage_service::{self, RealtimeLlmUsageSummary},
        transaction_service,
    },
};

pub(crate) const MAX_DURATION: Duration = Duration::from_secs(3600);
const RECOVERY_GRACE_SECS: i64 = 60;
const VERSION: i32 = 1;
const MAX_ROWS: usize = 128;

pub(crate) fn supports_path(path: &str) -> bool {
    matches!(path.trim_matches('/'), "realtime" | "v1/realtime")
}

pub(crate) struct Session {
    db: mongodb::Database,
    request: String,
    token: String,
    coordinator: String,
    sequence: i64,
    observing: bool,
    pub(crate) deadline: BridgeDeadline,
}

/// Forwarding and the zero-prefix marker are one durable transition. A failed
/// initialization is an ordinary pre-upgrade billing failure, never a live
/// socket without its recovery identity.
pub(crate) async fn start(
    db: &mongodb::Database,
    metered: &MeteredProxyContext,
    use_reported_usage: bool,
) -> AppResult<Session> {
    let route = metered.route.as_ref().ok_or_else(unavailable)?;
    let coordinator = meter::transaction_id(
        &route.billing_request_id,
        if route.platform_billable {
            BillingLayer::Platform
        } else {
            BillingLayer::Resale
        },
        None,
    );
    let token = uuid::Uuid::new_v4().to_string();
    let deadline = BridgeDeadline::new(MAX_DURATION);
    let prefix = bson::to_bson(&RealtimeUsagePrefix::default()).map_err(|_| unavailable())?;
    let context = (
        db.clone(),
        route.billing_request_id.clone(),
        coordinator.clone(),
        token.clone(),
        prefix,
        use_reported_usage,
    );
    transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, request, coordinator, token, prefix, use_reported_usage)| {
            Box::pin(start_in_session(
                tx,
                db,
                request,
                coordinator,
                token,
                prefix,
                *use_reported_usage,
            ))
        },
    )
    .await?;
    Ok(Session {
        db: transaction_service::durable_database(db),
        request: route.billing_request_id.clone(),
        token,
        coordinator,
        sequence: 0,
        observing: true,
        deadline,
    })
}

async fn start_in_session(
    tx: &mut mongodb::ClientSession,
    db: &mongodb::Database,
    request: &str,
    coordinator: &str,
    token: &str,
    prefix: &bson::Bson,
    use_reported_usage: bool,
) -> AppResult<()> {
    let collection = db.collection::<bson::Document>(METERS);
    let filter = doc! {
        "transaction_id": coordinator,
        "billing_request_id": request,
        "status": "reserved",
        "released": false,
        "quantity": bson::Bson::Null,
        "realtime_session": bson::Bson::Null,
    };
    let close_millis = MAX_DURATION.as_millis() as i64;
    let recover_millis = close_millis + RECOVERY_GRACE_SECS * 1000;
    let marker = doc! {"$set": {
        "status": "forwarded", "forwarded": true, "updated_at": "$$NOW",
        "realtime_session": {
            "version": VERSION, "token": {"$literal": token}, "sequence": 0_i64,
            "closed": false, "recovered": false, "use_reported_usage": use_reported_usage,
            "close_at": {"$add": ["$$NOW", close_millis]},
            "recover_after": {"$add": ["$$NOW", recover_millis]},
            "prefix": {"$literal": prefix},
        },
    }};
    let result = collection
        .update_one(filter, vec![marker])
        .session(&mut *tx)
        .await?;
    if result.matched_count != 1 {
        return Err(unavailable());
    }
    collection
        .update_many(
            doc! {
                "billing_request_id":request, "status":"reserved", "released":false,
            },
            vec![doc! {"$set": {
                "status":"forwarded", "forwarded":true, "updated_at":"$$NOW",
            }}],
        )
        .session(&mut *tx)
        .await?;
    #[cfg(test)]
    crate::billing_integration_tests::usage_checkpoints::realtime_ws::point(
        crate::billing_integration_tests::usage_checkpoints::realtime_ws::Phase::BeforeStartCommit,
    )
    .await;
    Ok(())
}

impl Session {
    pub(crate) fn observing(&self) -> bool {
        self.observing
    }
    pub(crate) async fn observe(&mut self, summary: &RealtimeLlmUsageSummary, bytes: i64) {
        if !self.observing {
            return;
        }
        let prefix = prefix(summary, bytes);
        let result = tokio::time::timeout(
            super::usage_checkpoint::CHECKPOINT_WRITE_TIMEOUT,
            self.record(prefix),
        )
        .await;
        let reason = match result {
            Ok(Ok(())) => return,
            Ok(Err(reason)) => reason,
            Err(_) => "timeout",
        };
        tracing::warn!(billing_request_id=%self.request, reason, "Realtime usage checkpoint disabled");
        self.observing = false;
    }
    async fn record(&mut self, prefix: RealtimeUsagePrefix) -> Result<(), &'static str> {
        self.sequence = self.sequence.checked_add(1).ok_or("invalid_evidence")?;
        let prefix = bson::to_bson(&prefix).map_err(|_| "invalid_evidence")?;
        #[cfg(test)]
        crate::billing_integration_tests::usage_checkpoints::realtime_ws::before_write(&self.db)
            .await;
        let filter = doc! {
            "transaction_id": &self.coordinator, "billing_request_id": &self.request,
            "status": "forwarded", "forwarded": true, "released": false,
            "quantity": bson::Bson::Null,
            "pending_platform_usage": bson::Bson::Null,
            "pending_resale_quantity": bson::Bson::Null,
            "realtime_session.token": &self.token, "realtime_session.closed": false,
            "realtime_session.sequence": {"$lt": self.sequence},
            "$expr": {"$lt": ["$$NOW", "$realtime_session.close_at"]},
        };
        let changed = self
            .db
            .collection::<bson::Document>(METERS)
            .update_one(
                filter,
                doc! {"$set": {
                    "realtime_session.sequence": self.sequence, "realtime_session.prefix": prefix,
                }},
            )
            .await
            .map_err(|_| "database_error")?;
        if changed.matched_count != 1 {
            return Err("no_match");
        }
        #[cfg(test)]
        crate::billing_integration_tests::usage_checkpoints::realtime_ws::point(
            crate::billing_integration_tests::usage_checkpoints::realtime_ws::Phase::AfterCheckpoint).await;
        Ok(())
    }
    pub(crate) async fn persist_close(
        &self,
        usage: PlatformUsage,
    ) -> AppResult<Vec<UsageMeterRow>> {
        recovery::publish(&self.db, &self.request, Some((&self.token, usage))).await
    }
    pub(crate) async fn finish(self, usage: PlatformUsage) -> AppResult<()> {
        let rows = self.persist_close(usage).await?;
        crate::services::shutdown::process().spawn_cleanup(async move {
            if meter::settle_persisted(&self.db, rows).await.is_err() {
                tracing::warn!(billing_request_id=%self.request, "Realtime settlement deferred to durable retry");
            }
        });
        Ok(())
    }
}

fn prefix(summary: &RealtimeLlmUsageSummary, bytes: i64) -> RealtimeUsagePrefix {
    let usage = llm_usage_service::platform_usage(summary.reported_usage.as_ref(), bytes, false);
    RealtimeUsagePrefix {
        bytes: usage.bytes,
        reported_tokens: usage.tokens,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        cache_write_tokens: usage.cache_write_tokens,
        images: usage.images,
        token_breakdown: usage.token_breakdown,
        audio_tokens: usage.audio_tokens,
        image_tokens: usage.image_tokens,
        uncovered_bytes: summary.uncovered_bytes,
        reported_responses: summary.reported_response_count.min(i64::MAX as usize) as i64,
        estimated_responses: summary.estimated_response_count.min(i64::MAX as usize) as i64,
    }
}

/// Crash billing policy: charge the recorded prefix by the normal close rule.
/// Estimate only durable uncovered bytes, once. The unrecorded suffix is waived;
/// zero observed bytes/tokens is a billing decision, not proof of no consumption.
fn recovered_session_usage(
    prefix: &RealtimeUsagePrefix,
    use_reported_usage: bool,
) -> PlatformUsage {
    if !use_reported_usage {
        // Preserve routes that priced all transport bytes as estimated tokens.
        // A zero prefix is the explicit crash decision, not the old floor of 1.
        let tokens = if prefix.bytes > 0 {
            llm_usage_service::estimate_tokens_from_bytes(prefix.bytes)
        } else {
            0
        };
        return PlatformUsage::llm_completion(prefix.bytes, tokens);
    }
    let tail = if prefix.uncovered_bytes > 0 {
        llm_usage_service::estimate_tokens_from_bytes(prefix.uncovered_bytes)
    } else {
        0
    };
    PlatformUsage {
        requests: 1,
        bytes: prefix.bytes,
        tokens: prefix.reported_tokens.saturating_add(tail),
        input_tokens: prefix.input_tokens,
        output_tokens: prefix.output_tokens,
        cache_read_tokens: prefix.cache_read_tokens,
        cache_write_tokens: prefix.cache_write_tokens,
        images: prefix.images,
        token_breakdown: prefix.token_breakdown,
        audio_tokens: prefix.audio_tokens,
        image_tokens: prefix.image_tokens,
        ..Default::default()
    }
}

fn quantity(row: &UsageMeterRow, usage: &PlatformUsage) -> i64 {
    if row.layer == BillingLayer::Resale {
        match row.metric {
            BillingMetric::Requests => 1,
            BillingMetric::Bytes => usage.bytes.max(0),
            BillingMetric::Tokens => usage.tokens.max(0),
            _ => 0,
        }
    } else {
        meter::platform_quantity(row.metric, usage)
    }
}
fn unavailable() -> AppError {
    AppError::BillingProviderUnavailable("realtime_session_unavailable".into())
}
