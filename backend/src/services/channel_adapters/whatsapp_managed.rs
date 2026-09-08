//! Meta Embedded Signup protocol. No provider-specific behavior escapes this adapter.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use rand::Rng;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::whatsapp::{GRAPH_API_VERSION, graph_response, validate_id};
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{ChannelBot, ManagedBotSetup};
use crate::services::channel_managed::{
    ManagedOnboardingDescriptor, ManagedOnboardingInput, ManagedOnboardingResult,
    PlatformCredentialDescriptor, PlatformCredentialField,
};
use crate::services::channel_platform::{BotCredentials, BotIdentity, PlatformVerifySecrets};

pub const CREDENTIALS: PlatformCredentialDescriptor = PlatformCredentialDescriptor {
    backing: crate::services::channel_managed::PlatformCredentialBacking::Stored,
    provider: "meta",
    label: "Meta",
    fields: &[
        PlatformCredentialField {
            name: "app_id",
            label: "App ID",
            secret: false,
            required: true,
            numeric: true,
            help: "Meta App Dashboard > Basic. The existing Facebook provider app can be reused.",
        },
        PlatformCredentialField {
            name: "app_secret",
            label: "App Secret",
            secret: true,
            required: true,
            numeric: false,
            help: "Meta App Dashboard > Basic > App Secret.",
        },
        PlatformCredentialField {
            name: "embedded_signup_config_id",
            label: "Embedded Signup Configuration ID",
            secret: false,
            required: true,
            numeric: true,
            help: "Facebook Login for Business > Configurations > WhatsApp Embedded Signup.",
        },
    ],
    webhook_secret_field: Some("app_secret"),
    setup_checklist: &[
        "Create or choose a Meta Business app. NyxID's existing facebook provider app can be reused by adding the WhatsApp product.",
        "Add the WhatsApp product and enroll as a Tech Provider.",
        "Create a new Facebook Login for Business Embedded Signup configuration and select the Cloud API product to enable v4; enter its ID here. Allow the NyxID frontend domain and enable JavaScript SDK login.",
        "Complete Business Verification and App Review. Request advanced access for whatsapp_business_management and whatsapp_business_messaging.",
        "Configure the app-level Webhooks callback URL and Verify Token below and subscribe to messages.",
        "For Business App coexistence, enable Meta's Business App onboarding flow and subscribe to smb_app_state_sync, smb_message_echoes, and history. Historical chat import is not provided by NyxID.",
        "Customers add their own Meta payment method and pay Meta directly for WhatsApp messaging. Sharing a credit line requires Solution Partner status.",
    ],
};

pub const ONBOARDING: ManagedOnboardingDescriptor = ManagedOnboardingDescriptor {
    flow: "meta_embedded_signup",
    provider: "meta",
    bootstrap_fields: &["app_id", "embedded_signup_config_id"],
    completion_fields: &["code", "phone_number_id", "waba_id", "business_id"],
    graph_version: GRAPH_API_VERSION,
    signup_version: "v4",
    signup_extras: |feature| {
        if feature.is_empty() {
            json!({})
        } else {
            json!({ "featureType": feature })
        }
    },
    feature_types: &["", "whatsapp_business_app_onboarding"],
};

pub(super) fn proof(token: &str, secret: &str) -> Zeroizing<String> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
    mac.update(token.as_bytes());
    Zeroizing::new(hex::encode(mac.finalize().into_bytes()))
}

pub(super) fn authenticate(
    request: reqwest::RequestBuilder,
    credentials: &BotCredentials<'_>,
) -> AppResult<reqwest::RequestBuilder> {
    let request = request.bearer_auth(credentials.token);
    if let Some(secrets) = credentials.platform_secrets {
        let secret = secrets
            .get("app_secret")
            .ok_or_else(crate::services::channel_managed::unavailable)?;
        Ok(request.query(&[("appsecret_proof", proof(credentials.token, secret).as_str())]))
    } else {
        Ok(request)
    }
}

pub(super) fn base(credentials: &PlatformVerifySecrets) -> &str {
    #[cfg(test)]
    if let Some(base) = credentials.get("test_graph_base") {
        return base;
    }
    let _ = credentials;
    "https://graph.facebook.com"
}

fn url(base: &str, path: &str) -> String {
    format!("{base}/{GRAPH_API_VERSION}/{path}")
}

fn protocol_error() -> AppError {
    AppError::ChannelPlatformError(
        "Meta onboarding request failed. Retry signup or contact your platform administrator."
            .to_string(),
    )
}

async fn send(request: reqwest::RequestBuilder) -> AppResult<Value> {
    let response = request
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| protocol_error())?;
    graph_response(response).await
}

pub async fn complete(
    http: &reqwest::Client,
    platform: &PlatformVerifySecrets,
    input: &ManagedOnboardingInput,
) -> AppResult<ManagedOnboardingResult> {
    let app_id = platform
        .get("app_id")
        .ok_or_else(crate::services::channel_managed::unavailable)?;
    let secret = platform
        .get("app_secret")
        .ok_or_else(crate::services::channel_managed::unavailable)?;
    let base = base(platform);
    let waba = input.get("waba_id")?;
    validate_id(waba, "WABA ID")?;
    if let Ok(phone) = input.get("phone_number_id") {
        validate_id(phone, "Phone Number ID")?;
    }
    let business = input.0.get("business_id").map(|value| value.to_string());
    if let Some(id) = &business {
        validate_id(id, "Business ID")?;
    }
    let code = input.get("code")?;
    if code.is_empty() || code.len() > 8192 {
        return Err(protocol_error());
    }
    // Codes and app secrets remain on the server; reqwest errors are never propagated.
    let mut exchange = send(http.get(url(base, "oauth/access_token")).query(&[
        ("client_id", app_id),
        ("client_secret", secret),
        ("code", code),
    ]))
    .await?;
    let token = Zeroizing::new(
        exchange
            .get_mut("access_token")
            .and_then(|v| v.take().as_str().map(String::from))
            .filter(|v| !v.is_empty())
            .ok_or_else(protocol_error)?,
    );
    let app_token = Zeroizing::new(format!("{app_id}|{secret}"));
    let debug = send(
        http.get(url(base, "debug_token"))
            .bearer_auth(app_token.as_str())
            .query(&[
                ("input_token", token.as_str()),
                ("appsecret_proof", proof(&app_token, secret).as_str()),
            ]),
    )
    .await?;
    let data = &debug["data"];
    let now = chrono::Utc::now().timestamp();
    if data["is_valid"] != true
        || data["app_id"].as_str() != Some(app_id)
        || ["expires_at", "data_access_expires_at"]
            .iter()
            .any(|field| {
                data[*field]
                    .as_i64()
                    .is_some_and(|expiry| expiry != 0 && expiry <= now)
            })
        || [
            "whatsapp_business_management",
            "whatsapp_business_messaging",
        ]
        .iter()
        .any(|scope| {
            !data["granular_scopes"].as_array().is_some_and(|scopes| {
                scopes.iter().any(|item| {
                    item["scope"].as_str() == Some(scope)
                        && item["target_ids"].as_array().is_some_and(|ids| {
                            ids.iter().any(|id| {
                                id.as_str() == Some(waba)
                                    || id.as_u64().is_some_and(|id| id.to_string() == waba)
                            })
                        })
                })
            })
        })
    {
        return Err(AppError::ValidationError("Meta token does not authorize this app and WhatsApp Business Account with both required permissions".to_string()));
    }
    let credentials = BotCredentials {
        token: &token,
        platform_bot_id: None,
        platform_secrets: Some(platform),
    };
    // A token can access several WABAs. Prove the selected phone belongs to this WABA.
    // FINISH_ONLY_WABA can omit the phone ID: accept only one unambiguous number.
    let mut rows = BTreeSet::new();
    let mut after = None;
    let mut exhausted = false;
    for _ in 0..20 {
        let mut request = http
            .get(url(base, &format!("{waba}/phone_numbers")))
            .query(&[("fields", "id"), ("limit", "100")]);
        if let Some(cursor) = &after {
            request = request.query(&[("after", cursor)]);
        }
        let phones = send(authenticate(request, &credentials)?).await?;
        for row in phones["data"].as_array().ok_or_else(protocol_error)? {
            rows.insert(row["id"].as_str().ok_or_else(protocol_error)?.to_string());
        }
        exhausted = phones["paging"]["next"].is_null();
        if exhausted
            || input
                .get("phone_number_id")
                .is_ok_and(|phone| rows.contains(phone))
        {
            break;
        }
        // Never follow a provider-supplied URL with bearer credentials.
        let cursor = phones["paging"]["cursors"]["after"]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 4096)
            .ok_or_else(protocol_error)?;
        if after.as_deref() == Some(cursor) {
            return Err(protocol_error());
        }
        after = Some(cursor.to_string());
    }
    let phone = match input.get("phone_number_id") {
        Ok(phone) if rows.contains(phone) => phone.to_string(),
        Err(_) if rows.len() == 1 && exhausted => {
            rows.into_iter().next().ok_or_else(protocol_error)?
        }
        _ => return Err(AppError::ValidationError(
            "The selected phone number does not belong to this WABA or the selection is ambiguous"
                .to_string(),
        )),
    };
    validate_id(&phone, "Phone Number ID")?;
    let identity = send(authenticate(
        http.get(url(base, &phone))
            .query(&[("fields", "id,display_phone_number,verified_name")]),
        &credentials,
    )?)
    .await?;
    if identity["id"].as_str() != Some(&phone) {
        return Err(AppError::ValidationError(
            "WhatsApp phone number identity mismatch".to_string(),
        ));
    }
    let mode = send(authenticate(
        http.get(url(base, &phone))
            .query(&[("fields", "is_on_biz_app,platform_type")]),
        &credentials,
    )?)
    .await?;
    if mode["id"].as_str() != Some(&phone) {
        return Err(protocol_error());
    }
    let coexistence = mode["is_on_biz_app"] == true;
    if coexistence && mode["platform_type"] != "CLOUD_API" {
        return Err(protocol_error());
    }
    let display = identity["display_phone_number"]
        .as_str()
        .filter(|v| !v.is_empty())
        .or_else(|| identity["verified_name"].as_str())
        .unwrap_or(&phone)
        .to_string();
    Ok(ManagedOnboardingResult {
        token,
        identity: BotIdentity {
            platform_bot_id: phone.clone(),
            platform_bot_username: display,
        },
        fields: BTreeMap::from([("phone_number_id", phone), ("waba_id", waba.to_string())]),
        registration_pin: Zeroizing::new(format!(
            "{:06}",
            rand::thread_rng().gen_range(0..1_000_000_u32)
        )),
        setup: ManagedBotSetup {
            subscription: "pending".to_string(),
            webhook_override: "pending".to_string(),
            registration: if coexistence {
                "coexistence"
            } else {
                "pending"
            }
            .to_string(),
            business_id: business,
            coexistence,
            coexistence_sync: BTreeMap::new(),
        },
    })
}

pub async fn register(
    http: &reqwest::Client,
    credentials: &BotCredentials<'_>,
    pin: &str,
) -> AppResult<String> {
    let phone = credentials.platform_bot_id.ok_or_else(protocol_error)?;
    validate_id(phone, "Phone Number ID")?;
    let platform = credentials.platform_secrets.ok_or_else(protocol_error)?;
    let base = base(platform);
    let mode = send(authenticate(
        http.get(url(base, phone))
            .query(&[("fields", "is_on_biz_app,platform_type")]),
        credentials,
    )?)
    .await?;
    if mode["id"].as_str() != Some(phone) {
        return Err(protocol_error());
    }
    if mode["is_on_biz_app"] == true {
        return if mode["platform_type"] == "CLOUD_API" {
            Ok("coexistence".to_string())
        } else {
            Err(protocol_error())
        };
    }
    if pin.len() != 6 || !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err(protocol_error());
    }
    let result = send(authenticate(
        http.post(url(base, &format!("{phone}/register")))
            .json(&json!({ "messaging_product": "whatsapp", "pin": pin })),
        credentials,
    )?)
    .await;
    if result
        .as_ref()
        .is_ok_and(|response| response["success"] == true)
    {
        return Ok("registered".to_string());
    }
    // Meta's error-code catalog does not define a stable "already registered"
    // code. Confirm live connected state instead of trusting upstream free text.
    let status = send(authenticate(
        http.get(url(base, phone)).query(&[("fields", "id,status")]),
        credentials,
    )?)
    .await?;
    if status["id"].as_str() == Some(phone) && status["status"] == "CONNECTED" {
        return Ok("already_registered".to_string());
    }
    Err(protocol_error())
}

#[allow(clippy::too_many_arguments)]
pub async fn setup(
    http: &reqwest::Client,
    credentials: &BotCredentials<'_>,
    bot: &ChannelBot,
    webhook_url: &str,
    verify_token: &str,
    pin: &str,
    progress: &crate::services::channel_managed::ManagedProgress,
) -> AppResult<ManagedBotSetup> {
    let platform = credentials.platform_secrets.ok_or_else(protocol_error)?;
    let waba = bot.app_id.as_deref().ok_or_else(protocol_error)?;
    validate_id(waba, "WABA ID")?;
    let endpoint = url(base(platform), &format!("{waba}/subscribed_apps"));
    let mut state = bot.managed_setup.clone().ok_or_else(protocol_error)?;
    progress.stage("subscribing");
    state.webhook_override = "pending".to_string();
    let subscription = send(authenticate(http.post(&endpoint), credentials)?).await;
    state.subscription = if subscription.is_ok_and(|v| v["success"] == true) {
        "subscribed"
    } else {
        "failed"
    }
    .to_string();
    if state.subscription == "subscribed" {
        let override_result = send(authenticate(
            http.post(&endpoint).json(
                &json!({ "override_callback_uri": webhook_url, "verify_token": verify_token }),
            ),
            credentials,
        )?)
        .await;
        state.webhook_override = if override_result.is_ok_and(|v| v["success"] == true) {
            "configured"
        } else {
            "failed"
        }
        .to_string();
    }
    progress.stage("registering");
    state.registration = register(http, credentials, pin)
        .await
        .unwrap_or_else(|_| "failed".to_string());
    if state.coexistence && state.subscription == "subscribed" {
        for sync_type in ["smb_app_state_sync", "history"] {
            if state
                .coexistence_sync
                .get(sync_type)
                .is_some_and(|status| status == "requested")
            {
                continue;
            }
            let result = send(authenticate(
                http.post(url(
                    base(platform),
                    &format!("{}/smb_app_data", bot.platform_bot_id),
                ))
                .json(&json!({ "messaging_product": "whatsapp", "sync_type": sync_type })),
                credentials,
            )?)
            .await;
            state.coexistence_sync.insert(sync_type.to_string(), if result.is_ok_and(|v| v["request_id"].as_str().is_some_and(|id| !id.is_empty())) { "requested" } else { "failed" }.to_string());
        }
    }
    Ok(state)
}

pub fn handshake(
    credentials: &PlatformVerifySecrets,
    query: &HashMap<String, String>,
) -> AppResult<String> {
    validate_handshake(query)?;
    let denied = || AppError::Forbidden("Invalid platform subscription verification".to_string());
    let token = credentials
        .get(crate::services::platform_credential_service::VERIFY_TOKEN_FIELD)
        .ok_or_else(denied)?;
    let actual = query.get("hub.verify_token").ok_or_else(denied)?;
    if query.get("hub.mode").map(String::as_str) != Some("subscribe")
        || !bool::from(Sha256::digest(token.as_bytes()).ct_eq(&Sha256::digest(actual.as_bytes())))
    {
        return Err(denied());
    }
    query
        .get("hub.challenge")
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or_else(denied)
}

pub fn validate_handshake(query: &HashMap<String, String>) -> AppResult<()> {
    if query.get("hub.mode").map(String::as_str) != Some("subscribe")
        || query
            .get("hub.verify_token")
            .is_none_or(|token| token.is_empty())
    {
        return Err(AppError::Forbidden(
            "Invalid platform subscription verification".to_string(),
        ));
    }
    Ok(())
}

pub async fn remove_override(
    http: &reqwest::Client,
    credentials: &BotCredentials<'_>,
    bot: &ChannelBot,
) -> AppResult<()> {
    let platform = credentials.platform_secrets.ok_or_else(protocol_error)?;
    let waba = bot.app_id.as_deref().ok_or_else(protocol_error)?;
    validate_id(waba, "WABA ID")?;
    // Meta removes the WABA override when subscribing without any body parameters.
    let response = send(authenticate(
        http.post(url(base(platform), &format!("{waba}/subscribed_apps"))),
        credentials,
    )?)
    .await?;
    if response["success"] != true {
        return Err(protocol_error());
    }
    Ok(())
}

pub fn verify_signature(
    credentials: Option<&PlatformVerifySecrets>,
    headers: &HeaderMap,
    body: &[u8],
) -> AppResult<()> {
    let failed = super::whatsapp::verification_failed;
    let secret = credentials
        .and_then(|v| v.get("app_secret"))
        .filter(|v| !v.is_empty())
        .ok_or_else(failed)?;
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("sha256="))
        .filter(|v| v.len() == 64)
        .ok_or_else(failed)?;
    let signature = hex::decode(signature).map_err(|_| failed())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| failed())?;
    mac.update(body);
    mac.verify_slice(&signature).map_err(|_| failed())
}

pub fn targets(
    credentials: &PlatformVerifySecrets,
    headers: &HeaderMap,
    body: &[u8],
) -> AppResult<Vec<String>> {
    verify_signature(Some(credentials), headers, body)?;
    let payload: Value = serde_json::from_slice(body).map_err(|_| protocol_error())?;
    let mut ids = BTreeSet::new();
    if payload["object"] == "whatsapp_business_account" {
        for entry in payload["entry"].as_array().into_iter().flatten() {
            for change in entry["changes"].as_array().into_iter().flatten() {
                if let Some(id) = change["value"]["metadata"]["phone_number_id"]
                    .as_str()
                    .filter(|id| validate_id(id, "Phone Number ID").is_ok())
                {
                    ids.insert(id.to_string());
                }
            }
        }
    }
    Ok(ids.into_iter().collect())
}
