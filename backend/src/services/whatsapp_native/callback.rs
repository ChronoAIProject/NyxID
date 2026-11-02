//! Live Meta callback proof. A flag change or an owner checkbox is not proof.
use crate::{
    errors::AppResult,
    models::channel_whatsapp_native::{CallbackPhase, NativeWhatsapp},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
#[cfg(test)]
use std::collections::HashMap;
use std::{sync::LazyLock, time::Duration};
use zeroize::Zeroizing;

const LIMIT: usize = 256 * 1024;
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .expect("Meta callback client")
});

#[cfg(test)]
pub(crate) static TEST_GRAPH_BASES: LazyLock<std::sync::Mutex<HashMap<String, String>>> =
    LazyLock::new(Default::default);

pub(crate) fn graph_endpoint(app_id: &str, path: &str) -> String {
    #[cfg(test)]
    if let Some(base) = TEST_GRAPH_BASES.lock().unwrap().get(app_id).cloned() {
        return format!(
            "{}/{}/{}",
            base.trim_end_matches('/'),
            crate::services::channel_adapters::whatsapp::GRAPH_API_VERSION,
            path.trim_start_matches('/')
        );
    }
    #[cfg(not(test))]
    let _ = app_id;
    format!(
        "https://graph.facebook.com/{}/{}",
        crate::services::channel_adapters::whatsapp::GRAPH_API_VERSION,
        path.trim_start_matches('/')
    )
}

async fn response_json(mut response: reqwest::Response) -> Result<Value, ()> {
    if !response.status().is_success()
        || response.content_length().is_some_and(|n| n > LIMIT as u64)
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len().saturating_add(chunk.len()) > LIMIT {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice::<Value>(&bytes).map_err(|_| ())
}

pub struct CallbackProof {
    app_id: String,
    url: String,
    pub(crate) checked_at: DateTime<Utc>,
}
impl std::fmt::Debug for CallbackProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CallbackProof([REDACTED])")
    }
}

/// Prove the effective callback for this phone. The app subscription list is
/// checked together with the phone configuration and WABA override so a stale
/// app-level row cannot mask a phone-specific callback elsewhere.
pub async fn verify(
    app_id: &str,
    app_secret: &str,
    system_token: &str,
    phone_number_id: &str,
    waba_id: &str,
    expected_url: &str,
) -> AppResult<CallbackProof> {
    if !super::numeric_id(app_id) || app_secret.is_empty() || system_token.is_empty() {
        return Err(super::refusal("whatsapp_credentials_required"));
    }
    let parsed =
        url::Url::parse(expected_url).map_err(|_| super::refusal("callback_url_invalid"))?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(super::refusal("callback_url_invalid"));
    }
    let token = Zeroizing::new(format!("{app_id}|{app_secret}"));
    let subscriptions = graph_get(
        app_id,
        token.as_str(),
        &format!("{app_id}/subscriptions"),
        &[],
    )
    .await?;
    let phone = graph_get(
        app_id,
        system_token,
        phone_number_id,
        &[("fields", "webhook_configuration".into())],
    )
    .await?;
    // Meta's phone response is authoritative for callback precedence. The
    // WABA read cross-checks an override reported by that response, even when
    // a phone override takes precedence over it.
    let (_, waba_callback, _) = webhook_configuration(&phone["webhook_configuration"])
        .ok_or_else(|| super::refusal("native_whatsapp_callback_unverified"))?;
    let subscribed_apps = if waba_callback.is_some() {
        graph_get(
            app_id,
            system_token,
            &format!("{waba_id}/subscribed_apps"),
            &[],
        )
        .await?
    } else {
        Value::Null
    };
    proof(
        app_id,
        expected_url,
        &subscriptions,
        &phone,
        &subscribed_apps,
    )
}

/// Configure Meta after CMA returns its per-channel callback URL. The token is
/// sent in the form body for Meta's GET handshake and never appears in a URL.
pub async fn configure_subscription(
    app_id: &str,
    app_secret: &str,
    callback_url: &str,
    verify_token: &str,
) -> AppResult<()> {
    let parsed =
        url::Url::parse(callback_url).map_err(|_| super::refusal("callback_url_invalid"))?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || verify_token.is_empty()
    {
        return Err(super::refusal("callback_url_invalid"));
    }
    let token = Zeroizing::new(format!("{app_id}|{app_secret}"));
    let response = tokio::time::timeout(
        Duration::from_secs(15),
        CLIENT
            .post(graph_endpoint(app_id, &format!("{app_id}/subscriptions")))
            .bearer_auth(token.as_str())
            .form(&[
                ("object", "whatsapp_business_account"),
                ("callback_url", callback_url),
                ("verify_token", verify_token),
                ("fields", "messages"),
            ])
            .send(),
    )
    .await
    .map_err(|_| super::refusal("callback_configuration_unavailable"))?
    .map_err(|_| super::refusal("callback_configuration_unavailable"))?;
    response_json(response)
        .await
        .map(|_| ())
        .map_err(|_| super::refusal("callback_configuration_failed"))
}

async fn graph_get(
    app_id: &str,
    bearer: &str,
    path: &str,
    query: &[(&str, String)],
) -> AppResult<Value> {
    let response = tokio::time::timeout(
        Duration::from_secs(15),
        CLIENT
            .get(graph_endpoint(app_id, path))
            .bearer_auth(bearer)
            .query(query)
            .send(),
    )
    .await
    .map_err(|_| super::refusal("callback_verification_unavailable"))?
    .map_err(|_| super::refusal("callback_verification_unavailable"))?;
    response_json(response)
        .await
        .map_err(|_| super::refusal("callback_verification_unavailable"))
}

fn proof(
    app_id: &str,
    expected_url: &str,
    subscriptions: &Value,
    phone: &Value,
    subscribed_apps: &Value,
) -> AppResult<CallbackProof> {
    let (effective, waba_callback, source) = webhook_configuration(&phone["webhook_configuration"])
        .ok_or_else(|| super::refusal("native_whatsapp_callback_unverified"))?;
    if effective != expected_url {
        return Err(super::refusal("native_whatsapp_callback_unverified"));
    }
    let matching = subscriptions["data"].as_array().is_some_and(|items| {
        items.iter().take(100).any(|item| {
            item["object"] == "whatsapp_business_account"
                && item["active"] == true
                && (source != CallbackSource::Application
                    || item["callback_url"].as_str() == Some(expected_url))
                && item["fields"].as_array().is_some_and(|fields| {
                    fields.iter().any(|field| {
                        field.as_str() == Some("messages") || field["name"] == "messages"
                    })
                })
        })
    });
    if !matching {
        return Err(super::refusal("native_whatsapp_callback_unverified"));
    }
    if let Some(expected_waba_callback) = waba_callback {
        let actual_waba_callback = subscribed_apps_override(subscribed_apps)
            .map_err(|_| super::refusal("native_whatsapp_callback_unverified"))?;
        if actual_waba_callback.is_some_and(|actual| actual != expected_waba_callback) {
            return Err(super::refusal("native_whatsapp_callback_unverified"));
        }
    }
    Ok(CallbackProof {
        app_id: app_id.to_owned(),
        url: expected_url.to_owned(),
        checked_at: Utc::now(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CallbackSource {
    Phone,
    Waba,
    Application,
}

/// Meta resolves a phone's callback as phone override, WABA override, then
/// the app callback. Empty strings mean that override is absent.
fn webhook_configuration(value: &Value) -> Option<(&str, Option<&str>, CallbackSource)> {
    let object = value.as_object()?;
    let phone = optional_callback(object.get("phone_number"))?;
    let waba = optional_callback(object.get("whatsapp_business_account"))?;
    let application = optional_callback(object.get("application"))?;
    phone
        .map(|url| (url, waba, CallbackSource::Phone))
        .or_else(|| waba.map(|url| (url, Some(url), CallbackSource::Waba)))
        .or_else(|| application.map(|url| (url, waba, CallbackSource::Application)))
}

// The outer None means malformed. Never mistake an invalid high-priority
// override for an absent one and fall back to a lower-priority callback.
fn optional_callback(value: Option<&Value>) -> Option<Option<&str>> {
    match value {
        None => Some(None),
        Some(Value::String(value)) => Some((!value.is_empty()).then_some(value.as_str())),
        Some(_) => None,
    }
}

fn subscribed_apps_override(value: &Value) -> Result<Option<&str>, ()> {
    let Some(data) = value["data"].as_array() else {
        return Err(());
    };
    let mut found = None;
    for item in data {
        let item = item.as_object().ok_or(())?;
        if let Some(url) = optional_callback(item.get("override_callback_uri")).ok_or(())? {
            if found.is_some_and(|existing| existing != url) {
                return Err(());
            }
            found = Some(url);
        }
    }
    Ok(found)
}

fn fresh(config: &NativeWhatsapp, proof: &CallbackProof, url: &str) -> bool {
    proof.app_id == config.meta_app_id
        && proof.url == url
        && (0..=60).contains(&(Utc::now() - proof.checked_at).num_seconds())
}

/// Only after remote preparation; this resets evidence for this exact attempt.
pub fn prepare(
    config: &mut NativeWhatsapp,
    channel_id: String,
    webhook_url: String,
) -> AppResult<()> {
    if config.callback.phase != CallbackPhase::Relay || channel_id.is_empty() {
        return Err(super::refusal("native_whatsapp_preparation_conflict"));
    }
    config.callback = crate::models::channel_whatsapp_native::CallbackState {
        phase: CallbackPhase::AwaitingGateway,
        gateway_channel_id: Some(channel_id),
        gateway_webhook_url: Some(webhook_url),
        transition_started_at: Some(Utc::now()),
        ..Default::default()
    };
    Ok(())
}

/// Call only after the relay adapter verified the raw POST HMAC. A signed
/// private message is routing evidence; receipts and status events are not.
pub fn observe_relay_intake(
    config: &mut NativeWhatsapp,
    bot: &crate::models::channel_bot::ChannelBot,
    payload: &Value,
) -> bool {
    if config.callback.phase != CallbackPhase::AwaitingRelay {
        return false;
    }
    let seen = relay_message_evidence(config, bot, payload);
    if seen {
        config.callback.intake_seen_at = Some(Utc::now());
        config.callback.relay_observed_at = Some(Utc::now());
    }
    seen
}

/// Detect a signed relay message in any native phase. Gateway-phase callers
/// record a diagnostic and drop it explicitly; awaiting-relay callers admit it.
pub fn relay_message_evidence(
    config: &NativeWhatsapp,
    bot: &crate::models::channel_bot::ChannelBot,
    payload: &Value,
) -> bool {
    if !super::matches_bot(config, bot) || payload["object"] != "whatsapp_business_account" {
        return false;
    }
    payload["entry"].as_array().into_iter().flatten().any(|entry| {
        entry["id"].as_str() == Some(config.waba_id.as_str())
            && entry["changes"].as_array().into_iter().flatten().any(|change| {
                let value = &change["value"];
                change["field"] == "messages" && value["messaging_product"] == "whatsapp"
                    && value["metadata"]["phone_number_id"].as_str() == Some(config.phone_number_id.as_str())
                    && value["messages"].as_array().into_iter().flatten().any(|message| {
                        message["type"] == "text" && message["group_id"].is_null()
                            && message["recipient_type"] != "group"
                            && message["from"].as_str().is_some_and(super::numeric_id)
                            && message["id"].as_str().is_some_and(crate::services::channel_delivery_service::valid_message_id)
                            && message["timestamp"].as_str().and_then(|s| s.parse::<i64>().ok()).is_some()
                    })
            })
    })
}

/// Apply only inside the shared, fenced build-beside/swap transaction.
pub fn confirm_gateway(config: &mut NativeWhatsapp, proof: &CallbackProof) -> AppResult<()> {
    if config.callback.phase != CallbackPhase::AwaitingGateway
        || config
            .callback
            .gateway_channel_id
            .as_deref()
            .is_none_or(str::is_empty)
        || !config
            .callback
            .gateway_webhook_url
            .as_deref()
            .is_some_and(|url| fresh(config, proof, url))
    {
        return Err(super::refusal("native_callback_not_verified"));
    }
    config.callback.phase = CallbackPhase::Gateway;
    config.callback.gateway_verified_at = Some(proof.checked_at);
    config.callback.relay_verified_at = None;
    Ok(())
}

/// Flag-off starts restoration; it cannot delete the still-live native intake.
pub fn request_restore(config: &mut NativeWhatsapp) -> AppResult<()> {
    if !matches!(
        config.callback.phase,
        CallbackPhase::Gateway | CallbackPhase::AwaitingRelay
    ) {
        return Err(super::refusal("native_callback_not_active"));
    }
    if config.callback.phase != CallbackPhase::AwaitingRelay {
        config.callback.phase = CallbackPhase::AwaitingRelay;
        config.callback.transition_started_at = Some(Utc::now());
        config.callback.intake_seen_at = None;
        config.callback.relay_observed_at = None;
    }
    Ok(())
}

/// The caller supplies NyxID's server-generated relay URL, never owner input.
/// Remote release/key cleanup follows this proof in the shared migration protocol.
pub fn confirm_relay(
    config: &mut NativeWhatsapp,
    proof: &CallbackProof,
    relay_url: &str,
) -> AppResult<()> {
    if config.callback.phase != CallbackPhase::AwaitingRelay || !fresh(config, proof, relay_url) {
        return Err(super::refusal("relay_callback_not_verified"));
    }
    config.callback.phase = CallbackPhase::Relay;
    config.callback.relay_verified_at = Some(proof.checked_at);
    config.callback.gateway_verified_at = None;
    config.callback.relay_observed_at = None;
    config.callback.relay_token_replaced_at = None;
    config.callback.last_error = None;
    Ok(())
}

/// This gates message dispatch only. Awaiting relay admits signed messages so
/// they can prove Meta has already returned to NyxID's callback.
pub fn suppress_relay_messages(config: &NativeWhatsapp) -> bool {
    matches!(
        config.callback.phase,
        CallbackPhase::Gateway | CallbackPhase::Unknown
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn relay_evidence_is_admitted_during_rollback_and_fences_native_intake() {
        let bot = super::super::tests::bot();
        let mut config = super::super::tests::config();
        config.bot_id = bot.id.clone();
        config.owner_id = bot.user_id.clone();
        config.callback.phase = CallbackPhase::Relay;
        prepare(
            &mut config,
            "channel".into(),
            "https://gateway.example/v1/whatsapp/channel".into(),
        )
        .unwrap();
        // Preparing a callback must not pause the still-working relay. Native
        // preparation records evidence only; shared cutover admits its turns.
        assert!(!suppress_relay_messages(&config));
        config.callback.phase = CallbackPhase::Gateway;
        request_restore(&mut config).unwrap();
        assert!(config.callback.intake_seen_at.is_none());
        let mut body = json!({"object":"whatsapp_business_account", "entry":[{"id":"222", "changes":[{
            "field":"messages", "value":{"messaging_product":"whatsapp", "metadata":{"phone_number_id":"333"},
            "messages":[{"type":"text", "id":"wamid.restore", "from":"15551234567", "timestamp":Utc::now().timestamp().to_string()}]}}]}]});
        assert!(observe_relay_intake(&mut config, &bot, &body));
        assert!(config.callback.relay_observed_at.is_some());
        config.callback.phase = CallbackPhase::Gateway;
        assert!(relay_message_evidence(&config, &bot, &body));
        // Gateway-phase relay deliveries are diagnosed and dropped explicitly;
        // they must never enter the native admission path by accident.
        assert!(suppress_relay_messages(&config));
        assert!(
            super::super::verified_account(
                &config,
                &bot,
                "channel",
                &super::super::tests::context()
            )
            .is_none()
        );
        body["entry"][0]["changes"][0]["value"]["messages"][0]["group_id"] = json!("123");
        assert!(!observe_relay_intake(&mut config, &bot, &body));
        body["entry"][0]["changes"][0]["value"]["messages"][0]["group_id"] = Value::Null;
        body["entry"][0]["changes"][0]["value"]["messages"][0]["timestamp"] = json!("bad");
        assert!(!observe_relay_intake(&mut config, &bot, &body));
    }

    async fn assert_documented_callback_matrix(app_id: &str, expected: &str) {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path, query_param},
        };
        let server = MockServer::start().await;
        TEST_GRAPH_BASES
            .lock()
            .unwrap()
            .insert(app_id.into(), server.uri());
        let other = "https://other.example/whatsapp";
        let phone = |p: &str, w: &str, a: &str| {
            json!({"id":"333", "webhook_configuration":{
                "phone_number":p, "whatsapp_business_account":w, "application":a
            }})
        };
        let apps = |url: &str| json!({"data":[{"override_callback_uri":url}]});
        let mut cases = vec![
            (
                "phone elsewhere",
                phone(other, "", expected),
                expected,
                apps(""),
                false,
            ),
            (
                "phone beats matching WABA",
                phone(other, expected, expected),
                expected,
                apps(expected),
                false,
            ),
            (
                "WABA elsewhere without phone",
                phone("", other, expected),
                expected,
                apps(other),
                false,
            ),
            (
                "phone wins over app",
                phone(expected, "", other),
                other,
                apps(""),
                true,
            ),
            (
                "phone wins over WABA and app",
                phone(expected, other, other),
                other,
                apps(other),
                true,
            ),
            (
                "WABA wins over app",
                phone("", expected, other),
                other,
                apps(expected),
                true,
            ),
            (
                "app only",
                phone("", "", expected),
                expected,
                apps(""),
                true,
            ),
            (
                "app elsewhere",
                phone("", "", other),
                expected,
                apps(""),
                false,
            ),
            (
                "app subscription mismatch",
                phone("", "", expected),
                other,
                apps(""),
                false,
            ),
            (
                "overrides omitted",
                json!({"webhook_configuration":{"application":expected}}),
                expected,
                apps(""),
                true,
            ),
            (
                "missing configuration",
                json!({"id":"333"}),
                expected,
                apps(""),
                false,
            ),
            (
                "null configuration",
                json!({"webhook_configuration":null}),
                expected,
                apps(""),
                false,
            ),
            (
                "array configuration",
                json!({"webhook_configuration":[]}),
                expected,
                apps(""),
                false,
            ),
            (
                "string configuration",
                json!({"webhook_configuration":expected}),
                expected,
                apps(""),
                false,
            ),
            (
                "empty configuration",
                json!({"webhook_configuration":{}}),
                expected,
                apps(""),
                false,
            ),
            ("all empty", phone("", "", ""), expected, apps(""), false),
            (
                "WABA read agrees",
                phone("", expected, other),
                other,
                apps(expected),
                true,
            ),
            (
                "WABA read absent",
                phone("", expected, other),
                other,
                json!({"data":[]}),
                true,
            ),
            (
                "WABA read disagrees",
                phone("", expected, other),
                other,
                apps(other),
                false,
            ),
            (
                "WABA conflict under phone override",
                phone(expected, other, other),
                other,
                apps(expected),
                false,
            ),
            (
                "WABA read malformed",
                phone("", expected, other),
                other,
                json!({"data":[{"override_callback_uri":42}]}),
                false,
            ),
        ];
        for field in ["phone_number", "whatsapp_business_account", "application"] {
            for malformed in [Value::Null, json!(42), json!([]), json!({})] {
                let mut body = phone("", "", expected);
                body["webhook_configuration"][field] = malformed;
                cases.push(("malformed callback field", body, expected, apps(""), false));
            }
        }
        for (name, body, app_url, subscribed_apps, verified) in cases {
            server.reset().await;
            Mock::given(method("GET"))
                .and(path(format!("/v25.0/{app_id}/subscriptions")))
                .and(header(
                    "authorization",
                    format!("Bearer {app_id}|app-secret"),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{
                    "object":"whatsapp_business_account", "active":true,
                    "callback_url":app_url, "fields":[{"name":"messages"}]
                }]})))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/v25.0/333"))
                .and(query_param("fields", "webhook_configuration"))
                .and(header("authorization", "Bearer system-token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/v25.0/222/subscribed_apps"))
                .and(header("authorization", "Bearer system-token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(subscribed_apps))
                .mount(&server)
                .await;
            let result = verify(app_id, "app-secret", "system-token", "333", "222", expected).await;
            assert_eq!(result.is_ok(), verified, "{name}");
            if !verified {
                assert!(
                    matches!(result, Err(crate::errors::AppError::ValidationError(ref code))
                    if code == "native_whatsapp_callback_unverified"),
                    "{name}"
                );
            }
            let requests = server.received_requests().await.unwrap();
            assert!(requests.iter().all(|request| {
                let query = request.url.query().unwrap_or_default();
                !query.contains("access_token")
                    && !query.contains("app-secret")
                    && !query.contains("system-token")
            }));
            if name == "app only" {
                assert_eq!(
                    requests.len(),
                    2,
                    "no redundant WABA read without an override"
                );
            }
        }
        TEST_GRAPH_BASES.lock().unwrap().remove(app_id);
    }

    #[tokio::test]
    async fn gateway_callback_proof_uses_documented_meta_routing_precedence() {
        assert_documented_callback_matrix("12301", "https://gateway.example/v1/whatsapp/channel")
            .await;
    }

    #[tokio::test]
    async fn rollback_callback_proof_uses_the_same_documented_meta_routing_precedence() {
        assert_documented_callback_matrix(
            "12302",
            "https://nyx.example/api/v1/webhooks/channel/whatsapp/bot",
        )
        .await;
    }

    #[test]
    fn every_effective_source_still_requires_an_active_messages_subscription() {
        for expected in ["https://gateway.example/wa", "https://nyx.example/relay"] {
            for source in ["phone_number", "whatsapp_business_account", "application"] {
                let phone = json!({"webhook_configuration":{source:expected}});
                let apps = json!({"data":[{"override_callback_uri":expected}]});
                let valid = json!({"data":[{"object":"whatsapp_business_account", "active":true,
                    "callback_url":expected, "fields":[{"name":"messages"}]}]});
                assert!(proof("123", expected, &valid, &phone, &apps).is_ok());
                for (field, value) in [
                    ("object", json!("page")),
                    ("active", json!(false)),
                    ("fields", json!(["statuses"])),
                ] {
                    let mut invalid = valid.clone();
                    invalid["data"][0][field] = value;
                    assert!(proof("123", expected, &invalid, &phone, &apps).is_err());
                }
                assert!(proof("123", expected, &json!({"data":[]}), &phone, &apps).is_err());
            }
        }
    }

    #[test]
    fn cutover_and_rollback_each_require_fresh_exact_callback_proof() {
        let mut config = super::super::tests::config();
        let native = "https://gateway.example/v1/whatsapp/channel";
        let relay = "https://nyx.example/api/v1/webhooks/channel/whatsapp/bot";
        config.callback.phase = CallbackPhase::AwaitingGateway;
        config.callback.gateway_webhook_url = Some(native.into());
        config.callback.transition_started_at = Some(Utc::now());
        let mut proof = CallbackProof {
            app_id: config.meta_app_id.clone(),
            url: native.into(),
            checked_at: Utc::now() - chrono::Duration::seconds(61),
        };
        assert!(confirm_gateway(&mut config, &proof).is_err());
        proof.checked_at = Utc::now();
        confirm_gateway(&mut config, &proof).unwrap();
        request_restore(&mut config).unwrap();
        assert!(!suppress_relay_messages(&config));
        assert!(confirm_relay(&mut config, &proof, relay).is_err());
        proof.url = relay.into();
        proof.checked_at = Utc::now() - chrono::Duration::seconds(61);
        assert!(confirm_relay(&mut config, &proof, relay).is_err());
        proof.checked_at = Utc::now();
        confirm_relay(&mut config, &proof, relay).unwrap();
        assert!(!suppress_relay_messages(&config));
        assert!(config.callback.gateway_verified_at.is_none());
    }

    #[tokio::test]
    async fn graph_subscription_write_and_effective_callback_proof_are_bounded() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_string_contains, method, path, query_param},
        };
        let server = MockServer::start().await;
        TEST_GRAPH_BASES
            .lock()
            .unwrap()
            .insert("123".into(), server.uri());
        Mock::given(method("POST"))
            .and(path("/v25.0/123/subscriptions"))
            .and(body_string_contains("object=whatsapp_business_account"))
            .and(body_string_contains(
                "callback_url=https%3A%2F%2Fgateway.example%2Fwa",
            ))
            .and(body_string_contains("verify_token=token-only-in-body"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .mount(&server)
            .await;
        configure_subscription(
            "123",
            "secret-that-is-not-in-the-url",
            "https://gateway.example/wa",
            "token-only-in-body",
        )
        .await
        .unwrap();
        Mock::given(method("GET"))
            .and(path("/v25.0/123/subscriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data":[{"object":"whatsapp_business_account","active":true,
                    "callback_url":"https://gateway.example/wa","fields":[{"name":"messages"}]}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v25.0/333"))
            .and(query_param("fields", "webhook_configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "webhook_configuration":{
                    "phone_number":"",
                    "whatsapp_business_account":"https://gateway.example/wa",
                    "application":"https://app.example/wa"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v25.0/222/subscribed_apps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data":[{"override_callback_uri":"https://gateway.example/wa"}]
            })))
            .mount(&server)
            .await;
        verify(
            "123",
            "secret-that-is-not-in-the-url",
            "system-token",
            "333",
            "222",
            "https://gateway.example/wa",
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().all(|request| {
            let query = request.url.query().unwrap_or_default();
            !query.contains("access_token") && !query.contains("secret-that-is-not-in-the-url")
        }));
        TEST_GRAPH_BASES.lock().unwrap().remove("123");
    }
}
