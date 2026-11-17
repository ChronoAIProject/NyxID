//! Platform custody never transfers to a merchant, including after local retirement.
use super::super::{profiles, protocol, session};
use super::{conflict, resolver, unavailable};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_adapter::*,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
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
use std::collections::BTreeSet;

pub fn sentinel(line: &str) -> String {
    format!("managed-line-custody:{line}")
}
/// Absent legacy counters deserialize as zero. Account authority writers
/// advance `fence`; admissions and proofs compare the observed value exactly
/// and write only the separate `admission::ADMISSION_TOUCH` conflict counter.
pub(super) fn account_fence_value(fence: i64) -> bson::Bson {
    if fence == 0 {
        bson::Bson::Document(doc! {"$in":[0_i64,bson::Bson::Null]})
    } else {
        bson::Bson::Int64(fence)
    }
}
/// Voice lines that still reserve Telnyx underwriting capacity. Suspended,
/// cooling-down and pending lines can carry calls again; retired lines cannot.
pub(super) fn capacity_filter(account: &str) -> bson::Document {
    doc! {"trusted_account_id":account,"voice":{"$ne":bson::Bson::Null},"state":{"$ne":"retired"}}
}
/// Lines whose number the supplier still holds for this account. Rotation proves
/// every one (retired included); supplier-released numbers are gone and skipped.
fn custody_proof_filter(account: &str) -> bson::Document {
    doc! {"trusted_account_id":account,"supplier_released_at":bson::Bson::Null}
}
fn resources(line: &ManagedPhoneLine) -> Vec<(&'static str, &str)> {
    let mut values = vec![
        ("number_id", line.number_resource_id.as_str()),
        ("phone", line.phone_number.as_str()),
    ];
    let mut apps = BTreeSet::new();
    for cap in [&line.voice, &line.sms].into_iter().flatten() {
        if apps.insert(cap.application_id.as_str()) {
            values.push(("application", cap.application_id.as_str()));
        }
    }
    values
}
pub async fn verify(db: &Database, line: &ManagedPhoneLine) -> AppResult<()> {
    for (kind, id) in resources(line) {
        if db.collection::<ResourceClaim>(CLAIMS).find_one(doc!{"identity_key":session::identity(&[&line.provider,kind,id]),"user_id":sentinel(&line.id)}).await?.is_none(){return Err(unavailable());}
    }
    Ok(())
}
pub async fn refuse_legacy(db: &Database, b: &Binding) -> AppResult<()> {
    if matches!(b.credential, CredentialBinding::ManagedLine { .. }) {
        return Err(conflict(
            "Manage this binding through its managed phone line",
        ));
    }
    refuse_resources(
        db,
        profiles::provider(b.profile),
        &b.resource_id,
        &b.phone_number,
        &b.application_id,
    )
    .await
}
pub async fn refuse_resources(
    db: &Database,
    provider: &str,
    resource: &str,
    phone: &str,
    application: &str,
) -> AppResult<()> {
    for (kind, id) in [
        ("number_id", resource),
        ("phone", phone),
        ("application", application),
    ] {
        if let Some(claim) = db
            .collection::<ResourceClaim>(CLAIMS)
            .find_one(doc! {"identity_key":session::identity(&[provider,kind,id])})
            .await?
            && claim.user_id.starts_with("managed-line-custody:")
        {
            return Err(conflict("Provider resource is in managed platform custody"));
        }
    }
    Ok(())
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationInput {
    pub line_id: String,
    pub trusted_account_id: String,
    pub phone_number: String,
    pub country: String,
    pub number_resource_id: String,
    pub voice: Option<Capability>,
    #[serde(default)]
    pub standard_voice_options: Vec<StandardVoiceOption>,
    pub sms: Option<Capability>,
    pub rental_billing_service_id: String,
    pub regulatory: RegulatoryInput,
    pub supplier_tariffs: Vec<SupplierTariffInput>,
    pub call_slot_limit: i64,
    pub calls_per_minute: i64,
    pub sms_per_minute: i64,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegulatoryInput {
    pub requirement_reference: String,
    pub registration_status: String,
    pub sms_sender_registration: Option<String>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplierTariffInput {
    pub kind: CostKind,
    pub amount: String,
    pub currency: String,
    pub unit: String,
    pub increment: i64,
    pub minimum: i64,
    pub source: String,
    pub revision: i64,
    pub fx_policy_revision: String,
    pub effective_at: chrono::DateTime<Utc>,
}
impl From<SupplierTariffInput> for SupplierTariff {
    fn from(t: SupplierTariffInput) -> Self {
        Self {
            kind: t.kind,
            amount: t.amount,
            currency: t.currency,
            unit: t.unit,
            increment: t.increment,
            minimum: t.minimum,
            source: t.source,
            revision: t.revision,
            fx_policy_revision: t.fx_policy_revision,
            effective_at: t.effective_at,
        }
    }
}
fn validate(input: &RegistrationInput) -> AppResult<()> {
    if !uuid::Uuid::parse_str(&input.line_id).is_ok_and(|id| id.get_version_num() == 4)
        || !protocol::phone(&input.phone_number)
        || !protocol::handle(&input.number_resource_id)
        || !super::valid_country(&input.country)
        || (input.voice.is_none() && input.sms.is_none())
    {
        return Err(AppError::ValidationError(
            "Invalid managed phone line registration".into(),
        ));
    }
    validate_operations(
        &input.regulatory,
        input.sms.is_some(),
        &input.supplier_tariffs,
        (
            input.call_slot_limit,
            input.calls_per_minute,
            input.sms_per_minute,
        ),
    )?;
    for cap in [&input.voice, &input.sms].into_iter().flatten() {
        if !protocol::handle(&cap.application_id)
            || uuid::Uuid::parse_str(&cap.billing_service_id).is_err()
        {
            return Err(unavailable());
        }
    }
    if uuid::Uuid::parse_str(&input.trusted_account_id).is_err()
        || uuid::Uuid::parse_str(&input.rental_billing_service_id).is_err()
    {
        return Err(unavailable());
    }
    Ok(())
}
/// Operator-editable fields share registration validation. Identity, resources,
/// capabilities and billing services are immutable after registration.
fn validate_operations(
    regulatory: &RegulatoryInput,
    sms: bool,
    tariffs: &[SupplierTariffInput],
    (slots, calls, messages): (i64, i64, i64),
) -> AppResult<()> {
    if !(1..=100).contains(&slots)
        || !(1..=1000).contains(&calls)
        || !(1..=1000).contains(&messages)
        || (sms
            && regulatory
                .sms_sender_registration
                .as_ref()
                .is_none_or(|r| r.trim().is_empty() || r.len() > 512))
        || regulatory.registration_status != "approved"
        || regulatory.requirement_reference.trim().is_empty()
        || regulatory.requirement_reference.len() > 512
        || tariffs.is_empty()
        || tariffs.len() > 16
    {
        return Err(AppError::ValidationError(
            "Invalid managed phone line registration".into(),
        ));
    }
    for t in tariffs {
        let amount = t
            .amount
            .parse::<crate::models::credits::Credits>()
            .map_err(|_| unavailable())?;
        let source = url::Url::parse(&t.source).map_err(|_| unavailable())?;
        if amount < crate::models::credits::Credits::ZERO
            || t.currency.len() != 3
            || !t.currency.bytes().all(|c| c.is_ascii_uppercase())
            || t.increment <= 0
            || t.minimum < 0
            || t.revision <= 0
            || t.fx_policy_revision.is_empty()
            || t.unit.is_empty()
            || source.scheme() != "https"
            || source.host_str().is_none()
        {
            return Err(unavailable());
        }
    }
    Ok(())
}
pub fn stable_url(base: &str, id: &str, sms: bool) -> String {
    format!(
        "{}/api/v1/webhooks/managed-lines/{id}/{}",
        base.trim_end_matches('/'),
        if sms { "sms" } else { "voice" }
    )
}

/// Uses the canonical platform credential, never a merchant catalog grant.
pub async fn register(
    state: &AppState,
    actor: &AuditActor,
    input: RegistrationInput,
) -> AppResult<ManagedPhoneLine> {
    super::require_ready(&state.db).await?;
    validate(&input)?;
    super::eligibility::require_admin(&state.db, actor).await?;
    let account = state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":&input.trusted_account_id})
        .await?
        .ok_or_else(unavailable)?;
    if !matches!(account.provider.as_str(), "twilio" | "telnyx")
        || !protocol::handle(&account.account_id)
    {
        return Err(unavailable());
    }
    if account.provider == "telnyx" {
        protocol::validate_public_key(
            account
                .webhook_public_key
                .as_deref()
                .ok_or_else(unavailable)?,
        )?;
    }
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&account.catalog_service_id,"is_active":true})
        .await?
        .ok_or_else(unavailable)?;
    let mut marked = service.clone();
    marked.managed_comms_role = Some(ManagedCommsRole::Credential);
    resolver::fixed_service(&marked, &account.provider)?;
    if service.proxy_operation_policy.is_none() {
        return Err(unavailable());
    }
    super::voice::validate_options(
        &account.provider,
        input.voice.is_some(),
        &input.standard_voice_options,
    )?;
    let key =
        crate::services::catalog_credential_record_service::text(&state.encryption_keys, &service)
            .await?;
    let now = Utc::now();
    let mut line = ManagedPhoneLine {
        id: input.line_id,
        provider: account.provider.clone(),
        trusted_account_id: account.id.clone(),
        phone_number: input.phone_number,
        country: input.country,
        number_resource_id: input.number_resource_id,
        standard_voice_options: input.standard_voice_options,
        voice: input.voice,
        sms: input.sms,
        rental_billing_service_id: input.rental_billing_service_id,
        regulatory: RegulatoryEvidence {
            requirement_reference: input.regulatory.requirement_reference,
            registration_status: input.regulatory.registration_status,
            sms_sender_registration: input.regulatory.sms_sender_registration,
            verified_by: actor.user_id.clone(),
            verified_at: now,
        },
        supplier_tariffs: input.supplier_tariffs.into_iter().map(Into::into).collect(),
        provider_channel_limit: None,
        call_slot_limit: input.call_slot_limit,
        calls_per_minute: input.calls_per_minute,
        sms_per_minute: input.sms_per_minute,
        state: LineState::Available,
        suspension: None,
        current_claim_id: None,
        next_claim_generation: 1,
        revision: 1,
        fence: 1,
        available_at: None,
        supplier_released_at: None,
        created_at: now,
        updated_at: now,
    };
    let limit = prove(state, &line, &account, &service, &key, None).await?;
    line.provider_channel_limit = limit;
    // Telnyx voice registration requires evidenced underwriting capacity.
    // M4 supplies its human-only authoring and managed call execution contract.
    persist(state, actor, &line, &account, &service).await?;
    Ok(line)
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateInput {
    pub revision: i64,
    pub regulatory: RegulatoryInput,
    pub supplier_tariffs: Vec<SupplierTariffInput>,
    pub call_slot_limit: i64,
    pub calls_per_minute: i64,
    pub sms_per_minute: i64,
}
/// Operator edit and revalidation. Provider ownership, stable URLs and the Telnyx
/// channel limit are proved again with the canonical credential before commit,
/// then underwriting capacity is revalidated. Identity, resources, capabilities
/// and billing services stay immutable; retired lines cannot be edited.
pub async fn update(
    state: &AppState,
    actor: &AuditActor,
    id: &str,
    input: UpdateInput,
) -> AppResult<ManagedPhoneLine> {
    update_with_origin(state, actor, id, input, None).await
}
pub(super) async fn update_with_origin(
    state: &AppState,
    actor: &AuditActor,
    id: &str,
    input: UpdateInput,
    fixture: Option<&str>,
) -> AppResult<ManagedPhoneLine> {
    super::require_ready(&state.db).await?;
    super::eligibility::require_admin(&state.db, actor).await?;
    let current = state
        .db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":id,"revision":input.revision})
        .await?
        .ok_or_else(|| conflict("Managed line revision changed"))?;
    if current.state == LineState::Retired {
        return Err(conflict("Retired managed lines cannot be edited"));
    }
    validate_operations(
        &input.regulatory,
        current.sms.is_some(),
        &input.supplier_tariffs,
        (
            input.call_slot_limit,
            input.calls_per_minute,
            input.sms_per_minute,
        ),
    )?;
    let account = state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":&current.trusted_account_id})
        .await?
        .ok_or_else(unavailable)?;
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&account.catalog_service_id,"is_active":true,"managed_comms_role":"credential"})
        .await?
        .ok_or_else(unavailable)?;
    resolver::fixed_service(&service, &account.provider)?;
    let key =
        crate::services::catalog_credential_record_service::text(&state.encryption_keys, &service)
            .await?;
    let now = Utc::now();
    let mut line = current.clone();
    line.regulatory = RegulatoryEvidence {
        requirement_reference: input.regulatory.requirement_reference,
        registration_status: input.regulatory.registration_status,
        sms_sender_registration: input.regulatory.sms_sender_registration,
        verified_by: actor.user_id.clone(),
        verified_at: now,
    };
    line.supplier_tariffs = input.supplier_tariffs.into_iter().map(Into::into).collect();
    line.call_slot_limit = input.call_slot_limit;
    line.calls_per_minute = input.calls_per_minute;
    line.sms_per_minute = input.sms_per_minute;
    line.provider_channel_limit = prove(state, &line, &account, &service, &key, fixture).await?;
    let db = transaction_service::durable_database(&state.db);
    let context = (
        &db,
        actor,
        &current,
        &line,
        &account,
        &service,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    transaction_service::run_db_only(&db,&context,|tx,c|Box::pin(async move{
        let &(db,actor,current,line,account,service,key)=c;
        super::eligibility::admin_fence(db,tx,actor).await?;
        // The proved account itself is unchanged: compare its authority version
        // and only touch it, so concurrent proofs and admissions are not refused.
        if db.collection::<ProviderAccount>(ACCOUNTS).update_one(doc!{"_id":&account.id,"revision":account.revision,"fence":account_fence_value(account.fence),"catalog_service_id":&service.id},doc!{"$inc":{super::admission::ADMISSION_TOUCH:1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        if db.collection::<bson::Document>(SERVICES).update_one(doc!{"_id":&service.id,"is_active":true,"managed_comms_role":"credential","updated_at":bson::DateTime::from_chrono(service.updated_at)},doc!{"$inc":{"managed_comms_fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Credential service changed during provider proof"));}
        revalidate_capacity(db,tx,account,line).await?;
        let changed=db.collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(
            doc!{"_id":&current.id,"revision":current.revision,"fence":current.fence,"state":{"$ne":"retired"}},
            doc!{"$set":{
                "regulatory":bson::to_bson(&line.regulatory).map_err(|_|unavailable())?,
                "supplier_tariffs":bson::to_bson(&line.supplier_tariffs).map_err(|_|unavailable())?,
                "call_slot_limit":line.call_slot_limit,
                "calls_per_minute":line.calls_per_minute,
                "sms_per_minute":line.sms_per_minute,
                "provider_channel_limit":bson::to_bson(&line.provider_channel_limit).map_err(|_|unavailable())?,
                "updated_at":bson::DateTime::from_chrono(line.regulatory.verified_at),
            },"$inc":{"revision":1_i64,"fence":1_i64}},
        ).session(&mut *tx).await?;
        if changed.matched_count!=1{return Err(conflict("Managed line changed during provider proof"));}
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_line_updated",serde_json::json!({"line_id":current.id,"revision":current.revision+1,"provider_channel_limit":line.provider_channel_limit})).await?;
        Ok(())
    })).await?;
    line.revision += 1;
    line.fence += 1;
    line.updated_at = now;
    Ok(line)
}
/// Platform-admin inventory page; callers project it into metadata-only DTOs.
pub(super) async fn prove(
    state: &AppState,
    line: &ManagedPhoneLine,
    account: &ProviderAccount,
    service: &DownstreamService,
    key: &str,
    fixture: Option<&str>,
) -> AppResult<Option<i64>> {
    if line.provider == "telnyx" {
        // Number/application responses do not document account identifiers.
        // Prove the key's identity explicitly, then read each exact resource
        // with that same authenticated key. The profile's numeric `id` is not
        // the account identity; only its `user_id` is authoritative here.
        let profile = provider_read(
            service,
            account,
            key,
            Profile::TelnyxRelay,
            "/v2/user_profile",
            fixture,
        )
        .await?;
        if profile["data"]["user_id"] != account.account_id {
            return Err(conflict("Provider proof must identify the trusted account"));
        }
    }
    let mut limit = None;
    for (is_sms, cap) in [(false, &line.voice), (true, &line.sms)] {
        let Some(cap) = cap else { continue };
        let profile = match (line.provider.as_str(), is_sms) {
            ("twilio", false) => Profile::TwilioRelay,
            ("twilio", true) => Profile::TwilioSms,
            ("telnyx", false) => Profile::TelnyxRelay,
            ("telnyx", true) => Profile::TelnyxSms,
            _ => return Err(unavailable()),
        };
        let paths = if line.provider == "twilio" {
            vec![
                format!(
                    "/2010-04-01/Accounts/{}/IncomingPhoneNumbers/{}.json",
                    account.account_id, line.number_resource_id
                ),
                format!(
                    "/2010-04-01/Accounts/{}/Applications/{}.json",
                    account.account_id, cap.application_id
                ),
            ]
        } else {
            vec![
                format!("/v2/phone_numbers/{}", line.number_resource_id),
                format!(
                    "/v2/{}/{}",
                    if is_sms {
                        "messaging_profiles"
                    } else {
                        "call_control_applications"
                    },
                    cap.application_id
                ),
                // The general number read contains routing associations, not
                // capability evidence. Read the capability-specific settings.
                format!(
                    "/v2/phone_numbers/{}/{}",
                    line.number_resource_id,
                    if is_sms { "messaging" } else { "voice" }
                ),
            ]
        };
        for (i, path) in paths.iter().enumerate() {
            let v = provider_read(service, account, key, profile, path, fixture).await?;
            let expected = stable_url(&state.config.base_url, &line.id, is_sms);
            if line.provider == "twilio" {
                if v["account_sid"] != account.account_id
                    || v["sid"]
                        != if i == 0 {
                            line.number_resource_id.as_str()
                        } else {
                            cap.application_id.as_str()
                        }
                {
                    return Err(unavailable());
                }
                if !is_sms && !v["voice_fallback_url"].is_null() && v["voice_fallback_url"] != "" {
                    return Err(conflict("Twilio voice fallback URL must be empty"));
                }
                if i == 0 {
                    if v["phone_number"] != line.phone_number
                        || v[if is_sms {
                            "sms_application_sid"
                        } else {
                            "voice_application_sid"
                        }] != cap.application_id
                        || v["capabilities"][if is_sms { "sms" } else { "voice" }] != true
                    {
                        return Err(unavailable());
                    }
                } else if v[if is_sms { "sms_url" } else { "voice_url" }] != expected
                    || v[if is_sms { "sms_method" } else { "voice_method" }] != "POST"
                {
                    return Err(conflict(
                        "Provider inbound URL must equal the stable managed line URL",
                    ));
                }
            } else {
                let d = &v["data"];
                if d["id"]
                    != if i == 1 {
                        cap.application_id.as_str()
                    } else {
                        line.number_resource_id.as_str()
                    }
                {
                    return Err(unavailable());
                }
                if i == 0 {
                    if d["phone_number"] != line.phone_number
                        || d["status"] != "active"
                        || d[if is_sms {
                            "messaging_profile_id"
                        } else {
                            "connection_id"
                        }] != cap.application_id
                    {
                        return Err(unavailable());
                    }
                } else if i == 1 {
                    if d[if is_sms {
                        "webhook_url"
                    } else {
                        "webhook_event_url"
                    }] != expected
                    {
                        return Err(conflict(
                            "Provider inbound URL must equal the stable managed line URL",
                        ));
                    }
                    let fallback = &d[if is_sms {
                        "webhook_failover_url"
                    } else {
                        "webhook_event_failover_url"
                    }];
                    if !fallback.is_null() && fallback != "" && fallback != &expected {
                        return Err(conflict(
                            "Provider failover URL must be empty or equal the stable managed line URL",
                        ));
                    }
                    if !is_sms {
                        limit = d
                            .pointer("/inbound/channel_limit")
                            .and_then(serde_json::Value::as_i64)
                            .filter(|n| *n > 0);
                        if limit.is_none() {
                            return Err(conflict("Telnyx inbound channel limit is required"));
                        }
                    }
                } else if d["phone_number"] != line.phone_number
                    || d[if is_sms {
                        "messaging_profile_id"
                    } else {
                        "connection_id"
                    }] != cap.application_id
                    // SMS must support receiving and replying. Missing, null
                    // and inbound-only features all fail closed.
                    || (is_sms && d["features"]["sms"]["domestic_two_way"] != true)
                {
                    return Err(conflict("Provider number capability is not available"));
                }
            }
        }
    }
    Ok(limit)
}

/// All registration/rotation proof reads use the same fixed origin, canonical
/// credential, operation policy, redirect refusal and bounded response.
async fn provider_read(
    service: &DownstreamService,
    account: &ProviderAccount,
    key: &str,
    profile: Profile,
    path: &str,
    fixture: Option<&str>,
) -> AppResult<serde_json::Value> {
    let origin = profiles::origin(profile, fixture)?;
    let base = url::Url::parse(&service.base_url).map_err(|_| unavailable())?;
    let relative = path
        .split('?')
        .next()
        .ok_or_else(unavailable)?
        .strip_prefix(base.path().trim_end_matches('/'))
        .ok_or_else(unavailable)?;
    let canonical = crate::services::proxy_authorization::CanonicalPath::from_mcp_built(relative)?;
    crate::services::proxy_authorization::authorize_proxy_operation(service, "GET", &canonical)?;
    let mut req = crate::services::proxy_service::no_redirect_http_client()
        .get(format!("{origin}{path}"))
        .timeout(std::time::Duration::from_secs(15));
    req = if account.provider == "twilio" {
        let (a, k) = protocol::twilio_key(key, &account.account_id)?;
        req.basic_auth(a.to_string(), Some(k))
    } else {
        req.bearer_auth(key)
    };
    let response = req.send().await.map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let bytes = profiles::bounded_response(response).await?;
    serde_json::from_slice(&bytes).map_err(|_| unavailable())
}

/// Account-only identity proof for rotation when no number remains in supplier
/// custody: the same fixed origin, canonical policy, redirect refusal and bound.
async fn account_identity(
    service: &DownstreamService,
    account: &ProviderAccount,
    key: &str,
    fixture: Option<&str>,
) -> AppResult<()> {
    let (profile, path, field) = match account.provider.as_str() {
        "telnyx" => (
            Profile::TelnyxRelay,
            "/v2/user_profile".to_owned(),
            "/data/user_id",
        ),
        "twilio" => (
            Profile::TwilioRelay,
            format!("/2010-04-01/Accounts/{}.json", account.account_id),
            "/sid",
        ),
        _ => return Err(unavailable()),
    };
    let value = provider_read(service, account, key, profile, &path, fixture)
        .await
        .map_err(|_| {
            conflict(
                "No number remains in supplier custody; rotation needs the provider account identity read, which failed or is not allowed by the credential service operation policy",
            )
        })?;
    if value.pointer(field).and_then(serde_json::Value::as_str) != Some(account.account_id.as_str())
    {
        return Err(conflict("Provider proof must identify the trusted account"));
    }
    Ok(())
}

/// Telnyx's list endpoint returns metadata only (identifier/id/timestamps),
/// never the integration secret value. Bound the walk; inability to prove a
/// reference fails configuration/rotation closed.
async fn integration_reference(
    service: &DownstreamService,
    account: &ProviderAccount,
    key: &str,
    reference: &str,
    fixture: Option<&str>,
) -> AppResult<()> {
    resolver::fixed_service(service, "telnyx")?;
    if account.provider != "telnyx" {
        return Err(unavailable());
    }
    let profile = provider_read(
        service,
        account,
        key,
        Profile::TelnyxRelay,
        "/v2/user_profile",
        fixture,
    )
    .await?;
    if profile["data"]["user_id"] != account.account_id {
        return Err(unavailable());
    }
    for page in 1..=10 {
        let metadata = provider_read(
            service,
            account,
            key,
            Profile::TelnyxRelay,
            &format!("/v2/integration_secrets?page[size]=100&page[number]={page}"),
            fixture,
        )
        .await?;
        let rows = metadata
            .get("data")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(unavailable)?;
        if rows
            .iter()
            .any(|r| r.get("identifier").and_then(serde_json::Value::as_str) == Some(reference))
        {
            return Ok(());
        }
        if rows.len() < 100 {
            break;
        }
    }
    Err(conflict(
        "Telnyx integration secret reference could not be proved",
    ))
}
pub(super) async fn prove_integration_reference(
    state: &AppState,
    account: &ProviderAccount,
    reference: &str,
    fixture: Option<&str>,
) -> AppResult<()> {
    let service=state.db.collection::<DownstreamService>(SERVICES).find_one(doc!{"_id":&account.catalog_service_id,"is_active":true,"managed_comms_role":"credential"}).await?.ok_or_else(unavailable)?;
    resolver::fixed_service(&service, "telnyx")?;
    let key =
        crate::services::catalog_credential_record_service::text(&state.encryption_keys, &service)
            .await?;
    integration_reference(&service, account, &key, reference, fixture).await
}

/// Check the dedicated-row invariant in the designation/rotation transaction.
/// There are deliberately no active/deleted filters: disabled connections and
/// tombstones still belong to their original users. Indirect credential and
/// agent references require a UserService/UserEndpoint, both excluded here.
pub(super) async fn dedicated_service(
    db: &Database,
    tx: &mut ClientSession,
    row: &DownstreamService,
) -> AppResult<()> {
    if row.service_category != "internal"
        || row.visibility != "private"
        || row.owner_user_id.is_some()
        || !row.anonymous_endpoints.is_empty()
        || row.provider_config_id.is_some()
    {
        return Err(conflict(
            "Managed services require dedicated private internal platform-admin rows",
        ));
    }
    // The acting, transactionally fenced Admin establishes designation provenance.
    // A creator's later demotion must never prevent credential recovery.
    // UserApiKey and AgentServiceBinding reference UserEndpoint/UserService;
    // excluding those rows covers these indirectly. UserProviderToken references
    // ProviderConfig; managed rows forbid provider linkage above.
    // Financial history and service definitions are not connections.
    for (collection, fields) in [
        (
            crate::models::user_service::COLLECTION_NAME,
            vec!["catalog_service_id"],
        ),
        (
            crate::models::user_endpoint::COLLECTION_NAME,
            vec!["catalog_service_id"],
        ),
        (
            crate::models::user_service_connection::COLLECTION_NAME,
            vec!["service_id"],
        ),
        (
            crate::models::node_service_binding::COLLECTION_NAME,
            vec!["service_id"],
        ),
        (ALLOCATIONS, vec!["catalog_service_id"]),
        (
            crate::models::api_key::COLLECTION_NAME,
            vec!["allowed_service_ids", "allowed_platform_service_ids"],
        ),
        (
            crate::models::assistant_agent::COLLECTION_NAME,
            vec![
                "grants.platform_service_ids",
                "owner_model_route.catalog_service_id",
            ],
        ),
        (
            crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME,
            vec!["catalog_service_id"],
        ),
        (
            crate::models::assistant_conversation::COLLECTION_NAME,
            vec!["owner_model_route.catalog_service_id"],
        ),
        (
            crate::models::nyxbot_channel::COLLECTION_NAME,
            vec!["org_specialist.guests.service_ids"],
        ),
        (
            crate::models::managed_execution::COLLECTION_NAME,
            vec!["catalog_service_id"],
        ),
        (
            crate::models::permission_policy::COLLECTION_NAME,
            vec!["catalog_service_id"],
        ),
        (
            crate::models::connect_link::COLLECTION_NAME,
            vec!["service_id"],
        ),
        (BINDINGS, vec!["credential.catalog_service_id"]),
        (
            crate::models::platform_bot_config::COLLECTION_NAME,
            vec!["guide.model_service_id"],
        ),
        (
            crate::models::platform_model::ROUTING_COLLECTION,
            vec!["routes.credential_service_id"],
        ),
        (
            crate::models::platform_model::COLLECTION_NAME,
            vec!["billing_service_id"],
        ),
        (
            crate::models::service_account::COLLECTION_NAME,
            vec![
                "curation_grant.service_ids",
                "curation_grant.ornn_proxy_service_id",
            ],
        ),
    ] {
        let clauses: Vec<_> = fields
            .into_iter()
            .map(|field| {
                let mut d = doc! {};
                d.insert(field, &row.id);
                d
            })
            .collect();
        if db
            .collection::<bson::Document>(collection)
            .find_one(doc! {"$or":clauses})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Err(conflict(
                "Managed service already has connection, credential or binding references",
            ));
        }
    }
    // Operation scopes are maps keyed by connection/route identity; catalog
    // IDs live in their values, so a dotted field cannot cover these bindings.
    for (collection, field) in [
        (
            crate::models::assistant_agent::COLLECTION_NAME,
            "operation_scopes",
        ),
        (
            crate::models::assistant_agent::COLLECTION_NAME,
            "guest_operation_scopes",
        ),
        (
            crate::models::api_key::COLLECTION_NAME,
            "assistant_operation_scopes",
        ),
    ] {
        let source = format!("${field}");
        if db.collection::<bson::Document>(collection).find_one(doc!{"$expr":{"$in":[&row.id,{"$map":{"input":{"$objectToArray":{"$ifNull":[source,doc!{}]}},"as":"scope","in":"$$scope.v.catalog_service_id"}}]}}).session(&mut *tx).await?.is_some() {
            return Err(conflict("Managed service already has agent operation bindings"));
        }
    }
    Ok(())
}

pub(super) async fn persist(
    state: &AppState,
    actor: &AuditActor,
    line: &ManagedPhoneLine,
    account: &ProviderAccount,
    service: &DownstreamService,
) -> AppResult<()> {
    let db = &state.db;
    super::require_ready(db).await?;
    let context = (
        db,
        actor,
        line,
        account,
        service,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    transaction_service::run_db_only(db,&context,|tx,c|Box::pin(async move{
        let &(db,actor,line,account,service,key)=c;
        super::eligibility::admin_fence(db,tx,actor).await?;
        let changed=db.collection::<ProviderAccount>(ACCOUNTS).update_one(doc!{"_id":&account.id,"revision":account.revision,"fence":account_fence_value(account.fence),"catalog_service_id":&service.id},doc!{"$inc":{super::admission::ADMISSION_TOUCH:1_i64}}).session(&mut *tx).await?;
        if changed.matched_count!=1{return Err(unavailable());}
        if db.collection::<ProviderAccount>(ACCOUNTS).find_one(doc!{"catalog_service_id":&service.id,"_id":{"$ne":&account.id}}).session(&mut *tx).await?.is_some() { return Err(conflict("Managed credentials require one canonical trusted account")); }
        revalidate_capacity(db,tx,account,line).await?;
        let mut services=vec![(service.id.as_str(),ManagedCommsRole::Credential),(line.rental_billing_service_id.as_str(),ManagedCommsRole::Rental)];
        if let Some(cap)=&line.voice{services.push((&cap.billing_service_id,ManagedCommsRole::Voice));}
        if let Some(cap)=&line.sms{services.push((&cap.billing_service_id,ManagedCommsRole::Sms));}
        let mut seen=BTreeSet::new();
        for (id,role) in services {
            if !seen.insert(id){return Err(conflict("Each managed capability requires its own billing service"));}
            let row=db.collection::<DownstreamService>(SERVICES).find_one(doc!{"_id":id,"is_active":true}).session(&mut *tx).await?.ok_or_else(unavailable)?;
            dedicated_service(db,tx,&row).await?;
            if row.platform_key.as_ref().is_some_and(|p|p.enabled)||row.managed_comms_role.is_some_and(|r|r!=role)|| (role!=ManagedCommsRole::Credential&&(!row.credential_encrypted.is_empty()||row.provider_config_id.is_some()||row.token_exchange_config.is_some())) {return Err(conflict("Managed services cannot expose generic platform keys or billing credentials"));}
            if role==ManagedCommsRole::Credential && (row.updated_at!=service.updated_at || row.credential_encrypted!=service.credential_encrypted){return Err(conflict("Credential service changed during provider proof"));}
            db.collection::<DownstreamService>(SERVICES).update_one(doc!{"_id":id},doc!{"$set":{"managed_comms_role":bson::to_bson(&role).map_err(|_|unavailable())?},"$inc":{"managed_comms_fence":1_i64}}).session(&mut *tx).await?;
        }
        for (kind,id) in resources(line){
            let field = match kind {"number_id"=>"resource_id","phone"=>"phone_number",_=>"application_id"};
            let mut allocation=doc!{"provider":&line.provider}; allocation.insert(field,id);
            let mut binding=doc!{"profile":{"$in":if line.provider=="twilio" {vec!["twilio_relay","twilio_sms"]} else {vec!["telnyx_relay","telnyx_sms"]}}}; binding.insert(field,id);
            if db.collection::<bson::Document>(ALLOCATIONS).find_one(allocation).session(&mut *tx).await?.is_some() || db.collection::<bson::Document>(BINDINGS).find_one(binding).session(&mut *tx).await?.is_some() {
                return Err(conflict("Number or application is already used by a legacy allocation or binding"));
            }
            let identity=session::identity(&[&line.provider,kind,id]);
            if db.collection::<ResourceClaim>(CLAIMS).find_one(doc!{"identity_key":&identity}).session(&mut *tx).await?.is_some(){return Err(conflict("Number or application already has an owner"));}
            db.collection::<ResourceClaim>(CLAIMS).insert_one(ResourceClaim{id:uuid::Uuid::new_v4().to_string(),identity_key:identity,revision:1,user_id:sentinel(&line.id),provider:line.provider.clone(),resource_kind:kind.into(),resource_id:id.into(),created_at:Utc::now()}).session(&mut *tx).await?;
        }
        db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME).insert_one(line).session(&mut *tx).await?;
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_line_registered",serde_json::json!({"line_id":line.id,"trusted_account_id":account.id,"has_voice":line.voice.is_some(),"has_sms":line.sms.is_some()})).await?;
        Ok(())
    })).await
}

pub async fn account_dependencies(
    db: &Database,
    tx: &mut ClientSession,
    id: &str,
) -> AppResult<()> {
    if db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"trusted_account_id":id})
        .session(&mut *tx)
        .await?
        .is_some()
        || db
            .collection::<bson::Document>(ManagedEffectRoute::COLLECTION_NAME)
            .find_one(doc! {"trusted_account_id":id,"financially_finalized":false})
            .session(&mut *tx)
            .await?
            .is_some()
    {
        return Err(conflict(
            "Managed lines or unfinished effects depend on this trusted account",
        ));
    }
    Ok(())
}
pub async fn validate_catalog_update(
    db: &Database,
    tx: &mut ClientSession,
    live: &DownstreamService,
    fields: &bson::Document,
) -> AppResult<()> {
    let mut candidate = bson::to_document(live).map_err(|_| unavailable())?;
    candidate.extend(fields.clone());
    let next: DownstreamService = bson::from_document(candidate).map_err(|_| unavailable())?;
    if next.managed_comms_role != live.managed_comms_role
        || (next.managed_comms_role.is_some()
            && next.platform_key.as_ref().is_some_and(|p| p.enabled))
    {
        return Err(conflict(
            "Managed role is server controlled and cannot enable platform access",
        ));
    }
    if live.managed_comms_role.is_none() {
        return Ok(());
    }
    super::require_ready(db).await?;
    dedicated_service(db, tx, &next).await?;
    if live.managed_comms_role != Some(ManagedCommsRole::Credential)
        && (!next.credential_encrypted.is_empty()
            || next.provider_config_id.is_some()
            || next.token_exchange_config.is_some())
    {
        return Err(conflict("Managed billing services contain prices only"));
    }
    let dependencies=db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME).find_one(doc!{"$or":[{"rental_billing_service_id":&live.id},{"voice.billing_service_id":&live.id},{"sms.billing_service_id":&live.id}]}).session(&mut *tx).await?.is_some()
        || db.collection::<ProviderAccount>(ACCOUNTS).find_one(doc!{"catalog_service_id":&live.id}).session(&mut *tx).await?.is_some()
        || db.collection::<ManagedVoiceAccount>(ManagedVoiceAccount::COLLECTION_NAME).find_one(doc!{"premium.billing_service_id":&live.id}).session(&mut *tx).await?.is_some()
        || db.collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME).find_one(doc!{"premium.rate.service_id":&live.id,"financially_finalized":false}).session(&mut *tx).await?.is_some();
    if dependencies
        && (!next.is_active
            || [
                "credential_encrypted",
                "base_url",
                "auth_method",
                "auth_key_name",
                "auth_scheme",
                "destination_targets",
                "proxy_operation_policy",
                "provider_config_id",
                "token_exchange_config",
                "forward_access_token",
                "inject_delegation_token",
                "identity_propagation_mode",
                "default_request_headers",
                "owner_user_id",
                "created_by",
                "secret_context_versions",
                "ws_frame_injections",
            ]
            .iter()
            .any(|field| {
                fields.get(*field).is_some_and(|value| {
                    bson::to_document(live)
                        .ok()
                        .and_then(|doc| doc.get(*field).cloned())
                        .as_ref()
                        != Some(value)
                })
            }))
    {
        return Err(conflict(
            "Managed dependencies require the dedicated verified account or line operation",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotationInput {
    pub expected_revision: i64,
    #[serde(deserialize_with = "secret_input")]
    pub credential: zeroize::Zeroizing<String>,
}
fn secret_input<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<zeroize::Zeroizing<String>, D::Error> {
    String::deserialize(d).map(zeroize::Zeroizing::new)
}
impl std::fmt::Debug for RotationInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RotationInput { [REDACTED] }")
    }
}
/// Re-prove every retained line with the successor credential before committing.
/// No account/service identity, reference, policy or destination can change here.
pub async fn rotate(
    state: &AppState,
    actor: &AuditActor,
    id: &str,
    input: RotationInput,
) -> AppResult<()> {
    rotate_with_origin(state, actor, id, input, None).await
}
pub(super) async fn rotate_with_origin(
    state: &AppState,
    actor: &AuditActor,
    id: &str,
    input: RotationInput,
    fixture: Option<&str>,
) -> AppResult<()> {
    super::require_ready(&state.db).await?;
    use crate::services::catalog_credential_record_service as storage;
    use futures::TryStreamExt;
    super::eligibility::require_admin(&state.db, actor).await?;
    if input.credential.is_empty() || input.credential.len() > 65_536 {
        return Err(unavailable());
    }
    let account = state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":id,"revision":input.expected_revision})
        .await?
        .ok_or_else(unavailable)?;
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&account.catalog_service_id,"managed_comms_role":"credential"})
        .await?
        .ok_or_else(unavailable)?;
    resolver::fixed_service(&service, &account.provider)?;
    let lines: Vec<ManagedPhoneLine> = state
        .db
        .collection(ManagedPhoneLine::COLLECTION_NAME)
        .find(custody_proof_filter(id))
        .limit(1001)
        .await?
        .try_collect()
        .await?;
    if lines.len() > 1000 {
        return Err(conflict(
            "Verified rotation requires a bounded retained inventory proof",
        ));
    }
    if lines.is_empty() {
        // Every number was released by the supplier: prove the account itself.
        account_identity(&service, &account, &input.credential, fixture).await?;
    }
    for line in &lines {
        let capacity = prove(state, line, &account, &service, &input.credential, fixture).await?;
        if capacity != line.provider_channel_limit {
            return Err(conflict(
                "Provider capacity changed; reconcile underwriting before rotation",
            ));
        }
    }
    if let Some(config) = super::synthesis::config(&state.db, id).await?
        && let Some(premium) = config.premium
    {
        integration_reference(
            &service,
            &account,
            &input.credential,
            &premium.api_key_ref,
            fixture,
        )
        .await?;
    }
    // Call/message GETs verify the successor can read every unfinished effect;
    // cleanup keeps the retained canonical account and fixed provider API origin.
    let routes: Vec<ManagedEffectRoute> = state
        .db
        .collection(ManagedEffectRoute::COLLECTION_NAME)
        .find(doc! {"trusted_account_id":id,"financially_finalized":false})
        .limit(1001)
        .await?
        .try_collect()
        .await?;
    if routes.len() > 1000 {
        return Err(conflict(
            "Too many pending effects for one verified rotation",
        ));
    }
    for route in &routes {
        resolver::prove_cleanup_credential(&route.admitted, &input.credential, fixture).await?;
    }
    let ciphertext = storage::encrypt(
        &state.encryption_keys,
        &service.id,
        storage::owner(&service)?,
        input.credential.as_bytes(),
    )
    .await?;
    let versions = storage::initial_versions(&state.encryption_keys, true);
    let context = (
        &state.db,
        actor,
        &account,
        &service,
        &lines,
        &ciphertext,
        &versions,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    transaction_service::run_db_only(&state.db,&context,|tx,c|Box::pin(async move {
        let &(db,actor,account,service,lines,ciphertext,versions,key)=c;
        super::eligibility::admin_fence(db,tx,actor).await?;
        let changed=db.collection::<ProviderAccount>(ACCOUNTS).update_one(doc!{"_id":&account.id,"revision":account.revision,"fence":account_fence_value(account.fence),"catalog_service_id":&service.id},doc!{"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        if changed.matched_count!=1{return Err(unavailable());}
        let live=db.collection::<DownstreamService>(SERVICES).find_one(doc!{"_id":&service.id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        dedicated_service(db,tx,&live).await?;
        for line in lines {
            for id in std::iter::once(&line.rental_billing_service_id).chain(line.voice.iter().chain(line.sms.iter()).map(|cap| &cap.billing_service_id)) {
                let billing=db.collection::<DownstreamService>(SERVICES).find_one(doc!{"_id":id}).session(&mut *tx).await?.ok_or_else(unavailable)?;
                dedicated_service(db,tx,&billing).await?;
                db.collection::<DownstreamService>(SERVICES).update_one(doc!{"_id":id},doc!{"$inc":{"managed_comms_fence":1_i64}}).session(&mut *tx).await?;
            }
        }
        // Registration and all effect admissions contend on the account row.
        // Bind to exactly the inventory and service revision that were proved.
        if db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME).count_documents(custody_proof_filter(&account.id)).session(&mut *tx).await?!=lines.len() as u64 {return Err(conflict("Inventory changed during credential proof"));}
        for line in lines {
            if db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":&line.id,"revision":line.revision},doc!{"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Line changed during credential proof"));}
        }
        let changed=db.collection::<bson::Document>(SERVICES).update_one(doc!{"_id":&service.id,"updated_at":bson::DateTime::from_chrono(service.updated_at),"credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:service.credential_encrypted.clone()}},doc!{"$set":{"credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext.clone()},"secret_context_versions":bson::to_bson(versions).map_err(|_|unavailable())?,"updated_at":bson::DateTime::now()},"$inc":{"managed_comms_fence":1_i64,"authority_fence":1_i64}}).session(&mut *tx).await?;
        if changed.matched_count!=1{return Err(conflict("Canonical credential changed during proof"));}
        audit_service::log_actor_event_in_session(db,tx,key,actor,"managed_credential_rotated",serde_json::json!({"trusted_account_id":account.id,"catalog_service_id":service.id,"proved_lines":lines.len()})).await?;
        Ok(())
    })).await
}

pub(super) fn worst_case_exposure(
    p: &ManagedUnderwriting,
) -> AppResult<crate::models::credits::Credits> {
    use crate::models::credits::Credits;
    if !p.enabled
        || p.accepted_by.is_empty()
        || p.acceptance_version.is_empty()
        || p.paid_tier_evidence_reference.is_empty()
        || p.max_leg_evidence_reference.is_empty()
        || p.max_leg_seconds < 1800
        || p.max_leg_seconds > 86_400
        || p.detail_record_fields.is_empty()
        || p.supplier_tariff_revision <= 0
        || p.fx_policy_revision.is_empty()
        || p.revision <= 0
        || p.rolling_budget <= Credits::ZERO
        || p.components.len() > 16
    {
        return Err(conflict(
            "Evidenced paid-tier Telnyx underwriting is required",
        ));
    }
    let mut total = Credits::ZERO;
    let duration = p.max_leg_seconds - 1740;
    for required in [CostKind::CarrierInbound, CostKind::Relay, CostKind::Stt] {
        if p.components.iter().filter(|c| c.kind == required).count() != 1 {
            return Err(unavailable());
        }
    }
    for c in &p.components {
        if !matches!(
            c.kind,
            CostKind::CarrierInbound | CostKind::Relay | CostKind::Stt
        ) || c.credits_per_second < Credits::ZERO
            || c.increment_seconds <= 0
            || c.increment_seconds > 86_400
            || c.minimum_seconds < 0
            || c.minimum_seconds > 86_400
        {
            return Err(unavailable());
        }
        let n = duration.max(c.minimum_seconds);
        let rounded = ((n + c.increment_seconds - 1) / c.increment_seconds) * c.increment_seconds;
        total = total.checked_add(c.credits_per_second.checked_mul(rounded)?)?;
    }
    Ok(total)
}
async fn revalidate_capacity(
    db: &Database,
    tx: &mut ClientSession,
    account: &ProviderAccount,
    candidate: &ManagedPhoneLine,
) -> AppResult<()> {
    if account.provider != "telnyx" || candidate.voice.is_none() {
        return Ok(());
    }
    let policy = account.underwriting.as_ref().ok_or_else(|| {
        conflict("Telnyx voice requires evidenced underwriting before registration")
    })?;
    let exposure = worst_case_exposure(policy)?;
    let mut channels = candidate
        .provider_channel_limit
        .filter(|n| *n > 0)
        .ok_or_else(unavailable)?;
    let mut cursor = db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        // The candidate is counted once: registration has not inserted it yet
        // and an operator edit replaces its persisted channel limit.
        .find({
            let mut filter = capacity_filter(&account.id);
            filter.insert("_id", doc! {"$ne":&candidate.id});
            filter
        })
        .session(&mut *tx)
        .await?;
    while let Some(line) = cursor.next(&mut *tx).await.transpose()? {
        channels = channels
            .checked_add(
                line.provider_channel_limit
                    .filter(|n| *n > 0)
                    .ok_or_else(unavailable)?,
            )
            .ok_or_else(unavailable)?;
    }
    if exposure.checked_mul(channels)? > policy.rolling_budget {
        return Err(conflict(
            "Registered channel limits exceed the underwriting budget",
        ));
    }
    Ok(())
}
