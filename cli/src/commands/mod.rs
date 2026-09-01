pub mod admin;
pub mod ai_setup;
pub mod api_key;
pub mod approval;
pub mod auth_flows;
pub mod billing;
pub mod catalog;
pub mod channel_bot;
pub mod channel_event;
pub mod connect;
pub mod developer_app;
pub mod device;
pub mod doctor;
pub mod endpoint;
pub mod external_key;
pub(crate) mod grant_cascade;
pub mod lark_permission;
pub mod mcp;
pub mod mfa;
pub mod node;
pub mod node_credential;
pub mod notification;
pub mod oauth;
pub mod openclaw;
pub mod oracle;
pub mod oracle_worker_daemon;
pub mod org;
pub mod pairing;
pub mod pool;
pub mod profile;
pub mod provider;
pub mod proxy;
pub mod public;
pub mod repo;
pub mod service;
pub mod service_account;
pub mod session;
pub mod ssh;
pub mod status;
pub mod telemetry;
pub mod trigger;
pub mod update;
pub(crate) mod update_attestation;
pub mod whoami;

/// Truncate an id to its first 8 characters for compact table display,
/// UTF-8 safe.
///
/// `&id[..8]` panics when byte index 8 falls inside a multi-byte UTF-8
/// character — a `len() > 8` guard only checks the byte length, not char
/// boundaries. Ids are normally ASCII UUIDs/slugs, but a non-ASCII or
/// malformed server-supplied value must not crash the CLI. `str::get(..8)`
/// returns `None` on a non-boundary, so we fall back to the full id.
pub(crate) fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// Render a user-facing target from a response row without disclosing the
/// server-owned URL of an auto-connected service. The marker wins even when
/// an older backend still includes the raw field.
pub(crate) fn display_endpoint<'a>(
    item: &'a serde_json::Value,
    field: &str,
    legacy_field: Option<&str>,
) -> &'a str {
    if item["auto_connected"].as_bool() == Some(true) {
        return "platform managed";
    }
    item[field]
        .as_str()
        .or_else(|| legacy_field.and_then(|name| item[name].as_str()))
        .unwrap_or("-")
}

#[cfg(test)]
mod tests {
    use super::{display_endpoint, short_id};

    #[test]
    fn short_id_truncates_long_ascii_to_eight_bytes() {
        assert_eq!(short_id("req-abcdef123456"), "req-abcd");
    }

    #[test]
    fn short_id_returns_short_or_exact_input_unchanged() {
        assert_eq!(short_id("svc-1"), "svc-1");
        assert_eq!(short_id("exactly8"), "exactly8");
    }

    #[test]
    fn short_id_does_not_panic_on_multibyte_boundary() {
        // Byte index 8 lands inside a 3-byte char here; `&id[..8]` would
        // panic ("byte index 8 is not a char boundary"). The helper must
        // return the full string instead of crashing.
        let id = "日本語テスト";
        assert!(id.len() > 8, "precondition: more than 8 bytes");
        assert_eq!(short_id(id), id);
    }

    #[test]
    fn display_endpoint_hides_auto_connected_targets() {
        let omitted = serde_json::json!({ "auto_connected": true });
        assert_eq!(
            display_endpoint(&omitted, "endpoint_url", None),
            "platform managed"
        );

        let old_server = serde_json::json!({
            "auto_connected": true,
            "endpoint_url": "https://platform.internal.example/v1",
        });
        let rendered = display_endpoint(&old_server, "endpoint_url", None);
        assert_eq!(rendered, "platform managed");
        assert!(!rendered.contains("platform.internal.example"));
    }

    #[test]
    fn display_endpoint_keeps_user_managed_targets() {
        let item = serde_json::json!({
            "auto_connected": false,
            "endpoint_url": "https://api.example.com/v1",
        });
        assert_eq!(
            display_endpoint(&item, "endpoint_url", None),
            "https://api.example.com/v1"
        );
    }
}
