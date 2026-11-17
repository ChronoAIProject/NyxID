//! Stable, account-authenticated ingress. Routing identities outlive tenant rows.
use super::super::{
    binding, coordinator, profiles,
    protocol::{self, Ingress},
    session,
};
use super::{conflict, unavailable};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{channel_adapter::*, managed_comms::*},
};
use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde_json::{Value, json};

#[derive(Clone)]
pub(in crate::services::call_channels) struct EventIdentity {
    pub occurred_at: DateTime<Utc>,
    pub occurrence_verified: bool,
    pub deliverable: bool,
    pub call_leg_id: Option<String>,
    pub call_session_id: Option<String>,
}
impl std::fmt::Debug for EventIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedEventIdentity { [REDACTED] }")
    }
}
impl EventIdentity {
    pub fn reference(
        &self,
        l: &ManagedPhoneLine,
        a: &ProviderAccount,
        call: Option<&str>,
        message: Option<&str>,
    ) -> AppResult<AdmittedReference> {
        let cap = if message.is_some() {
            l.sms.as_ref()
        } else {
            l.voice.as_ref()
        }
        .ok_or_else(unavailable)?;
        if l.provider == "telnyx"
            && call.is_some()
            && (self.call_leg_id.is_none() || self.call_session_id.is_none())
        {
            return Err(unavailable());
        }
        Ok(AdmittedReference {
            trusted_account_id: a.id.clone(),
            credential_service_id: a.catalog_service_id.clone(),
            provider: l.provider.clone(),
            account_id: a.account_id.clone(),
            line_id: l.id.clone(),
            number_resource_id: l.number_resource_id.clone(),
            application_id: cap.application_id.clone(),
            provider_call_id: call.map(str::to_owned),
            provider_message_id: message.map(str::to_owned),
            call_leg_id: self.call_leg_id.clone(),
            call_session_id: self.call_session_id.clone(),
        })
    }
}
pub fn url(state: &AppState, line: &str, sms: bool) -> String {
    format!(
        "{}/api/v1/webhooks/managed-lines/{line}/{}",
        state.config.base_url.trim_end_matches('/'),
        if sms { "sms" } else { "voice" }
    )
}
fn invalid() -> AppError {
    AppError::ChannelWebhookVerificationFailed("Managed line provider verification failed".into())
}
fn date(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .or_else(|_| DateTime::parse_from_rfc2822(s))
        .ok()
        .map(|t| t.with_timezone(&Utc))
}
/// Only used for protocol parsing and exact-account cleanup; never persisted as a tenant.
pub(super) fn parser_binding(
    l: &ManagedPhoneLine,
    a: &ProviderAccount,
    sms: bool,
) -> AppResult<Binding> {
    let cap = if sms {
        l.sms.as_ref()
    } else {
        l.voice.as_ref()
    }
    .ok_or_else(unavailable)?;
    let profile = match (l.provider.as_str(), sms) {
        ("twilio", false) => Profile::TwilioRelay,
        ("twilio", true) => Profile::TwilioSms,
        ("telnyx", false) => Profile::TelnyxRelay,
        ("telnyx", true) => Profile::TelnyxSms,
        _ => return Err(unavailable()),
    };
    Ok(Binding {
        id: l.id.clone(),
        user_id: String::new(),
        profile,
        contract_version: 1,
        revision: l.revision,
        label: String::new(),
        agent_id: String::new(),
        credential: CredentialBinding::ManagedLine {
            line_id: l.id.clone(),
            managed_claim_id: String::new(),
            managed_claim_generation: 0,
        },
        account_id: a.account_id.clone(),
        resource_id: l.number_resource_id.clone(),
        phone_number: l.phone_number.clone(),
        application_id: cap.application_id.clone(),
        webhook_public_key: a.webhook_public_key.clone(),
        voice: String::new(),
        language: String::new(),
        enabled: false,
        verified_credential_identity: String::new(),
        trusted_account_revision: Some(a.revision),
        verified_at: Utc::now(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    })
}
pub(super) async fn rate(
    db: &Database,
    tx: &mut ClientSession,
    l: &ManagedPhoneLine,
    kind: &str,
    limit: i64,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if limit <= 0 {
        return Err(conflict("Managed line rate limit reached"));
    }
    // Caller has already written the shared line fence. Rolling windows are durable
    // and contenders on other replicas cannot both observe an unspent last slot.
    let key = format!("managed-rate:{kind}:{}", l.id);
    let rows = db.collection::<ManagedLineRateWindow>(ManagedLineRateWindow::COLLECTION_NAME);
    let old = rows.find_one(doc! {"key":&key}).session(&mut *tx).await?;
    let mut events = old.map(|row| row.events).unwrap_or_default();
    events.retain(|t| t.to_chrono() > now - Duration::minutes(1));
    if events.len() >= limit as usize {
        return Err(conflict("Managed line rate limit reached"));
    }
    events.push(bson::DateTime::from_chrono(now));
    rows.update_one(doc!{"key":&key},doc!{"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"key":&key},"$set":{"events":events,"expires_at":bson::DateTime::from_chrono(now+Duration::minutes(2))}}).upsert(true).session(&mut *tx).await?;
    Ok(())
}
pub(super) async fn current(
    state: &AppState,
    l: &ManagedPhoneLine,
    sms: bool,
    at: DateTime<Utc>,
) -> AppResult<Binding> {
    let claim = state
        .db
        .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .find_one(doc! {"_id":l.current_claim_id.as_deref().ok_or_else(unavailable)?})
        .await?
        .ok_or_else(unavailable)?;
    if at < claim.covered_from {
        return Err(unavailable());
    }
    let id = if sms {
        claim.sms_binding_id.as_deref()
    } else {
        claim.voice_binding_id.as_deref()
    }
    .ok_or_else(unavailable)?;
    let b = binding::load(state, id).await?;
    binding::live(state, &b).await?;
    Ok(b)
}
async fn orphan(
    state: &AppState,
    l: &ManagedPhoneLine,
    a: &ProviderAccount,
    id: &EventIdentity,
    call: &str,
) -> AppResult<()> {
    let reference = id.reference(l, a, Some(call), None)?;
    let r = ManagedEffectRoute {
        id: uuid::Uuid::new_v4().to_string(),
        trusted_account_id: a.id.clone(),
        provider: l.provider.clone(),
        provider_effect_id: call.into(),
        line_id: l.id.clone(),
        managed_claim_id: None,
        managed_claim_generation: None,
        binding_id: None,
        effect_id: uuid::Uuid::new_v4().to_string(),
        kind: EffectKind::Call,
        financially_finalized: true,
        admitted: reference,
        covered_from: id.occurred_at,
        covered_until: None,
        created_at: Utc::now(),
    };
    state
        .db
        .collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
        .update_one(
            doc! {"trusted_account_id":&a.id,"provider":&l.provider,"provider_effect_id":call},
            doc! {"$setOnInsert":bson::to_document(&r).map_err(|_|unavailable())?},
        )
        .upsert(true)
        .await?;
    Ok(())
}
async fn reject(
    state: &AppState,
    b: &Binding,
    a: &ProviderAccount,
    call: &str,
) -> AppResult<String> {
    if b.profile == Profile::TwilioRelay {
        return Ok("<Response><Reject/></Response>".into());
    }
    let key = super::resolver::existing_authentication(
        state,
        &AdmittedReference {
            trusted_account_id: a.id.clone(),
            credential_service_id: a.catalog_service_id.clone(),
            provider: a.provider.clone(),
            account_id: a.account_id.clone(),
            line_id: String::new(),
            number_resource_id: String::new(),
            application_id: String::new(),
            provider_call_id: None,
            provider_message_id: None,
            call_leg_id: None,
            call_session_id: None,
        },
    )
    .await?;
    let path = format!(
        "/v2/calls/{}/actions/reject",
        protocol::call_segment(b.profile, call)?
    );
    let response=crate::services::proxy_service::no_redirect_http_client().post(profiles::binding_url(b,&path)?).bearer_auth(key.key.as_str()).json(&json!({"cause":"CALL_REJECTED","command_id":uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL,format!("managed-reject:{}:{call}",a.id).as_bytes()).to_string()})).timeout(std::time::Duration::from_secs(10)).send().await.map_err(|_|unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    Ok(String::new())
}
pub fn receive<'a>(
    state: AppState,
    line: &'a str,
    sms: bool,
    headers: &'a HeaderMap,
    body: &'a [u8],
) -> futures::future::BoxFuture<'a, AppResult<String>> {
    Box::pin(receive_inner(state, line, sms, headers, body))
}
async fn receive_inner(
    state: AppState,
    line: &str,
    sms: bool,
    headers: &HeaderMap,
    body: &[u8],
) -> AppResult<String> {
    if body.len() > protocol::MAX_INGRESS {
        return Err(invalid());
    }
    let l = state
        .db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":line})
        .await?
        .ok_or_else(invalid)?;
    let a = state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":&l.trusted_account_id,"provider":&l.provider})
        .await?
        .ok_or_else(invalid)?;
    let parser = parser_binding(&l, &a, sms)?;
    let authentication = super::resolver::existing_authentication(
        &state,
        &AdmittedReference {
            trusted_account_id: a.id.clone(),
            credential_service_id: a.catalog_service_id.clone(),
            provider: l.provider.clone(),
            account_id: a.account_id.clone(),
            line_id: l.id.clone(),
            number_resource_id: l.number_resource_id.clone(),
            application_id: parser.application_id.clone(),
            provider_call_id: None,
            provider_message_id: None,
            call_leg_id: None,
            call_session_id: None,
        },
    )
    .await?;
    // No request host, forwarded URL or alias may change this exact configured URL.
    let (event, mut id) = if l.provider == "twilio" {
        let fields = protocol::form(body)?;
        let (_, secret) = protocol::twilio_key(&authentication.key, &a.account_id)?;
        protocol::verify_twilio(headers, &url(&state, line, sms), &fields, secret)?;
        let mut content_fields = fields.clone();
        let deliverable = !sms
            || (fields.get("NumMedia").is_none_or(|v| v == "0")
                && fields
                    .get("Body")
                    .is_some_and(|s| !s.is_empty() && s.len() <= 1600));
        if sms
            && fields
                .get("SmsStatus")
                .or(fields.get("MessageStatus"))
                .is_some_and(|s| s == "received")
            && !deliverable
        {
            content_fields.insert("Body".into(), "[discarded]".into());
            content_fields.insert("NumMedia".into(), "0".into());
        }
        let event = protocol::twilio(&parser, &content_fields)?;
        let at = fields.get("Timestamp").and_then(|v| date(v));
        (
            event,
            EventIdentity {
                occurred_at: at.unwrap_or_else(Utc::now),
                occurrence_verified: at.is_some(),
                deliverable,
                call_leg_id: None,
                call_session_id: None,
            },
        )
    } else {
        protocol::verify_telnyx(
            headers,
            body,
            authentication
                .webhook_public_key
                .as_deref()
                .ok_or_else(invalid)?,
            Utc::now().timestamp(),
        )?;
        let mut raw: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
        let deliverable = !sms
            || (raw
                .pointer("/data/payload/media")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
                && raw
                    .pointer("/data/payload/text")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty() && s.len() <= 1600));
        if sms && raw["data"]["event_type"] == "message.received" && !deliverable {
            raw["data"]["payload"]["text"] = json!("[discarded]");
            raw["data"]["payload"]["media"] = json!([]);
        }
        let normalized = serde_json::to_vec(&raw).map_err(|_| invalid())?;
        let at = raw
            .pointer("/data/occurred_at")
            .and_then(Value::as_str)
            .and_then(date)
            .ok_or_else(invalid)?;
        let handle = |field: &str| {
            raw.pointer(&format!("/data/payload/{field}"))
                .and_then(Value::as_str)
                .filter(|s| protocol::handle(s))
                .map(str::to_owned)
        };
        let mut event = protocol::telnyx(&parser, &normalized)?;
        if let Ingress::CallEnded { at, .. } = &mut event {
            *at = raw
                .pointer("/data/payload/end_time")
                .and_then(Value::as_str)
                .and_then(date);
        }
        (
            event,
            EventIdentity {
                occurred_at: at,
                occurrence_verified: true,
                deliverable,
                call_leg_id: handle("call_leg_id"),
                call_session_id: handle("call_session_id"),
            },
        )
    };
    let effect = match &event {
        Ingress::Call { call_id, .. }
        | Ingress::CallAnswered { call_id, .. }
        | Ingress::CallEnded { call_id, .. }
        | Ingress::RelayEnded { call_id, .. } => Some(call_id),
        Ingress::Sms { message_id, .. } | Ingress::Receipt { message_id, .. } => Some(message_id),
        Ingress::Ignore => None,
    };
    let Some(effect) = effect.cloned() else {
        return Ok(String::new());
    };
    // A send acknowledgement can precede creation of its routing tombstone.
    // Recover only an exact retained send, never the line's current claimant.
    if matches!(event, Ingress::Receipt { .. })
        && let Some(sent) = state
            .db
            .collection::<EventReceipt>(EVENTS)
            .find_one(doc! {"event_kind":"sms_send","provider_message_id":&effect,
            "admitted_reference.trusted_account_id":&a.id,"admitted_reference.line_id":&l.id})
            .await?
    {
        super::super::sms_delivery::reconcile(&state, &sent.id).await?;
    }
    let route = state
        .db
        .collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
        .find_one(
            doc! {"trusted_account_id":&a.id,"provider":&l.provider,"provider_effect_id":&effect},
        )
        .await?;
    if let Some(route) = &route {
        if route.line_id != l.id {
            return Err(invalid());
        }
        if route.financially_finalized && !matches!(event, Ingress::Receipt { .. }) {
            if matches!(event, Ingress::Call { .. }) && route.binding_id.is_none() {
                return reject(&state, &parser, &a, &effect).await;
            }
            return Ok(if sms {
                "<Response/>".into()
            } else {
                String::new()
            });
        }
        if let Some(binding_id) = &route.binding_id
            && let Ok(b) = binding::load(&state, binding_id).await
        {
            if matches!(event, Ingress::Sms { .. }) {
                super::sms::recover_one(&state, &route.effect_id).await?;
                return Ok("<Response/>".into());
            }
            if !sms {
                if matches!(event, Ingress::Call { .. }) && binding::live(&state, &b).await.is_err()
                {
                    if let Ok(s) = session::load(&state, &route.effect_id).await {
                        let _ = coordinator::close(&state, &b, &s, false).await;
                    }
                    return Ok(String::new());
                }
                if let Ok(s) = session::load(&state, &route.effect_id).await {
                    super::calls::observe(&state, &s, &id, &event).await?;
                }
                if matches!(event, Ingress::CallEnded { .. }) {
                    return Ok(String::new());
                }
            }
            return coordinator::dispatch_verified(state, b, event, Some(id)).await;
        }
        // Retained financial state never recreates erased sessions/bindings.
        return Ok(String::new());
    }
    // Twilio initial requests omit Timestamp. Read provider creation evidence;
    // inability to establish occurrence quarantines the effect before tenant use.
    if !id.occurrence_verified {
        let reference = id.reference(
            &l,
            &a,
            if sms { None } else { Some(&effect) },
            if sms { Some(&effect) } else { None },
        )?;
        let op = if sms {
            super::resolver::CleanupOperation::ReadMessage
        } else {
            super::resolver::CleanupOperation::ReadCall
        };
        if let Ok(v) = super::resolver::request_existing(
            &state,
            &parser,
            &reference,
            op,
            Some(&profiles::binding_origin(&parser)?),
        )
        .await
        {
            let matches =
                v["account_sid"] == a.account_id && v["sid"] == effect && v["to"] == l.phone_number;
            if matches && let Some(at) = v["date_created"].as_str().and_then(date) {
                id.occurred_at = at;
                id.occurrence_verified = true;
            }
        }
    }
    if id.occurred_at > Utc::now() + Duration::minutes(5) {
        id.occurrence_verified = false;
    }
    if let Ingress::Sms {
        message_id,
        from,
        to,
        text,
        parts,
        ..
    } = event
    {
        return super::sms::ingest(&state, &l, &a, &id, &message_id, &from, &to, &text, parts)
            .await;
    }
    if let Ingress::Call { call_id, .. } = &event {
        let call_id = call_id.clone();
        if id.occurrence_verified
            && let Ok(b) = current(&state, &l, false, id.occurred_at).await
        {
            match coordinator::dispatch_verified(state.clone(), b, event, Some(id.clone())).await {
                Ok(result) => return Ok(result),
                Err(error) => {
                    tracing::info!(line_id=%l.id,"Managed call admission or dispatch refused");
                    // An admitted answer may be uncertain: retain its recovery route and
                    // terminate rather than substituting a new rejected identity.
                    if let Some(route) = state
                        .db
                        .collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
                        .find_one(doc! {"trusted_account_id":&a.id,"provider_effect_id":&effect})
                        .await?
                        && let Ok(s) = session::load(&state, &route.effect_id).await
                        && let Ok(b) = binding::load(&state, &s.binding_id).await
                    {
                        let _ = coordinator::close(&state, &b, &s, false).await;
                    }
                    // Persist rejection even if funding/capacity changes before a replay.
                    // Upsert cannot overwrite an admission that committed concurrently.
                    orphan(&state, &l, &a, &id, &call_id).await?;
                    let _ = error;
                    return reject(&state, &parser, &a, &call_id).await;
                }
            }
        }
        orphan(&state, &l, &a, &id, &call_id).await?;
        return reject(&state, &parser, &a, &call_id).await;
    }
    // Unknown terminal/receipt events get metadata-only quarantine tombstones.
    if !sms {
        orphan(&state, &l, &a, &id, &effect).await?;
    } else {
        state.db.collection::<ManagedLineQuarantine>(ManagedLineQuarantine::COLLECTION_NAME).update_one(
            doc! {"trusted_account_id":&a.id,"provider_effect_id":&effect,"line_id":&l.id},
            doc! {"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),
                "trusted_account_id":&a.id,"provider_effect_id":&effect,"line_id":&l.id,
                "kind":"unknown_sms_receipt","occurred_at":bson::DateTime::from_chrono(id.occurred_at)}}
        ).upsert(true).await?;
    }
    Ok(String::new())
}
