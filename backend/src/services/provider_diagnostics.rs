//! Credential-provider diagnostics are a finite vocabulary. Provider text is
//! untrusted credential-bearing input, even for malformed successful replies.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub code: &'static str,
    pub message: &'static str,
}

pub fn oauth_code(code: &str) -> Rejection {
    let (code, message) = match code {
        "invalid_request" => (
            "invalid_request",
            "The provider rejected the request parameters. Check the configured authorization flow.",
        ),
        "invalid_client" => (
            "invalid_client",
            "The provider rejected the app credentials. Check the client ID and secret.",
        ),
        "invalid_grant" => (
            "invalid_grant",
            "The authorization has expired or was revoked. Reconnect the account.",
        ),
        "unauthorized_client" => (
            "unauthorized_client",
            "The provider does not allow this authorization flow for the app.",
        ),
        "unsupported_grant_type" => (
            "unsupported_grant_type",
            "The provider does not support the configured authorization flow.",
        ),
        "invalid_scope" => (
            "invalid_scope",
            "The provider rejected the requested permissions. Check the app scopes.",
        ),
        "access_denied" => ("access_denied", "The provider denied authorization."),
        "authorization_pending" => ("authorization_pending", "Authorization is pending."),
        "slow_down" => ("slow_down", "The provider requested slower polling."),
        "expired_token" => (
            "expired_token",
            "The authorization request has expired. Start again.",
        ),
        "temporarily_unavailable" => (
            "temporarily_unavailable",
            "The provider is temporarily unavailable. Retry later.",
        ),
        "server_error" => (
            "server_error",
            "The provider could not complete the request. Retry later.",
        ),
        _ => (
            "provider_rejected",
            "The provider rejected authorization. Check the app configuration or reconnect the account.",
        ),
    };
    Rejection { code, message }
}

pub fn rejection(value: &Value) -> Option<Rejection> {
    if let Some(code) = value
        .get("code")
        .and_then(Value::as_i64)
        .filter(|code| *code != 0)
    {
        return Some(match code {
            99991663 => Rejection {
                code: "lark_app_ticket_invalid",
                message: "The provider rejected the app ticket. Check the app credentials and reconnect.",
            },
            20029 => Rejection {
                code: "lark_redirect_uri_mismatch",
                message: "The provider rejected the redirect URI. Check the app callback configuration.",
            },
            _ => oauth_code(""),
        });
    }
    value
        .get("error")
        .filter(|error| !error.is_null())
        .map(|error| oauth_code(error.as_str().unwrap_or_default()))
}

/// Fixed code suitable for logs/status; never returns a borrowed provider value.
pub fn response_code(body: &str) -> &'static str {
    match serde_json::from_str::<Value>(body) {
        Ok(value) => rejection(&value).map_or("invalid_token_response", |error| error.code),
        Err(_) => "invalid_token_response",
    }
}

pub fn refresh_failure(status: reqwest::StatusCode, body: &str) -> String {
    let error = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| rejection(&value));
    match error {
        Some(error) => format!(
            "Refresh failed (HTTP {}, {}): {}",
            status.as_u16(),
            error.code,
            error.message
        ),
        None => format!(
            "Refresh failed (HTTP {}). Reconnect the account or check the provider configuration.",
            status.as_u16()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_provider_diagnostics_never_echo_untrusted_fields_or_types() {
        let secret = "distinctive-provider-secret-e5b6499f";
        for body in [
            format!(
                r#"{{"error":"{secret}","error_description":"{secret}","refresh_token":"{secret}"}}"#
            ),
            format!(r#"{{"error":"invalid_grant","error_description":"{secret}"}}"#),
            format!(r#"{{"code":99991663,"msg":"{secret}"}}"#),
            format!(r#"{{"code":20029,"msg":"{secret}"}}"#),
            format!(r#"{{"error":{{"password":"{secret}"}}}}"#),
            format!(r#"{{"access_token":{{"secret":"{secret}"}},"refresh_token":"{secret}"}}"#),
            format!("malformed {secret}"),
        ] {
            let diagnostic = refresh_failure(reqwest::StatusCode::BAD_REQUEST, &body);
            assert!(!diagnostic.contains(secret));
            assert!(!response_code(&body).contains(secret));
            if let Ok(value) = serde_json::from_str::<Value>(&body) {
                assert!(!format!("{:?}", rejection(&value)).contains(secret));
            }
        }
        assert_eq!(
            response_code(r#"{"code":99991663,"msg":"arbitrary"}"#),
            "lark_app_ticket_invalid"
        );
        assert_eq!(
            response_code(r#"{"error":"invalid_grant"}"#),
            "invalid_grant"
        );
    }
}
