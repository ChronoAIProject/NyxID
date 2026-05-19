//! Tabular printer for the Lark / Feishu permission deep link the
//! backend attaches to channel-bot and AI-services responses.
//!
//! The backend ships `permission_setup_url` + `permission_setup_scopes`
//! on responses for Lark / Feishu surfaces (`POST /channel-bots`,
//! `GET /channel-bots/{id}`, `PATCH /channel-bots/{id}`, `POST /keys`,
//! `GET /keys/{id}`, `PUT /keys/{id}`). When the field is present, this
//! helper prints a "Configure Permissions" block so CLI users get the
//! same one-click setup workflow as the web UI. JSON output already
//! passes through the URL verbatim, so this helper is only invoked from
//! table-mode rendering.

use serde_json::Value;

/// Build the "Configure Permissions" block as a String when the
/// response carries a Lark / Feishu permission deep link, or `None` if
/// it doesn't. Splitting the formatting from the actual `eprintln!`
/// keeps the rendered text test-assertable; `print_permission_block`
/// is the thin wrapper callers actually use.
pub fn format_permission_block(response: &Value) -> Option<String> {
    let url = match response.get("permission_setup_url").and_then(Value::as_str) {
        Some(value) if !value.is_empty() => value,
        _ => return None,
    };

    let mut out = String::new();
    out.push('\n');
    out.push_str("Configure Permissions:\n");
    out.push_str(
        "  Open this link in your browser to grant the required scopes in the developer console:\n",
    );
    out.push_str(&format!("    {url}\n"));

    if let Some(scopes) = response
        .get("permission_setup_scopes")
        .and_then(Value::as_array)
    {
        let scope_strings: Vec<&str> = scopes.iter().filter_map(Value::as_str).collect();
        if !scope_strings.is_empty() {
            out.push_str("  Scopes pre-selected:\n");
            for scope in scope_strings {
                out.push_str(&format!("    - {scope}\n"));
            }
        }
    }

    Some(out)
}

/// Print a "Configure Permissions" block when the response carries a
/// Lark / Feishu permission deep link. Silent no-op for non-Lark
/// responses so callers can invoke unconditionally.
pub fn print_permission_block(response: &Value) {
    if let Some(block) = format_permission_block(response) {
        // The block already starts with a leading blank line, and each
        // line is newline-terminated, so use `eprint!` (not `eprintln!`)
        // to avoid trailing whitespace in the rendered output.
        eprint!("{block}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_when_field_missing() {
        let payload = serde_json::json!({"id": "abc"});
        assert!(format_permission_block(&payload).is_none());
    }

    #[test]
    fn returns_none_when_url_is_empty_string() {
        let payload = serde_json::json!({"permission_setup_url": ""});
        assert!(format_permission_block(&payload).is_none());
    }

    #[test]
    fn returns_none_when_url_is_null() {
        let payload = serde_json::json!({"permission_setup_url": null});
        assert!(format_permission_block(&payload).is_none());
    }

    #[test]
    fn returns_none_when_url_is_non_string() {
        // Defensive: a numeric or object value should not panic.
        let payload = serde_json::json!({"permission_setup_url": 42});
        assert!(format_permission_block(&payload).is_none());
        let payload = serde_json::json!({"permission_setup_url": {"not": "a string"}});
        assert!(format_permission_block(&payload).is_none());
    }

    #[test]
    fn renders_url_only_when_scopes_missing() {
        let payload = serde_json::json!({
            "permission_setup_url": "https://open.larksuite.com/app/cli_test/auth?op_from=openapi",
        });
        let block = format_permission_block(&payload).expect("expected a block");
        assert!(block.contains("Configure Permissions:"));
        assert!(block.contains("https://open.larksuite.com/app/cli_test/auth?op_from=openapi"));
        // No "Scopes pre-selected:" header when the array is absent.
        assert!(!block.contains("Scopes pre-selected:"));
    }

    #[test]
    fn renders_url_only_when_scopes_array_is_empty() {
        let payload = serde_json::json!({
            "permission_setup_url": "https://open.larksuite.com/app/cli_test/auth?op_from=openapi",
            "permission_setup_scopes": [],
        });
        let block = format_permission_block(&payload).expect("expected a block");
        assert!(!block.contains("Scopes pre-selected:"));
    }

    #[test]
    fn renders_full_block_with_url_and_scopes() {
        let payload = serde_json::json!({
            "permission_setup_url": "https://open.larksuite.com/app/cli_test/auth?q=im%3Amessage&op_from=openapi",
            "permission_setup_scopes": ["im:message", "im:message:send_as_bot"],
        });
        let block = format_permission_block(&payload).expect("expected a block");

        // Header + URL.
        assert!(block.contains("Configure Permissions:"));
        assert!(block.contains(
            "https://open.larksuite.com/app/cli_test/auth?q=im%3Amessage&op_from=openapi"
        ));
        // Scope list appears verbatim, one per line.
        assert!(block.contains("Scopes pre-selected:"));
        assert!(block.contains("    - im:message\n"));
        assert!(block.contains("    - im:message:send_as_bot\n"));

        // Block starts with a blank line so it visually separates from
        // whatever the caller just printed.
        assert!(block.starts_with('\n'));
    }

    #[test]
    fn renders_block_when_scopes_contain_non_string_entries() {
        // Defensive: a malformed `permission_setup_scopes` array
        // shouldn't panic — non-string entries are silently dropped.
        let payload = serde_json::json!({
            "permission_setup_url": "https://open.feishu.cn/app/cli_test/auth?op_from=openapi",
            "permission_setup_scopes": ["im:message", 42, null, "im:message:send_as_bot"],
        });
        let block = format_permission_block(&payload).expect("expected a block");
        assert!(block.contains("    - im:message\n"));
        assert!(block.contains("    - im:message:send_as_bot\n"));
        // 42 and null don't survive the filter_map.
        assert!(!block.contains("- 42"));
        assert!(!block.contains("- null"));
    }
}
