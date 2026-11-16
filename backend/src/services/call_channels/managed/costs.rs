//! Supplier evidence has its own lifecycle. Retail settlement, tenant erasure,
//! and readiness do not authorize or stop an exact retained supplier read.
use super::{conflict, resolver, unavailable};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_adapter::*,
        credits::{Credits, SCALE},
        managed_comms::*,
    },
    services::{
        audit_service::{self, AuditActor},
        transaction_service,
    },
};
use chrono::{DateTime, Datelike, Duration, Months, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyInput {
    pub revision: i64,
    pub fx: Vec<CostFxRate>,
    pub schemas: Vec<CostRecordSchema>,
    pub inventory: Option<InventoryTariffInput>,
}
/// API input for the supplier rental cycle. The anchor is an RFC 3339 instant;
/// storage keeps the BSON-date `InventoryTariff` (millisecond precision).
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryTariffInput {
    pub cycle: String,
    pub cycle_anchor: DateTime<Utc>,
    pub amount: String,
    pub currency: String,
    pub source: String,
    pub fx_policy_revision: String,
}
impl InventoryTariffInput {
    fn stored(&self) -> InventoryTariff {
        InventoryTariff {
            cycle: self.cycle.clone(),
            // Compare and persist at the stored precision so an unchanged
            // anchor never looks like an immutable-anchor rewrite.
            cycle_anchor: bson::DateTime::from_chrono(self.cycle_anchor).to_chrono(),
            amount: self.amount.clone(),
            currency: self.currency.clone(),
            source: self.source.clone(),
            fx_policy_revision: self.fx_policy_revision.clone(),
        }
    }
}
fn reference(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 2048
}
fn currency(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase())
}
fn https(s: &str) -> bool {
    url::Url::parse(s).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
    })
}
/// Currency values use the same checked 12-decimal arithmetic as retail, but
/// retain their original currency. They are never used to move customer money.
pub(super) fn amount(s: &str) -> AppResult<Credits> {
    let v: Credits = s.parse().map_err(|_| unavailable())?;
    Ok(if v < Credits::ZERO {
        v.checked_neg()?
    } else {
        v
    })
}
fn positive(s: &str) -> AppResult<Credits> {
    let v = amount(s)?;
    if v <= Credits::ZERO {
        return Err(unavailable());
    }
    Ok(v)
}
fn validate(i: &PolicyInput) -> AppResult<()> {
    if i.revision < 0 || i.fx.len() > 32 || i.schemas.len() > 12 {
        return Err(unavailable());
    }
    for (n, f) in i.fx.iter().enumerate() {
        positive(&f.credits_per_unit)?;
        if !currency(&f.currency)
            || !reference(&f.revision)
            || i.fx[..n].iter().any(|old| old.currency == f.currency)
        {
            return Err(unavailable());
        }
    }
    for (n, s) in i.schemas.iter().enumerate() {
        // An attested schema selects only closed component/record families; the
        // transport binds every query and response to the retained effect.
        if ![
            "call-control",
            "conversation-relay",
            "speech-to-text",
            "text-to-speech",
            "messaging",
        ]
        .contains(&s.record_type.as_str())
            || !["cost", "price"].contains(&s.amount_field.as_str())
            || !["currency", "price_unit"].contains(&s.currency_field.as_str())
            || !reference(&s.options_reference)
            || !reference(&s.fixture_reference)
            || i.schemas[..n].iter().any(|old| old.kind == s.kind)
            || s.record_type
                != match s.kind {
                    CostKind::CarrierInbound => "call-control",
                    CostKind::Relay => "conversation-relay",
                    CostKind::Stt => "speech-to-text",
                    CostKind::Tts | CostKind::PremiumTts => "text-to-speech",
                    CostKind::SmsPart => "messaging",
                }
        {
            return Err(unavailable());
        }
    }
    if let Some(t) = &i.inventory {
        positive(&t.amount)?;
        if t.cycle != "calendar_month"
            || !currency(&t.currency)
            || !https(&t.source)
            || !reference(&t.fx_policy_revision)
            || t.cycle_anchor > Utc::now() + Duration::days(366)
        {
            return Err(unavailable());
        }
    }
    Ok(())
}
pub async fn configure(
    state: &AppState,
    actor: &AuditActor,
    line: &str,
    i: PolicyInput,
) -> AppResult<()> {
    super::require_ready(&state.db).await?;
    validate(&i)?;
    let inventory = i.inventory.as_ref().map(InventoryTariffInput::stored);
    let db = transaction_service::durable_database(&state.db);
    transaction_service::run_db_only(
        &db,
        &(
            &db,
            actor,
            line,
            &i,
            &inventory,
            state.audit_chain_hmac_key.as_ref().as_ref(),
        ),
        |tx, c| {
            Box::pin(async move {
                let (db, actor, line, i, inventory, key) = *c;
                super::eligibility::admin_fence(db, tx, actor).await?;
                let l = db
                    .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
                    .find_one(doc! {"_id":line})
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(unavailable)?;
                let rows = db.collection::<ManagedCostPolicy>(ManagedCostPolicy::COLLECTION_NAME);
                let old = rows
                    .find_one(doc! {"line_id":line})
                    .session(&mut *tx)
                    .await?;
                if old.as_ref().map_or(0, |p| p.revision) != i.revision {
                    return Err(conflict("Cost policy changed"));
                }
                if let Some(old) = &old {
                    // Cycle identity and frozen costs cannot be silently rewritten.
                    if let (Some(a), Some(b)) = (&old.inventory, inventory)
                        && a.cycle_anchor != b.cycle_anchor
                    {
                        return Err(conflict("Supplier cycle anchor is immutable"));
                    }
                    if old.inventory.is_some() && inventory.is_none() {
                        return Err(conflict(
                            "Retain supplier inventory policy until supplier release",
                        ));
                    }
                }
                let row = ManagedCostPolicy {
                    id: old.map_or_else(|| uuid::Uuid::new_v4().to_string(), |p| p.id),
                    line_id: line.into(),
                    revision: i.revision + 1,
                    fx: i.fx.clone(),
                    schemas: i.schemas.clone(),
                    inventory: inventory.clone(),
                };
                rows.replace_one(doc! {"line_id":line}, row)
                    .upsert(true)
                    .session(&mut *tx)
                    .await?;
                if db
                    .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":line,"fence":l.fence},
                        doc! {"$inc":{super::admission::ADMISSION_TOUCH:1_i64}},
                    )
                    .session(&mut *tx)
                    .await?
                    .matched_count
                    != 1
                {
                    return Err(unavailable());
                }
                audit_service::log_actor_event_in_session(
                    db,
                    tx,
                    key,
                    actor,
                    "managed_cost_policy_configured",
                    serde_json::json!({"line_id":line,"revision":i.revision+1}),
                )
                .await?;
                Ok(())
            })
        },
    )
    .await
}
/// Admission freezes source tariffs and FX, even if cost capture runs after a
/// tariff change or after every tenant-owned row has been deleted.
#[allow(clippy::too_many_arguments)]
pub async fn register(
    db: &Database,
    tx: &mut ClientSession,
    line: &ManagedPhoneLine,
    effect: &str,
    kind: EffectKind,
    r: &AdmittedReference,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let policy = db
        .collection::<ManagedCostPolicy>(ManagedCostPolicy::COLLECTION_NAME)
        .find_one(doc! {"line_id":&line.id})
        .session(&mut *tx)
        .await?;
    let premium = if matches!(kind, EffectKind::Call) {
        db.collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME)
            .find_one(doc! {"session_id":effect})
            .session(&mut *tx)
            .await?
            .is_some_and(|a| a.premium.is_some())
    } else {
        false
    };
    let required = if matches!(kind, EffectKind::Call) {
        vec![
            CostKind::CarrierInbound,
            CostKind::Relay,
            CostKind::Stt,
            if premium {
                CostKind::PremiumTts
            } else {
                CostKind::Tts
            },
        ]
    } else {
        vec![CostKind::SmsPart]
    };
    let row = ManagedCommsCostEvidence {
        id: uuid::Uuid::new_v4().to_string(),
        terminal_evidence: None,
        effect_id: effect.into(),
        line_id: line.id.clone(),
        kind,
        components: required.iter().map(|k| unknown(*k)).collect(),
        required_components: required,
        absorbed_seconds: None,
        absorbed_sms_parts: 0,
        retry_count: 0,
        next_attempt_at: now,
        created_at: now,
        admitted: Some(r.clone()),
        tariffs: line.supplier_tariffs.clone(),
        policy,
        provider_quantity: None,
        synthesis_characters: None,
        billed_credits: None,
        capture_complete: false,
    };
    let mut document = evidence_document(&row)?;
    if let Some(claim) = &line.current_claim_id
        && let Some(c) = db
            .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
            .find_one(doc! {"_id":claim})
            .session(&mut *tx)
            .await?
    {
        document.insert("tenant_owner_id", c.user_id);
    }
    db.collection::<Document>(ManagedCommsCostEvidence::COLLECTION_NAME)
        .insert_one(document)
        .session(&mut *tx)
        .await?;
    Ok(())
}
fn unknown(kind: CostKind) -> CostComponent {
    CostComponent {
        kind,
        source: CostSource::Unknown,
        provider_record_id: None,
        amount: None,
        currency: String::new(),
        fx_policy_revision: String::new(),
        supplier_rounded_quantity: None,
    }
}
/// Supplier rounding is separate from the retail billable horizon.
pub(super) fn estimate(t: &SupplierTariff, quantity: i64) -> AppResult<(String, i64)> {
    if quantity < 0 || t.increment <= 0 || t.minimum < 0 {
        return Err(unavailable());
    }
    let q = if quantity == 0 {
        0
    } else {
        quantity
            .max(t.minimum)
            .checked_add(t.increment - 1)
            .ok_or_else(unavailable)?
            / t.increment
            * t.increment
    };
    let divisor = match t.unit.as_str() {
        "minute" | "minutes" => 60_i128,
        "second" | "seconds" | "character" | "characters" | "part" | "parts" => 1,
        _ => return Err(unavailable()),
    };
    let rate: Credits = t.amount.parse().map_err(|_| unavailable())?;
    if rate < Credits::ZERO {
        return Err(unavailable());
    }
    let pico = rate.checked_mul(q)?.pico();
    Ok((
        Credits::from_pico(pico.checked_add(divisor - 1).ok_or_else(unavailable)? / divisor)?
            .to_string(),
        q,
    ))
}
fn fx_revision(row: &ManagedCommsCostEvidence, currency: &str) -> String {
    row.policy
        .as_ref()
        .and_then(|p| p.fx.iter().find(|f| f.currency == currency))
        .map_or_else(String::new, |f| f.revision.clone())
}
fn actual(
    row: &ManagedCommsCostEvidence,
    kind: CostKind,
    id: &str,
    value: &str,
    cur: &str,
    q: Option<i64>,
) -> AppResult<CostComponent> {
    if !reference(id) || !currency(cur) {
        return Err(unavailable());
    }
    Ok(CostComponent {
        kind,
        source: CostSource::ProviderRecord,
        provider_record_id: Some(id.into()),
        amount: Some(amount(value)?.to_string()),
        currency: cur.into(),
        fx_policy_revision: fx_revision(row, cur),
        supplier_rounded_quantity: q,
    })
}
/// Fixed identities are checked again even though the request is authenticated.
/// A supplier's negative debit is normalized without converting its currency.
pub(super) fn twilio_record(row: &ManagedCommsCostEvidence, v: &Value) -> AppResult<CostComponent> {
    let r = row.admitted.as_ref().ok_or_else(unavailable)?;
    let call = matches!(row.kind, EffectKind::Call);
    let id = if call {
        r.provider_call_id.as_deref()
    } else {
        r.provider_message_id.as_deref()
    }
    .ok_or_else(unavailable)?;
    if (call
        && !matches!(
            v["status"].as_str(),
            Some("completed" | "failed" | "busy" | "no-answer" | "canceled")
        ))
        || (!call
            && !matches!(
                v["status"].as_str(),
                Some("delivered" | "undelivered" | "failed" | "received")
            ))
    {
        return Err(unavailable());
    }
    if v["sid"] != id || v["account_sid"] != r.account_id {
        return Err(unavailable());
    }
    actual(
        row,
        if call {
            CostKind::CarrierInbound
        } else {
            CostKind::SmsPart
        },
        id,
        v["price"].as_str().ok_or_else(unavailable)?,
        &v["price_unit"]
            .as_str()
            .ok_or_else(unavailable)?
            .to_ascii_uppercase(),
        if call {
            row.provider_quantity.and_then(|quantity| {
                row.tariffs
                    .iter()
                    .find(|t| t.kind == CostKind::CarrierInbound)
                    .and_then(|t| estimate(t, quantity).ok().map(|(_, q)| q))
            })
        } else {
            integer(&v["num_segments"])
        },
    )
}
fn integer(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str()?.parse().ok())
}
pub(super) fn telnyx_record(
    row: &ManagedCommsCostEvidence,
    s: &CostRecordSchema,
    v: &Value,
) -> AppResult<CostComponent> {
    let r = row.admitted.as_ref().ok_or_else(unavailable)?;
    let rows = v["data"].as_array().ok_or_else(unavailable)?;
    // Exactly one record for the exact admitted identity; duplicates and broad
    // ambiguous result sets cannot establish an actual component.
    let matching: Vec<_> = rows
        .iter()
        .filter(|v| {
            v["record_type"] == s.record_type
                && if s.kind == CostKind::SmsPart {
                    r.provider_message_id
                        .as_deref()
                        .is_some_and(|id| v["message_id"] == id)
                        && v["messaging_profile_id"] == r.application_id
                } else {
                    r.call_leg_id
                        .as_deref()
                        .is_some_and(|id| v["call_leg_id"] == id)
                        && v["connection_id"] == r.application_id
                }
        })
        .collect();
    if matching.len() != 1 {
        return Err(unavailable());
    }
    let v = matching[0];
    // Amount objects are supported only with a verified currency. Messaging
    // carrier fees are part of this component, never an absorbed-cost duplicate.
    let a = &v[&s.amount_field];
    let mut total = amount(
        a.as_str()
            .or_else(|| a["amount"].as_str())
            .ok_or_else(unavailable)?,
    )?;
    let cur = v[&s.currency_field]
        .as_str()
        .or_else(|| a["currency"].as_str())
        .ok_or_else(unavailable)?
        .to_ascii_uppercase();
    if s.kind == CostKind::SmsPart {
        let fee = &v["carrier_fee"];
        let fee_cur = fee["currency"].as_str().unwrap_or(&cur);
        if fee_cur != cur {
            return Err(unavailable());
        }
        total = total.checked_add(amount(
            fee.as_str()
                .or_else(|| fee["amount"].as_str())
                .ok_or_else(unavailable)?,
        )?)?;
    }
    actual(
        row,
        s.kind,
        v["id"].as_str().ok_or_else(unavailable)?,
        &total.to_string(),
        &cur,
        integer(&v["billed_quantity"]),
    )
}
fn profile(r: &AdmittedReference, call: bool) -> Profile {
    match (r.provider.as_str(), call) {
        ("twilio", true) => Profile::TwilioRelay,
        ("twilio", false) => Profile::TwilioSms,
        (_, true) => Profile::TelnyxRelay,
        (_, false) => Profile::TelnyxSms,
    }
}
async fn retained_read(
    state: &AppState,
    r: &AdmittedReference,
    op: resolver::CleanupOperation,
    call: bool,
) -> AppResult<Value> {
    let origin = super::super::profiles::origin_for_account(profile(r, call), &r.account_id)?;
    resolver::request_reference(state, r, op, Some(&origin)).await
}
async fn refresh_metadata(state: &AppState, row: &mut ManagedCommsCostEvidence) -> AppResult<()> {
    if matches!(row.kind, EffectKind::Call) {
        if let Some(s) = state
            .db
            .collection::<CallSession>(SESSIONS)
            .find_one(doc! {"_id":&row.effect_id})
            .await?
        {
            row.provider_quantity = s.provider_seconds.or(row.provider_quantity);
            row.absorbed_seconds = s.absorbed_seconds.or(row.absorbed_seconds);
        }
        if let Some(a) = state
            .db
            .collection::<ManagedCharacterAllowance>(ManagedCharacterAllowance::COLLECTION_NAME)
            .find_one(doc! {"session_id":&row.effect_id})
            .await?
        {
            row.synthesis_characters = Some(a.consumed);
            if a.premium_fallback {
                row.required_components = vec![
                    CostKind::CarrierInbound,
                    CostKind::Relay,
                    CostKind::Stt,
                    CostKind::Tts,
                ];
            }
        }
        if let Some(r) = state
            .db
            .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
            .find_one(doc! {"session_id":&row.effect_id})
            .await?
            && r.financially_finalized
        {
            let fallback = state
                .db
                .collection::<Document>(ManagedCallAdmission::COLLECTION_NAME)
                .find_one(doc! {"_id":&r.id,"premium_fallback":"proven_pre_dispatch"})
                .await?
                .is_some();
            let mut billed = r
                .base
                .rate
                .rate
                .checked_mul(row.provider_quantity.ok_or_else(unavailable)?.min(1800))?;
            if let Some(p) = r.premium.filter(|_| !fallback) {
                billed = billed.checked_add(
                    p.rate
                        .rate
                        .checked_mul(row.provider_quantity.ok_or_else(unavailable)?.min(1800))?,
                )?;
            }
            row.billed_credits = Some(billed);
        }
    } else if let Some(s) = state
        .db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .find_one(doc! {"_id":&row.effect_id})
        .await?
    {
        row.provider_quantity = Some(s.parts);
        row.absorbed_sms_parts = s
            .outcomes
            .iter()
            .filter(|p| matches!(p, SmsPartOutcome::Absorbed { .. }))
            .count() as i64;
        if s.financially_finalized {
            let mut billed = Credits::ZERO;
            for p in &s.outcomes {
                if let SmsPartOutcome::CustomerFunded {
                    billing_request_id, ..
                } = p
                {
                    billed =
                        billed.checked_add(billed_request(&state.db, billing_request_id).await?)?;
                }
            }
            row.billed_credits = Some(billed);
        }
    } else if let Some(s) = state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"_id":&row.effect_id,"event_kind":"sms_send"})
        .await?
    {
        if let Some(id) = &s.provider_message_id {
            row.admitted
                .as_mut()
                .ok_or_else(unavailable)?
                .provider_message_id = Some(id.clone());
        }
        row.provider_quantity = s.observed_parts.or(row.provider_quantity);
        row.absorbed_sms_parts = s
            .observed_parts
            .unwrap_or(0)
            .saturating_sub(s.reserved_parts.unwrap_or(0))
            .max(0);
        // Settled at zero without provider evidence: every reserved part's
        // supplier cost is unknown and absorbed by the platform.
        if super::reservations::waived(&state.db, &s.id).await? {
            row.terminal_evidence = Some(super::reservations::WAIVED.into());
            row.absorbed_sms_parts = s.reserved_parts.unwrap_or(0).max(0);
        }
        if s.billing_finalized {
            let mut billed = Credits::ZERO;
            for part in 0..s.reserved_parts.ok_or_else(unavailable)? {
                billed = billed.checked_add(
                    billed_request(
                        &state.db,
                        &super::super::sms_delivery::outbound_request_id(&s, part)?,
                    )
                    .await?,
                )?;
            }
            row.billed_credits = Some(billed);
        }
    }
    Ok(())
}
async fn billed_request(db: &Database, id: &str) -> AppResult<Credits> {
    use crate::models::usage_meter::{COLLECTION_NAME, UsageMeterRow};
    let r = db
        .collection::<UsageMeterRow>(COLLECTION_NAME)
        .find_one(doc! {"billing_request_id":id,"released":true})
        .await?
        .ok_or_else(unavailable)?;
    let f = r.funding.as_ref().ok_or_else(unavailable)?;
    Credits::from_pico(crate::services::billing::amounts::rate_pico(
        f.credits_per_unit_pico,
        f.credits_per_unit_micros,
    ))?
    .checked_mul(r.quantity.ok_or_else(unavailable)?)
    .map_err(Into::into)
}
pub async fn capture(state: &AppState, id: &str) -> AppResult<()> {
    let rows = state
        .db
        .collection::<ManagedCommsCostEvidence>(ManagedCommsCostEvidence::COLLECTION_NAME);
    let mut row = rows
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(unavailable)?;
    refresh_metadata(state, &mut row).await?;
    let r = row.admitted.clone().ok_or_else(unavailable)?;
    let call = matches!(row.kind, EffectKind::Call);
    let mut components = Vec::new();
    for kind in &row.required_components {
        let previous = row
            .components
            .iter()
            .find(|c| c.kind == *kind && c.source == CostSource::ProviderRecord)
            .cloned();
        if let Some(c) = previous {
            components.push(c);
            continue;
        }
        let mut found = None;
        if r.provider == "twilio"
            && (*kind == CostKind::CarrierInbound || *kind == CostKind::SmsPart)
        {
            let op = if call {
                resolver::CleanupOperation::ReadCall
            } else {
                resolver::CleanupOperation::ReadMessage
            };
            if let Ok(v) = retained_read(state, &r, op, call).await {
                found = twilio_record(&row, &v).ok();
            }
        } else if r.provider == "telnyx"
            && let Some(schema) = row
                .policy
                .as_ref()
                .and_then(|p| p.schemas.iter().find(|s| s.kind == *kind))
            && let Ok(v) = retained_read(
                state,
                &r,
                resolver::CleanupOperation::ReadCostRecord(resolver::CostRecordType::parse(
                    &schema.record_type,
                )?),
                call,
            )
            .await
        {
            found = telnyx_record(&row, schema, &v).ok();
        }
        if found.is_none()
            && let Some(t) = row
                .tariffs
                .iter()
                .find(|t| t.kind == *kind && t.effective_at <= row.created_at)
        {
            let q = if ["character", "characters"].contains(&t.unit.as_str()) {
                row.synthesis_characters
            } else {
                row.provider_quantity
            };
            if let Some(q) = q
                && let Ok((amount, q)) = estimate(t, q)
            {
                found = Some(CostComponent {
                    kind: *kind,
                    source: CostSource::TariffEstimate,
                    provider_record_id: None,
                    amount: Some(amount),
                    currency: t.currency.clone(),
                    fx_policy_revision: t.fx_policy_revision.clone(),
                    supplier_rounded_quantity: Some(q),
                });
            }
        }
        components.push(found.unwrap_or_else(|| unknown(*kind)));
    }
    row.components = components;
    row.retry_count = row.retry_count.saturating_add(1);
    row.capture_complete =
        row.billed_credits.is_some() && (quality(&row).1 || row.terminal_evidence.is_some());
    row.next_attempt_at = Utc::now()
        + Duration::seconds((30_i64.saturating_mul(1_i64 << row.retry_count.min(11))).min(86400));
    // capture() is also used before erasure. CAS prevents stale workers from
    // replacing newer provider evidence or a retained financial snapshot.
    let mut filter = doc! {"_id":id,"retry_count":row.retry_count-1};
    let mut document = evidence_document(&row)?;
    if let Some(old) = state
        .db
        .collection::<Document>(ManagedCommsCostEvidence::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await?
        && let Ok(owner) = old.get_str("tenant_owner_id")
    {
        document.insert("tenant_owner_id", owner);
        filter.insert("tenant_owner_id", owner);
    }
    let matched = state
        .db
        .collection::<Document>(ManagedCommsCostEvidence::COLLECTION_NAME)
        .replace_one(filter, document)
        .await?
        .matched_count;
    if matched != 1 {
        return Err(conflict("Cost capture was advanced by another worker"));
    }
    Ok(())
}
/// Exact report projections are replaced atomically with their source evidence.
/// FX multiplication/ceil stays in checked pico arithmetic; MongoDB only sums
/// Decimal128 credits, avoiding floating point or a second rounding policy.
pub(super) fn evidence_document(row: &ManagedCommsCostEvidence) -> AppResult<Document> {
    let mut actual = Credits::ZERO;
    let mut estimate = Credits::ZERO;
    let mut unknown = 0_i64;
    for kind in &row.required_components {
        let c = row.components.iter().find(|c| c.kind == *kind);
        let converted = c.and_then(|c| {
            c.amount.as_deref().and_then(|v| {
                convert(
                    v,
                    &c.currency,
                    &c.fx_policy_revision,
                    row.policy.as_ref().map_or(&[], |p| p.fx.as_slice()),
                )
                .ok()
            })
        });
        match (c, converted) {
            (Some(c), Some(v)) if c.source == CostSource::ProviderRecord => {
                actual = actual.checked_add(v)?
            }
            (Some(c), Some(v)) if c.source == CostSource::TariffEstimate => {
                estimate = estimate.checked_add(v)?
            }
            _ => unknown += 1,
        }
    }
    let mut document = bson::to_document(row).map_err(|_| unavailable())?;
    document.insert("report_totals",doc!{"actual":actual.decimal(),"estimate":estimate.decimal(),"unknown":unknown,"all_actual":quality(row).1});
    Ok(document)
}
/// Coverage is distinct from quality; estimates can cover all components.
pub(super) fn quality(r: &ManagedCommsCostEvidence) -> (bool, bool) {
    let covered = r.required_components.iter().all(|k| {
        r.components
            .iter()
            .any(|c| c.kind == *k && c.amount.is_some() && c.source != CostSource::Unknown)
    });
    let actual = covered
        && r.required_components.iter().all(|k| {
            r.components.iter().any(|c| {
                c.kind == *k && c.amount.is_some() && c.source == CostSource::ProviderRecord
            })
        });
    (covered, actual)
}
/// Freeze metadata before tenant rows vanish. Network availability never blocks
/// erasure: supplier evidence can be fetched later from the retained reference.
pub async fn retain_for_erasure(state: &AppState, owner: &str) -> AppResult<bool> {
    let db = &state.db;
    let rows: Vec<ManagedCommsCostEvidence> = db
        .collection::<ManagedCommsCostEvidence>(ManagedCommsCostEvidence::COLLECTION_NAME)
        .find(doc! {"tenant_owner_id":owner})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() {
        return Ok(true);
    }
    for mut row in rows {
        refresh_metadata(state, &mut row).await?;
        let previous_retry = row.retry_count;
        row.retry_count += 1;
        if row.billed_credits.is_none() {
            return Err(conflict(
                "Retain finalized financial metadata before erasure",
            ));
        }
        let document = evidence_document(&row)?;
        if db
            .collection::<Document>(ManagedCommsCostEvidence::COLLECTION_NAME)
            .replace_one(
                doc! {"_id":&row.id,"retry_count":previous_retry,"tenant_owner_id":owner},
                document,
            )
            .await?
            .matched_count
            != 1
        {
            return Ok(false);
        }
    }
    Ok(false)
}
fn boundary(anchor: DateTime<Utc>, index: u32) -> AppResult<DateTime<Utc>> {
    anchor
        .checked_add_months(Months::new(index))
        .ok_or_else(unavailable)
}
pub(super) fn cycle(
    anchor: DateTime<Utc>,
    index: u32,
) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
    Ok((boundary(anchor, index)?, boundary(anchor, index + 1)?))
}
async fn inventory_line(state: &AppState, l: &ManagedPhoneLine) -> AppResult<()> {
    let Some(p) = state
        .db
        .collection::<ManagedCostPolicy>(ManagedCostPolicy::COLLECTION_NAME)
        .find_one(doc! {"line_id":&l.id})
        .await?
    else {
        return Ok(());
    };
    let Some(t) = &p.inventory else {
        return Ok(());
    };
    let rows = state
        .db
        .collection::<ManagedInventoryCost>(ManagedInventoryCost::COLLECTION_NAME);
    // Continue from durable progress, independent of state. A released line
    // still records its final supplier cycle, never an invented partial cycle.
    let last = rows
        .find_one(doc! {"line_id":&l.id})
        .sort(doc! {"cycle_from":-1})
        .await?;
    let first = last.as_ref().map_or(0, |r| {
        ((r.cycle_from.year() - t.cycle_anchor.year()) * 12 + r.cycle_from.month() as i32
            - t.cycle_anchor.month() as i32
            + 1)
        .max(0) as u32
    });
    for n in first..first.saturating_add(12) {
        let (from, until) = cycle(t.cycle_anchor, n)?;
        if from > Utc::now() || l.supplier_released_at.is_some_and(|at| from >= at) {
            break;
        }
        let row = ManagedInventoryCost {
            id: uuid::Uuid::new_v4().to_string(),
            line_id: l.id.clone(),
            cycle_key: from.to_rfc3339(),
            cycle_from: from,
            cycle_until: until,
            amount: amount(&t.amount)?.to_string(),
            currency: t.currency.clone(),
            source: CostSource::TariffEstimate,
            provider_record_id: None,
            fx_policy_revision: t.fx_policy_revision.clone(),
            fx: p.fx.clone(),
            created_at: Utc::now(),
        };
        rows.update_one(
            doc! {"line_id":&l.id,"cycle_key":&row.cycle_key},
            doc! {"$setOnInsert":bson::to_document(&row).map_err(|_|unavailable())?},
        )
        .upsert(true)
        .await?;
    }
    Ok(())
}
pub async fn sweep(state: &AppState) -> AppResult<()> {
    let rows = state
        .db
        .collection::<ManagedCommsCostEvidence>(ManagedCommsCostEvidence::COLLECTION_NAME);
    let due: Vec<_> = rows
        .find(doc! {"capture_complete":false,"next_attempt_at":{"$lte":bson::DateTime::now()}})
        .sort(doc! {"next_attempt_at":1,"_id":1})
        .limit(20)
        .await?
        .try_collect()
        .await?;
    for r in due {
        let Some(_claim) = state.shutdown.background() else {
            return Ok(());
        };
        // Move the due time before I/O, so a poisoned row cannot starve later
        // work. A crash becomes retryable after this bounded lease.
        if rows.update_one(doc!{"_id":&r.id,"retry_count":r.retry_count,"next_attempt_at":bson::DateTime::from_chrono(r.next_attempt_at)},doc!{"$set":{"next_attempt_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(5))}}).await?.matched_count!=1{continue;}
        if capture(state, &r.id).await.is_err() {
            tracing::warn!(cost_evidence_id=%r.id,"Managed supplier cost evidence pending");
        }
    }
    let lines: Vec<ManagedPhoneLine> = state
        .db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find(doc! {})
        .sort(doc! {"cost_inventory_checked_at":1,"_id":1})
        .limit(20)
        .await?
        .try_collect()
        .await?;
    for l in lines {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        state
            .db
            .collection::<Document>(ManagedPhoneLine::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&l.id},
                doc! {"$set":{"cost_inventory_checked_at":bson::DateTime::now()}},
            )
            .await?;
        if inventory_line(state, &l).await.is_err() {
            tracing::warn!(line_id=%l.id,"Managed supplier inventory cost pending");
        }
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportQuery {
    pub from: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub after: Option<String>,
    pub line_id: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct CostReport {
    pub lines: Vec<LineReport>,
    pub next_after: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct LineReport {
    pub line_id: String,
    pub days: Vec<DayReport>,
    pub budgets: Vec<BudgetReport>,
    pub inventory_policy_missing: bool,
}
#[derive(Debug, Serialize)]
pub struct BudgetReport {
    pub kind: String,
    pub limit: String,
    pub recognized: String,
    pub unresolved_holds: String,
}
#[derive(Debug, Default, Serialize)]
pub struct DayReport {
    pub day: String,
    pub billed_credits: Option<String>,
    pub provider_actual_cost_credits: String,
    pub estimated_cost_credits: String,
    pub inventory_cost_credits: Option<String>,
    pub estimated_margin_credits: Option<String>,
    pub actual_margin_credits: Option<String>,
    pub unknown_components: i64,
    pub absorbed_seconds: i64,
    pub absorbed_sms_parts: i64,
    pub effect_count: i64,
}
#[derive(Debug, Serialize)]
pub struct EffectReport {
    pub id: String,
    /// Call session, inbound SMS liability or caller-confirmation operation ID.
    pub effect_id: String,
    pub kind: EffectKind,
    pub created_at: String,
    pub component_coverage: bool,
    pub evidence_quality: String,
    pub original_costs: Vec<ComponentReport>,
}
#[derive(Debug, Serialize)]
pub struct ComponentReport {
    pub kind: CostKind,
    pub source: CostSource,
    pub amount: Option<String>,
    pub currency: String,
    pub fx_policy_revision: String,
    pub supplier_rounded_quantity: Option<i64>,
}
pub(super) fn convert(
    value: &str,
    currency: &str,
    revision: &str,
    fx: &[CostFxRate],
) -> AppResult<Credits> {
    let f = fx
        .iter()
        .find(|f| f.currency == currency && f.revision == revision)
        .ok_or_else(unavailable)?;
    let pico = amount(value)?
        .pico()
        .checked_mul(positive(&f.credits_per_unit)?.pico())
        .ok_or_else(unavailable)?;
    Ok(Credits::from_pico(
        pico.checked_add(SCALE - 1).ok_or_else(unavailable)? / SCALE,
    )?)
}
async fn budget_report(db: &Database, id: &str, kind: &str) -> AppResult<BudgetReport> {
    let b = db
        .collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(unavailable)?;
    let counts = match b.window {
        BudgetWindow::RollingThirtyDays => {
            doc! {"$gt":["$recognized_at",bson::DateTime::from_chrono(Utc::now()-Duration::days(30))]}
        }
        BudgetWindow::CalendarMonth { year, month } => {
            doc! {"$and":[{"$eq":[{"$year":"$admitted_at"},year]},{"$eq":[{"$month":"$admitted_at"},month as i32]}]}
        }
    };
    let totals=db.collection::<Document>(ManagedBudgetEntry::COLLECTION_NAME).aggregate(vec![
        doc!{"$match":{"budget_id":id}},
        doc!{"$group":{"_id":bson::Bson::Null,
            "recognized":{"$sum":{"$cond":[counts,{"$ifNull":["$recognized",0]},0]}},
            "holds":{"$sum":{"$cond":[{"$eq":[{"$ifNull":["$recognized",bson::Bson::Null]},bson::Bson::Null]},"$held",0]}}}},
    ]).max_time(std::time::Duration::from_secs(3)).await?.try_next().await?;
    let recognized = totals
        .as_ref()
        .map_or(Ok(Credits::ZERO), |d| total_credits(d, "recognized"))?;
    let holds = totals
        .as_ref()
        .map_or(Ok(Credits::ZERO), |d| total_credits(d, "holds"))?;
    Ok(BudgetReport {
        kind: kind.into(),
        limit: b.limit.to_string(),
        recognized: recognized.to_string(),
        unresolved_holds: holds.to_string(),
    })
}
fn total_credits(d: &Document, key: &str) -> AppResult<Credits> {
    Credits::from_bson(d.get(key).cloned().ok_or_else(unavailable)?, SCALE)
        .map_err(|_| unavailable())
}
fn total_i64(d: &Document, key: &str) -> AppResult<i64> {
    d.get_i64(key)
        .or_else(|_| d.get_i32(key).map(i64::from))
        .map_err(|_| unavailable())
}
fn validate_window(from: DateTime<Utc>, until: DateTime<Utc>) -> AppResult<()> {
    if until <= from
        || until - from > Duration::days(31)
        || from.time() != chrono::NaiveTime::MIN
        || until.time() != chrono::NaiveTime::MIN
    {
        return Err(AppError::ValidationError(
            "Use UTC midnight boundaries and at most 31 days".into(),
        ));
    }
    Ok(())
}
fn effect_filter(line: &str, from: DateTime<Utc>, until: DateTime<Utc>) -> Document {
    doc! {"line_id":line,"created_at":{"$gte":bson::DateTime::from_chrono(from),"$lt":bson::DateTime::from_chrono(until)}}
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectQuery {
    pub line_id: String,
    pub from: DateTime<Utc>,
    pub until: DateTime<Utc>,
    /// Opaque row UUID from next_after; scoped again by line and window.
    pub after: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct EffectPage {
    pub effects: Vec<EffectReport>,
    pub next_after: Option<String>,
}
pub async fn effect_report(db: &Database, q: EffectQuery) -> AppResult<EffectPage> {
    validate_window(q.from, q.until)?;
    let mut filter = effect_filter(&q.line_id, q.from, q.until);
    if let Some(after) = q.after {
        filter.insert("_id", doc! {"$gt":after});
    }
    let mut rows: Vec<ManagedCommsCostEvidence> = db
        .collection::<ManagedCommsCostEvidence>(ManagedCommsCostEvidence::COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(101)
        .max_time(std::time::Duration::from_secs(3))
        .await?
        .try_collect()
        .await?;
    let next_after = if rows.len() > 100 {
        rows.pop();
        rows.last().map(|r| r.id.clone())
    } else {
        None
    };
    let effects = rows
        .into_iter()
        .map(|e| {
            let (coverage, actual) = quality(&e);
            EffectReport {
                id: e.id,
                effect_id: e.effect_id,
                kind: e.kind,
                created_at: e.created_at.to_rfc3339(),
                component_coverage: coverage,
                evidence_quality: if actual { "actual" } else { "estimated" }.into(),
                original_costs: e
                    .required_components
                    .iter()
                    .map(|kind| {
                        let c = e
                            .components
                            .iter()
                            .find(|c| c.kind == *kind)
                            .cloned()
                            .unwrap_or_else(|| unknown(*kind));
                        ComponentReport {
                            kind: c.kind,
                            source: c.source,
                            amount: c.amount,
                            currency: c.currency,
                            fx_policy_revision: c.fx_policy_revision,
                            supplier_rounded_quantity: c.supplier_rounded_quantity,
                        }
                    })
                    .collect(),
            }
        })
        .collect();
    Ok(EffectPage {
        effects,
        next_after,
    })
}
pub async fn report(db: &Database, q: ReportQuery) -> AppResult<CostReport> {
    validate_window(q.from, q.until)?;
    let mut filter = doc! {};
    if let Some(id) = &q.line_id {
        filter.insert("_id", id);
    }
    if let Some(id) = &q.after {
        filter.insert("_id", doc! {"$gt":id});
    }
    let mut lines: Vec<ManagedPhoneLine> = db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(11)
        .await?
        .try_collect()
        .await?;
    let next_after = if lines.len() > 10 {
        lines.pop();
        lines.last().map(|l| l.id.clone())
    } else {
        None
    };
    let mut result = vec![];
    for l in lines {
        let totals:Vec<Document>=db.collection::<Document>(ManagedCommsCostEvidence::COLLECTION_NAME).aggregate(vec![
            doc!{"$match":effect_filter(&l.id,q.from,q.until)},
            doc!{"$group":{"_id":{"$dateToString":{"format":"%Y-%m-%d","date":"$created_at","timezone":"UTC"}},
                "billed":{"$sum":{"$ifNull":["$billed_credits",0]}},
                "missing_billed":{"$sum":{"$cond":[{"$eq":[{"$ifNull":["$billed_credits",bson::Bson::Null]},bson::Bson::Null]},1_i64,0_i64]}},
                "actual":{"$sum":{"$ifNull":["$report_totals.actual",0]}},
                "estimate":{"$sum":{"$ifNull":["$report_totals.estimate",0]}},
                "unknown":{"$sum":{"$ifNull":["$report_totals.unknown",{"$size":"$required_components"}]}},
                "all_actual":{"$min":{"$ifNull":["$report_totals.all_actual",false]}},
                "absorbed_seconds":{"$sum":{"$ifNull":["$absorbed_seconds",0]}},
                "absorbed_sms_parts":{"$sum":"$absorbed_sms_parts"},
                "effects":{"$sum":1_i64}}},
            doc!{"$sort":{"_id":1}},
        ]).max_time(std::time::Duration::from_secs(3)).await?.try_collect().await?;
        let inventory:Vec<ManagedInventoryCost>=db.collection::<ManagedInventoryCost>(ManagedInventoryCost::COLLECTION_NAME).find(doc!{"line_id":&l.id,"cycle_from":{"$lt":bson::DateTime::from_chrono(q.until)},"cycle_until":{"$gt":bson::DateTime::from_chrono(q.from)}}).limit(40).await?.try_collect().await?;
        // Billing services can price multiple lines. The retained claim, rather
        // than the shared rate, establishes which line earned this rental.
        let period_documents: Vec<Document> = db
            .collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
            .aggregate(vec![
                doc! {"$match":{"rate.service_id":&l.rental_billing_service_id,"covered_from":{"$lt":bson::DateTime::from_chrono(q.until)},"covered_until":{"$gt":bson::DateTime::from_chrono(q.from)}}},
                doc! {"$lookup":{"from":ManagedLineClaim::COLLECTION_NAME,"localField":"claim_id","foreignField":"_id","as":"claim"}},
                doc! {"$match":{"claim.line_id":&l.id}},
                doc! {"$unset":"claim"},
                doc! {"$limit":101},
            ])
            .max_time(std::time::Duration::from_secs(3))
            .await?
            .try_collect()
            .await?;
        let periods: Vec<ManagedLinePeriod> = period_documents
            .into_iter()
            .map(|d| bson::from_document(d).map_err(|_| unavailable()))
            .collect::<AppResult<_>>()?;
        if inventory.len() >= 40 || periods.len() > 100 {
            return Err(AppError::AdminUsageQueryTimeout);
        }
        let policy = db
            .collection::<ManagedCostPolicy>(ManagedCostPolicy::COLLECTION_NAME)
            .find_one(doc! {"line_id":&l.id})
            .await?;
        let mut days = vec![];
        let mut day = q.from;
        while day < q.until {
            let end = day + Duration::days(1);
            let mut billed = Credits::ZERO;
            let mut actual_cost = Credits::ZERO;
            let mut estimate_cost = Credits::ZERO;
            let mut inventory_cost = Credits::ZERO;
            let mut complete = true;
            let mut all_actual = true;
            let mut d = DayReport {
                day: day.date_naive().to_string(),
                ..Default::default()
            };
            if let Some(t) = totals
                .iter()
                .find(|t| t.get_str("_id").ok() == Some(d.day.as_str()))
            {
                billed = total_credits(t, "billed")?;
                actual_cost = total_credits(t, "actual")?;
                estimate_cost = total_credits(t, "estimate")?;
                d.unknown_components = total_i64(t, "unknown")?;
                complete = total_i64(t, "missing_billed")? == 0 && d.unknown_components == 0;
                all_actual = t.get_bool("all_actual").map_err(|_| unavailable())?;
                d.absorbed_seconds = total_i64(t, "absorbed_seconds")?;
                d.absorbed_sms_parts = total_i64(t, "absorbed_sms_parts")?;
                d.effect_count = total_i64(t, "effects")?;
            }
            for p in &periods {
                let seconds = (p.covered_until.min(end) - p.covered_from.max(day))
                    .num_seconds()
                    .max(0);
                if seconds > 0 {
                    if p.funded {
                        billed = billed.checked_add(Credits::from_pico(
                            p.rate.rate.checked_mul(seconds)?.pico() / 86400,
                        )?)?;
                    } else {
                        complete = false;
                    }
                }
            }
            let mut inventory_known = policy.as_ref().is_some_and(|p| p.inventory.is_some());
            let mut cycle_seen = false;
            for c in &inventory {
                let seconds = (c.cycle_until.min(end) - c.cycle_from.max(day))
                    .num_seconds()
                    .max(0);
                if seconds == 0 {
                    continue;
                }
                cycle_seen = true;
                match convert(&c.amount, &c.currency, &c.fx_policy_revision, &c.fx) {
                    Ok(v) => {
                        let duration = (c.cycle_until - c.cycle_from).num_seconds();
                        inventory_cost = inventory_cost.checked_add(Credits::from_pico(
                            v.pico()
                                .checked_mul(i128::from(seconds))
                                .ok_or_else(unavailable)?
                                / i128::from(duration),
                        )?)?;
                    }
                    Err(_) => inventory_known = false,
                }
                all_actual &= c.source == CostSource::ProviderRecord;
            }
            // A missing inventory cycle cannot appear as free inventory.
            inventory_known &= cycle_seen
                || day < l.created_at
                || l.supplier_released_at.is_some_and(|at| day >= at);
            d.billed_credits = if complete {
                Some(billed.to_string())
            } else {
                None
            };
            d.provider_actual_cost_credits = actual_cost.to_string();
            d.estimated_cost_credits = estimate_cost.to_string();
            d.inventory_cost_credits = inventory_known.then(|| inventory_cost.to_string());
            if complete && inventory_known {
                let margin = billed
                    .checked_sub(actual_cost)?
                    .checked_sub(estimate_cost)?
                    .checked_sub(inventory_cost)?;
                d.estimated_margin_credits = Some(margin.to_string());
                if all_actual {
                    d.actual_margin_credits = Some(margin.to_string());
                }
            }
            days.push(d);
            day = end;
        }
        let mut budgets = vec![];
        let account = db
            .collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME)
            .find_one(doc! {"budget_key":format!("managed-underwriting:{}",l.trusted_account_id)})
            .await?;
        if let Some(b) = account {
            budgets.push(budget_report(db, &b.id, "underwriting").await?);
        }
        if let Some(c) = &l.current_claim_id
            && let Some(c) = db
                .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
                .find_one(doc! {"_id":c})
                .await?
            && let Some(b) = db
                .collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME)
                .find_one(doc! {"budget_key":format!("managed-spend:{}",c.user_id)})
                .await?
        {
            budgets.push(budget_report(db, &b.id, "monthly_spend").await?);
        }
        result.push(LineReport {
            line_id: l.id,
            days,
            budgets,
            inventory_policy_missing: policy.is_none_or(|p| p.inventory.is_none()),
        });
    }
    Ok(CostReport {
        lines: result,
        next_after,
    })
}
#[derive(Debug, Serialize)]
pub struct IntegritySummary {
    pub report_path: &'static str,
    pub pending_cost_effects: u64,
    pub unknown_inventory_lines: u64,
}
pub async fn integrity(db: &Database) -> AppResult<IntegritySummary> {
    let pending = db
        .collection::<ManagedCommsCostEvidence>(ManagedCommsCostEvidence::COLLECTION_NAME)
        .count_documents(doc! {"capture_complete":false})
        .max_time(std::time::Duration::from_secs(3))
        .await?;
    // Bounded administrative diagnostic; counts inventory policies absent from
    // the current line roster without touching provider services.
    let mut cursor=db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).aggregate(vec![doc!{"$lookup":{"from":ManagedCostPolicy::COLLECTION_NAME,"localField":"_id","foreignField":"line_id","as":"policy"}},doc!{"$match":{"policy.inventory":{"$in":[bson::Bson::Null]}}},doc!{"$count":"count"}]).max_time(std::time::Duration::from_secs(3)).await?;
    let missing = cursor
        .try_next()
        .await?
        .and_then(|d| {
            d.get_i64("count")
                .ok()
                .or_else(|| d.get_i32("count").ok().map(i64::from))
        })
        .unwrap_or(0);
    Ok(IntegritySummary {
        report_path: "/api/v1/admin/managed-lines/report",
        pending_cost_effects: pending,
        unknown_inventory_lines: missing as u64,
    })
}
