//! Manifest-only OAuth playbook evidence. None of these fields reaches the API.
use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    kind: String,
    url: String,
    note: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevocationEvidence {
    kind: String,
    source: String,
    note: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Playbook {
    scope_source: Option<Evidence>,
    revocation: Option<RevocationEvidence>,
}
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Operation {
    pub name: String,
    pub method: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_body_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_description: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Smoke {
    operation: Option<String>,
    method: Option<String>,
    path: Option<String>,
}

fn evidence_url(value: &str) -> bool {
    value.len() <= 2048
        && url::Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
}
fn safe_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 2048
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
}
pub(super) fn operations(row: &Value) -> Result<Vec<Operation>> {
    row.get("operations")
        .map(|value| {
            serde_json::from_value(value.clone()).map_err(|_| anyhow!("Invalid operation fields"))
        })
        .unwrap_or_else(|| Ok(vec![]))
}
pub(super) fn validate(provider: &Value, service: bool) -> Result<()> {
    if !service {
        if let Some(value) = provider.get("playbook") {
            let playbook: Playbook = serde_json::from_value(value.clone())
                .map_err(|_| anyhow!("Invalid playbook fields"))?;
            if let Some(evidence) = playbook.scope_source
                && (!["vendor_cli", "documented_minimum", "no_scopes"]
                    .contains(&evidence.kind.as_str())
                    || !evidence_url(&evidence.url)
                    || evidence.note.as_ref().is_some_and(|note| note.len() > 4096))
            {
                bail!(
                    "Scope evidence requires an applicable kind, bounded HTTPS url without userinfo, and bounded note"
                );
            }
            if let Some(evidence) = playbook.revocation
                && (!["configured", "vendor_unsupported", "nyxid_incompatible"]
                    .contains(&evidence.kind.as_str())
                    || !evidence_url(&evidence.source)
                    || evidence.note.as_ref().is_some_and(|note| note.len() > 4096))
            {
                bail!(
                    "Revocation evidence requires an applicable kind, bounded HTTPS source without userinfo, and bounded note"
                );
            }
        }
        return Ok(());
    }
    let operations = operations(provider)?;
    if let Some(scheme) = provider.get("auth_scheme").filter(|value| !value.is_null()) {
        let valid = scheme.as_str().is_some_and(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        });
        if !valid
            || !provider
                .get("auth_method")
                .is_some_and(|method| method == "bearer" || method == "oauth2")
        {
            bail!("auth_scheme requires bearer auth and an RFC 9110 token of 1–64 bytes");
        }
    }
    if provider.get("operations").is_some()
        && provider["openapi_spec_url"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    {
        bail!("Use explicit operations or openapi_spec_url, not both");
    }
    let mut names = std::collections::HashSet::new();
    if operations.len() > 200 {
        bail!("At most 200 explicit operations are allowed");
    }
    for operation in &operations {
        if operation.name.is_empty()
            || operation.name.len() > 100
            || !operation.name.as_bytes()[0].is_ascii_lowercase()
            || !operation
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || !names.insert(&operation.name)
            || !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&operation.method.as_str())
            || !operation.path.starts_with('/')
            || operation.path.len() > 2048
            || operation.path.chars().any(char::is_control)
        {
            bail!("Invalid or duplicate explicit operation");
        }
    }
    if let Some(value) = provider.get("smoke_test") {
        let smoke: Smoke = serde_json::from_value(value.clone())
            .map_err(|_| anyhow!("Invalid smoke_test fields"))?;
        let safe = match (smoke.operation, smoke.method, smoke.path) {
            (Some(name), None, None) => operations.iter().any(|operation| {
                operation.name == name && operation.method == "GET" && safe_path(&operation.path)
            }),
            (None, Some(method), Some(path)) => method == "GET" && safe_path(&path),
            _ => false,
        };
        if !safe {
            bail!(
                "smoke_test must name a declared literal GET operation or supply a literal GET path without query, headers or placeholders"
            );
        }
    }
    Ok(())
}
pub(super) fn api_fields(row: &Value) -> Value {
    let mut body = row.clone();
    if let Some(fields) = body.as_object_mut() {
        for field in ["playbook", "operations", "smoke_test"] {
            fields.remove(field);
        }
    }
    body
}
fn item(met: bool, reason: &str, requires_admin: bool) -> Value {
    json!({"state":if met {"met"} else {"missing"},"reason":if met {"configured"} else {reason},"requires_admin":requires_admin && !met})
}
fn na(reason: &str) -> Value {
    json!({"state":"not_applicable","reason":reason,"requires_admin":false})
}

pub(super) fn report(
    desired: &Value,
    desired_provider: Option<&Value>,
    provider: Option<&Value>,
    provider_expected: bool,
    endpoints: &[Value],
    me: &Value,
    icon_present: bool,
) -> (Value, Value, Option<String>) {
    let Some(provider) = provider else {
        if provider_expected || desired["offering_kind"] == "tool" {
            let reason = if provider_expected {
                "referenced_provider_missing"
            } else {
                "use_tools_readiness"
            };
            return (
                json!({"c1":item(false,reason,false),"c2":item(false,reason,false),"c3":item(false,reason,false),"c4":item(false,reason,false),"c5":item(false,reason,false),"c6":item(false,reason,false),"smoke_test":"missing","complete":false}),
                json!({"status":"not_verified","pending_gates":[]}),
                None,
            );
        }
        let reason = "provider-less non-Tool service";
        return (
            json!({"c1":na(reason),"c2":na(reason),"c3":na(reason),"c4":na(reason),"c5":na(reason),"c6":na(reason),"smoke_test":"not_applicable","complete":true}),
            json!({"status":"not_verified","pending_gates":[]}),
            None,
        );
    };
    let oauth = matches!(
        provider["provider_type"].as_str(),
        Some("oauth2" | "device_code")
    );
    let evidence = desired_provider
        .map(|row| &row["playbook"])
        .unwrap_or(&Value::Null);
    let can_write = me["is_admin"] == true || me["capabilities"]["manage_catalog"] == true;
    // Shared provider metadata and icons are editable by Catalog Managers.
    // Foreign client-bound changes additionally require no connections and,
    // when a client exists, re-entry of that same client. Presence alone does
    // not prove either condition; preserve that guidance in the report.
    let client_boundary = if me["is_admin"] == true || provider["created_by"] == me["id"] {
        "catalog_writer"
    } else {
        "no_user_connections_and_existing_client_reentry; replacement_clear_activation_require_admin"
    };
    let client_ok = provider["has_client_id"] == true
        && (provider["provider_type"] == "device_code" || provider["has_client_secret"] == true);
    let c1 = if oauth {
        item(client_ok, "managed_client_missing", !can_write)
    } else {
        na("API key provider")
    };
    let scope_source = &evidence["scope_source"];
    let no_scopes = scope_source["kind"] == "no_scopes"
        && scope_source["url"].as_str().is_some_and(evidence_url);
    let scopes_match = desired_provider.is_some_and(|desired| {
        desired
            .get("default_scopes")
            .is_some_and(|scopes| scopes == &provider["default_scopes"])
    });
    let c2 = if !oauth {
        na("API key provider")
    } else if no_scopes
        && provider["supports_oauth_scopes"] == false
        && scopes_match
        && provider["default_scopes"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        na("documented no scopes")
    } else {
        item(
            scopes_match
                && provider["supports_oauth_scopes"] != false
                && provider["default_scopes"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
                && matches!(
                    scope_source["kind"].as_str(),
                    Some("vendor_cli" | "documented_minimum")
                )
                && scope_source["url"].as_str().is_some_and(evidence_url),
            "scope_source_or_scope_match_missing",
            !can_write,
        )
    };
    let urls = if provider["provider_type"] == "device_code" {
        &[
            "authorization_url",
            "token_url",
            "device_code_url",
            "device_token_url",
        ][..]
    } else {
        &["authorization_url", "token_url"][..]
    };
    let urls_ok = urls
        .iter()
        .all(|key| provider[*key].as_str().is_some_and(|url| !url.is_empty()));
    let revoke = &evidence["revocation"];
    let revoke_source = revoke["source"].as_str().is_some_and(evidence_url);
    let revoke_ok = revoke_source
        && match revoke["kind"].as_str() {
            Some("configured") => {
                (provider["revocation_url"]
                    .as_str()
                    .is_some_and(|url| !url.is_empty())
                    || provider["revocation"].is_object())
                    && desired_provider.is_some_and(|desired| {
                        ["revocation_url", "revocation"].iter().all(|key| {
                            desired
                                .get(*key)
                                .is_none_or(|value| matches_fields(value, &provider[*key]))
                        })
                    })
            }
            Some("vendor_unsupported" | "nyxid_incompatible") => true,
            _ => false,
        };
    let c3 = if oauth {
        item(
            urls_ok && revoke_ok,
            "oauth_urls_or_revocation_applicability_missing",
            !can_write,
        )
    } else {
        na("API key provider")
    };
    let c4 = if oauth {
        item(
            matches!(provider["credential_mode"].as_str(), Some("admin" | "both")),
            "credential_mode_user",
            !can_write,
        )
    } else {
        na("API key provider")
    };
    let c5 = item(
        (desired["operations"].is_array()
            || desired["openapi_spec_url"]
                .as_str()
                .is_some_and(|url| !url.is_empty()))
            && endpoints.iter().any(|row| row["is_active"] == true),
        "active_operations_missing",
        false,
    );
    let c6 = item(icon_present, "provider_svg_icon_missing", !can_write);
    let smoke = smoke_command(desired);
    let complete = [&c1, &c2, &c3, &c4, &c5, &c6]
        .iter()
        .all(|row| row["state"] != "missing")
        && smoke.is_some();
    (
        json!({"client_bound_write_requirement":client_boundary,"c1":c1,"c2":c2,"c3":c3,"c4":c4,"c5":c5,"c6":c6,"smoke_test":if smoke.is_some(){"declared"}else{"missing"},"complete":complete}),
        json!({"status":"not_verified","pending_gates":["api_smoke_test_real_vendor","outside_user_connection_admin_confirmed"]}),
        smoke,
    )
}

/// A path-shaped reference alone cannot prove an uploaded asset. Fetch only
/// the configured server's public, canonical asset route, without credentials
/// or redirects, and bound the response just like the server's SVG upload.
pub(super) async fn served_icon(base_url: &str, provider: &Value) -> Result<bool> {
    let Some(url) = provider["icon_url"].as_str() else {
        return Ok(false);
    };
    let prefix = format!("{}/catalog-icons/", base_url.trim_end_matches('/'));
    if !url
        .strip_prefix(&prefix)
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id))
    {
        return Ok(false);
    }
    let mut response = crate::tls::client_builder()?
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(10))
        .build()?
        .get(url)
        .send()
        .await
        .map_err(|_| anyhow!("Cannot check catalog SVG asset"))?;
    if response.status() != reqwest::StatusCode::OK
        || !response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|header| header.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("image/svg+xml"))
            })
    {
        return Ok(false);
    }
    let mut size = 0usize;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("Cannot read catalog SVG asset"))?
    {
        size += chunk.len();
        if size > 128 * 1024 {
            return Ok(false);
        }
    }
    Ok(size != 0)
}
pub(super) fn smoke_command(row: &Value) -> Option<String> {
    let smoke: Smoke = serde_json::from_value(row.get("smoke_test")?.clone()).ok()?;
    let path = if let Some(operation) = smoke.operation {
        operations(row)
            .ok()?
            .into_iter()
            .find(|candidate| candidate.name == operation && candidate.method == "GET")?
            .path
    } else {
        if smoke.method.as_deref() != Some("GET") {
            return None;
        }
        smoke.path?
    };
    if !safe_path(&path) {
        return None;
    }
    Some(format!(
        "nyxid proxy request {} {} --method GET",
        row["slug"].as_str()?,
        path
    ))
}

/// Responses may include defaulted revocation options not authored in the manifest.
pub(super) fn matches_fields(desired: &Value, current: &Value) -> bool {
    if let Some(fields) = desired.as_object() {
        fields
            .iter()
            .all(|(key, value)| matches_fields(value, &current[key]))
    } else {
        desired == current
    }
}
pub(super) fn connect_command(slug: &str, provider: Option<&Value>) -> String {
    let option = match provider.and_then(|row| row["provider_type"].as_str()) {
        Some("oauth2") => " --oauth",
        Some("device_code") => " --device-code",
        Some("api_key") => " --credential-env VENDOR_API_KEY",
        _ => "",
    };
    format!("nyxid service add {slug}{option}")
}
