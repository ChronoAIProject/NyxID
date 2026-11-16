//! Managed line inventory, authority and metadata-only financial recovery records.
use super::{credits::Credits, service_billing::BillingMetric};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedCommsRole {
    Credential,
    Voice,
    Sms,
    Rental,
    Premium,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineState {
    Available,
    Claimed,
    Suspended,
    Releasing,
    Cooldown,
    PendingReassignment,
    Retired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Suspension {
    Admin,
    Unpaid,
    EligibilityRevoked,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceTier {
    Standard,
    Premium,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandardVoiceOption {
    pub voice: String,
    pub language: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermsVersions {
    pub managed_line: String,
    pub elevenlabs_eua: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub application_id: String,
    pub billing_service_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegulatoryEvidence {
    pub requirement_reference: String,
    pub registration_status: String,
    pub sms_sender_registration: Option<String>,
    pub verified_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub verified_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplierTariff {
    pub kind: CostKind,
    pub amount: String,
    pub currency: String,
    pub unit: String,
    pub increment: i64,
    pub minimum: i64,
    pub source: String,
    pub revision: i64,
    pub fx_policy_revision: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub effective_at: DateTime<Utc>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostKind {
    CarrierInbound,
    Relay,
    Stt,
    Tts,
    PremiumTts,
    SmsPart,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    ProviderRecord,
    TariffEstimate,
    Unknown,
}

/// No caller number, message body, transcript, secret, or mutable tenant authority.
#[derive(Clone, Serialize, Deserialize)]
pub struct AdmittedReference {
    pub trusted_account_id: String,
    pub credential_service_id: String,
    pub provider: String,
    pub account_id: String,
    pub line_id: String,
    pub number_resource_id: String,
    pub application_id: String,
    pub provider_call_id: Option<String>,
    pub provider_message_id: Option<String>,
    pub call_leg_id: Option<String>,
    pub call_session_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateSnapshot {
    pub service_id: String,
    pub service_slug: String,
    pub metric: BillingMetric,
    #[serde(with = "super::credits::whole")]
    pub rate: Credits,
    pub lago_metric_code: String,
    pub revision: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedPhoneLine {
    #[serde(rename = "_id")]
    pub id: String,
    pub provider: String,
    pub trusted_account_id: String,
    pub phone_number: String,
    pub country: String,
    pub number_resource_id: String,
    pub voice: Option<Capability>,
    #[serde(default)]
    pub standard_voice_options: Vec<StandardVoiceOption>,
    pub sms: Option<Capability>,
    pub rental_billing_service_id: String,
    pub regulatory: RegulatoryEvidence,
    pub supplier_tariffs: Vec<SupplierTariff>,
    pub provider_channel_limit: Option<i64>,
    pub call_slot_limit: i64,
    pub calls_per_minute: i64,
    pub sms_per_minute: i64,
    pub state: LineState,
    pub suspension: Option<Suspension>,
    pub current_claim_id: Option<String>,
    pub next_claim_generation: i64,
    pub revision: i64,
    pub fence: i64,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub available_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub supplier_released_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
impl ManagedPhoneLine {
    pub const COLLECTION_NAME: &'static str = "managed_phone_lines";
}
impl std::fmt::Debug for ManagedPhoneLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedPhoneLine")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedLineClaim {
    #[serde(rename = "_id")]
    pub id: String,
    pub line_id: String,
    pub user_id: String,
    pub managed_claim_generation: i64,
    pub eligibility_revision: i64,
    pub accepted_terms: TermsVersions,
    pub agent_id: String,
    pub voice_tier: VoiceTier,
    #[serde(default)]
    pub standard_voice: Option<StandardVoiceOption>,
    pub sms_enabled: bool,
    pub caller_confirmations: bool,
    pub auto_renew: bool,
    pub accepted_rental_rate: RateSnapshot,
    pub state: LineState,
    pub suspension: Option<Suspension>,
    pub voice_binding_id: Option<String>,
    pub sms_binding_id: Option<String>,
    pub fence: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub covered_from: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub release_requested_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub suspended_at: Option<DateTime<Utc>>,
}
impl ManagedLineClaim {
    pub const COLLECTION_NAME: &'static str = "managed_line_claims";
}
impl std::fmt::Debug for ManagedLineClaim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedLineClaim")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedLineQuote {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub line_id: String,
    pub kind: QuoteKind,
    #[serde(default)]
    pub line_revision: i64,
    #[serde(default)]
    pub eligibility_revision: i64,
    #[serde(default)]
    pub automatic_request_id: Option<String>,
    pub rental_rate: RateSnapshot,
    pub terms: TermsVersions,
    pub usage_rates: Vec<RateSnapshot>,
    #[serde(default)]
    pub standard_voice_options: Vec<StandardVoiceOption>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub accept_before: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub accepted_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub cancelled_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    /// Unaccepted consent expires; acceptance removes this TTL in its transaction.
    #[serde(default, with = "super::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}
impl ManagedLineQuote {
    pub const COLLECTION_NAME: &'static str = "managed_line_quotes";
}
impl std::fmt::Debug for ManagedLineQuote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedLineQuote")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedLinePeriod {
    #[serde(rename = "_id")]
    pub id: String,
    pub claim_id: String,
    pub managed_claim_generation: i64,
    pub period_index: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub covered_from: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub covered_until: DateTime<Utc>,
    pub quantity: i64,
    pub rate: RateSnapshot,
    pub billing_request_id: String,
    pub usage_row_ids: Vec<String>,
    pub funded: bool,
    pub spend_hold_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedLinePeriod {
    pub const COLLECTION_NAME: &'static str = "managed_line_periods";
}

/// Metadata-only recovery diagnostic. No owner identity, content or credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedRentalDiagnostic {
    #[serde(rename = "_id")]
    pub id: String,
    pub usage_row_id: String,
    pub billing_request_id: String,
    pub code: String,
    pub active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub detected_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub resolved_at: Option<DateTime<Utc>>,
}
impl ManagedRentalDiagnostic {
    pub const COLLECTION_NAME: &'static str = "managed_rental_diagnostics";
}
impl std::fmt::Debug for ManagedLinePeriod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedLinePeriod")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedCommsEligibility {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub business_legal_name: String,
    pub registration_id: String,
    pub jurisdiction: String,
    pub allowed_use_case: String,
    pub sms_sender_path: String,
    pub sms_registration_status: String,
    pub required_terms: TermsVersions,
    pub accepted_terms: Option<TermsVersions>,
    pub premium_allowed: bool,
    pub max_lines: i64,
    #[serde(with = "super::credits::whole::optional")]
    pub monthly_spend_limit: Option<Credits>,
    pub approved_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub approved_at: DateTime<Utc>,
    pub revision: i64,
    pub fence: i64,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub terms_accepted_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
}
impl ManagedCommsEligibility {
    pub const COLLECTION_NAME: &'static str = "managed_comms_eligibility";
}
impl std::fmt::Debug for ManagedCommsEligibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedCommsEligibility")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// At most one rolling rate row per line and kind. TTL is two minutes after
/// the last admitted effect, safely beyond the sixty-second counting window.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedLineRateWindow {
    #[serde(rename = "_id")]
    pub id: String,
    pub key: String,
    pub events: Vec<bson::DateTime>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}
impl ManagedLineRateWindow {
    pub const COLLECTION_NAME: &'static str = "managed_line_rate_windows";
}

/// Unknown authenticated SMS receipt identities cannot route to a current tenant.
/// Retained metadata tombstones contain no owner, caller, content or credential.
/// They follow managed routing-evidence retention and deliberately have no TTL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedLineQuarantine {
    #[serde(rename = "_id")]
    pub id: String,
    pub trusted_account_id: String,
    pub provider_effect_id: String,
    pub line_id: String,
    pub kind: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub occurred_at: DateTime<Utc>,
}
impl ManagedLineQuarantine {
    pub const COLLECTION_NAME: &'static str = "managed_line_quarantines";
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedCallAdmission {
    #[serde(rename = "_id")]
    pub id: String,
    pub session_id: String,
    pub managed_claim_id: String,
    pub managed_claim_generation: i64,
    pub admitted: AdmittedReference,
    pub base: MeterAdmission,
    pub premium: Option<MeterAdmission>,
    #[serde(default)]
    pub underwriting_snapshot: Option<ManagedUnderwriting>,
    pub underwriting_hold_id: Option<String>,
    pub spend_hold_id: Option<String>,
    pub slot_held: bool,
    pub financially_finalized: bool,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub answer_dispatch_anchor: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub hangup_deadline: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedCallAdmission {
    pub const COLLECTION_NAME: &'static str = "managed_call_admissions";
}
impl std::fmt::Debug for ManagedCallAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedCallAdmission")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedEffectRoute {
    #[serde(rename = "_id")]
    pub id: String,
    pub trusted_account_id: String,
    pub provider: String,
    pub provider_effect_id: String,
    pub line_id: String,
    pub managed_claim_id: Option<String>,
    pub managed_claim_generation: Option<i64>,
    pub binding_id: Option<String>,
    pub effect_id: String,
    pub kind: EffectKind,
    pub financially_finalized: bool,
    pub admitted: AdmittedReference,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub covered_from: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub covered_until: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedEffectRoute {
    pub const COLLECTION_NAME: &'static str = "managed_effect_routes";
}
impl std::fmt::Debug for ManagedEffectRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedEffectRoute")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedInboundSms {
    #[serde(rename = "_id")]
    pub id: String,
    pub trusted_account_id: String,
    pub provider_message_id: String,
    pub line_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub occurred_at: DateTime<Utc>,
    pub parts: i64,
    pub outcomes: Vec<SmsPartOutcome>,
    #[serde(default)]
    pub delivery_claimed: bool,
    pub quarantined: bool,
    pub financially_finalized: bool,
    pub admitted: AdmittedReference,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedInboundSms {
    pub const COLLECTION_NAME: &'static str = "managed_inbound_sms";
}
impl std::fmt::Debug for ManagedInboundSms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedInboundSms")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedBudget {
    #[serde(rename = "_id")]
    pub id: String,
    pub budget_key: String,
    pub window: BudgetWindow,
    #[serde(with = "super::credits::whole")]
    pub limit: Credits,
    pub revision: i64,
    pub fence: i64,
}
impl ManagedBudget {
    pub const COLLECTION_NAME: &'static str = "managed_budgets";
}
impl std::fmt::Debug for ManagedBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedBudget")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedBudgetEntry {
    #[serde(rename = "_id")]
    pub id: String,
    pub budget_id: String,
    pub effect_key: String,
    #[serde(with = "super::credits::whole")]
    pub held: Credits,
    #[serde(with = "super::credits::whole::optional")]
    pub recognized: Option<Credits>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub admitted_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub recognized_at: Option<DateTime<Utc>>,
}
impl ManagedBudgetEntry {
    pub const COLLECTION_NAME: &'static str = "managed_budget_entries";
}
impl std::fmt::Debug for ManagedBudgetEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedBudgetEntry")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedCommsCostEvidence {
    #[serde(default)]
    pub admitted: Option<AdmittedReference>,
    #[serde(default)]
    pub tariffs: Vec<SupplierTariff>,
    #[serde(default)]
    pub policy: Option<ManagedCostPolicy>,
    #[serde(default)]
    pub provider_quantity: Option<i64>,
    #[serde(default)]
    pub synthesis_characters: Option<i64>,
    #[serde(default, with = "super::credits::whole::optional")]
    pub billed_credits: Option<Credits>,
    #[serde(default)]
    pub capture_complete: bool,
    /// `provider_evidence_unavailable` when the effect settled at zero without
    /// provider evidence; its supplier cost stays unknown and fully absorbed.
    #[serde(default)]
    pub terminal_evidence: Option<String>,
    #[serde(rename = "_id")]
    pub id: String,
    pub effect_id: String,
    pub line_id: String,
    pub kind: EffectKind,
    pub components: Vec<CostComponent>,
    pub required_components: Vec<CostKind>,
    pub absorbed_seconds: Option<i64>,
    pub absorbed_sms_parts: i64,
    pub retry_count: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub next_attempt_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedCommsCostEvidence {
    pub const COLLECTION_NAME: &'static str = "managed_comms_cost_evidence";
}
impl std::fmt::Debug for ManagedCommsCostEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedCommsCostEvidence")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedInventoryCost {
    #[serde(default)]
    pub fx: Vec<CostFxRate>,
    #[serde(rename = "_id")]
    pub id: String,
    pub line_id: String,
    pub cycle_key: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub cycle_from: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub cycle_until: DateTime<Utc>,
    pub amount: String,
    pub currency: String,
    pub source: CostSource,
    pub provider_record_id: Option<String>,
    pub fx_policy_revision: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedInventoryCost {
    pub const COLLECTION_NAME: &'static str = "managed_inventory_costs";
}
impl std::fmt::Debug for ManagedInventoryCost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedInventoryCost")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedCallConfirmation {
    #[serde(rename = "_id")]
    pub id: String,
    pub session_id: String,
    pub tool_invocation_id: String,
    pub ordinal: i64,
    pub payload_digest: String,
    pub voice_binding_id: String,
    pub voice_binding_revision: i64,
    pub session_generation: i64,
    pub sms_binding_id: String,
    pub sms_binding_revision: i64,
    pub status: ConfirmationState,
    pub billing_request_ids: Vec<String>,
    pub model_asserted_caller_consent: bool,
    pub managed_claim_id: Option<String>,
    pub managed_claim_generation: Option<i64>,
    pub admitted: Option<AdmittedReference>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedCallConfirmation {
    pub const COLLECTION_NAME: &'static str = "managed_call_confirmations";
}
impl std::fmt::Debug for ManagedCallConfirmation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedCallConfirmation")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedLifecycleRequest {
    #[serde(rename = "_id")]
    pub id: String,
    pub actor_id: String,
    pub request_id: String,
    pub payload_digest: String,
    pub kind: String,
    pub line_id: String,
    pub claim_id: Option<String>,
    pub period_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedLifecycleRequest {
    pub const COLLECTION_NAME: &'static str = "managed_line_requests";
}
impl std::fmt::Debug for ManagedLifecycleRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedLifecycleRequest")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuoteKind {
    Claim,
    Renewal {
        claim_id: String,
        generation: i64,
        period_index: i64,
    },
    Resume {
        claim_id: String,
        generation: i64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeterAdmission {
    pub billing_request_id: String,
    pub usage_row_ids: Vec<String>,
    pub rate: RateSnapshot,
    pub reserved_quantity: i64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Call,
    Sms,
    Confirmation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SmsPartOutcome {
    Pending {
        part: i64,
    },
    CustomerFunded {
        part: i64,
        claim_id: String,
        billing_request_id: String,
    },
    Absorbed {
        part: i64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BudgetWindow {
    RollingThirtyDays,
    CalendarMonth { year: i32, month: u32 },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostComponent {
    pub kind: CostKind,
    pub source: CostSource,
    pub provider_record_id: Option<String>,
    pub amount: Option<String>,
    pub currency: String,
    pub fx_policy_revision: String,
    pub supplier_rounded_quantity: Option<i64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationState {
    Prepared,
    Sending,
    Sent,
    FailedBeforeDispatch,
    Uncertain,
}
impl std::fmt::Debug for AdmittedReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdmittedReference { [REDACTED] }")
    }
}
impl std::fmt::Debug for RegulatoryEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegulatoryEvidence { [REDACTED] }")
    }
}

/// Frozen underwriting inputs; authoring and enablement are delivered in M4.
#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedUnderwriting {
    #[serde(default)]
    pub acceptance_text: String,
    #[serde(default)]
    pub timestamp_fixture_reference: Option<String>,
    #[serde(default)]
    pub detail_record_options_reference: Option<String>,
    #[serde(default)]
    pub detail_record_fixture_reference: Option<String>,
    #[serde(default)]
    pub character_tariff_reference: String,
    pub enabled: bool,
    pub accepted_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub accepted_at: DateTime<Utc>,
    pub acceptance_version: String,
    pub paid_tier_evidence_reference: String,
    pub max_leg_seconds: i64,
    pub max_leg_evidence_reference: String,
    pub detail_record_fields: Vec<String>,
    pub supplier_tariff_revision: i64,
    pub fx_policy_revision: String,
    pub components: Vec<ExposureComponent>,
    #[serde(with = "super::credits::whole")]
    pub rolling_budget: Credits,
    pub revision: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ExposureComponent {
    pub kind: CostKind,
    #[serde(with = "super::credits::whole")]
    pub credits_per_second: Credits,
    pub increment_seconds: i64,
    pub minimum_seconds: i64,
}
impl std::fmt::Debug for ManagedUnderwriting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedUnderwriting { [REDACTED] }")
    }
}

/// Metadata-only durable notification intent; an uncertain delivery is not retried.
#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedLineNotice {
    #[serde(rename = "_id")]
    pub id: String,
    pub claim_id: String,
    pub user_id: String,
    pub line_id: String,
    pub key: String,
    pub kind: String,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub dispatched_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedLineNotice {
    pub const COLLECTION_NAME: &'static str = "managed_line_notices";
}

/// Managed-only erasure preparation barrier, retained until tenant cleanup.
#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedLineErasure {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedLineErasure {
    pub const COLLECTION_NAME: &'static str = "managed_line_erasure_preparations";
}

/// References only. The integration secret itself remains in Telnyx custody.
#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedVoiceAccount {
    #[serde(rename = "_id")]
    pub id: String,
    pub trusted_account_id: String,
    pub revision: i64,
    pub per_call_characters: i64,
    #[serde(default)]
    pub greeting: Option<String>,
    #[serde(default)]
    pub wrapup: Option<String>,
    pub premium: Option<PremiumVoiceConfig>,
}
impl ManagedVoiceAccount {
    pub const COLLECTION_NAME: &'static str = "managed_voice_accounts";
}
#[derive(Clone, Serialize, Deserialize)]
pub struct PremiumVoiceConfig {
    pub api_key_ref: String,
    pub voices: Vec<StandardVoiceOption>,
    pub billing_account_identity: String,
    pub monthly_characters: i64,
    pub plan_tier: String,
    pub oem_terms_version: String,
    pub acknowledged_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub acknowledged_at: DateTime<Utc>,
    pub billing_service_id: String,
}
impl std::fmt::Debug for ManagedVoiceAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedVoiceAccount { [REDACTED] }")
    }
}
impl std::fmt::Debug for PremiumVoiceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PremiumVoiceConfig { [REDACTED] }")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedCharacterAllowance {
    #[serde(rename = "_id")]
    pub id: String,
    pub session_id: String,
    pub limit: i64,
    pub consumed: i64,
    pub wrapup_consumed: i64,
    #[serde(default)]
    pub greeting: Option<String>,
    #[serde(default)]
    pub wrapup: Option<String>,
    pub reserve_month_id: Option<String>,
    pub reserve_released: bool,
    pub premium: Option<PremiumVoiceConfig>,
    pub premium_fallback: bool,
    pub fence: i64,
}
impl ManagedCharacterAllowance {
    pub const COLLECTION_NAME: &'static str = "managed_character_allowances";
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedCharacterMonth {
    #[serde(rename = "_id")]
    pub id: String,
    pub identity_key: String,
    pub consumed: i64,
    pub fence: i64,
}
impl ManagedCharacterMonth {
    pub const COLLECTION_NAME: &'static str = "managed_character_months";
}
/// An attempt has no replay transition. Even a crash before write stays consumed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedCharacterBlock {
    #[serde(rename = "_id")]
    pub id: String,
    pub session_id: String,
    pub writer_id: String,
    pub generation: i64,
    pub characters: i64,
    pub wrapup: bool,
    pub month_id: Option<String>,
    pub status: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
impl ManagedCharacterBlock {
    pub const COLLECTION_NAME: &'static str = "managed_character_blocks";
}
#[derive(Clone, Serialize, Deserialize)]
pub struct CallerConfirmationBinding {
    #[serde(rename = "_id")]
    pub id: String,
    pub voice_binding_id: String,
    pub voice_binding_revision: i64,
    pub sms_binding_id: String,
    pub sms_binding_revision: i64,
    pub voice_credential_identity: String,
    pub sms_credential_identity: String,
    pub enabled: bool,
    pub fence: i64,
}
impl CallerConfirmationBinding {
    pub const COLLECTION_NAME: &'static str = "caller_confirmation_bindings";
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedVoiceSwitch {
    #[serde(rename = "_id")]
    pub id: String,
    pub premium_enabled: bool,
    pub fence: i64,
}
impl ManagedVoiceSwitch {
    pub const COLLECTION_NAME: &'static str = "managed_voice_switches";
}

impl std::fmt::Debug for CallerConfirmationBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CallerConfirmationBinding { [REDACTED] }")
    }
}
/// Policy is frozen on effects and supplier inventory cycles, independently of retail prices.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostFxRate {
    pub currency: String,
    pub credits_per_unit: String,
    pub revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostRecordSchema {
    pub kind: CostKind,
    pub record_type: String,
    pub amount_field: String,
    pub currency_field: String,
    pub options_reference: String,
    pub fixture_reference: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryTariff {
    pub cycle: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub cycle_anchor: DateTime<Utc>,
    pub amount: String,
    pub currency: String,
    pub source: String,
    pub fx_policy_revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedCostPolicy {
    #[serde(rename = "_id")]
    pub id: String,
    pub line_id: String,
    pub revision: i64,
    pub fx: Vec<CostFxRate>,
    pub schemas: Vec<CostRecordSchema>,
    pub inventory: Option<InventoryTariff>,
}
impl ManagedCostPolicy {
    pub const COLLECTION_NAME: &'static str = "managed_cost_policies";
}
