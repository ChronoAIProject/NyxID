use anyhow::Error;
use serde::Deserialize;

use crate::api::ApiError;
use crate::cli::OutputFormat;

const GRANT_CASCADE_ERROR_CODE: i64 = 11500;

#[derive(Debug, Deserialize)]
struct GrantCascadeSibling {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GrantCascadeDetails {
    provider_name: String,
    siblings: Vec<GrantCascadeSibling>,
    #[serde(default)]
    unaffected_other_app: Vec<GrantCascadeSibling>,
    token_scope_available: bool,
}

pub(crate) fn append_revocation_query(
    mut path: String,
    cascade_grant: bool,
    keep_upstream: bool,
) -> String {
    if cascade_grant {
        push_query_param(&mut path, "cascade_grant=true");
    }
    if keep_upstream {
        push_query_param(&mut path, "grant_scope=token");
    }
    path
}

fn push_query_param(path: &mut String, param: &str) {
    path.push(if path.contains('?') { '&' } else { '?' });
    path.push_str(param);
}

pub(crate) fn report_if_confirmation_required(error: &Error, output: OutputFormat) {
    if !matches!(output, OutputFormat::Table) {
        return;
    }
    let Some(message) = confirmation_message(error) else {
        return;
    };
    eprintln!("{message}");
}

fn confirmation_message(error: &Error) -> Option<String> {
    let api_error = error.downcast_ref::<ApiError>()?;
    if api_error.status() != reqwest::StatusCode::CONFLICT {
        return None;
    }
    let response = api_error.response()?;
    if response.error_code != GRANT_CASCADE_ERROR_CODE {
        return None;
    }
    let details: GrantCascadeDetails = serde_json::from_value(response.details.clone()?).ok()?;

    let mut lines = vec![format!(
        "{} authorization is shared by these NyxID services:",
        details.provider_name
    )];
    lines.extend(
        details
            .siblings
            .iter()
            .map(|sibling| format!("  - {}", sibling.name)),
    );
    lines.extend(
        details
            .unaffected_other_app
            .iter()
            .map(|sibling| format!("  - {} (not affected: different OAuth app)", sibling.name)),
    );
    lines.push(format!(
        "Retry with --cascade-grant to disconnect {} everywhere.",
        details.provider_name
    ));
    if details.token_scope_available {
        lines.push(
            "Retry with --keep-upstream to remove only this service while keeping the upstream grant."
                .to_string(),
        );
    } else {
        lines.push(
            "Retry with --keep-upstream to remove only this service from NyxID; upstream authorization will remain active."
                .to_string(),
        );
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_revocation_flags_after_existing_query() {
        assert_eq!(
            append_revocation_query(
                "/providers/github/disconnect?target_org_id=org-1".to_string(),
                true,
                false,
            ),
            "/providers/github/disconnect?target_org_id=org-1&cascade_grant=true"
        );
        assert_eq!(
            append_revocation_query("/keys/service-1".to_string(), false, true),
            "/keys/service-1?grant_scope=token"
        );
    }

    #[test]
    fn formats_sibling_names_and_actionable_hints() {
        let error: Error = ApiError::new(
            "/keys/service-1",
            reqwest::StatusCode::CONFLICT,
            serde_json::json!({
                "error": "grant_cascade_confirmation_required",
                "error_code": 11500,
                "message": "Grant cascade confirmation required",
                "details": {
                    "provider_name": "GitHub",
                    "siblings": [{ "name": "GitHub Issues" }],
                    "unaffected_other_app": [{ "name": "Enterprise GitHub" }],
                    "token_scope_available": true
                }
            })
            .to_string(),
        )
        .into();

        let message = confirmation_message(&error).expect("confirmation message");
        assert!(message.contains("GitHub Issues"));
        assert!(message.contains("Enterprise GitHub (not affected: different OAuth app)"));
        assert!(message.contains("--cascade-grant"));
        assert!(message.contains("--keep-upstream"));
    }

    #[test]
    fn explains_local_only_fallback_when_token_revocation_is_unavailable() {
        let error: Error = ApiError::new(
            "/keys/service-1",
            reqwest::StatusCode::CONFLICT,
            serde_json::json!({
                "error": "grant_cascade_confirmation_required",
                "error_code": 11500,
                "message": "Grant cascade confirmation required",
                "details": {
                    "provider_name": "Facebook",
                    "siblings": [{ "name": "Facebook Pages" }],
                    "unaffected_other_app": [],
                    "token_scope_available": false
                }
            })
            .to_string(),
        )
        .into();

        let message = confirmation_message(&error).expect("confirmation message");
        assert!(message.contains("upstream authorization will remain active"));
    }
}
