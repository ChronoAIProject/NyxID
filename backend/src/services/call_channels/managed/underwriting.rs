//! Human acceptance of a bounded platform loss budget, independent of retail funds.
use super::{conflict, custody, eligibility, unavailable};
use crate::{
    AppState,
    errors::AppResult,
    models::{
        channel_adapter::{ACCOUNTS, ProviderAccount},
        credits::Credits,
        managed_comms::*,
    },
    services::{
        audit_service::{self, AuditActor},
        transaction_service,
    },
};
use chrono::Utc;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CeilingEvidence {
    WrittenConfirmation {
        reference: String,
    },
    PaidInboundTest {
        reference: String,
        hangup_cause: String,
        paid_tier: bool,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnderwritingInput {
    pub revision: i64,
    pub enabled: bool,
    pub acceptance_text: String,
    pub acceptance_version: String,
    pub paid_tier_evidence_reference: String,
    pub max_leg_seconds: i64,
    pub ceiling_evidence: CeilingEvidence,
    pub timestamp_fixture_reference: Option<String>,
    pub detail_record_options_reference: String,
    pub detail_record_fixture_reference: String,
    pub detail_record_fields: Vec<String>,
    pub supplier_tariff_revision: i64,
    pub fx_policy_revision: String,
    pub character_tariff_reference: String,
    pub components: Vec<ExposureComponentInput>,
    pub rolling_budget: String,
}
/// API input: an exact decimal credit string, parsed like every other Credits
/// input. Storage keeps the Decimal128 `ExposureComponent`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExposureComponentInput {
    pub kind: CostKind,
    pub credits_per_second: Credits,
    pub increment_seconds: i64,
    pub minimum_seconds: i64,
}
fn reference(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control)
}
fn policy(actor: &AuditActor, i: UnderwritingInput) -> AppResult<ManagedUnderwriting> {
    let ceiling = match i.ceiling_evidence {
        CeilingEvidence::WrittenConfirmation { reference } => reference,
        CeilingEvidence::PaidInboundTest {
            reference,
            hangup_cause,
            paid_tier,
        } if paid_tier && hangup_cause == "time_limit" => reference,
        _ => {
            return Err(conflict(
                "A paid inbound time_limit test or written provider ceiling is required",
            ));
        }
    };
    let p = ManagedUnderwriting {
        enabled: i.enabled,
        accepted_by: actor.user_id.clone(),
        accepted_at: Utc::now(),
        acceptance_text: i.acceptance_text,
        acceptance_version: i.acceptance_version,
        paid_tier_evidence_reference: i.paid_tier_evidence_reference,
        max_leg_seconds: i.max_leg_seconds,
        max_leg_evidence_reference: ceiling,
        timestamp_fixture_reference: i.timestamp_fixture_reference,
        detail_record_options_reference: Some(i.detail_record_options_reference),
        detail_record_fixture_reference: Some(i.detail_record_fixture_reference),
        detail_record_fields: i.detail_record_fields,
        supplier_tariff_revision: i.supplier_tariff_revision,
        fx_policy_revision: i.fx_policy_revision,
        character_tariff_reference: i.character_tariff_reference,
        components: i
            .components
            .into_iter()
            .map(|c| ExposureComponent {
                kind: c.kind,
                credits_per_second: c.credits_per_second,
                increment_seconds: c.increment_seconds,
                minimum_seconds: c.minimum_seconds,
            })
            .collect(),
        rolling_budget: i.rolling_budget.parse().map_err(|_| unavailable())?,
        revision: i.revision.checked_add(1).ok_or_else(unavailable)?,
    };
    validate(&p)?;
    Ok(p)
}
pub(in crate::services::call_channels) fn detail_enabled(p: &ManagedUnderwriting) -> bool {
    p.detail_record_options_reference
        .as_ref()
        .is_some_and(|s| reference(s))
        && p.detail_record_fixture_reference
            .as_ref()
            .is_some_and(|s| reference(s))
        && ["call_leg_id", "connection_id", "answered_at", "finished_at"]
            .iter()
            .all(|f| p.detail_record_fields.iter().any(|v| v == f))
}
pub fn validate(p: &ManagedUnderwriting) -> AppResult<()> {
    // A disabled policy may retain evidence for old admissions, but cannot admit.
    let mut enabled = p.clone();
    enabled.enabled = true;
    custody::worst_case_exposure(&enabled)?;
    if ![
        &p.acceptance_text,
        &p.acceptance_version,
        &p.paid_tier_evidence_reference,
        &p.max_leg_evidence_reference,
        &p.fx_policy_revision,
        &p.character_tariff_reference,
    ]
    .iter()
    .all(|s| reference(s))
        || p.timestamp_fixture_reference
            .as_ref()
            .is_some_and(|s| !reference(s))
        || !p
            .detail_record_options_reference
            .as_ref()
            .is_some_and(|s| reference(s))
        || !p
            .detail_record_fixture_reference
            .as_ref()
            .is_some_and(|s| reference(s))
        || ["call_leg_id", "connection_id", "answered_at", "finished_at"]
            .iter()
            .any(|f| !p.detail_record_fields.iter().any(|v| v == f))
    {
        return Err(conflict(
            "Underwriting requires complete tariff, FX and verified provider fixtures",
        ));
    }
    Ok(())
}
pub fn cost(p: &ManagedUnderwriting, seconds: i64) -> AppResult<Credits> {
    if seconds < 0 {
        return Err(unavailable());
    }
    if seconds == 0 {
        return Ok(Credits::ZERO);
    }
    let mut total = Credits::ZERO;
    for c in &p.components {
        if c.increment_seconds <= 0 {
            return Err(unavailable());
        }
        let n = seconds.max(c.minimum_seconds);
        let rounded = n
            .checked_add(c.increment_seconds - 1)
            .and_then(|n| n.checked_div(c.increment_seconds))
            .and_then(|n| n.checked_mul(c.increment_seconds))
            .ok_or_else(unavailable)?;
        total = total.checked_add(c.credits_per_second.checked_mul(rounded)?)?;
    }
    Ok(total)
}
pub async fn budget(
    db: &Database,
    tx: &mut ClientSession,
    account: &str,
    p: &ManagedUnderwriting,
) -> AppResult<ManagedBudget> {
    let rows = db.collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME);
    let key = format!("managed-underwriting:{account}");
    if let Some(old) = rows
        .find_one(doc! {"budget_key":&key})
        .session(&mut *tx)
        .await?
    {
        if old.limit != p.rolling_budget || old.revision != p.revision {
            return Err(conflict("Underwriting budget revision changed"));
        }
        return Ok(old);
    }
    let row = ManagedBudget {
        id: uuid::Uuid::new_v4().to_string(),
        budget_key: key,
        window: BudgetWindow::RollingThirtyDays,
        limit: p.rolling_budget,
        revision: p.revision,
        fence: 0,
    };
    rows.insert_one(&row).session(&mut *tx).await?;
    Ok(row)
}
pub async fn configure(
    state: &AppState,
    actor: &AuditActor,
    account: &str,
    input: UnderwritingInput,
) -> AppResult<()> {
    super::require_ready(&state.db).await?;
    eligibility::require_admin(&state.db, actor).await?;
    let revision = input.revision;
    let p = policy(actor, input)?;
    let db = transaction_service::durable_database(&state.db);
    let context = (
        &db,
        actor,
        account,
        revision,
        &p,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    transaction_service::run_db_only(&db,&context,|tx,c|Box::pin(async move {
        let (db,actor,account,revision,p,key)=*c;
        eligibility::admin_fence(db,tx,actor).await?;
        let accounts=db.collection::<ProviderAccount>(ACCOUNTS);
        let a=accounts.find_one(doc!{"_id":account,"provider":"telnyx"}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if a.underwriting.as_ref().map(|p|p.revision).unwrap_or(0)!=revision { return Err(conflict("Underwriting revision changed")); }
        let mut cursor=db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME).find(custody::capacity_filter(account)).session(&mut *tx).await?;
        let mut channels=0_i64;
        while let Some(l)=cursor.next(&mut *tx).await.transpose()? { channels=channels.checked_add(l.provider_channel_limit.filter(|n|*n>0).ok_or_else(unavailable)?).ok_or_else(unavailable)?; }
        let mut enabled=p.clone();enabled.enabled=true;
        if custody::worst_case_exposure(&enabled)?.checked_mul(channels)? > p.rolling_budget { return Err(conflict("Registered channel limits exceed the underwriting budget")); }
        if accounts.update_one(doc!{"_id":account,"revision":a.revision,"fence":custody::account_fence_value(a.fence)},doc!{"$set":{"underwriting":bson::to_bson(p).map_err(|_|unavailable())?,"updated_at":bson::DateTime::now()},"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1 { return Err(unavailable()); }
        db.collection::<bson::Document>(ManagedBudget::COLLECTION_NAME).update_one(doc!{"budget_key":format!("managed-underwriting:{account}")},doc!{"$set":{"limit":p.rolling_budget.decimal(),"revision":p.revision},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_underwriting_configured",serde_json::json!({"trusted_account_id":account,"revision":p.revision,"enabled":p.enabled})).await?;
        Ok(())
    })).await
}
