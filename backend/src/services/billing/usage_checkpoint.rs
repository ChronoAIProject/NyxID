//! Early numeric stream evidence. Observation never changes final quantities;
//! recovery freezes only final reports, then uses the normal settlement path.
mod parser;
mod recovery;

pub(crate) use recovery::{recover_request, recover_stale};

use chrono::Utc;
use mongodb::bson::{self, doc};

use super::{MeteredProxyContext, meter};
use crate::{
    errors::{AppError, AppResult},
    models::{
        platform_model_tariff::{ModelChargeVector, NativeUsageEvidence},
        service_billing::{BillingMetric, PlatformUsage},
        usage_checkpoint::{UsageCheckpoint, UsageCheckpointProtocol as Protocol},
        usage_meter::{BillingLayer, COLLECTION_NAME as METERS},
    },
};

const VERSION: i32 = 1;
const MAX_COMPONENTS: usize = 128;

pub(crate) fn partial_evidence(
    value: &serde_json::Value,
    protocol: Protocol,
) -> Option<NativeUsageEvidence> {
    parser::evidence(value, protocol).ok()
}

// A checkpoint is a best-effort crash bridge, not a condition of delivery. One
// second bounds the added wait even when server selection or a write hangs.
pub(crate) const CHECKPOINT_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

pub(crate) fn supports_path(path: &str) -> bool {
    !matches!(
        crate::services::assistant_model_observer::protocol(path),
        crate::services::assistant_model_observer::Protocol::Unknown
    )
}

#[derive(Default)]
pub(crate) struct StreamCheckpoints {
    recorder: Option<Box<Recorder>>,
}

struct Recorder {
    db: mongodb::Database,
    request_id: String,
    transaction_id: String,
    last: Option<UsageCheckpoint>,
    sequence: i64,
    recovery_delay_secs: u64,
    parser: parser::Parser,
    metrics: Vec<BillingMetric>,
}

fn unavailable() -> AppError {
    AppError::BillingProviderUnavailable("usage_checkpoint_unavailable".into())
}

impl StreamCheckpoints {
    pub(crate) fn new(
        db: &mongodb::Database,
        metered: &MeteredProxyContext,
        enabled: bool,
        recovery_delay_secs: u64,
    ) -> Self {
        let recorder = metered
            .route
            .as_ref()
            .filter(|ctx| enabled && ctx.has_billable_layers())
            .map(|ctx| {
                Box::new(Recorder {
                    db: crate::services::transaction_service::durable_database(db),
                    request_id: ctx.billing_request_id.clone(),
                    transaction_id: meter::transaction_id(
                        &ctx.billing_request_id,
                        if ctx.platform_billable {
                            BillingLayer::Platform
                        } else {
                            BillingLayer::Resale
                        },
                        None,
                    ),
                    last: None,
                    sequence: 0,
                    recovery_delay_secs,
                    parser: Default::default(),
                    metrics: ctx
                        .platform_specs()
                        .filter(|_| ctx.platform_billable)
                        .map(|(metric, _)| metric)
                        .chain(ctx.resale.iter().map(|spec| spec.metric))
                        .collect(),
                })
            });
        Self { recorder }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.recorder.is_some()
    }

    /// Reuse the relay's complete, decoded event. Interim counters remain in
    /// the parser only; only a report recovery can promote causes a write.
    pub(crate) async fn observe_value(
        &mut self,
        event_type: Option<&str>,
        value: &serde_json::Value,
    ) {
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        match recorder.parser.event(event_type, value) {
            Ok(Some((protocol, evidence, true)))
                if recorder
                    .metrics
                    .iter()
                    .any(|metric| quantity_from(protocol, &evidence, *metric).is_some()) =>
            {
                self.publish(protocol, evidence, true, None).await
            }
            Ok(Some(_)) if recorder.last.is_some() => self.invalidate().await,
            Ok(_) => {}
            Err(()) => self.invalidate().await,
        }
    }

    /// A later ambiguous report can withdraw the prior final report once.
    /// After withdrawal (or its failure), this stream does no further work.
    pub(crate) async fn invalidate(&mut self) {
        let last = self.recorder.as_ref().and_then(|r| r.last.clone());
        if let Some(last) = last {
            self.publish(last.protocol, last.evidence, false, None)
                .await;
        }
        self.recorder = None;
    }

    pub(crate) async fn model_observation(
        &mut self,
        protocol: Protocol,
        evidence: NativeUsageEvidence,
        final_report: bool,
        charges: Option<Box<ModelChargeVector>>,
    ) {
        if final_report && charges.is_some() {
            self.publish(protocol, evidence, true, charges).await;
        } else if self.recorder.as_ref().is_some_and(|r| r.last.is_some()) {
            self.invalidate().await;
        }
    }

    async fn publish(
        &mut self,
        protocol: Protocol,
        evidence: NativeUsageEvidence,
        final_report: bool,
        charges: Option<Box<ModelChargeVector>>,
    ) {
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        let result = tokio::time::timeout(
            CHECKPOINT_WRITE_TIMEOUT,
            recorder.record(protocol, evidence, final_report, charges),
        )
        .await;
        let reason = match result {
            Ok(Ok(())) => return,
            Ok(Err(reason)) => reason,
            Err(_) => "timeout",
        };
        tracing::warn!(billing_request_id = %recorder.request_id, reason,
            "Stream usage checkpoint disabled");
        self.recorder = None;
    }
}

impl Recorder {
    async fn record(
        &mut self,
        protocol: Protocol,
        evidence: NativeUsageEvidence,
        final_report: bool,
        model_charges: Option<Box<ModelChargeVector>>,
    ) -> Result<(), &'static str> {
        if self.last.as_ref().is_some_and(|last| {
            last.protocol == protocol
                && last.evidence == evidence
                && last.final_report == final_report
                && last.model_charges == model_charges
        }) {
            return Ok(());
        }
        self.sequence = self.sequence.checked_add(1).ok_or("invalid_evidence")?;
        let checkpoint = UsageCheckpoint {
            version: VERSION,
            sequence: self.sequence,
            protocol,
            evidence,
            final_report,
            model_charges,
            recovered: false,
            observed_at: Utc::now(),
            recover_after: Utc::now()
                + chrono::Duration::seconds(self.recovery_delay_secs.min(i32::MAX as u64) as i64),
        };
        if !valid(&checkpoint) {
            return Err("invalid_evidence");
        }
        let encoded = bson::to_bson(&checkpoint).map_err(|_| "invalid_evidence")?;
        #[cfg(test)]
        crate::billing_integration_tests::usage_checkpoints::before_checkpoint_write(&self.db)
            .await;
        // Finalization, release and recovery touch this same attempt row. Losing
        // their predicate is ordinary; it disables this observer, not delivery.
        let changed = self.db.collection::<bson::Document>(METERS).update_one(doc! {
            "transaction_id": &self.transaction_id,
            "billing_request_id": &self.request_id,
            "status": "forwarded", "forwarded": true, "released": false,
            "funding": {"$type":"object"},
            "quantity": bson::Bson::Null, "pending_platform_usage": bson::Bson::Null,
            "pending_resale_quantity": bson::Bson::Null,
            "usage_checkpoint.recovered": {"$ne":true},
            "$or":[{"usage_checkpoint":bson::Bson::Null}, {"usage_checkpoint.sequence":{"$lt":self.sequence}}],
        }, doc! {"$set":{"usage_checkpoint":encoded, "updated_at":bson::DateTime::now()}})
            .await.map_err(|_| "database_error")?;
        if changed.matched_count != 1 {
            return Err("no_match");
        }
        self.last = Some(checkpoint);
        #[cfg(test)]
        crate::billing_integration_tests::usage_checkpoints::point(
            crate::billing_integration_tests::usage_checkpoints::Phase::AfterCheckpoint,
        )
        .await;
        Ok(())
    }
}

fn valid(checkpoint: &UsageCheckpoint) -> bool {
    let e = &checkpoint.evidence;
    checkpoint.version == VERSION
        && checkpoint.sequence > 0
        && [
            e.input,
            e.output,
            e.total,
            e.cache_read,
            e.cache_write,
            e.reasoning,
            e.tool_input,
            e.input_image,
            e.input_audio,
            e.output_image,
            e.output_audio,
            e.cached_image,
            e.cached_audio,
            e.images,
            e.requests,
        ]
        .into_iter()
        .flatten()
        .all(|n| n >= 0)
        && e.writes.len() <= 4
        && e.writes.values().all(|n| *n >= 0)
        && checkpoint.model_charges.as_ref().is_none_or(|v| {
            v.native_usage == *e
                && v.quantities.len() <= MAX_COMPONENTS
                && v.quantities
                    .iter()
                    .all(|(code, n)| code.len() <= 256 && *n >= 0)
                && v.public_model.len() <= 256
                && v.tariff_id.len() <= 256
        })
}

fn quantity(checkpoint: &UsageCheckpoint, metric: BillingMetric) -> Option<i64> {
    quantity_from(checkpoint.protocol, &checkpoint.evidence, metric)
}

fn quantity_from(
    protocol: Protocol,
    e: &NativeUsageEvidence,
    metric: BillingMetric,
) -> Option<i64> {
    match metric {
        BillingMetric::Tokens => e.total,
        BillingMetric::Requests => e.requests,
        BillingMetric::InputTokens if protocol == Protocol::Messages => e.input,
        BillingMetric::InputTokens => {
            // Unlike a final adapter, recovery cannot infer absent cache
            // reads. Chat/Responses have no separate cache-creation class;
            // any explicitly supplied extension still subtracts that class.
            // A missing cache-read counter retains this component's hold.
            e.input?
                .checked_sub(e.cache_read?)?
                .checked_sub(e.cache_write.unwrap_or(0))
                .filter(|n| *n >= 0)
        }
        BillingMetric::OutputTokens => e.output,
        BillingMetric::CacheReadTokens => e.cache_read,
        BillingMetric::CacheWriteTokens => e.cache_write,
        BillingMetric::Images => e.images,
        // Wire bytes, durations, runtime allocations and tariff-only classes
        // are not implied by an ordinary provider token report.
        _ => None,
    }
}

fn platform_usage(checkpoint: &UsageCheckpoint) -> PlatformUsage {
    let e = &checkpoint.evidence;
    PlatformUsage {
        model_charges: checkpoint.model_charges.clone(),
        requests: e.requests.unwrap_or(0),
        tokens: e.total.unwrap_or(0),
        input_tokens: quantity(checkpoint, BillingMetric::InputTokens).unwrap_or(0),
        output_tokens: e.output.unwrap_or(0),
        cache_read_tokens: e.cache_read.unwrap_or(0),
        cache_write_tokens: e.cache_write.unwrap_or(0),
        images: e.images.unwrap_or(0),
        audio_tokens: (e.input_audio.is_some() || e.output_audio.is_some()).then_some(
            crate::models::service_billing::AudioTokens {
                input_tokens: e.input_audio.unwrap_or(0),
                output_tokens: e.output_audio.unwrap_or(0),
            },
        ),
        image_tokens: (e.input_image.is_some() || e.output_image.is_some()).then_some(
            crate::models::service_billing::ImageTokens {
                input_tokens: e.input_image.unwrap_or(0),
                output_tokens: e.output_image.unwrap_or(0),
            },
        ),
        token_breakdown: Some(crate::models::service_billing::TokenBreakdown {
            prompt_tokens: e.input.unwrap_or(0),
            completion_tokens: e.output.unwrap_or(0),
            cached_tokens: e.cache_read.unwrap_or(0),
            cache_creation_tokens: e.cache_write.unwrap_or(0),
        }),
        ..Default::default()
    }
}
