//! Provider wire formats and authentication; untrusted bodies never grant authority.
use crate::{
    errors::{AppError, AppResult},
    models::channel_adapter::{Binding, Profile},
};
use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

pub const MAX_INGRESS: usize = 65536;
pub const MAX_FRAME: usize = 16384;
fn invalid() -> AppError {
    AppError::ChannelWebhookVerificationFailed("Provider request verification failed".into())
}
pub fn phone(s: &str) -> bool {
    s.starts_with('+') && (8..=16).contains(&s.len()) && s[1..].bytes().all(|c| c.is_ascii_digit())
}
pub fn handle(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b':'))
}
pub fn call_handle(profile: Profile, s: &str) -> bool {
    if profile == Profile::TelnyxRelay {
        let unpadded = s.trim_end_matches('=');
        !unpadded.is_empty()
            && s.len() <= 512
            && s.len() - unpadded.len() <= 2
            && unpadded
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b':'))
    } else {
        handle(s)
    }
}
pub fn call_segment(profile: Profile, s: &str) -> AppResult<String> {
    if !call_handle(profile, s) {
        return Err(invalid());
    }
    Ok(s.replace('=', "%3D").replace(':', "%3A"))
}
/// Public Twilio routing identity, distinct from the credential's auth token.
/// Only a canonical AC-prefixed 128-bit SID can become an account path segment;
/// rendering preserves the exact path used by operation authorization.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TwilioAccountSid(u128);
impl TwilioAccountSid {
    pub fn parse(value: &str) -> AppResult<Self> {
        let digits = value.strip_prefix("AC").ok_or_else(invalid)?;
        if digits.len() != 32
            || !digits
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(invalid());
        }
        u128::from_str_radix(digits, 16)
            .map(Self)
            .map_err(|_| invalid())
    }
}
impl std::fmt::Display for TwilioAccountSid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AC{:032x}", self.0)
    }
}
pub fn twilio_key<'a>(raw: &'a str, account: &str) -> AppResult<(TwilioAccountSid, &'a str)> {
    let sid = TwilioAccountSid::parse(account)?;
    let (id, secret) = raw.split_once(':').ok_or_else(invalid)?;
    if TwilioAccountSid::parse(id)? != sid || secret.is_empty() || secret.contains(':') {
        return Err(invalid());
    }
    Ok((sid, secret))
}
pub fn form(body: &[u8]) -> AppResult<BTreeMap<String, String>> {
    if body.len() > MAX_INGRESS || std::str::from_utf8(body).is_err() {
        return Err(invalid());
    }
    let mut result = BTreeMap::new();
    for (k, v) in url::form_urlencoded::parse(body) {
        if k.len() > 128
            || v.len() > 8192
            || result.insert(k.into_owned(), v.into_owned()).is_some()
        {
            return Err(invalid());
        }
    }
    Ok(result)
}
pub fn verify_twilio(
    headers: &HeaderMap,
    external_url: &str,
    fields: &BTreeMap<String, String>,
    secret: &str,
) -> AppResult<()> {
    let signature = headers
        .get("x-twilio-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(invalid)?;
    let signature = STANDARD.decode(signature).map_err(|_| invalid())?;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes()).map_err(|_| invalid())?;
    mac.update(external_url.as_bytes());
    for (k, v) in fields {
        mac.update(k.as_bytes());
        mac.update(v.as_bytes());
    }
    mac.verify_slice(&signature).map_err(|_| invalid())
}

/// The endpoint comes from server configuration. Preserve every byte except
/// the upgrade scheme; never derive an alternate URL from request headers.
pub fn verify_twilio_upgrade(
    headers: &HeaderMap,
    websocket_url: &str,
    secret: &str,
) -> AppResult<()> {
    let http_url = Zeroizing::new(if let Some(rest) = websocket_url.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = websocket_url.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        return Err(invalid());
    });
    let fields = BTreeMap::new();
    verify_twilio(headers, websocket_url, &fields, secret)
        .or_else(|_| verify_twilio(headers, &http_url, &fields, secret))
}
pub fn validate_public_key(public_key: &str) -> AppResult<()> {
    let public: [u8; 32] = STANDARD
        .decode(public_key)
        .map_err(|_| invalid())?
        .try_into()
        .map_err(|_| invalid())?;
    ed25519_dalek::VerifyingKey::from_bytes(&public).map_err(|_| invalid())?;
    Ok(())
}
pub fn verify_telnyx(
    headers: &HeaderMap,
    body: &[u8],
    public_key: &str,
    now: i64,
) -> AppResult<()> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    if body.len() > MAX_INGRESS {
        return Err(invalid());
    }
    let ts = headers
        .get("telnyx-timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(invalid)?;
    let timestamp: i64 = ts.parse().map_err(|_| invalid())?;
    if now.abs_diff(timestamp) > 300 {
        return Err(invalid());
    }
    let public: [u8; 32] = STANDARD
        .decode(public_key)
        .map_err(|_| invalid())?
        .try_into()
        .map_err(|_| invalid())?;
    let key = VerifyingKey::from_bytes(&public).map_err(|_| invalid())?;
    let bytes = STANDARD
        .decode(
            headers
                .get("telnyx-signature-ed25519")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(invalid)?,
        )
        .map_err(|_| invalid())?;
    let signature = Signature::from_slice(&bytes).map_err(|_| invalid())?;
    let mut signed = Vec::with_capacity(body.len() + ts.len() + 1);
    signed.extend_from_slice(ts.as_bytes());
    signed.push(b'|');
    signed.extend_from_slice(body);
    key.verify(&signed, &signature).map_err(|_| invalid())
}

pub enum Ingress {
    Call {
        event_id: String,
        call_id: String,
        from: String,
        to: String,
    },
    RelayEnded {
        event_id: String,
        call_id: String,
    },
    CallAnswered {
        event_id: String,
        call_id: String,
        at: Option<DateTime<Utc>>,
    },
    CallEnded {
        event_id: String,
        call_id: String,
        at: Option<DateTime<Utc>>,
        seconds: Option<i64>,
    },
    Sms {
        event_id: String,
        message_id: String,
        from: String,
        to: String,
        text: String,
        parts: i64,
    },
    Receipt {
        event_id: String,
        message_id: String,
        from: String,
        to: String,
        status: String,
        parts: Option<i64>,
    },
    Ignore,
}
impl std::fmt::Debug for Ingress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VerifiedProviderIngress { [REDACTED] }")
    }
}
fn str_field<'a>(v: &'a Value, field: &str) -> AppResult<&'a str> {
    v.get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or_else(invalid)
}
fn get<'a>(v: &'a BTreeMap<String, String>, key: &str) -> AppResult<&'a str> {
    v.get(key).map(String::as_str).ok_or_else(invalid)
}

fn timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|s| s.with_timezone(&Utc))
}

pub fn telnyx(b: &Binding, body: &[u8]) -> AppResult<Ingress> {
    let v: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    let data = v.get("data").ok_or_else(invalid)?;
    let payload = data.get("payload").ok_or_else(invalid)?;
    let event_id = str_field(data, "id")?.to_owned();
    match str_field(data, "event_type")? {
        "call.initiated" if b.profile == Profile::TelnyxRelay => {
            if str_field(payload, "connection_id")? != b.application_id
                || str_field(payload, "to")? != b.phone_number
                || str_field(payload, "direction")? != "incoming"
            {
                return Err(invalid());
            }
            let from = str_field(payload, "from")?;
            if !phone(from) {
                return Err(invalid());
            }
            Ok(Ingress::Call {
                event_id,
                call_id: str_field(payload, "call_control_id")?.into(),
                from: from.into(),
                to: b.phone_number.clone(),
            })
        }
        "call.answered" if b.profile == Profile::TelnyxRelay => {
            if str_field(payload, "connection_id")? != b.application_id {
                return Err(invalid());
            }
            Ok(Ingress::CallAnswered {
                event_id,
                call_id: str_field(payload, "call_control_id")?.into(),
                at: timestamp(data.get("occurred_at")),
            })
        }
        "call.conversation.ended" if b.profile == Profile::TelnyxRelay => Ok(Ingress::RelayEnded {
            event_id,
            call_id: str_field(payload, "call_control_id")?.into(),
        }),
        "call.hangup" if b.profile == Profile::TelnyxRelay => {
            if payload.get("connection_id").is_some()
                && str_field(payload, "connection_id")? != b.application_id
            {
                return Err(invalid());
            }
            Ok(Ingress::CallEnded {
                event_id,
                call_id: str_field(payload, "call_control_id")?.into(),
                at: timestamp(payload.get("end_time"))
                    .or_else(|| timestamp(data.get("occurred_at"))),
                seconds: None,
            })
        }
        kind @ ("message.received" | "message.finalized") if b.profile == Profile::TelnyxSms => {
            if str_field(payload, "messaging_profile_id")? != b.application_id {
                return Err(invalid());
            }
            let from = payload
                .pointer("/from/phone_number")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let to = payload
                .get("to")
                .and_then(Value::as_array)
                .filter(|v| v.len() == 1)
                .and_then(|v| v.first())
                .ok_or_else(invalid)?;
            let to_phone = str_field(to, "phone_number")?;
            if !phone(from)
                || !phone(to_phone)
                || (kind == "message.received" && to_phone != b.phone_number)
                || (kind == "message.finalized" && from != b.phone_number)
            {
                return Err(invalid());
            }
            let message_id = str_field(payload, "id")?.into();
            let parts = payload
                .get("parts")
                .and_then(Value::as_i64)
                .filter(|n| (1..=16).contains(n));
            if kind == "message.finalized" {
                return Ok(Ingress::Receipt {
                    event_id,
                    message_id,
                    from: from.into(),
                    to: to_phone.into(),
                    status: str_field(to, "status")?.into(),
                    parts,
                });
            }
            if payload
                .get("media")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
            {
                return Err(AppError::ChannelMediaUnsupported);
            }
            let text = payload
                .get("text")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty() && t.len() <= 1600)
                .ok_or_else(invalid)?;
            Ok(Ingress::Sms {
                event_id,
                message_id,
                from: from.into(),
                to: to_phone.into(),
                text: text.into(),
                parts: parts.ok_or_else(invalid)?,
            })
        }
        _ => Ok(Ingress::Ignore),
    }
}

pub fn twilio(b: &Binding, fields: &BTreeMap<String, String>) -> AppResult<Ingress> {
    if get(fields, "AccountSid")? != b.account_id {
        return Err(invalid());
    }
    if b.profile == Profile::TwilioRelay {
        let call = get(fields, "CallSid")?;
        if !handle(call) {
            return Err(invalid());
        }
        let from = get(fields, "From")?;
        let to = get(fields, "To")?;
        if !phone(from) || to != b.phone_number {
            return Err(invalid());
        }
        let status = get(fields, "CallStatus")?;
        if matches!(
            status,
            "completed" | "failed" | "busy" | "no-answer" | "canceled"
        ) {
            return Ok(Ingress::CallEnded {
                event_id: format!("{call}:{status}"),
                call_id: call.into(),
                at: fields
                    .get("Timestamp")
                    .and_then(|s| DateTime::parse_from_rfc2822(s).ok())
                    .map(|s| s.with_timezone(&Utc)),
                seconds: fields
                    .get("CallDuration")
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|s| *s >= 0 && *s <= 86400),
            });
        }
        if get(fields, "Direction")? != "inbound" {
            return Err(invalid());
        }
        return Ok(Ingress::Call {
            event_id: format!("{call}:start"),
            call_id: call.into(),
            from: from.into(),
            to: to.into(),
        });
    }
    if b.profile != Profile::TwilioSms {
        return Err(invalid());
    }
    let message_id = get(fields, "MessageSid")?;
    if !handle(message_id) {
        return Err(invalid());
    }
    let status = get(fields, "MessageStatus").or_else(|_| get(fields, "SmsStatus"))?;
    let parts = fields
        .get("NumSegments")
        .and_then(|n| n.parse::<i64>().ok())
        .filter(|n| (1..=16).contains(n));
    if status != "received" {
        // Twilio status callbacks may omit From/To/NumSegments. Correlate the
        // authenticated message SID with our durable send before projecting it.
        let from = fields.get("From").map(String::as_str).unwrap_or_default();
        let to = fields.get("To").map(String::as_str).unwrap_or_default();
        if (!from.is_empty() && from != b.phone_number) || (!to.is_empty() && !phone(to)) {
            return Err(invalid());
        }
        return Ok(Ingress::Receipt {
            event_id: format!("{message_id}:{status}"),
            message_id: message_id.into(),
            from: from.into(),
            to: to.into(),
            status: status.into(),
            parts,
        });
    }
    let from = get(fields, "From")?;
    let to = get(fields, "To")?;
    if !phone(from) || !phone(to) {
        return Err(invalid());
    }
    if to != b.phone_number || fields.get("NumMedia").is_some_and(|n| n != "0") {
        return Err(invalid());
    }
    let text = get(fields, "Body")?;
    if text.is_empty() || text.len() > 1600 {
        return Err(invalid());
    }
    Ok(Ingress::Sms {
        event_id: message_id.into(),
        message_id: message_id.into(),
        from: from.into(),
        to: to.into(),
        text: text.into(),
        parts: parts.ok_or_else(invalid)?,
    })
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum RelayFrame {
    #[serde(rename = "setup")]
    Setup {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "accountSid")]
        account_id: String,
        #[serde(rename = "callSid")]
        call_id: String,
        from: String,
        to: String,
        direction: String,
    },
    #[serde(rename = "prompt")]
    Prompt {
        #[serde(rename = "voicePrompt")]
        text: String,
        last: bool,
    },
    #[serde(rename = "interrupt")]
    Interrupt {
        #[serde(rename = "utteranceUntilInterrupt")]
        heard_prefix: String,
        #[serde(rename = "durationUntilInterruptMs")]
        duration_ms: u64,
    },
    #[serde(rename = "dtmf")]
    Dtmf { digit: String },
    #[serde(rename = "error")]
    Error {},
    #[serde(rename = "eagerEndOfTurn")]
    Eager {},
    #[serde(rename = "turnResumed")]
    Resumed {},
}
impl std::fmt::Debug for RelayFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RelayFrame { [REDACTED] }")
    }
}
pub fn relay_frame(bytes: &[u8]) -> AppResult<RelayFrame> {
    if bytes.len() > MAX_FRAME {
        return Err(AppError::RequestBodyTooLarge {
            max_bytes: MAX_FRAME,
            context: "Relay frame".into(),
        });
    }
    serde_json::from_slice(bytes).map_err(|_| AppError::BadRequest("Invalid relay frame".into()))
}
pub fn text_frame(text: &str, last: bool) -> Value {
    json!({"type":"text","token":text,"last":last})
}
pub fn end_frame() -> Value {
    json!({"type":"end","handoffData":"{\"reason\":\"nyxid_session_closed\"}"})
}
pub fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn twiml(url: &str, voice: &str, language: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Response><Connect><ConversationRelay url=\"{}\" ttsProvider=\"Google\" voice=\"{}\" language=\"{}\" interruptible=\"any\" dtmfDetection=\"false\" /></Connect><Hangup /></Response>",
        xml(url),
        xml(voice),
        xml(language)
    )
}
