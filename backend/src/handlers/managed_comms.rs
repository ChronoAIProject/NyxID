//! First-party human management; responses contain no provider authority or secrets.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::managed_comms::{ManagedCommsEligibility, TermsVersions},
    mw::auth::AuthUser,
    services::{
        audit_service::AuditActor,
        call_channels::managed::{custody, eligibility},
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct EligibilityDto {
    pub id: String,
    pub owner_id: String,
    pub business_legal_name: String,
    pub registration_id: String,
    pub jurisdiction: String,
    pub allowed_use_case: String,
    pub sms_sender_path: String,
    pub sms_registration_status: String,
    pub required_terms: TermsVersions,
    pub accepted_terms: Option<TermsVersions>,
    pub terms_accepted_at: Option<String>,
    pub premium_allowed: bool,
    pub max_lines: i64,
    pub monthly_spend_limit: Option<String>,
    pub revision: i64,
    pub approved_at: String,
    pub revoked_at: Option<String>,
}
impl From<ManagedCommsEligibility> for EligibilityDto {
    fn from(r: ManagedCommsEligibility) -> Self {
        Self {
            id: r.id,
            owner_id: r.user_id,
            business_legal_name: r.business_legal_name,
            registration_id: r.registration_id,
            jurisdiction: r.jurisdiction,
            allowed_use_case: r.allowed_use_case,
            sms_sender_path: r.sms_sender_path,
            sms_registration_status: r.sms_registration_status,
            required_terms: r.required_terms,
            accepted_terms: r.accepted_terms,
            terms_accepted_at: r.terms_accepted_at.map(|v| v.to_rfc3339()),
            premium_allowed: r.premium_allowed,
            max_lines: r.max_lines,
            monthly_spend_limit: r.monthly_spend_limit.map(|v| v.to_string()),
            revision: r.revision,
            approved_at: r.approved_at.to_rfc3339(),
            revoked_at: r.revoked_at.map(|v| v.to_rfc3339()),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionInput {
    pub revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermsInput {
    pub revision: i64,
    pub terms: TermsVersions,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub after: Option<String>,
}
fn owner(id: &str) -> AppResult<()> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(AppError::ValidationError("Invalid owner ID".into()));
    }
    Ok(())
}
async fn admin(state: &AppState, auth: &AuthUser) -> AppResult<AuditActor> {
    super::login_client_context::require_first_party_human(auth)?;
    super::admin_helpers::require_admin(state, auth).await?;
    Ok(AuditActor::from_auth_user(auth))
}
pub async fn read(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<EligibilityDto>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        eligibility::load(&state.db, &auth.user_id.to_string())
            .await?
            .into(),
    ))
}
pub async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<TermsInput>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    eligibility::accept_terms(
        &state.db,
        &AuditActor::from_auth_user(&auth),
        input.revision,
        input.terms,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(page): Query<Page>,
) -> AppResult<Json<serde_json::Value>> {
    admin(&state, &auth).await?;
    if let Some(after) = page.after.as_deref() {
        owner(after)?;
    }
    let mut rows = eligibility::list(&state.db, page.after.as_deref()).await?;
    let next = if rows.len() > 100 {
        rows.pop();
        rows.last().map(|r| r.id.clone())
    } else {
        None
    };
    Ok(Json(
        serde_json::json!({"eligibility":rows.into_iter().map(EligibilityDto::from).collect::<Vec<_>>(),"next_after_id":next}),
    ))
}
pub async fn save(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<eligibility::EligibilityInput>,
) -> AppResult<Json<EligibilityDto>> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    Ok(Json(
        eligibility::save(
            &state.db,
            &actor,
            &id,
            input,
            state.audit_chain_hmac_key.as_ref().as_ref(),
        )
        .await?
        .into(),
    ))
}
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<RevisionInput>,
) -> AppResult<StatusCode> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    eligibility::revoke(
        &state,
        &actor,
        &id,
        input.revision,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn apply(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<RevisionInput>,
) -> AppResult<StatusCode> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    eligibility::apply_revision(
        &state.db,
        &actor,
        &id,
        input.revision,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Serialize)]
pub struct RegistrationDto {
    pub line_id: String,
    pub voice_url: String,
    pub sms_url: String,
}
/// Preallocate the stable UUID before the operator configures provider webhooks.
/// Registration proves those URLs; preallocation grants no execution authority.
pub async fn prepare(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<RegistrationDto>> {
    admin(&state, &auth).await?;
    crate::services::call_channels::managed::require_ready(&state.db).await?;
    let id = uuid::Uuid::new_v4().to_string();
    Ok(Json(RegistrationDto {
        voice_url: custody::stable_url(&state.config.base_url, &id, false),
        sms_url: custody::stable_url(&state.config.base_url, &id, true),
        line_id: id,
    }))
}
#[derive(Serialize)]
pub struct LineDto {
    pub id: String,
    pub state: String,
    pub revision: i64,
    pub has_voice: bool,
    pub has_sms: bool,
}
pub async fn register(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<custody::RegistrationInput>,
) -> AppResult<Json<LineDto>> {
    let actor = admin(&state, &auth).await?;
    let r = custody::register(&state, &actor, input).await?;
    Ok(Json(LineDto {
        id: r.id,
        state: "available".into(),
        revision: r.revision,
        has_voice: r.voice.is_some(),
        has_sms: r.sms.is_some(),
    }))
}
pub async fn rotate(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<custody::RotationInput>,
) -> AppResult<StatusCode> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    custody::rotate(&state, &actor, &id, input).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn configure_standard_voices(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<crate::services::call_channels::managed::voice::AllowlistInput>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    owner(&id)?;
    crate::services::call_channels::managed::voice::configure(
        &state,
        &AuditActor::from_auth_user(&auth),
        &id,
        input,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn select_standard_voice(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<crate::services::call_channels::managed::voice::SelectionInput>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    owner(&id)?;
    crate::services::call_channels::managed::voice::select(
        &state,
        &AuditActor::from_auth_user(&auth),
        &id,
        input,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct RateDto {
    pub metric: String,
    pub credits_per_unit: String,
    pub revision: String,
    /// `rental`, `calls`, `sms` or `premium_voice` (surcharge per voice second).
    pub purpose: &'static str,
}
impl From<crate::models::managed_comms::RateSnapshot> for RateDto {
    fn from(r: crate::models::managed_comms::RateSnapshot) -> Self {
        use crate::models::service_billing::BillingMetric;
        Self {
            purpose: match r.metric {
                BillingMetric::PhoneNumberDays => "rental",
                BillingMetric::Requests => "sms",
                _ => "calls",
            },
            metric: r.metric.as_str().into(),
            credits_per_unit: r.rate.to_string(),
            revision: r.revision,
        }
    }
}
#[derive(Serialize)]
pub struct StandardVoiceDto {
    pub voice: String,
    pub language: String,
}
impl From<crate::models::managed_comms::StandardVoiceOption> for StandardVoiceDto {
    fn from(option: crate::models::managed_comms::StandardVoiceOption) -> Self {
        Self {
            voice: option.voice,
            language: option.language,
        }
    }
}
#[derive(Serialize)]
pub struct QuoteDto {
    pub id: String,
    pub line_id: String,
    pub kind: crate::models::managed_comms::QuoteKind,
    pub rental: RateDto,
    pub usage: Vec<RateDto>,
    pub standard_voice_options: Vec<StandardVoiceDto>,
    pub terms: TermsVersions,
    pub accept_before: String,
    pub period_days: i64,
    pub paid_period_refundable: bool,
}
impl From<crate::models::managed_comms::ManagedLineQuote> for QuoteDto {
    fn from(q: crate::models::managed_comms::ManagedLineQuote) -> Self {
        Self {
            id: q.id,
            line_id: q.line_id,
            kind: q.kind,
            rental: q.rental_rate.into(),
            // Quotes list the line's own voice rate before the optional premium
            // surcharge (`lifecycle::quote_internal`); later voice rates are premium.
            usage: q
                .usage_rates
                .into_iter()
                .scan(false, |voice_seen, rate| {
                    let mut dto = RateDto::from(rate);
                    if dto.purpose == "calls" {
                        if *voice_seen {
                            dto.purpose = "premium_voice";
                        }
                        *voice_seen = true;
                    }
                    Some(dto)
                })
                .collect(),
            standard_voice_options: q
                .standard_voice_options
                .into_iter()
                .map(Into::into)
                .collect(),
            terms: q.terms,
            accept_before: q.accept_before.to_rfc3339(),
            period_days: 30,
            paid_period_refundable: false,
        }
    }
}
#[derive(Serialize)]
pub struct LifecycleDto {
    pub request_id: String,
    pub kind: String,
    pub line_id: String,
    pub claim_id: Option<String>,
    pub period_id: Option<String>,
}
impl From<crate::models::managed_comms::ManagedLifecycleRequest> for LifecycleDto {
    fn from(r: crate::models::managed_comms::ManagedLifecycleRequest) -> Self {
        Self {
            request_id: r.request_id,
            kind: r.kind,
            line_id: r.line_id,
            claim_id: r.claim_id,
            period_id: r.period_id,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteInput {
    pub kind: crate::models::managed_comms::QuoteKind,
}
pub async fn quote_line(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<QuoteInput>,
) -> AppResult<Json<QuoteDto>> {
    super::login_client_context::require_first_party_human(&auth)?;
    owner(&id)?;
    Ok(Json(
        crate::services::call_channels::managed::lifecycle::quote(
            &state,
            &auth.user_id.to_string(),
            &id,
            input.kind,
        )
        .await?
        .into(),
    ))
}
pub async fn claim_line(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<crate::services::call_channels::managed::lifecycle::ClaimInput>,
) -> AppResult<Json<LifecycleDto>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        crate::services::call_channels::managed::lifecycle::claim(
            &state,
            &AuditActor::from_auth_user(&auth),
            input,
        )
        .await?
        .into(),
    ))
}
pub async fn continue_line(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<crate::services::call_channels::managed::lifecycle::ContinueInput>,
) -> AppResult<Json<LifecycleDto>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        crate::services::call_channels::managed::lifecycle::continue_coverage(
            &state,
            &AuditActor::from_auth_user(&auth),
            input,
        )
        .await?
        .into(),
    ))
}
pub async fn release_line(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<crate::services::call_channels::managed::lifecycle::ReleaseInput>,
) -> AppResult<Json<LifecycleDto>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        crate::services::call_channels::managed::lifecycle::release(
            &state,
            &AuditActor::from_auth_user(&auth),
            input,
            false,
        )
        .await?
        .into(),
    ))
}
pub async fn admin_release_line(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<crate::services::call_channels::managed::lifecycle::ReleaseInput>,
) -> AppResult<Json<LifecycleDto>> {
    let actor = admin(&state, &auth).await?;
    Ok(Json(
        crate::services::call_channels::managed::lifecycle::release(&state, &actor, input, true)
            .await?
            .into(),
    ))
}
pub async fn approve_reassignment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<RevisionInput>,
) -> AppResult<StatusCode> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    crate::services::call_channels::managed::lifecycle::approve_reassignment(
        &state,
        &actor,
        &id,
        input.revision,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Serialize)]
pub struct MerchantLineDto {
    pub id: String,
    pub phone_number: String,
    pub country: String,
    pub state: String,
    pub voice: bool,
    pub standard_voice_options: Vec<StandardVoiceDto>,
    pub sms: bool,
    pub claim: Option<ClaimDto>,
}
#[derive(Serialize)]
pub struct ClaimDto {
    pub id: String,
    pub generation: i64,
    pub state: String,
    pub suspension: Option<String>,
    pub auto_renew: bool,
    pub standard_voice: Option<StandardVoiceDto>,
    pub covered_from: Option<String>,
    pub covered_until: Option<String>,
    pub period_index: Option<i64>,
    pub renewal_quote: Option<QuoteDto>,
    pub agent_id: String,
    pub voice_tier: crate::models::managed_comms::VoiceTier,
    pub sms_enabled: bool,
    pub caller_confirmations: bool,
    /// Owner-only identifiers for the voice selection and caller-confirmation
    /// routes; bindings never expose provider authority or credentials.
    pub voice_binding_id: Option<String>,
    pub voice_binding_revision: Option<i64>,
    pub sms_binding_id: Option<String>,
    pub release_requested_at: Option<String>,
    pub suspended_at: Option<String>,
}
fn wire<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| AppError::Internal("Invalid managed line state".into()))
}
pub async fn lines(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(page): Query<Page>,
) -> AppResult<Json<serde_json::Value>> {
    use crate::models::managed_comms::*;
    use crate::services::call_channels::managed::listing;
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    super::login_client_context::require_first_party_human(&auth)?;
    let user = auth.user_id.to_string();
    // Unreadable rows are skipped (metadata-only warning); merchants only see
    // them omitted, and paging still advances past them.
    let raw: Vec<mongodb::bson::Document> = state
        .db
        .collection::<mongodb::bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .find(doc! {"user_id":&user,"state":{"$in":["claimed","suspended","releasing"]}})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    let (claims, _) = listing::decode::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME, raw);
    let ids: Vec<_> = claims.iter().map(|c| c.line_id.clone()).collect();
    let mut filter = doc! {"$or":[{"state":"available"},{"_id":{"$in":ids}}]};
    if let Some(after) = page.after {
        owner(&after)?;
        filter.insert("_id", doc! {"$gt":after});
    }
    let listing::Page {
        rows,
        next_after_id: next,
        ..
    } = listing::page::<ManagedPhoneLine>(
        &state.db,
        ManagedPhoneLine::COLLECTION_NAME,
        filter,
        100,
    )
    .await?;
    // One read per projection for the whole page, never one per claimed row.
    let now = chrono::Utc::now();
    let mut ignored = Vec::new();
    let claim_ids: Vec<String> = claims.iter().map(|c| c.id.clone()).collect();
    let binding_ids: Vec<String> = claims
        .iter()
        .filter_map(|c| c.voice_binding_id.clone())
        .collect();
    let coverage = listing::coverage(&state.db, &claim_ids, now, &mut ignored).await?;
    let mut quotes = listing::open_quotes(&state.db, &user, &claim_ids, now, &mut ignored).await?;
    let revisions = listing::binding_revisions(&state.db, &user, &binding_ids).await?;
    let mut result = vec![];
    for l in rows {
        let claim = if let Some(c) = claims.iter().find(|c| c.line_id == l.id) {
            let paid = coverage.get(&c.id);
            Some(ClaimDto {
                id: c.id.clone(),
                generation: c.managed_claim_generation,
                state: wire(&c.state)?,
                suspension: c.suspension.as_ref().map(wire).transpose()?,
                auto_renew: c.auto_renew,
                covered_from: paid.map(|p| p.covered_from().to_rfc3339()),
                period_index: paid.map(|p| p.period_index),
                agent_id: c.agent_id.clone(),
                voice_tier: c.voice_tier,
                sms_enabled: c.sms_enabled,
                caller_confirmations: c.caller_confirmations,
                voice_binding_id: c.voice_binding_id.clone(),
                voice_binding_revision: c
                    .voice_binding_id
                    .as_ref()
                    .and_then(|id| revisions.get(id).copied()),
                sms_binding_id: c.sms_binding_id.clone(),
                release_requested_at: c.release_requested_at.map(|v| v.to_rfc3339()),
                suspended_at: c.suspended_at.map(|v| v.to_rfc3339()),
                standard_voice: l.voice.as_ref().map(|_| {
                    c.standard_voice
                        .clone()
                        .unwrap_or_else(|| {
                            crate::services::call_channels::managed::voice::default_option(
                                &l.provider,
                            )
                        })
                        .into()
                }),
                covered_until: paid.map(|p| p.latest_until.to_rfc3339()),
                renewal_quote: quotes.remove(&c.id).map(Into::into),
            })
        } else {
            None
        };
        result.push(MerchantLineDto {
            standard_voice_options: crate::services::call_channels::managed::voice::options(&l)
                .into_iter()
                .map(Into::into)
                .collect(),
            id: l.id,
            phone_number: l.phone_number,
            country: l.country,
            state: serde_json::to_value(l.state)
                .map_err(|_| AppError::Internal("Invalid line state".into()))?
                .as_str()
                .unwrap_or_default()
                .into(),
            voice: l.voice.is_some(),
            sms: l.sms.is_some(),
            claim,
        });
    }
    Ok(Json(
        serde_json::json!({"lines":result,"next_after_id":next}),
    ))
}

pub async fn voice_webhook(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> AppResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let output = crate::services::call_channels::managed::ingress::receive(
        state, &id, false, &headers, &body,
    )
    .await?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/xml")],
        output,
    )
        .into_response())
}
pub async fn sms_webhook(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> AppResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let output = crate::services::call_channels::managed::ingress::receive(
        state, &id, true, &headers, &body,
    )
    .await?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/xml")],
        output,
    )
        .into_response())
}
pub async fn underwriting(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<crate::services::call_channels::managed::underwriting::UnderwritingInput>,
) -> AppResult<StatusCode> {
    let actor = admin(&state, &auth).await?;
    crate::services::call_channels::managed::underwriting::configure(&state, &actor, &id, input)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceInput {
    pub provider_seconds: i64,
    pub provider_reference: String,
}
pub async fn resolve_evidence(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<EvidenceInput>,
) -> AppResult<StatusCode> {
    let actor = admin(&state, &auth).await?;
    crate::services::call_channels::managed::calls::resolve_evidence(
        &state,
        &actor,
        &id,
        input.provider_seconds,
        &input.provider_reference,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn configure_voice_account(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(i): Json<crate::services::call_channels::managed::synthesis::VoiceAccountInput>,
) -> AppResult<StatusCode> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    crate::services::call_channels::managed::synthesis::configure(&state, &actor, &id, i).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchInput {
    pub enabled: bool,
}
pub async fn premium_switch(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(i): Json<SwitchInput>,
) -> AppResult<StatusCode> {
    let actor = admin(&state, &auth).await?;
    crate::services::call_channels::managed::synthesis::set_enabled(&state, &actor, i.enabled)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn caller_confirmations(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(i): Json<crate::services::call_channels::managed::confirmations::ToggleInput>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    owner(&id)?;
    crate::services::call_channels::managed::confirmations::configure(
        &state,
        &AuditActor::from_auth_user(&auth),
        &id,
        i,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn cost_policy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(i): Json<crate::services::call_channels::managed::costs::PolicyInput>,
) -> AppResult<StatusCode> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    crate::services::call_channels::managed::costs::configure(&state, &actor, &id, i).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn cost_report(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<crate::services::call_channels::managed::costs::ReportQuery>,
) -> AppResult<Json<crate::services::call_channels::managed::costs::CostReport>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin_or_operator(&state, &auth, "managed_lines.report").await?;
    Ok(Json(
        crate::services::call_channels::managed::costs::report(&state.db, q).await?,
    ))
}

pub async fn cost_effects(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<crate::services::call_channels::managed::costs::EffectQuery>,
) -> AppResult<Json<crate::services::call_channels::managed::costs::EffectPage>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin_or_operator(&state, &auth, "managed_lines.report").await?;
    owner(&q.line_id)?;
    Ok(Json(
        crate::services::call_channels::managed::costs::effect_report(&state.db, q).await?,
    ))
}

#[derive(Serialize)]
pub struct CapabilityDto {
    pub application_id: String,
    pub billing_service_id: String,
}
#[derive(Serialize)]
pub struct RegulatoryDto {
    pub requirement_reference: String,
    pub registration_status: String,
    pub sms_sender_registration: Option<String>,
    pub verified_by: String,
    pub verified_at: String,
}
#[derive(Serialize)]
pub struct TariffDto {
    pub kind: crate::models::managed_comms::CostKind,
    pub amount: String,
    pub currency: String,
    pub unit: String,
    pub increment: i64,
    pub minimum: i64,
    pub source: String,
    pub revision: i64,
    pub fx_policy_revision: String,
    pub effective_at: String,
}
#[derive(Serialize)]
pub struct AdminClaimDto {
    pub id: String,
    pub owner_id: String,
    pub generation: i64,
    pub state: crate::models::managed_comms::LineState,
    pub suspension: Option<crate::models::managed_comms::Suspension>,
    pub voice_tier: crate::models::managed_comms::VoiceTier,
    pub sms_enabled: bool,
    pub caller_confirmations: bool,
    pub auto_renew: bool,
    pub covered_until: Option<String>,
}
/// Platform-admin inventory projection: identifiers, policy and evidence
/// references only; no credential, caller, transcript or message content.
#[derive(Serialize)]
pub struct AdminLineDto {
    pub id: String,
    pub provider: String,
    pub trusted_account_id: String,
    pub phone_number: String,
    pub country: String,
    pub number_resource_id: String,
    pub voice: Option<CapabilityDto>,
    pub sms: Option<CapabilityDto>,
    pub rental_billing_service_id: String,
    pub standard_voice_options: Vec<StandardVoiceDto>,
    pub regulatory: RegulatoryDto,
    pub supplier_tariffs: Vec<TariffDto>,
    pub provider_channel_limit: Option<i64>,
    pub call_slot_limit: i64,
    pub calls_per_minute: i64,
    pub sms_per_minute: i64,
    pub state: crate::models::managed_comms::LineState,
    pub suspension: Option<crate::models::managed_comms::Suspension>,
    pub claim: Option<AdminClaimDto>,
    pub live_calls: u64,
    pub cost_policy_revision: Option<i64>,
    pub voice_url: String,
    pub sms_url: String,
    pub revision: i64,
    pub available_at: Option<String>,
    pub supplier_released_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
pub async fn admin_lines(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(page): Query<Page>,
) -> AppResult<Json<serde_json::Value>> {
    use crate::models::managed_comms::*;
    use crate::services::call_channels::managed::listing;
    use mongodb::bson::doc;
    admin(&state, &auth).await?;
    let mut filter = doc! {};
    if let Some(after) = page.after.as_deref() {
        owner(after)?;
        filter.insert("_id", doc! {"$gt":after});
    }
    // Unreadable rows are skipped and named in `unreadable_ids` for repair;
    // one malformed document never fails the operator listing.
    let listing::Page {
        rows,
        next_after_id: next,
        mut unreadable,
    } = listing::page::<ManagedPhoneLine>(
        &state.db,
        ManagedPhoneLine::COLLECTION_NAME,
        filter,
        100,
    )
    .await?;
    let claim_ids: Vec<_> = rows
        .iter()
        .filter_map(|l| l.current_claim_id.clone())
        .collect();
    let (claims, mut bad) = listing::rows::<ManagedLineClaim>(
        &state.db,
        ManagedLineClaim::COLLECTION_NAME,
        doc! {"_id":{"$in":&claim_ids}},
    )
    .await?;
    unreadable.append(&mut bad);
    let line_ids: Vec<_> = rows.iter().map(|l| l.id.clone()).collect();
    let (policies, mut bad) = listing::rows::<ManagedCostPolicy>(
        &state.db,
        ManagedCostPolicy::COLLECTION_NAME,
        doc! {"line_id":{"$in":&line_ids}},
    )
    .await?;
    unreadable.append(&mut bad);
    let coverage =
        listing::coverage(&state.db, &claim_ids, chrono::Utc::now(), &mut unreadable).await?;
    let live = listing::live_calls(&state.db, &claim_ids).await?;
    let mut result = Vec::with_capacity(rows.len());
    for l in rows {
        let claim = claims
            .iter()
            .find(|c| Some(&c.id) == l.current_claim_id.as_ref())
            .map(|c| AdminClaimDto {
                id: c.id.clone(),
                owner_id: c.user_id.clone(),
                generation: c.managed_claim_generation,
                state: c.state,
                suspension: c.suspension,
                voice_tier: c.voice_tier,
                sms_enabled: c.sms_enabled,
                caller_confirmations: c.caller_confirmations,
                auto_renew: c.auto_renew,
                covered_until: coverage.get(&c.id).map(|p| p.latest_until.to_rfc3339()),
            });
        let live_calls = claim
            .as_ref()
            .and_then(|c| live.get(&c.id).copied())
            .unwrap_or(0);
        result.push(AdminLineDto {
            voice_url: custody::stable_url(&state.config.base_url, &l.id, false),
            sms_url: custody::stable_url(&state.config.base_url, &l.id, true),
            cost_policy_revision: policies
                .iter()
                .find(|p| p.line_id == l.id)
                .map(|p| p.revision),
            standard_voice_options: crate::services::call_channels::managed::voice::options(&l)
                .into_iter()
                .map(Into::into)
                .collect(),
            id: l.id,
            provider: l.provider,
            trusted_account_id: l.trusted_account_id,
            phone_number: l.phone_number,
            country: l.country,
            number_resource_id: l.number_resource_id,
            voice: l.voice.map(|c| CapabilityDto {
                application_id: c.application_id,
                billing_service_id: c.billing_service_id,
            }),
            sms: l.sms.map(|c| CapabilityDto {
                application_id: c.application_id,
                billing_service_id: c.billing_service_id,
            }),
            rental_billing_service_id: l.rental_billing_service_id,
            regulatory: RegulatoryDto {
                requirement_reference: l.regulatory.requirement_reference,
                registration_status: l.regulatory.registration_status,
                sms_sender_registration: l.regulatory.sms_sender_registration,
                verified_by: l.regulatory.verified_by,
                verified_at: l.regulatory.verified_at.to_rfc3339(),
            },
            supplier_tariffs: l
                .supplier_tariffs
                .into_iter()
                .map(|t| TariffDto {
                    kind: t.kind,
                    amount: t.amount,
                    currency: t.currency,
                    unit: t.unit,
                    increment: t.increment,
                    minimum: t.minimum,
                    source: t.source,
                    revision: t.revision,
                    fx_policy_revision: t.fx_policy_revision,
                    effective_at: t.effective_at.to_rfc3339(),
                })
                .collect(),
            provider_channel_limit: l.provider_channel_limit,
            call_slot_limit: l.call_slot_limit,
            calls_per_minute: l.calls_per_minute,
            sms_per_minute: l.sms_per_minute,
            state: l.state,
            suspension: l.suspension,
            claim,
            live_calls,
            revision: l.revision,
            available_at: l.available_at.map(|v| v.to_rfc3339()),
            supplier_released_at: l.supplier_released_at.map(|v| v.to_rfc3339()),
            created_at: l.created_at.to_rfc3339(),
            updated_at: l.updated_at.to_rfc3339(),
        });
    }
    Ok(Json(serde_json::json!({
        "lines": result,
        "next_after_id": next,
        "premium_enabled": crate::services::call_channels::managed::synthesis::enabled(&state.db).await?,
        "unreadable_ids": unreadable,
    })))
}
pub async fn update_line(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<custody::UpdateInput>,
) -> AppResult<Json<LineDto>> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    let line = custody::update(&state, &actor, &id, input).await?;
    Ok(Json(LineDto {
        state: wire(&line.state)?,
        id: line.id,
        revision: line.revision,
        has_voice: line.voice.is_some(),
        has_sms: line.sms.is_some(),
    }))
}
pub async fn suspend_line(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<crate::services::call_channels::managed::lifecycle::LineRevisionInput>,
) -> AppResult<Json<crate::services::call_channels::managed::lifecycle::LineOperation>> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    Ok(Json(
        crate::services::call_channels::managed::lifecycle::suspend(&state, &actor, &id, input)
            .await?,
    ))
}
pub async fn resume_line(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<crate::services::call_channels::managed::lifecycle::LineRevisionInput>,
) -> AppResult<Json<crate::services::call_channels::managed::lifecycle::LineOperation>> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    Ok(Json(
        crate::services::call_channels::managed::lifecycle::resume(&state, &actor, &id, input)
            .await?,
    ))
}
pub async fn retire_line(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<crate::services::call_channels::managed::lifecycle::RetireInput>,
) -> AppResult<Json<crate::services::call_channels::managed::lifecycle::LineOperation>> {
    let actor = admin(&state, &auth).await?;
    owner(&id)?;
    Ok(Json(
        crate::services::call_channels::managed::lifecycle::retire(&state, &actor, &id, input)
            .await?,
    ))
}

/// Managed configuration for trusted Twilio/Telnyx accounts. Evidence
/// references and the Telnyx integration-secret identifier are returned for
/// operator review and edits; credentials and secret values never are.
pub async fn managed_accounts(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(page): Query<Page>,
) -> AppResult<Json<serde_json::Value>> {
    use crate::models::channel_adapter::{ACCOUNTS, ProviderAccount};
    use crate::models::managed_comms::ManagedVoiceAccount;
    use crate::services::call_channels::managed::listing;
    use mongodb::bson::doc;
    admin(&state, &auth).await?;
    let mut filter = doc! {"provider":{"$in":["twilio","telnyx"]}};
    if let Some(after) = page.after.as_deref() {
        owner(after)?;
        filter.insert("_id", doc! {"$gt":after});
    }
    let listing::Page {
        rows,
        next_after_id: next,
        mut unreadable,
    } = listing::page::<ProviderAccount>(&state.db, ACCOUNTS, filter, 100).await?;
    let ids: Vec<_> = rows.iter().map(|a| a.id.clone()).collect();
    let (voices, mut bad) = listing::rows::<ManagedVoiceAccount>(
        &state.db,
        ManagedVoiceAccount::COLLECTION_NAME,
        doc! {"trusted_account_id":{"$in":&ids}},
    )
    .await?;
    unreadable.append(&mut bad);
    let accounts: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|a| {
            let voice = voices.iter().find(|v| v.trusted_account_id == a.id);
            serde_json::json!({
                "id": a.id,
                "provider": a.provider,
                "account_id": a.account_id,
                "catalog_service_id": a.catalog_service_id,
                "revision": a.revision,
                "underwriting": a.underwriting.as_ref().map(|u| serde_json::json!({
                    "revision": u.revision,
                    "enabled": u.enabled,
                    "accepted_by": u.accepted_by,
                    "accepted_at": u.accepted_at.to_rfc3339(),
                    "acceptance_text": u.acceptance_text,
                    "acceptance_version": u.acceptance_version,
                    "paid_tier_evidence_reference": u.paid_tier_evidence_reference,
                    "max_leg_seconds": u.max_leg_seconds,
                    "max_leg_evidence_reference": u.max_leg_evidence_reference,
                    "timestamp_fixture_reference": u.timestamp_fixture_reference,
                    "detail_record_options_reference": u.detail_record_options_reference,
                    "detail_record_fixture_reference": u.detail_record_fixture_reference,
                    "detail_record_fields": u.detail_record_fields,
                    "supplier_tariff_revision": u.supplier_tariff_revision,
                    "fx_policy_revision": u.fx_policy_revision,
                    "character_tariff_reference": u.character_tariff_reference,
                    "components": u.components.iter().map(|c| serde_json::json!({
                        "kind": c.kind,
                        "credits_per_second": c.credits_per_second.to_string(),
                        "increment_seconds": c.increment_seconds,
                        "minimum_seconds": c.minimum_seconds,
                    })).collect::<Vec<_>>(),
                    "rolling_budget": u.rolling_budget.to_string(),
                })),
                "voice": voice.map(|v| serde_json::json!({
                    "revision": v.revision,
                    "per_call_characters": v.per_call_characters,
                    "greeting": v.greeting,
                    "wrapup": v.wrapup,
                    "premium": v.premium.as_ref().map(|p| serde_json::json!({
                        "api_key_ref": p.api_key_ref,
                        "voices": p.voices.iter().cloned().map(StandardVoiceDto::from).collect::<Vec<_>>(),
                        "billing_account_identity": p.billing_account_identity,
                        "monthly_characters": p.monthly_characters,
                        "plan_tier": p.plan_tier,
                        "oem_terms_version": p.oem_terms_version,
                        "acknowledged_by": p.acknowledged_by,
                        "acknowledged_at": p.acknowledged_at.to_rfc3339(),
                        "billing_service_id": p.billing_service_id,
                    })),
                })),
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "accounts": accounts,
        "next_after_id": next,
        "premium_enabled": crate::services::call_channels::managed::synthesis::enabled(&state.db).await?,
        "unreadable_ids": unreadable,
    })))
}
