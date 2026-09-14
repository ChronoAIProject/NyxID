//! Reviewed, non-billable provider observations. Success proves only `claim`.

use std::time::Duration;

use reqwest::{Method, header::HeaderMap};
use serde_json::Value;

pub use crate::models::service_validation_record::ValidationOutcome;

#[derive(Clone, Copy, Debug)]
pub enum ProbeTarget {
    Relative(&'static str),
    AbsoluteAllowlisted(&'static str),
}

#[derive(Debug)]
pub struct ValidatorProfile {
    pub id: &'static str,
    pub version: u32,
    pub catalog_slugs: &'static [&'static str],
    pub method: Method,
    pub target: ProbeTarget,
    pub body: Option<&'static str>,
    pub max_body_bytes: usize,
    pub classify: fn(&ProbeResponse) -> ValidationOutcome,
    pub claim: &'static str,
    pub billable: bool,
}

/// Kept only for the bounded classification call, never persisted or logged.
pub struct ProbeResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for ProbeResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProbeResponse")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

pub static PROFILES: &[ValidatorProfile] = &[
    ValidatorProfile {
        id: "github_user_v1",
        version: 1,
        catalog_slugs: &["api-github", "api-github-pat"],
        method: Method::GET,
        target: ProbeTarget::Relative("user"),
        body: None,
        max_body_bytes: 32 * 1024,
        classify: github,
        claim: "GitHub accepted this credential for the authenticated user endpoint. Repository access, organization SSO, and write permissions require separate checks.",
        billable: false,
    },
    ValidatorProfile {
        id: "llm_models_v1",
        version: 1,
        catalog_slugs: &[
            "llm-anthropic",
            "llm-openai",
            "llm-google-ai",
            "llm-mistral",
            "llm-cohere",
            "llm-deepseek",
        ],
        method: Method::GET,
        target: ProbeTarget::Relative("models"),
        body: None,
        max_body_bytes: 512 * 1024,
        classify: models,
        claim: "The provider returned a non-empty model list using this connection. This does not establish inference permissions, model availability, or sufficient credit for a request.",
        billable: false,
    },
    ValidatorProfile {
        id: "openrouter_key_v1",
        version: 1,
        catalog_slugs: &["llm-openrouter"],
        method: Method::GET,
        target: ProbeTarget::Relative("key"),
        body: None,
        max_body_bytes: 32 * 1024,
        classify: openrouter,
        claim: "OpenRouter returned metadata for this API key. This does not establish sufficient credit or permission to run a particular model.",
        billable: false,
    },
    ValidatorProfile {
        id: "slack_auth_test_v1",
        version: 1,
        catalog_slugs: &["api-slack", "api-slack-bot"],
        method: Method::POST,
        target: ProbeTarget::Relative("auth.test"),
        body: None,
        max_body_bytes: 32 * 1024,
        classify: slack,
        claim: "Slack accepted this token in auth.test. Channel access and permission to read or send messages require separate checks.",
        billable: false,
    },
    ValidatorProfile {
        id: "lark_user_info_v1",
        version: 1,
        catalog_slugs: &["api-lark", "api-feishu"],
        method: Method::GET,
        target: ProbeTarget::Relative("authen/v1/user_info"),
        body: None,
        max_body_bytes: 32 * 1024,
        classify: lark,
        claim: "Lark or Feishu accepted this user token for user information. Access to documents, chats, and other resources requires separate checks.",
        billable: false,
    },
    ValidatorProfile {
        id: "telegram_get_me_v1",
        version: 1,
        catalog_slugs: &["api-telegram-bot"],
        method: Method::GET,
        target: ProbeTarget::Relative("getMe"),
        body: None,
        max_body_bytes: 32 * 1024,
        classify: telegram,
        claim: "Telegram accepted this bot token in getMe. This does not establish permission to send messages to a particular chat.",
        billable: false,
    },
    ValidatorProfile {
        id: "twitch_users_v1",
        version: 1,
        catalog_slugs: &["api-twitch"],
        method: Method::GET,
        target: ProbeTarget::Relative("users"),
        body: None,
        max_body_bytes: 32 * 1024,
        classify: twitch,
        claim: "Twitch accepted the token and Client-Id for its users endpoint. Other scopes and operations require separate checks.",
        billable: false,
    },
];

pub fn for_slug(slug: &str) -> Option<&'static ValidatorProfile> {
    PROFILES
        .iter()
        .find(|profile| profile.catalog_slugs.contains(&slug))
}

impl ValidatorProfile {
    pub fn target_for(&self, slug: &str) -> ProbeTarget {
        if self.id == "llm_models_v1" && slug == "llm-cohere" {
            ProbeTarget::AbsoluteAllowlisted("https://api.cohere.com/v1/models")
        } else {
            self.target
        }
    }

    pub fn classify_response(&self, response: &ProbeResponse) -> ValidationOutcome {
        if response.body.len() > self.max_body_bytes {
            return ValidationOutcome::TransportUnknown;
        }
        if response.status >= 500 {
            return ValidationOutcome::TransportUnknown;
        }
        if response.status == 429 && self.id != "llm_models_v1" {
            return rate_limited(response);
        }
        (self.classify)(response)
    }
}

pub fn outcome_code(outcome: &ValidationOutcome) -> &'static str {
    match outcome {
        ValidationOutcome::Authenticated => "authenticated",
        ValidationOutcome::PermissionDenied => "permission_denied",
        ValidationOutcome::CredentialRejected => "credential_rejected",
        ValidationOutcome::ConfigurationError => "configuration_error",
        ValidationOutcome::BillingBlocked => "billing_blocked",
        ValidationOutcome::RateLimited { .. } => "rate_limited",
        ValidationOutcome::TransportUnknown => "transport_unknown",
        ValidationOutcome::Unsupported => "unsupported",
    }
}

fn json(response: &ProbeResponse) -> Value {
    serde_json::from_slice(&response.body).unwrap_or(Value::Null)
}

fn rate_limited(response: &ProbeResponse) -> ValidationOutcome {
    let header = response
        .headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok());
    let retry_after = header.and_then(|v| {
        v.parse::<u64>().ok().map(Duration::from_secs).or_else(|| {
            chrono::DateTime::parse_from_rfc2822(v).ok().map(|date| {
                (date.with_timezone(&chrono::Utc) - chrono::Utc::now())
                    .to_std()
                    .unwrap_or_default()
            })
        })
    });
    let body_retry = json(response)
        .pointer("/parameters/retry_after")
        .and_then(Value::as_u64)
        .map(Duration::from_secs);
    ValidationOutcome::RateLimited {
        retry_after: retry_after.max(body_retry),
    }
}

fn github(response: &ProbeResponse) -> ValidationOutcome {
    use ValidationOutcome::*;
    let body = json(response);
    match response.status {
        200 if body["login"]
            .as_str()
            .is_some_and(|login| !login.is_empty()) =>
        {
            Authenticated
        }
        401 => CredentialRejected,
        403 | 404 if response.headers.contains_key("x-github-sso") => PermissionDenied,
        403 => {
            let message = body["message"].as_str().unwrap_or("").to_ascii_lowercase();
            if message.contains("rate limit") || message.contains("abuse detection") {
                rate_limited(response)
            } else {
                PermissionDenied
            }
        }
        _ => TransportUnknown,
    }
}

fn models(response: &ProbeResponse) -> ValidationOutcome {
    use ValidationOutcome::*;
    let body = json(response);
    let code = body
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = body
        .pointer("/error/type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if response.status == 402
        || (kind == "invalid_request_error" && message.contains("credit balance is too low"))
        || [code, kind].iter().any(|v| {
            matches!(
                *v,
                "insufficient_quota"
                    | "billing_not_active"
                    | "credit_balance_too_low"
                    | "insufficient_balance"
            )
        })
    {
        return BillingBlocked;
    }
    if response.status == 400
        && body
            .pointer("/error/details")
            .and_then(Value::as_array)
            .is_some_and(|details| {
                details
                    .iter()
                    .any(|detail| detail["reason"] == "API_KEY_INVALID")
            })
    {
        return CredentialRejected;
    }
    match response.status {
        200 if ["data", "models"].iter().any(|field| {
            body[field]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        }) =>
        {
            Authenticated
        }
        401 => CredentialRejected,
        403 => PermissionDenied,
        429 => rate_limited(response),
        _ => TransportUnknown,
    }
}

fn openrouter(response: &ProbeResponse) -> ValidationOutcome {
    match response.status {
        200 if json(response)["data"]
            .as_object()
            .is_some_and(|data| data.get("label").is_some_and(Value::is_string)) =>
        {
            ValidationOutcome::Authenticated
        }
        401 => ValidationOutcome::CredentialRejected,
        402 => ValidationOutcome::BillingBlocked,
        403 => ValidationOutcome::PermissionDenied,
        _ => ValidationOutcome::TransportUnknown,
    }
}

fn slack(response: &ProbeResponse) -> ValidationOutcome {
    use ValidationOutcome::*;
    let body = json(response);
    if response.status == 200 && body["ok"] == true {
        return Authenticated;
    }
    if body["ok"] != false {
        return TransportUnknown;
    }
    match body["error"].as_str() {
        Some("invalid_auth" | "token_revoked" | "token_expired" | "account_inactive") => {
            CredentialRejected
        }
        Some("missing_scope") => PermissionDenied,
        Some("ratelimited") => rate_limited(response),
        _ => TransportUnknown,
    }
}

fn lark(response: &ProbeResponse) -> ValidationOutcome {
    use ValidationOutcome::*;
    match json(response)["code"].as_i64() {
        Some(0) if response.status == 200 => Authenticated,
        Some(99991663 | 99991668) => CredentialRejected,
        Some(99991400) => rate_limited(response),
        Some(99991401 | 99991403) => TransportUnknown,
        Some(_) => ConfigurationError,
        None => TransportUnknown,
    }
}

fn telegram(response: &ProbeResponse) -> ValidationOutcome {
    let body = json(response);
    if response.status == 200 && body["ok"] == true {
        ValidationOutcome::Authenticated
    } else if body["ok"] == false && (response.status == 401 || body["error_code"] == 401) {
        ValidationOutcome::CredentialRejected
    } else if body["ok"] == false
        && (body["error_code"] == 429 || body.pointer("/parameters/retry_after").is_some())
    {
        rate_limited(response)
    } else {
        ValidationOutcome::TransportUnknown
    }
}

fn twitch(response: &ProbeResponse) -> ValidationOutcome {
    match response.status {
        200 => ValidationOutcome::Authenticated,
        401 if json(response)["message"]
            .as_str()
            .is_some_and(|message| message.to_ascii_lowercase().contains("client-id")) =>
        {
            ValidationOutcome::ConfigurationError
        }
        401 => ValidationOutcome::CredentialRejected,
        _ => ValidationOutcome::TransportUnknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ValidationOutcome::*;

    fn check(slug: &str, status: u16, body: &str, expected: ValidationOutcome) {
        let response = ProbeResponse {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        };
        assert_eq!(
            for_slug(slug).unwrap().classify_response(&response),
            expected
        );
    }

    #[test]
    fn github_user_fixtures() {
        check(
            "api-github",
            200,
            r#"{"login":"octocat","id":1}"#,
            Authenticated,
        );
        check(
            "api-github-pat",
            401,
            r#"{"message":"Bad credentials"}"#,
            CredentialRejected,
        );
        check(
            "api-github",
            403,
            r#"{"message":"API rate limit exceeded"}"#,
            RateLimited { retry_after: None },
        );
        check(
            "api-github",
            403,
            r#"{"message":"Resource not accessible by integration"}"#,
            PermissionDenied,
        );
        for status in [403, 404] {
            let mut response = ProbeResponse {
                status,
                headers: HeaderMap::new(),
                body: vec![],
            };
            response
                .headers
                .insert("x-github-sso", "required".parse().unwrap());
            assert_eq!(
                for_slug("api-github").unwrap().classify_response(&response),
                PermissionDenied
            );
        }
    }

    #[test]
    fn llm_models_fixtures() {
        for slug in ["llm-openai", "llm-anthropic", "llm-mistral", "llm-deepseek"] {
            check(slug, 200, r#"{"data":[{"id":"model"}]}"#, Authenticated);
            check(
                slug,
                401,
                r#"{"error":{"type":"authentication_error"}}"#,
                CredentialRejected,
            );
            check(slug, 403, "{}", PermissionDenied);
            check(slug, 200, r#"{"data":[]}"#, TransportUnknown);
        }
        check(
            "llm-google-ai",
            200,
            r#"{"models":[{"name":"models/gemini"}]}"#,
            Authenticated,
        );
        check(
            "llm-google-ai",
            400,
            r#"{"error":{"code":400,"status":"INVALID_ARGUMENT","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"API_KEY_INVALID","domain":"googleapis.com"}]}}"#,
            CredentialRejected,
        );
        check(
            "llm-openai",
            429,
            r#"{"error":{"type":"insufficient_quota","code":"insufficient_quota"}}"#,
            BillingBlocked,
        );
        check(
            "llm-anthropic",
            400,
            r#"{"error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API."}}"#,
            BillingBlocked,
        );
        // Doc-derived fixture: https://docs.cohere.com/reference/list-models.
        // Authenticated capture is deferred to the phase-2 fixture checklist.
        check(
            "llm-cohere",
            200,
            r#"{"models":[{"name":"command-r","endpoints":["chat"]}]}"#,
            Authenticated,
        );
        assert!(matches!(
            for_slug("llm-cohere").unwrap().target_for("llm-cohere"),
            ProbeTarget::AbsoluteAllowlisted("https://api.cohere.com/v1/models")
        ));
    }

    #[test]
    fn openrouter_key_fixtures() {
        check(
            "llm-openrouter",
            200,
            r#"{"data":{"label":"key","limit":null,"usage":0,"is_free_tier":false}}"#,
            Authenticated,
        );
        check(
            "llm-openrouter",
            200,
            r#"{"data":[{"id":"public-model"}]}"#,
            TransportUnknown,
        );
        check(
            "llm-openrouter",
            401,
            r#"{"error":{"message":"No auth credentials found","code":401}}"#,
            CredentialRejected,
        );
        assert!(matches!(
            for_slug("llm-openrouter").unwrap().target,
            ProbeTarget::Relative("key")
        ));
    }

    #[test]
    fn slack_auth_test_fixtures() {
        check(
            "api-slack",
            200,
            r#"{"ok":true,"team_id":"T1","user_id":"U1"}"#,
            Authenticated,
        );
        for error in [
            "invalid_auth",
            "token_revoked",
            "token_expired",
            "account_inactive",
        ] {
            check(
                "api-slack-bot",
                200,
                &format!(r#"{{"ok":false,"error":"{error}"}}"#),
                CredentialRejected,
            );
        }
        check(
            "api-slack",
            200,
            r#"{"ok":false,"error":"missing_scope","needed":"identify"}"#,
            PermissionDenied,
        );
    }

    #[test]
    fn lark_user_info_fixtures() {
        check(
            "api-lark",
            200,
            r#"{"code":0,"data":{"open_id":"ou_example"}}"#,
            Authenticated,
        );
        check(
            "api-feishu",
            200,
            r#"{"code":99991663,"msg":"Invalid access token for authorization."}"#,
            CredentialRejected,
        );
        check(
            "api-lark",
            200,
            r#"{"code":99991668,"msg":"Invalid access token for authorization."}"#,
            CredentialRejected,
        );
        check(
            "api-lark",
            200,
            r#"{"code":99991662,"msg":"tenant token required"}"#,
            ConfigurationError,
        );
        check("api-feishu", 503, r#"{"code":0}"#, TransportUnknown);
    }

    #[test]
    fn telegram_get_me_fixtures() {
        check(
            "api-telegram-bot",
            200,
            r#"{"ok":true,"result":{"id":1,"is_bot":true}}"#,
            Authenticated,
        );
        check(
            "api-telegram-bot",
            401,
            r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#,
            CredentialRejected,
        );
        check(
            "api-telegram-bot",
            429,
            r#"{"ok":false,"error_code":429,"parameters":{"retry_after":120}}"#,
            RateLimited {
                retry_after: Some(Duration::from_secs(120)),
            },
        );
    }

    #[test]
    fn twitch_users_fixtures() {
        check(
            "api-twitch",
            200,
            r#"{"data":[{"id":"1","login":"user"}]}"#,
            Authenticated,
        );
        check(
            "api-twitch",
            401,
            r#"{"error":"Unauthorized","status":401,"message":"Client-ID and OAuth token do not match"}"#,
            ConfigurationError,
        );
        check(
            "api-twitch",
            401,
            r#"{"error":"Unauthorized","status":401,"message":"Invalid OAuth token"}"#,
            CredentialRejected,
        );
    }

    #[test]
    fn registry_is_bounded_non_billable_and_explicit() {
        assert_eq!(PROFILES.len(), 7);
        for profile in PROFILES {
            assert!(!profile.billable);
            assert!(!profile.claim.is_empty());
            let mut response = ProbeResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: vec![b' '; profile.max_body_bytes + 1],
            };
            assert_eq!(profile.classify_response(&response), TransportUnknown);
            response.body.clear();
            response.status = 503;
            assert_eq!(profile.classify_response(&response), TransportUnknown);
            response.status = 429;
            response
                .headers
                .insert("retry-after", "180".parse().unwrap());
            assert_eq!(
                profile.classify_response(&response),
                RateLimited {
                    retry_after: Some(Duration::from_secs(180))
                }
            );
        }
        for slug in [
            "llm-openai-codex",
            "llm-openclaw",
            "api-firecrawl",
            "api-tiktok",
            "api-lark-bot",
            "api-feishu-bot",
            "aws-cost-explorer",
            "api-google-cloud",
            "custom",
            "llm-openai-2",
        ] {
            assert!(for_slug(slug).is_none());
        }
    }
}
