//! Fixed-origin, exact-policy carrier operations, with provider resource proof.
use super::{
    binding::{self, Resolved},
    profiles, protocol,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::channel_adapter::*,
};
use serde_json::{Value, json};

/// New effects must use current transport authority and policy after all proof
/// and billing awaits. Cleanup remains restricted to the durable admitted call.
async fn dispatch_authority(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    s: &CallSession,
) -> AppResult<Resolved> {
    if s.binding_id != b.id
        || s.binding_revision != b.revision
        || s.credential_identity != r.identity
    {
        return Err(AppError::AssistantTurnRequired);
    }
    super::session::fence(state, s).await?;
    let live = binding::live(state, b).await?;
    if live.identity != s.credential_identity {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(live)
}
pub async fn request(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> AppResult<Value> {
    let encoded = body
        .as_ref()
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid provider operation".into()))?;
    binding::authorize(
        state,
        b,
        r,
        method,
        path.split('?').next().unwrap_or(path),
        encoded.as_deref(),
    )
    .await?;
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|_| AppError::BadRequest("Invalid method".into()))?;
    let mut request = crate::services::proxy_service::no_redirect_http_client()
        .request(method, profiles::binding_url(b, path)?)
        .timeout(std::time::Duration::from_secs(15));
    request = if profiles::provider(b.profile) == "twilio" {
        let (a, k) = protocol::twilio_key(&r.key, &b.account_id)?;
        request.basic_auth(a.to_string(), Some(k))
    } else {
        request.bearer_auth(r.key.as_str())
    };
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|_| AppError::ChannelPlatformError("Carrier outcome uncertain".into()))?;
    let ok = response.status().is_success();
    let bytes = profiles::bounded_response(response).await?;
    if !ok {
        return Err(AppError::ChannelPlatformError(
            "Carrier command refused or uncertain".into(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| AppError::ChannelPlatformError("Carrier outcome uncertain".into()))
}
pub async fn prove_call(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    call: &str,
    from: &str,
    to: &str,
) -> AppResult<bool> {
    if !protocol::call_handle(b.profile, call) || to != b.phone_number || !protocol::phone(from) {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "Call resource mismatch".into(),
        ));
    }
    let mut active = true;
    let valid = if b.profile == Profile::TelnyxRelay {
        // This documented endpoint proves call membership in the exclusively
        // allocated application, independently of webhook-supplied call IDs.
        let base = format!("/v2/connections/{}/active_calls", b.application_id);
        let mut path = base.clone();
        let mut found = false;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..20 {
            if !seen.insert(path.clone()) {
                break;
            }
            let v = request(state, b, r, "GET", &path, None).await?;
            let rows = v["data"]
                .as_array()
                .ok_or(AppError::VoiceProviderUnavailable)?;
            if rows.iter().any(|row| row["call_control_id"] == call) {
                found = true;
                break;
            }
            let Some(next) = v
                .pointer("/meta/next")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                break;
            };
            let parsed = url::Url::parse(&format!("https://api.telnyx.com{next}"))
                .map_err(|_| AppError::VoiceProviderUnavailable)?;
            if parsed.path() != base
                || next.len() > 4096
                || !next.starts_with(&format!("{base}?"))
                || parsed.query_pairs().any(|(k, v)| {
                    !matches!(k.as_ref(), "page[after]" | "page[size]") || v.len() > 2048
                })
            {
                return Err(AppError::VoiceProviderUnavailable);
            }
            path = next.into();
        }
        found
    } else if b.profile == Profile::TwilioRelay {
        let v = request(
            state,
            b,
            r,
            "GET",
            &format!("/2010-04-01/Accounts/{}/Calls/{call}.json", b.account_id),
            None,
        )
        .await?;
        active = v["status"] == "in-progress";
        v["sid"] == call
            && v["phone_number_sid"] == b.resource_id
            && v["account_sid"] == b.account_id
            && v["from"] == from
            && v["to"] == to
            && v["direction"] == "inbound"
    } else {
        false
    };
    if !valid {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "Call does not belong to the allocated provider resource".into(),
        ));
    }
    Ok(active)
}
pub async fn telnyx_command(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    s: &CallSession,
    operation: &str,
    mut payload: Value,
) -> AppResult<()> {
    if b.profile != Profile::TelnyxRelay
        || !matches!(operation, "answer" | "conversation_relay_start" | "hangup")
    {
        return Err(AppError::ChannelPlatformSendUnsupported);
    }
    // Re-prove resource membership at the effect boundary, including cleanup.
    prove_call(state, b, r, &s.provider_call_id, &s.from, &s.to).await?;
    payload["command_id"] = json!(
        uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("{}:{operation}", s.id).as_bytes()
        )
        .to_string()
    );
    let live;
    let r = if operation == "hangup" {
        r
    } else {
        live = dispatch_authority(state, b, r, s).await?;
        &live
    };
    request(
        state,
        b,
        r,
        "POST",
        &format!(
            "/v2/calls/{}/actions/{operation}",
            protocol::call_segment(b.profile, &s.provider_call_id)?
        ),
        Some(payload),
    )
    .await?;
    Ok(())
}

pub async fn twilio_update(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    s: &CallSession,
    fields: &[(&str, &str)],
) -> AppResult<()> {
    if b.profile != Profile::TwilioRelay {
        return Err(AppError::ChannelPlatformSendUnsupported);
    }
    let active = prove_call(state, b, r, &s.provider_call_id, &s.from, &s.to).await?;
    if fields.iter().any(|(name, _)| *name == "TimeLimit") && !active {
        return Err(AppError::VoiceProviderUnavailable);
    }
    let sid = protocol::TwilioAccountSid::parse(&b.account_id)?;
    let path = format!(
        "/2010-04-01/Accounts/{}/Calls/{}.json",
        sid, s.provider_call_id
    );
    let encoded: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    let live;
    let r = if fields == [("Status", "completed")] {
        r
    } else {
        live = dispatch_authority(state, b, r, s).await?;
        &live
    };
    binding::authorize(state, b, r, "POST", &path, Some(encoded.as_bytes())).await?;
    let (a, key) = protocol::twilio_key(&r.key, &b.account_id)?;
    let response = crate::services::proxy_service::no_redirect_http_client()
        .post(profiles::binding_url(b, &path)?)
        .basic_auth(a.to_string(), Some(key))
        .form(fields)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| AppError::VoiceProviderUnavailable)?;
    if !response.status().is_success() {
        return Err(AppError::VoiceProviderUnavailable);
    }
    let bytes = profiles::bounded_response(response).await?;
    let confirmed: Value =
        serde_json::from_slice(&bytes).map_err(|_| AppError::VoiceProviderUnavailable)?;
    if confirmed["sid"] != s.provider_call_id || confirmed["account_sid"] != b.account_id {
        return Err(AppError::VoiceProviderUnavailable);
    }
    Ok(())
}

/// Reads are constrained to the durable admitted call, including after disable.
pub async fn terminal_status(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    s: &CallSession,
) -> AppResult<Option<(chrono::DateTime<chrono::Utc>, i64)>> {
    if s.binding_id != b.id || s.to != b.phone_number {
        return Err(AppError::AssistantTurnRequired);
    }
    if b.profile == Profile::TwilioRelay {
        let v = request(
            state,
            b,
            r,
            "GET",
            &format!(
                "/2010-04-01/Accounts/{}/Calls/{}.json",
                b.account_id, s.provider_call_id
            ),
            None,
        )
        .await?;
        if v["sid"] != s.provider_call_id
            || v["account_sid"] != b.account_id
            || v["phone_number_sid"] != b.resource_id
            || v["from"] != s.from
            || v["to"] != s.to
        {
            return Err(AppError::AssistantTurnRequired);
        }
        if !matches!(
            v["status"].as_str(),
            Some("completed" | "failed" | "busy" | "no-answer" | "canceled")
        ) {
            return Ok(None);
        }
        let at = v["end_time"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
            .map(|s| s.with_timezone(&chrono::Utc));
        let seconds = v["duration"].as_str().and_then(|s| s.parse::<i64>().ok());
        if matches!(
            v["status"].as_str(),
            Some("failed" | "busy" | "no-answer" | "canceled")
        ) {
            return Ok(Some((
                at.unwrap_or_else(chrono::Utc::now),
                seconds.unwrap_or(0),
            )));
        }
        return Ok(at.zip(seconds));
    }
    let v = request(
        state,
        b,
        r,
        "GET",
        &format!(
            "/v2/calls/{}",
            protocol::call_segment(b.profile, &s.provider_call_id)?
        ),
        None,
    )
    .await?;
    let v = &v["data"];
    if v["call_control_id"] != s.provider_call_id || v["is_alive"] != false {
        return Ok(None);
    }
    let at = v["end_time"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|s| s.with_timezone(&chrono::Utc));
    Ok(at.zip(v["call_duration"].as_i64()))
}

/// API evidence for billable SMS parts; webhook keys supplied by an owner are
/// transport authentication, never independent price/quantity attestation.
pub async fn message_parts(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    id: &str,
    from: &str,
    to: &str,
    inbound: bool,
) -> AppResult<i64> {
    if !protocol::handle(id) {
        return Err(AppError::AssistantTurnRequired);
    }
    let (path, twilio) = if b.profile == Profile::TwilioSms {
        (
            format!("/2010-04-01/Accounts/{}/Messages/{id}.json", b.account_id),
            true,
        )
    } else if b.profile == Profile::TelnyxSms {
        (format!("/v2/messages/{id}"), false)
    } else {
        return Err(AppError::ChannelPlatformSendUnsupported);
    };
    let v = request(state, b, r, "GET", &path, None).await?;
    let parts = if twilio {
        if v["sid"] != id || v["account_sid"] != b.account_id || v["from"] != from || v["to"] != to
        {
            return Err(AppError::AssistantTurnRequired);
        }
        v["num_segments"]
            .as_str()
            .and_then(|v| v.parse::<i64>().ok())
    } else {
        let v = &v["data"];
        if v["id"] != id
            || v["messaging_profile_id"] != b.application_id
            || v["from"]["phone_number"] != from
            || v["to"]
                .as_array()
                .is_none_or(|items| items.len() != 1 || items[0]["phone_number"] != to)
            || v["direction"] != if inbound { "inbound" } else { "outbound" }
        {
            return Err(AppError::AssistantTurnRequired);
        }
        v["parts"].as_i64()
    };
    parts
        .filter(|v| (1..=16).contains(v))
        .ok_or(AppError::VoiceProviderUnavailable)
}
