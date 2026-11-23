//! Classify uncompleted ordinary requests without inferring death from age alone.
pub(super) mod holdless;
mod inventory;
mod ownership;
mod selection;
mod voice;
use crate::{
    errors::{AppError, AppResult},
    models::{credits::Credits, service_billing::BillingMetric, usage_meter::UsageHold},
};
use futures::TryStreamExt;
pub(crate) use inventory::diagnostics;
use mongodb::bson::{self, Bson, Document, doc};
use ownership::{Registry, registry};
pub(crate) use selection::indexes;

const METERS: &str = "usage_meter";
const MAX_ROWS: usize = 10_000;
const MAX_ALLOCATIONS: usize = 128;

fn unavailable() -> AppError {
    AppError::Internal("Request ownership inventory unavailable".into())
}
// Diagnostic and recovery deadlines can cancel an in-flight getMore. In
// MongoDB 3.7, dropping a pending SessionCursorStream panics while returning
// its buffer to the cursor. advance() leaves that buffer outside the stream's
// polling state, allowing the cancelled cursor and transaction to be dropped.
async fn next_document(
    cursor: &mut mongodb::SessionCursor<Document>,
    session: &mut mongodb::ClientSession,
) -> AppResult<Option<Document>> {
    if cursor.advance(session).await? {
        Ok(Some(cursor.deserialize_current()?))
    } else {
        Ok(None)
    }
}
async fn read_documents(
    cursor: &mut mongodb::SessionCursor<Document>,
    session: &mut mongodb::ClientSession,
) -> AppResult<Vec<Document>> {
    let mut rows = Vec::new();
    while let Some(row) = next_document(cursor, session).await? {
        rows.push(row);
    }
    Ok(rows)
}
fn present(row: &Document, key: &str) -> bool {
    row.get(key).is_some_and(|v| v != &Bson::Null)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Classification {
    pub label: &'static str,
    pub eligible: bool,
}
fn classified(label: &'static str) -> Classification {
    Classification {
        label,
        eligible: matches!(label, "eligible_now" | "eligible_voice"),
    }
}

pub(crate) fn classify(row: &Document, registry: Option<&Registry>) -> AppResult<Classification> {
    classify_for(row, registry, false)
}

fn classify_for(
    row: &Document,
    registry: Option<&Registry>,
    holdless: bool,
) -> AppResult<Classification> {
    if let Ok(siblings) = row.get_array("request_rows") {
        let siblings: Vec<Document> = siblings
            .iter()
            .map(|v| v.as_document().cloned().ok_or_else(unavailable))
            .collect::<AppResult<_>>()?;
        if siblings.len() > 128 {
            return Err(unavailable());
        }
        if let Some(label) = request_blocker(&siblings) {
            return Ok(classified(label));
        }
    }
    let metric: BillingMetric =
        bson::from_bson(row.get("metric").cloned().ok_or_else(unavailable)?)
            .map_err(|_| unavailable())?;
    let request = row
        .get_str("billing_request_id")
        .map_err(|_| unavailable())?;
    // Carrier calls have their own terminal-provider evidence, including old
    // rows written before forwarded_owner existed.
    if request.starts_with("adapter-call:") {
        return Ok(classified("excluded_channel_outbound"));
    }
    for (field, label) in [
        ("pool_attempt", "excluded_pool"),
        ("realtime_session", "excluded_realtime"),
        ("managed_execution_id", "excluded_managed"),
        ("model_tariff", "excluded_certified_model"),
    ] {
        if present(row, field) {
            return Ok(classified(label));
        }
    }
    if metric.is_runtime_allocation() {
        return Ok(classified("excluded_runtime"));
    }
    let voice_request = voice::is_voice(row);
    if holdless && voice_request {
        return Ok(classified("excluded_voice"));
    }
    if voice_request && voice::identity(request).is_none() {
        return Ok(classified("excluded_voice_unknown_shape"));
    }
    if let Ok(owner) = row.get_document("forwarded_owner") {
        match owner.get_str("ingress").map_err(|_| unavailable())? {
            "proxy" | "llm_gateway" | "llm_provider" => {}
            "mcp" => return Ok(classified("excluded_mcp")),
            "ssh_exec" => return Ok(classified("excluded_ssh_exec")),
            "ssh_tunnel" => return Ok(classified("excluded_ssh_tunnel")),
            "ssh_web_terminal" => return Ok(classified("excluded_ssh_web_terminal")),
            "channel_inbound" => return Ok(classified("excluded_channel_inbound")),
            "channel_outbound" => return Ok(classified("excluded_channel_outbound")),
            _ => return Ok(classified("excluded_unknown_ingress")),
        }
    }
    if present(row, "quantity") {
        return Ok(classified("final_quantity_awaiting_retry"));
    }
    if present(row, "pending_platform_usage") || present(row, "pending_resale_quantity") {
        return Ok(classified("durable_finalization_intent"));
    }
    if row
        .get_document("usage_checkpoint")
        .ok()
        .is_some_and(|c| c.get_bool("final_report").unwrap_or(false))
    {
        return Ok(classified("has_final_checkpoint"));
    }
    if row.get_str("status").ok() != Some("forwarded")
        || row.get_bool("forwarded").ok() != Some(true)
        || row.get_bool("released").ok() != Some(false)
    {
        return Ok(classified("other_terminal_state"));
    }
    let hold: UsageHold = bson::from_document(row.clone()).map_err(|_| unavailable())?;
    if hold.id.is_empty()
        || request.is_empty()
        || hold.billing_owner_id.is_empty()
        || hold.reserved_credits < Credits::ZERO
    {
        return Err(unavailable());
    }
    if !holdless && row.get_str("wallet_id").ok().is_none_or(str::is_empty) {
        return Ok(classified("excluded_metering_only"));
    }
    if let Some(f) = &hold.funding {
        if f.allowance_reservations.len() > MAX_ALLOCATIONS
            || f.grant_reservations.len() > MAX_ALLOCATIONS
            || f.allowance_consumptions.len() > MAX_ALLOCATIONS
            || f.grant_consumptions.len() > MAX_ALLOCATIONS
            || f.allowance_reservations
                .iter()
                .any(|a| a.quantity < 0 || a.period_id.is_empty() || a.allowance_id.is_empty())
            || f.grant_reservations.iter().any(|g| {
                g.amount < Credits::ZERO
                    || g.grant_id.is_empty()
                    || g.service_limit_key.as_ref().is_some_and(String::is_empty)
            })
        {
            return Err(unavailable());
        }
        let grants: std::collections::BTreeSet<_> =
            f.grant_reservations.iter().map(|r| &r.grant_id).collect();
        let periods: std::collections::BTreeSet<_> = f
            .allowance_reservations
            .iter()
            .map(|r| &r.period_id)
            .collect();
        if grants.len() != f.grant_reservations.len()
            || periods.len() != f.allowance_reservations.len()
        {
            return Err(unavailable());
        }
        if !holdless
            && (f.settled
                || f.settlement_claim_id.is_some()
                || !f.grant_consumptions.is_empty()
                || !f.allowance_consumptions.is_empty())
        {
            return Ok(classified("money_protocol_in_progress"));
        }
    }
    let Some(registry) = registry else {
        return Ok(classified("registry_unavailable"));
    };
    let mut result = classified(registry.ownership(row)?);
    if voice_request {
        if result.eligible {
            result = match voice::decide(row, registry) {
                voice::Decision::Ready(_) => classified("eligible_voice"),
                voice::Decision::Deferred(reason) => classified(reason),
            };
        } else if result.label == "owned_by_live_generation" {
            result = classified("voice_live_session");
        }
    }
    if present(row, "usage_checkpoint") && (!voice_request || result.eligible) {
        result.label = "has_nonfinal_checkpoint";
    }
    Ok(result)
}

// Ownership is at request scope: a Realtime/checkpoint coordinator or durable
// component intent also protects its sibling rows without repeating metadata.
fn request_blocker(rows: &[Document]) -> Option<&'static str> {
    for (field, label) in [
        ("pool_attempt", "excluded_pool"),
        ("realtime_session", "excluded_realtime"),
        ("managed_execution_id", "excluded_managed"),
        ("model_tariff", "excluded_certified_model"),
    ] {
        if rows.iter().any(|r| present(r, field)) {
            return Some(label);
        }
    }
    for row in rows {
        if row
            .get_str("billing_request_id")
            .ok()
            .is_some_and(|id| id.starts_with("adapter-call:"))
        {
            return Some("excluded_channel_outbound");
        }

        if voice::is_voice(row)
            && row
                .get_str("billing_request_id")
                .ok()
                .and_then(voice::identity)
                .is_none()
        {
            return Some("excluded_voice_unknown_shape");
        }
        if let Ok(metric) =
            bson::from_bson::<BillingMetric>(row.get("metric").cloned().unwrap_or(Bson::Null))
            && metric.is_runtime_allocation()
        {
            return Some("excluded_runtime");
        }
        if present(row, "pending_platform_usage") || present(row, "pending_resale_quantity") {
            return Some("durable_finalization_intent");
        }
        if row.get_document("usage_checkpoint").ok().is_some_and(|c| {
            c.get_bool("final_report").unwrap_or(false) && !c.get_bool("recovered").unwrap_or(false)
        }) {
            return Some("has_final_checkpoint");
        }
    }
    None
}

#[cfg(test)]
mod tests;

mod policy;
mod recovery;
mod terminal;
pub(crate) async fn recover(db: &mongodb::Database) -> AppResult<()> {
    let mut failures = super::recovery_batch::Failures::default();
    failures.absorb(recovery::recover(db).await)?;
    failures.absorb(holdless::recover(db).await)?;
    failures.finish(())
}
const STATE: &str = "billing_stuck_request_recovery";
const STATE_ID: &str = "ordinary";
#[cfg(test)]
mod holdless_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
use recovery_tests::{Phase, point as test_point};

#[cfg(test)]
pub(crate) mod voice_tests;

#[cfg(test)]
mod voice_component_tests;
