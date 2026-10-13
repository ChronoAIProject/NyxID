//! Secret-free Tool onboarding, with one planner shared by apply and the wizard.
use crate::{
    api::ApiClient,
    cli::{AdminToolsCommands, OutputFormat},
};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::{IsTerminal, Write},
    path::Path,
};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    tools: Vec<Tool>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Tool {
    slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    twin_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_key_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    openapi_spec_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supplier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    topics: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issues_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub examples_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_limitations: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asyncapi_spec_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capabilities: Option<ToolCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    required_permissions: Option<Vec<String>>,
    /// Omission preserves all existing publication choices; [] pauses all live operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operations: Option<Vec<String>>,
    /// CLI readiness lanes; this is never sent to the service API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lanes: Option<Vec<Lane>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum Lane {
    Platform,
    Byok,
}
impl Tool {
    fn declared_lanes(&self) -> &[Lane] {
        self.lanes.as_deref().unwrap_or(&[Lane::Platform])
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolCapabilities {
    #[serde(default)]
    supports_proxy_read: bool,
    #[serde(default)]
    supports_proxy_write: bool,
    #[serde(default)]
    supports_proxy_binary_upload: bool,
    #[serde(default)]
    supports_direct_downstream_auth: bool,
    #[serde(default)]
    supports_authoring_via_nyx: bool,
    #[serde(default)]
    supports_websocket: bool,
    #[serde(default)]
    supports_streaming: bool,
}

fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && s.as_bytes()[0].is_ascii_alphanumeric()
}
fn valid_operation(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}
fn valid_url(s: &str, base: bool) -> bool {
    if s.len() > 2048 || s != s.trim() || s.chars().any(char::is_control) {
        return false;
    }
    url::Url::parse(s).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
            && (!base || u.query().is_none())
    })
}
#[cfg(test)]
fn parse(raw: &str) -> Result<Manifest> {
    // Do not format parser errors: YAML snippets and unknown supplied values may contain secrets.
    parse_expanded(raw, &[], false)
}
fn parse_expanded(raw: &str, provided: &[String], interactive: bool) -> Result<Manifest> {
    let mut value: Value = if let Ok(v) = serde_json::from_str(raw) {
        v
    } else {
        serde_saphyr::from_str(raw)
            .map_err(|_| anyhow!("Invalid Tools manifest structure or unknown fields"))?
    };
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow!("Tools manifest must be an object"))?;
    let variables = object.remove("variables").unwrap_or(json!({}));
    let declared = variables
        .as_object()
        .ok_or_else(|| anyhow!("variables must be a map"))?;
    let mut supplied = std::collections::BTreeMap::new();
    for item in provided {
        let (name, val) = item
            .split_once('=')
            .ok_or_else(|| anyhow!("--set requires NAME=VALUE"))?;
        if !declared.contains_key(name)
            || supplied.insert(name.to_owned(), val.to_owned()).is_some()
        {
            bail!("Undeclared or duplicate variable override");
        }
    }
    for (name, spec) in declared {
        if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
            bail!("Invalid variable name");
        }
        let spec = spec
            .as_object()
            .ok_or_else(|| anyhow!("Invalid variable declaration"))?;
        if spec
            .keys()
            .any(|k| !["prompt", "default", "allowed"].contains(&k.as_str()))
        {
            bail!("Unknown variable declaration field");
        }
        let allowed = spec
            .get("allowed")
            .map(|v| {
                v.as_array()
                    .ok_or_else(|| anyhow!("Invalid variable allowed values"))
            })
            .transpose()?;
        if allowed.is_some_and(|a| a.iter().any(|v| !v.is_string())) {
            bail!("Invalid variable allowed values");
        }
        let val = if let Some(val) = supplied.get(name) {
            val.clone()
        } else if interactive {
            ask(
                spec.get("prompt").and_then(Value::as_str).unwrap_or(name),
                spec.get("default").and_then(Value::as_str),
            )?
        } else {
            spec.get("default")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Missing manifest variable; supply --set"))?
                .to_owned()
        };
        if val.is_empty()
            || !val
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || allowed.is_some_and(|a| !a.iter().any(|v| v == &val))
        {
            bail!("Invalid manifest variable value");
        }
        supplied.insert(name.clone(), val);
    }
    fn expand(v: &mut Value, vars: &std::collections::BTreeMap<String, String>) -> Result<()> {
        match v {
            Value::String(s) => {
                for (name, val) in vars {
                    *s = s.replace(&format!("${{{name}}}"), val);
                }
                if s.contains("${") {
                    bail!("Unresolved manifest variable");
                }
            }
            Value::Array(a) => {
                for v in a {
                    expand(v, vars)?;
                }
            }
            Value::Object(o) => {
                for v in o.values_mut() {
                    expand(v, vars)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    expand(&mut value, &supplied)?;
    let m: Manifest = serde_json::from_value(value)
        .map_err(|_| anyhow!("Invalid Tools manifest structure or unknown fields"))?;
    validate_manifest(&m)?;
    Ok(m)
}
fn validate_manifest(m: &Manifest) -> Result<()> {
    if m.version != 1 || m.tools.is_empty() || m.tools.len() > 100 {
        bail!("Tools manifest requires version 1 and 1–100 tools");
    }
    let mut slugs = HashSet::new();
    for t in &m.tools {
        let mut unique_lanes = HashSet::new();
        if t.declared_lanes().is_empty()
            || !t
                .declared_lanes()
                .iter()
                .all(|lane| unique_lanes.insert(*lane))
        {
            bail!("lanes must be a unique, non-empty list of platform/byok");
        }
        if t.auth_method.as_deref() == Some("none") && t.declared_lanes().contains(&Lane::Byok) {
            bail!("BYOK is invalid for no-auth Tools");
        }
        if !valid_slug(&t.slug) || !slugs.insert(&t.slug) {
            bail!("Invalid or duplicate Tool slug");
        }
        if (t.name.is_none() && t.twin_of.is_none())
            || t.name
                .as_ref()
                .is_some_and(|s| s.trim().is_empty() || s.len() > 200)
            || t.supplier.as_ref().is_some_and(|s| s.chars().count() > 128)
            || t.description.as_ref().is_some_and(|s| s.len() > 500)
        {
            bail!("Invalid Tool metadata length");
        }
        if t.twin_of.is_some() == t.base_url.is_some() {
            bail!("Exactly one of twin_of and base_url is required");
        }
        if let Some(source) = &t.twin_of {
            if !valid_slug(source) && uuid::Uuid::parse_str(source).is_err() {
                bail!("Invalid twin source reference");
            }
            if t.auth_method.is_some() || t.auth_key_name.is_some() {
                bail!("Twins inherit auth_method and auth_key_name; omit auth fields");
            }
        } else {
            if !t.base_url.as_ref().is_some_and(|s| valid_url(s, true)) {
                bail!("Invalid Tool base_url (HTTP(S), no userinfo/query/fragment)");
            }
            let method = t
                .auth_method
                .as_deref()
                .ok_or_else(|| anyhow!("New APIs require auth_method"))?;
            if !["none", "bearer", "header", "query", "basic"].contains(&method) {
                bail!("Unsupported Tool auth_method");
            }
            let named = matches!(method, "header" | "query");
            if named != t.auth_key_name.is_some()
                || t.auth_key_name.as_ref().is_some_and(|s| {
                    s.is_empty()
                        || s.len() > 256
                        || !s
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
                })
            {
                bail!("header/query auth requires a valid auth_key_name; other methods omit it");
            }
            if method == "none" && t.credential_env.is_some() {
                bail!("No-auth Tools cannot configure a credential");
            }
        }
        for url in [
            &t.homepage_url,
            &t.repository_url,
            &t.issues_url,
            &t.examples_url,
        ] {
            if url.as_ref().is_some_and(|v| !valid_url(v, false)) {
                bail!("Invalid metadata URL");
            }
        }
        if t.icon_url
            .as_ref()
            .is_some_and(|url| !url.is_empty() && !valid_url(url, false))
        {
            bail!("Invalid icon URL");
        }
        if t.asyncapi_spec_url.is_some() || t.custom_user_agent.is_some() {
            bail!(
                "AsyncAPI and custom user-agent configuration require explicit Admin review outside onboarding"
            );
        }
        if [&t.auth_notes, &t.known_limitations]
            .iter()
            .any(|v| v.as_ref().is_some_and(|s| s.len() > 4096))
            || t.required_permissions
                .as_ref()
                .is_some_and(|a| a.len() > 50 || a.iter().any(|s| s.is_empty() || s.len() > 256))
        {
            bail!("Invalid metadata length");
        }
        if t.openapi_spec_url
            .as_ref()
            .is_some_and(|s| !valid_url(s, false))
        {
            bail!("Invalid openapi_spec_url");
        }
        if t.credential_env.as_ref().is_some_and(|s| {
            s.is_empty()
                || s.len() > 128
                || !(s.as_bytes()[0].is_ascii_alphabetic() || s.starts_with('_'))
                || !s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        }) {
            bail!("credential_env must be an environment-variable name");
        }
        if let Some(topics) = &t.topics
            && (topics.len() > 20
                || topics.iter().collect::<HashSet<_>>().len() != topics.len()
                || topics.iter().any(|s| !valid_slug(s)))
        {
            bail!("Invalid, duplicate or excessive Tool topics");
        }
        if let Some(ops) = &t.operations
            && (ops.len() > 200
                || ops.iter().collect::<HashSet<_>>().len() != ops.len()
                || ops.iter().any(|s| !valid_operation(s)))
        {
            bail!("Invalid, duplicate or excessive operation selection");
        }
    }
    Ok(())
}
fn read(path: &Path, vars: &[String], interactive: bool) -> Result<Manifest> {
    let raw = Zeroizing::new(
        std::fs::read_to_string(path).map_err(|_| anyhow!("Cannot read Tools manifest"))?,
    );
    if raw.len() > 1024 * 1024 {
        bail!("Tools manifest exceeds 1 MiB");
    }
    parse_expanded(&raw, vars, interactive)
}

#[derive(Debug, Serialize)]
struct PlanRow {
    slug: String,
    action: &'static str,
    changed_fields: Vec<String>,
    discovery: bool,
    credential: &'static str,
    admin_work: Vec<&'static str>,
    operation_changes: Vec<Value>,
}
struct Planned {
    desired: Tool,
    current: Option<Value>,
    create: Value,
    update: Value,
    endpoints: Vec<Value>,
    row: PlanRow,
}
fn rows(value: &Value, key: &str) -> Vec<Value> {
    value[key].as_array().cloned().unwrap_or_default()
}
fn metadata_matches(actual: &Value, desired: &Value) -> bool {
    match desired {
        Value::Object(map) => map
            .iter()
            .all(|(key, value)| metadata_matches(&actual[key], value)),
        _ => actual == desired,
    }
}
fn tool_shape(v: &Value) -> bool {
    v["offering_kind"] == "tool"
        && v["service_type"] == "http"
        && v["service_category"] == "internal"
        && v["requires_user_credential"] == false
        && v["provider_config_id"].is_null()
}
fn credential_needed(v: &Value) -> bool {
    v["auth_method"] != "none"
        && v["credential_configured"] != true
        && v["tools_credential_frozen"] != true
}
fn selected(t: &Tool, name: &str) -> bool {
    t.operations
        .as_ref()
        .is_some_and(|names| names.iter().any(|n| n == name))
}

async fn plan(api: &mut ApiClient, m: &Manifest, me: &Value) -> Result<Vec<Planned>> {
    if m.tools.iter().any(|tool| {
        tool.topics
            .as_ref()
            .is_some_and(|topics| !topics.is_empty())
    }) {
        let vocabulary: Vec<Value> = api.get("/tools/topics").await?;
        for topic in m.tools.iter().flat_map(|tool| tool.topics.iter().flatten()) {
            if !vocabulary.iter().any(|row| row["slug"] == *topic) {
                bail!("Tool topic is absent from the server vocabulary");
            }
        }
    }
    let inventory: Value = api.get("/services?include_inactive=true").await?;
    let inventory = rows(&inventory, "services");
    let admin = me["is_admin"] == true;
    if !admin && me["capabilities"]["manage_tools"] != true {
        bail!("Admin or Operator with Tools Manager is required");
    }
    let mut planned = Vec::new();
    // Read and validate every row before beginning mutations.
    for t in &m.tools {
        let current = super::admin_catalog::unambiguous_row(&inventory, &t.slug)?.cloned();
        let source = if current.is_none() {
            if let Some(reference) = &t.twin_of {
                let source = super::admin_catalog::unambiguous_row(&inventory, reference)?
                    .ok_or_else(|| anyhow!("Twin source is missing or not visible"))?;
                if source["is_active"] == false
                    || source["service_type"] != "http"
                    || !["none", "bearer", "header", "query", "basic"]
                        .contains(&source["auth_method"].as_str().unwrap_or_default())
                {
                    bail!("Twin source is not an eligible active HTTP service");
                }
                Some(source)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(v) = &current {
            if !tool_shape(v) {
                bail!("Existing slug is not a provider-less internal HTTP Tool");
            }
            if !admin
                && (v["tools_manager_eligible"] != true
                    || v["tools_manager_created"] != true
                    || v["created_by"] != me["id"]
                    || (v["owner_user_id"].is_string() && v["owner_user_id"] != me["id"]))
            {
                bail!(
                    "Existing Tool eligibility is unproven, incompatible, legacy or foreign; ask an Admin"
                );
            }
            if let Some(source) = &t.twin_of {
                let source_slug = super::admin_catalog::unambiguous_row(&inventory, source)?
                    .and_then(|v| v["slug"].as_str())
                    .unwrap_or(source);
                if v["import_source"]["kind"] != "catalog_twin"
                    || v["import_source"]["reference"] != source_slug
                {
                    bail!("Existing Tool has a different twin source");
                }
            } else if v["import_source"]["kind"] == "catalog_twin" {
                bail!("Existing Tool is a twin; new-API manifest is incompatible");
            }
            for (key, desired) in [
                ("auth_method", t.auth_method.as_ref()),
                ("auth_key_name", t.auth_key_name.as_ref()),
            ] {
                if desired.is_some_and(|d| v[key] != *d) {
                    bail!("Existing Tool has incompatible creation-only auth settings");
                }
            }
            if t.base_url.as_ref().is_some_and(|d| v["base_url"] != *d)
                && (v["tools_credential_frozen"] == true || v["credential_configured"] == true)
            {
                bail!("Existing credential destination is frozen; use an explicit Admin update");
            }
        }
        let effective_auth = current
            .as_ref()
            .or(source)
            .and_then(|row| row["auth_method"].as_str())
            .or(t.auth_method.as_deref());
        if t.declared_lanes().contains(&Lane::Byok) && effective_auth == Some("none") {
            bail!("BYOK is invalid for no-auth Tools, including inherited twin authentication");
        }
        let mut create = json!({"slug":t.slug,"offering_kind":"tool","service_category":"internal","service_type":"http"});
        if let Some(name) = &t.name {
            create["name"] = name.clone().into();
        }
        if admin {
            // Authenticated drafts need an explicit platform binding. Audience
            // approval stays a separate, deliberate Admin setup action.
            create["platform_key"] =
                json!({"enabled":true,"audience":"restricted","allowed_owner_ids":[]});
        }
        for (key, v) in [
            ("base_url", &t.base_url),
            ("auth_method", &t.auth_method),
            ("auth_key_name", &t.auth_key_name),
            ("openapi_spec_url", &t.openapi_spec_url),
            ("description", &t.description),
            ("supplier", &t.supplier),
        ] {
            if let Some(v) = v {
                create[key] = v.clone().into();
            }
        }
        let metadata = serde_json::to_value(t)?;
        for key in [
            "homepage_url",
            "repository_url",
            "issues_url",
            "examples_url",
            "icon_url",
            "auth_notes",
            "known_limitations",
            "capabilities",
            "required_permissions",
        ] {
            if let Some(v) = metadata.get(key) {
                create[key] = v.clone();
            }
        }
        if let Some(topics) = &t.topics {
            create["topics"] = json!(topics);
        }
        if let Some(source) = &t.twin_of {
            create["twin_of_service_id"] = source.clone().into();
        }
        let mut update = json!({});
        if let Some(v) = &current {
            for key in [
                "name",
                "description",
                "supplier",
                "topics",
                "base_url",
                "openapi_spec_url",
                "homepage_url",
                "repository_url",
                "issues_url",
                "examples_url",
                "icon_url",
                "auth_notes",
                "known_limitations",
                "capabilities",
                "required_permissions",
            ] {
                if let Some(wanted) = create.get(key)
                    && !metadata_matches(&v[key], wanted)
                {
                    update[key] = wanted.clone();
                }
            }
            if update.get("openapi_spec_url").is_some() {
                bail!(
                    "Desired spec URL changed; update it explicitly and rediscover/review operations before applying"
                );
            }
        }
        let endpoints = match &current {
            Some(v) => {
                let data: Value = api
                    .get(&format!(
                        "/services/{}/endpoints",
                        v["id"].as_str().unwrap_or_default()
                    ))
                    .await?;
                rows(&data, "endpoints")
            }
            None => Vec::new(),
        };
        let discovery = endpoints.is_empty()
            && (t.openapi_spec_url.is_some()
                || current
                    .as_ref()
                    .is_some_and(|v| v["openapi_spec_url"].is_string())
                || t.twin_of.is_some());
        if !endpoints.is_empty() {
            check_selection(t, &endpoints)?;
        }
        let credential = match &current {
            Some(v) if v["credential_configured"] == true || v["auth_method"] == "none" => {
                "configured"
            }
            Some(v) if v["tools_credential_frozen"] == true => "pending_admin",
            None if t.auth_method.as_deref() == Some("none")
                || source.is_some_and(|s| s["auth_method"] == "none") =>
            {
                "not_required"
            }
            _ => "pending_credential",
        };
        let changed_fields = update.as_object().unwrap().keys().cloned().collect();
        let operation_changes = endpoints
            .iter()
            .filter_map(|e| {
                let name = e["name"].as_str()?;
                let desired = if selected(t, name) {
                    if admin || e["publication"] == "published" {
                        "published"
                    } else {
                        "validated"
                    }
                } else if t.operations.is_some() && e["publication"] == "published" {
                    "paused"
                } else {
                    return None;
                };
                (e["publication"] != desired)
                    .then(|| json!({"name":name,"from":e["publication"],"to":desired}))
            })
            .collect();
        let row = PlanRow {
            operation_changes,
            slug: t.slug.clone(),
            action: if current.is_none() {
                "create"
            } else if update.as_object().unwrap().is_empty() {
                "unchanged"
            } else {
                "update"
            },
            changed_fields,
            discovery,
            credential,
            admin_work: if admin {
                vec![]
            } else {
                vec![
                    "approve audience and pricing",
                    "publish selected operations",
                ]
            },
        };
        planned.push(Planned {
            desired: t.clone(),
            current,
            create,
            update,
            endpoints,
            row,
        });
    }
    Ok(planned)
}
fn check_selection(t: &Tool, endpoints: &[Value]) -> Result<()> {
    let mut names = HashSet::new();
    if endpoints
        .iter()
        .filter_map(|e| e["name"].as_str())
        .any(|name| !names.insert(name))
    {
        bail!("Server returned duplicate operation names; resolve them before applying");
    }
    if t.operations.as_ref().is_some_and(|names| {
        names
            .iter()
            .any(|n| !endpoints.iter().any(|e| e["name"] == *n))
    }) {
        bail!("Selected operation is missing; inspect endpoints and correct the manifest");
    }
    Ok(())
}

#[derive(Serialize)]
struct SecretBody<'a> {
    credential: &'a str,
}
async fn set_credential(api: &mut ApiClient, id: &str, secret: &str) -> Result<Value> {
    // Upstream bodies can reflect supplied secrets. Never propagate the error chain.
    api.put(&format!("/services/{id}"), &SecretBody{credential:secret}).await.map_err(|_| anyhow!("Shared credential installation failed; check authority and first-set state (response redacted)"))
}
fn env_secret(t: &Tool) -> Result<Zeroizing<String>> {
    let name = t.credential_env.as_deref().ok_or_else(|| anyhow!("Missing credential_env; provide an environment-variable name or use the interactive wizard"))?;
    let value = Zeroizing::new(
        std::env::var(name)
            .map_err(|_| anyhow!("Required credential environment variable is unavailable"))?,
    );
    if value.trim().is_empty() {
        bail!("Credential environment variable is empty");
    }
    Ok(value)
}
async fn apply(
    api: &mut ApiClient,
    plans: Vec<Planned>,
    me: &Value,
    interactive: bool,
) -> Result<(Vec<PlanRow>, Manifest)> {
    apply_options(api, plans, me, interactive, false).await
}
async fn apply_options(
    api: &mut ApiClient,
    mut plans: Vec<Planned>,
    me: &Value,
    interactive: bool,
    skip_credentials: bool,
) -> Result<(Vec<PlanRow>, Manifest)> {
    let admin = me["is_admin"] == true;
    let mut report = Vec::new();
    for p in &mut plans {
        let mut wrote = false;
        let mut current = if let Some(v) = &p.current {
            v.clone()
        } else {
            wrote = true;
            api.post("/services", &p.create).await?
        };
        let id = current["id"]
            .as_str()
            .ok_or_else(|| anyhow!("Server omitted Tool id"))?
            .to_owned();
        if !p.update.as_object().unwrap().is_empty() {
            current = api.put(&format!("/services/{id}"), &p.update).await?;
            wrote = true;
        }
        if !skip_credentials && credential_needed(&current) {
            let secret = if p.desired.credential_env.is_some() {
                env_secret(&p.desired)?
            } else if interactive {
                terminal()?;
                Zeroizing::new(
                    rpassword::prompt_password("Shared credential (hidden): ")
                        .map_err(|_| anyhow!("Credential input failed"))?,
                )
            } else {
                bail!("Tool needs a credential; set credential_env in the manifest");
            };
            if secret.trim().is_empty() {
                bail!("Credential must not be empty");
            }
            // The client already enforces HTTPS/loopback and rejects redirects.
            current = set_credential(api, &id, &secret).await?;
            wrote = true;
        }
        let endpoints: Value = api.get(&format!("/services/{id}/endpoints")).await?;
        p.endpoints = rows(&endpoints, "endpoints");
        if p.endpoints.is_empty() && current["openapi_spec_url"].is_string() {
            let discovered: Value = api
                .post(&format!("/services/{id}/discover-endpoints"), &json!({}))
                .await?;
            p.endpoints = rows(&discovered, "endpoints");
            if p.endpoints.is_empty() {
                bail!("Spec has no operations; configure a usable spec before applying again");
            }
            wrote = true;
        }
        if interactive && p.desired.operations.is_none() {
            terminal()?;
            eprintln!(
                "Operations: {}",
                p.endpoints
                    .iter()
                    .filter_map(|e| e["name"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let names = ask(
                "Select operation names (comma-separated; empty selects none)",
                None,
            )?;
            p.desired.operations = Some(
                names
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            );
        }
        check_selection(&p.desired, &p.endpoints)?;
        if p.desired.operations.is_some() {
            let prepare: Vec<_> = p
                .endpoints
                .iter()
                .filter(|e| {
                    selected(&p.desired, e["name"].as_str().unwrap_or_default())
                        && if admin {
                            e["publication"] != "published" || e["is_active"] != true
                        } else {
                            matches!(e["publication"].as_str(), Some("draft" | "paused"))
                        }
                })
                .filter_map(|e| e["name"].as_str())
                .collect();
            let pause: Vec<_> = p
                .endpoints
                .iter()
                .filter(|e| {
                    !selected(&p.desired, e["name"].as_str().unwrap_or_default())
                        && e["publication"] == "published"
                })
                .filter_map(|e| e["name"].as_str())
                .collect();
            for (names, state) in [
                (prepare, if admin { "published" } else { "validated" }),
                (pause, "paused"),
            ] {
                if !names.is_empty() {
                    let _: Value = api
                        .post(
                            &format!("/services/{id}/publication"),
                            &json!({"state":state,"endpoint_names":names}),
                        )
                        .await?;
                    wrote = true;
                }
            }
        }
        p.row.action = if !wrote {
            "unchanged"
        } else if p.current.is_none() {
            "created"
        } else {
            "updated"
        };
        p.row.credential =
            if current["credential_configured"] == true || current["auth_method"] == "none" {
                "configured"
            } else if current["tools_credential_frozen"] == true {
                "pending_admin"
            } else {
                "pending_credential"
            };
        p.row.discovery = false;
        report.push(std::mem::replace(
            &mut p.row,
            PlanRow {
                slug: String::new(),
                action: "unchanged",
                changed_fields: vec![],
                discovery: false,
                credential: "configured",
                admin_work: vec![],
                operation_changes: vec![],
            },
        ));
    }
    let manifest = Manifest {
        version: 1,
        tools: plans.iter().map(|p| p.desired.clone()).collect(),
    };
    Ok((report, manifest))
}

#[derive(Debug, Serialize)]
struct Readiness {
    slug: String,
    status: &'static str,
    reasons: Vec<&'static str>,
    credential: &'static str,
    audience: Value,
    ordinary_user_eligibility: &'static str,
    operations: Vec<Value>,
    pricing: Value,
    lark: Value,
    verification: Value,
    lanes: Value,
    admin_commands: Vec<String>,
    test_command: Option<String>,
}
fn lane_prices(v: &Value, chosen: &[&Value], lane_key: &str) -> (Value, Vec<Value>) {
    let billing = &v["billing"];
    let lane = &billing[lane_key];
    let legacy = if billing["platform_billable"] != true {
        json!({"effective":"free","credits_per_unit":"0"})
    } else if billing["platform_pricing"]["sync_status"] == "synced" {
        json!({"effective":"legacy_configured","primary":billing["platform_pricing"],"metric":billing["platform_metric"]})
    } else {
        json!({"effective":"external_plan_price_unknown","metric":billing["platform_metric"],"configured":billing["platform_pricing"]})
    };
    let lane_configured =
        !billing["platform_key_pricing"].is_null() || !billing["byok_pricing"].is_null();
    let base = if v["auth_method"] == "none" {
        json!({"effective":"free","credits_per_unit":"0","reason":"no_auth"})
    } else if lane_configured && lane.is_null() {
        json!({"effective":"free","credits_per_unit":"0","reason":"missing_credential_lane"})
    } else if lane["sync_status"] == "synced" {
        json!({"effective":"configured","primary":lane})
    } else {
        legacy
    };
    let components:Vec<_> = lane["components"].as_array().into_iter().flatten().map(|c| json!({"configured":c,"effective":if lane["sync_status"]=="synced" && c["sync_status"]=="synced" && v["auth_method"]!="none" { c.clone() } else {json!({"credits_per_unit":"0","reason":"component_or_primary_unsynced"})}})).collect();
    let pricing = json!({"effective":base["effective"],"primary":base,"configured":lane,"components":components});
    let operation_values: Vec<Value> = chosen.iter().map(|e| {
        let name=e["name"].as_str().unwrap_or_default();
        let op=lane["operations"].as_array().and_then(|a|a.iter().find(|o|o["operation"]==name));
        let effective = op.filter(|o| lane["metric"]=="requests" && lane["sync_status"]=="synced" && o["sync_status"]=="synced" && v["auth_method"]!="none").cloned().unwrap_or_else(||base.clone());
        json!({"name":name,"publication":e["publication"],"is_active":e["is_active"],"price":op.unwrap_or(lane),"effective_price":effective,"components":components})
    }).collect();
    (pricing, operation_values)
}
fn readiness_status(failed: bool, reasons: &[&str]) -> &'static str {
    if failed {
        "failed"
    } else if reasons.is_empty() {
        "ready"
    } else if reasons.contains(&"credential_missing") {
        "pending_credential"
    } else {
        "pending_admin"
    }
}
fn readiness(t: &Tool, current: Option<&Value>, endpoints: &[Value]) -> Readiness {
    let mut common = Vec::new();
    let mut failed = false;
    if current.is_none_or(|v| !tool_shape(v)) {
        common.push("missing_or_incompatible_tool");
        failed = true;
    }
    let v = current.cloned().unwrap_or(Value::Null);
    if v["is_active"] != true {
        common.push("service_inactive");
    }
    if check_selection(t, endpoints).is_err() {
        common.push("selected_operation_missing");
        failed = true;
    }
    let chosen: Vec<_> = endpoints
        .iter()
        .filter(|e| t.operations.is_none() || selected(t, e["name"].as_str().unwrap_or_default()))
        .collect();
    if chosen.is_empty() {
        common.push("no_selected_operations");
    }
    if chosen
        .iter()
        .any(|e| e["publication"] != "published" || e["is_active"] != true)
    {
        common.push("admin_publication_required");
    }
    let credential = if v["auth_method"] == "none" {
        "not_required"
    } else if v["credential_configured"] == true {
        "configured"
    } else {
        "missing"
    };
    let audience = v["platform_key"].clone();
    let public = audience["enabled"] == true && audience["audience"] == "public";
    let restricted = audience["enabled"] == true
        && audience["audience"] == "restricted"
        && audience["allowed_owner_ids"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty());
    let (pricing, operation_values) = lane_prices(&v, &chosen, "platform_key_pricing");
    let mut reasons = common.clone();
    let mut lane_reports = serde_json::Map::new();
    let mut gates = Vec::new();
    let mut admin_commands = Vec::new();
    for lane in t.declared_lanes() {
        let mut lane_reasons = common.clone();
        let mut lane_failed = failed;
        let (name, gate) = match lane {
            Lane::Platform => ("platform", "outside_user_platform_call_admin_confirmed"),
            Lane::Byok => ("byok", "outside_user_byok_call_admin_confirmed"),
        };
        gates.push(gate);
        let mut lane_report = match lane {
            Lane::Platform => {
                if credential == "missing" {
                    lane_reasons.push("credential_missing");
                }
                if !public && !restricted {
                    lane_reasons.push("admin_audience_setup_required");
                }
                json!({"credential":credential,"audience":{"configuration":audience,"ready":public||restricted},"pricing":pricing,"operations":operation_values})
            }
            Lane::Byok => {
                if !matches!(
                    v["auth_method"].as_str(),
                    Some("bearer" | "header" | "query" | "basic")
                ) {
                    lane_reasons.push("byok_requires_authenticated_tool");
                    lane_failed = true;
                }
                let (byok_pricing, operations) = lane_prices(&v, &chosen, "byok_pricing");
                json!({"credential":"user_key_required","auth_method":v["auth_method"],"connect_command":format!("nyxid service add {} --credential-env VENDOR_API_KEY",t.slug),"pricing":byok_pricing,"operations":operations})
            }
        };
        let status = readiness_status(lane_failed, &lane_reasons);
        lane_report["status"] = status.into();
        lane_report["pending_gates"] = json!([gate]);
        if v["auth_method"] != "none" && lane_report["pricing"]["effective"] == "free" {
            let reason = if *lane == Lane::Platform {
                "pricing_unconfigured_effective_free"
            } else {
                "byok_pricing_unconfigured_effective_free"
            };
            lane_reasons.push(reason);
            let (field, flag) = if *lane == Lane::Platform {
                ("platform_key_pricing", "platform-key")
            } else {
                ("byok_pricing", "byok")
            };
            admin_commands.push(format!("Review billing.{field}; configure with nyxid service update {} --catalog-admin --{flag}-metric requests --{flag}-price <credits>, or confirm intentional free access",t.slug));
        }
        for reason in &lane_reasons {
            if !reasons.contains(reason) {
                reasons.push(*reason);
            }
        }
        lane_report["reasons"] = json!(lane_reasons);
        lane_reports.insert(name.to_owned(), lane_report);
    }
    // Free-pricing advisories do not block configuration readiness.
    let blocking: Vec<_> = reasons
        .iter()
        .copied()
        .filter(|reason| {
            !matches!(
                *reason,
                "pricing_unconfigured_effective_free" | "byok_pricing_unconfigured_effective_free"
            )
        })
        .collect();
    failed |= lane_reports.values().any(|row| row["status"] == "failed");
    let status = readiness_status(failed, &blocking);
    let platform_declared = t.declared_lanes().contains(&Lane::Platform);
    if platform_declared && !public && !restricted {
        admin_commands.push(format!("nyxid service update {} --catalog-admin --platform-key-enabled true --platform-key-audience public",t.slug));
    }
    if v["is_active"] == false {
        admin_commands.push(format!(
            "nyxid service update {} --catalog-admin --active",
            t.slug
        ));
    }
    if platform_declared && credential == "missing" && v["tools_credential_frozen"] == true {
        admin_commands.push(format!(
            "nyxid service update {} --catalog-admin --credential-env {}",
            t.slug,
            t.credential_env
                .as_deref()
                .unwrap_or("TOOLS_VENDOR_CREDENTIAL")
        ));
    }
    let ops = chosen
        .iter()
        .filter(|e| e["publication"] != "published" || e["is_active"] != true)
        .filter_map(|e| e["name"].as_str())
        .filter(|name| valid_operation(name))
        .map(|name| format!(" --operation {name}"))
        .collect::<String>();
    if !ops.is_empty() {
        admin_commands.push(format!("nyxid catalog publish {}{ops}", t.slug));
    }
    Readiness {
        slug: t.slug.clone(),
        status,
        reasons,
        credential,
        audience: json!({"configuration":audience,"ready":public||restricted}),
        ordinary_user_eligibility: if status == "ready" && platform_declared && public {
            "eligible_authenticated_users"
        } else if status == "ready" && platform_declared && restricted {
            "restricted_membership_not_evaluated"
        } else if status == "ready" && !platform_declared {
            "requires_user_key_connection"
        } else {
            "not_ready"
        },
        operations: operation_values,
        pricing,
        lark: json!({"name":v["name"],"supplier":v["supplier"],"slug":t.slug,"lanes":t.declared_lanes(),"configuration":status,"remarks":"configured; not verified","operations":t.operations}),
        verification: json!({"status":"not_verified","pending_gates":gates,"billing_check":if v["auth_method"]=="none" {"not_applicable"} else {"acting_user_per_declared_lane"}}),
        lanes: Value::Object(lane_reports),
        test_command: chosen
            .iter()
            .find(|e| {
                e["method"] == "GET"
                    && e["path"].as_str().is_some_and(|p| {
                        !p.contains('{')
                            && p.bytes()
                                .all(|c| c.is_ascii_alphanumeric() || b"/._-".contains(&c))
                    })
            })
            .and_then(|e| e["path"].as_str())
            .map(|path| format!("nyxid proxy request {} {} --method GET", t.slug, path)),
        admin_commands,
    }
}

async fn validate(api: &mut ApiClient, m: &Manifest) -> Result<Vec<Readiness>> {
    let me: Value = api.get("/users/me").await?;
    let inventory: Value = api.get("/services?include_inactive=true").await?;
    let inventory = rows(&inventory, "services");
    let mut report = Vec::new();
    for t in &m.tools {
        let current = match super::admin_catalog::unambiguous_row(&inventory, &t.slug) {
            Ok(current) => current,
            Err(_) => {
                let mut row = readiness(t, None, &[]);
                row.reasons = vec!["ambiguous_existing_tool"];
                row.status = "failed";
                row.admin_commands.clear();
                row.test_command = None;
                row.lark["configuration"] = "failed".into();
                report.push(row);
                continue;
            }
        };
        let manager_ineligible = me["is_admin"] != true
            && current.is_some_and(|row| {
                row["tools_manager_eligible"] != true
                    || row["tools_manager_created"] != true
                    || row["created_by"] != me["id"]
                    || (row["owner_user_id"].is_string() && row["owner_user_id"] != me["id"])
            });
        let endpoints = if let Some(v) = current {
            let data: Value = api
                .get(&format!(
                    "/services/{}/endpoints",
                    v["id"].as_str().unwrap_or_default()
                ))
                .await?;
            rows(&data, "endpoints")
        } else {
            vec![]
        };
        let mut row = readiness(t, current, &endpoints);
        if manager_ineligible && row.status != "failed" {
            row.status = "pending_admin";
            row.reasons.push("manager_ineligible_shape");
            row.ordinary_user_eligibility = "not_ready";
            row.lark["configuration"] = "pending_admin".into();
            if let Some(lanes) = row.lanes.as_object_mut() {
                for lane in lanes.values_mut() {
                    lane["status"] = "pending_admin".into();
                }
            }
        }
        report.push(row);
    }
    Ok(report)
}

fn print<T: Serialize>(kind: &str, report: &T, output: OutputFormat) -> Result<()> {
    let value = json!({"version":1,"kind":kind,"tools":report});
    if matches!(output, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        let mut table = comfy_table::Table::new();
        table.set_header([
            "Tool",
            "Status",
            "Credential",
            "Changed fields",
            "Discovery",
            "Admin work",
        ]);
        for row in value["tools"].as_array().into_iter().flatten() {
            let text = |key: &str| row[key].as_str().unwrap_or("—").to_owned();
            let join = |key: &str| {
                row[key]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default()
            };
            table.add_row(vec![
                text("slug"),
                row["status"]
                    .as_str()
                    .or(row["action"].as_str())
                    .unwrap_or("—")
                    .to_owned(),
                text("credential"),
                join("changed_fields"),
                row["discovery"]
                    .as_bool()
                    .map(|v| if v { "needed" } else { "none" })
                    .unwrap_or("—")
                    .to_owned(),
                join("admin_work"),
            ]);
        }
        println!("{kind}\n{table}");
        for row in value["tools"].as_array().into_iter().flatten() {
            if row.get("reasons").is_some() {
                println!(
                    "{}: reasons={} audience={} eligibility={} pricing={}",
                    row["slug"].as_str().unwrap_or_default(),
                    row["reasons"],
                    row["audience"],
                    row["ordinary_user_eligibility"],
                    row["pricing"]
                );
                for operation in row["operations"].as_array().into_iter().flatten() {
                    println!("  operation: {operation}");
                }
                if let Some(command) = row["test_command"].as_str() {
                    println!("  Test: {command}");
                }
                for command in row["admin_commands"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    println!("  Admin: {command}");
                }
                println!("  Lanes: {}", row["lanes"]);
                println!("  Verification: {}", row["verification"]);
                println!("  Lark: {}", row["lark"]);
            }
            if let Some(url) = row["icon_url"].as_str() {
                println!("Icon: {url}");
            }
        }
    }
    Ok(())
}
#[derive(Debug, thiserror::Error)]
#[error("Tool readiness is pending Admin setup or credentials")]
pub struct PendingReadiness {
    pub exit_code: i32,
}
fn terminal() -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!(
            "Interactive Tools wizard requires a terminal; use --file/--template and --yes with credential_env"
        );
    }
    Ok(())
}
fn ask(label: &str, default: Option<&str>) -> Result<String> {
    eprint!(
        "{label}{}: ",
        default.map(|s| format!(" [{s}]")).unwrap_or_default()
    );
    std::io::stderr().flush()?;
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line)? == 0 {
        bail!("Wizard input ended");
    }
    let line = line.trim();
    Ok(if line.is_empty() {
        default.unwrap_or_default().to_owned()
    } else {
        line.to_owned()
    })
}
fn wizard() -> Result<Manifest> {
    terminal()?;
    let mode = ask("Creation mode: twin or new", Some("twin"))?;
    if !["twin", "new"].contains(&mode.as_str()) {
        bail!("Choose twin or new");
    }
    let slug = ask("Tool slug", None)?;
    let name = ask("Display name", None)?;
    let supplier = ask("Supplier", Some(&name))?;
    let topics = ask("Topics (comma-separated)", Some("web-search,page-fetch"))?;
    let mut t = Tool {
        slug,
        name: Some(name),
        twin_of: None,
        base_url: None,
        auth_method: None,
        auth_key_name: None,
        openapi_spec_url: None,
        description: None,
        supplier: Some(supplier),
        topics: Some(
            topics
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        credential_env: None,
        operations: None,
        ..Tool::default()
    };
    if mode == "twin" {
        t.twin_of = Some(ask("Source catalog slug", Some("api-firecrawl"))?);
    } else {
        t.base_url = Some(ask("API base URL", None)?);
        let auth = ask("Auth: none/bearer/header/query/basic", Some("bearer"))?;
        if matches!(auth.as_str(), "header" | "query") {
            t.auth_key_name = Some(ask("Auth key name", None)?);
        }
        t.auth_method = Some(auth);
        let spec = ask("OpenAPI spec URL (optional)", None)?;
        if !spec.is_empty() {
            t.openapi_spec_url = Some(spec);
        }
    }
    if t.auth_method.as_deref() != Some("none") {
        let env = ask(
            "Credential environment-variable name (empty for hidden prompt)",
            None,
        )?;
        if !env.is_empty() {
            t.credential_env = Some(env);
        }
    }
    let m = Manifest {
        version: 1,
        tools: vec![t],
    };
    validate_manifest(&m)?;
    Ok(m)
}
pub async fn run(command: AdminToolsCommands) -> Result<()> {
    match command {
        AdminToolsCommands::Icon { tool, file, auth } => {
            let bytes = std::fs::read(file).map_err(|_| anyhow!("Cannot read SVG file"))?;
            if bytes.len() > 128 * 1024 {
                bail!("SVG exceeds 128 KiB");
            }
            let svg = String::from_utf8(bytes).map_err(|_| anyhow!("SVG must be UTF-8"))?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let inventory: Value = api.get("/services?include_inactive=true").await?;
            let inventory = rows(&inventory, "services");
            let v = super::admin_catalog::unambiguous_row(&inventory, &tool)?
                .ok_or_else(|| anyhow!("Tool not found"))?;
            if !tool_shape(v) {
                bail!("Icon requires a provider-less Tool");
            }
            let result: Value = api
                .post(
                    &format!("/services/{}/icon", v["id"].as_str().unwrap_or_default()),
                    &json!({"svg":svg}),
                )
                .await?;
            print("icon", &vec![result], auth.output)
        }
        AdminToolsCommands::Apply {
            file,
            dry_run,
            vars,
            auth,
        } => {
            let m = read(&file, &vars, false)?;
            let mut api = if dry_run {
                ApiClient::from_auth(&auth)?.without_token_refresh()
            } else {
                ApiClient::from_auth_with_credential_transport(&auth)?
            };
            let me: Value = api.get("/users/me").await?;
            let plans = plan(&mut api, &m, &me).await?;
            if dry_run {
                print(
                    "plan",
                    &plans.iter().map(|p| &p.row).collect::<Vec<_>>(),
                    auth.output,
                )
            } else {
                let (report, _) = apply(&mut api, plans, &me, false).await?;
                print("apply", &report, auth.output)
            }
        }
        AdminToolsCommands::Validate { file, vars, auth } => {
            let m = read(&file, &vars, false)?;
            let mut api = ApiClient::from_auth(&auth)?.without_token_refresh();
            let report = validate(&mut api, &m).await?;
            print("validation", &report, auth.output)?;
            if report.iter().any(|r| r.status == "failed") {
                return Err(PendingReadiness { exit_code: 1 }.into());
            }
            if report.iter().any(|r| r.status != "ready") {
                return Err(PendingReadiness { exit_code: 2 }.into());
            }
            Ok(())
        }
        AdminToolsCommands::Add {
            file,
            template,
            save,
            yes,
            vars,
            skip_credentials,
            credential_env,
            auth,
        } => {
            let interactive =
                !yes && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
            let mut m = match (file, template) {
                (Some(p), None) => read(&p, &vars, interactive)?,
                (None, Some(t)) if t == "firecrawl" => parse_expanded(
                    include_str!("../../templates/tools/firecrawl.yaml"),
                    &vars,
                    interactive,
                )?,
                (None, None) => wizard()?,
                _ => bail!("Unknown Tools template; available: firecrawl"),
            };
            if let Some(env) = credential_env {
                if m.tools.len() != 1 {
                    bail!("--credential-env requires one Tool");
                }
                m.tools[0].credential_env = Some(env);
                validate_manifest(&m)?;
            }
            if !yes {
                terminal()?;
            }
            let mut api = ApiClient::from_auth_with_credential_transport(&auth)?;
            let me: Value = api.get("/users/me").await?;
            let plans = plan(&mut api, &m, &me).await?;
            let plan_report = json!(plans.iter().map(|p| &p.row).collect::<Vec<_>>());
            if !matches!(auth.output, OutputFormat::Json) {
                print("plan", &plan_report, auth.output)?;
            }
            if !yes && ask("Apply these changes?", Some("no"))? != "yes" {
                bail!("Tool onboarding cancelled");
            }
            let (report, selected_manifest) =
                apply_options(&mut api, plans, &me, interactive, skip_credentials).await?;
            if let Some(path) = save {
                // Include the wizard's final explicit operation choice, never its secret.
                std::fs::write(path, serde_json::to_string_pretty(&selected_manifest)?)?;
            }
            let apply_report = report;
            if !matches!(auth.output, OutputFormat::Json) {
                print("apply", &apply_report, auth.output)?;
            }
            let report = validate(&mut api, &selected_manifest).await?;
            if matches!(auth.output, OutputFormat::Json) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"version":1,"kind":"onboarding","plan":plan_report,"apply":apply_report,"tools":report})
                    )?
                );
            } else {
                print("validation", &report, auth.output)?;
            }

            if report.iter().any(|r| r.status == "failed") {
                return Err(PendingReadiness { exit_code: 1 }.into());
            }
            if report.iter().any(|r| r.status != "ready") {
                return Err(PendingReadiness { exit_code: 2 }.into());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    fn manifest() -> Manifest {
        parse(r#"{"version":1,"tools":[{"slug":"tools-test","name":"Test","base_url":"https://vendor.example/api","auth_method":"bearer","credential_env":"THIS_ENV_MUST_NEVER_BE_READ","operations":["search"]}]}"#).unwrap()
    }
    fn tool() -> Value {
        json!({"id":"tool-id","created_by":"manager","owner_user_id":"manager","slug":"tools-test","name":"Test","offering_kind":"tool","service_type":"http","service_category":"internal","requires_user_credential":false,"provider_config_id":null,"base_url":"https://vendor.example/api","auth_method":"bearer","auth_key_name":"Authorization","is_active":true,"tools_manager_created":true,"tools_manager_eligible":true,"credential_configured":true,"tools_credential_frozen":true,"platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[]}})
    }
    fn me(admin: bool) -> Value {
        json!({"id":"manager","is_admin":admin,"capabilities":{"manage_tools":true}})
    }
    fn endpoint(state: &str) -> Value {
        json!({"id":"endpoint-id","name":"search","publication":state,"is_active":state=="published"})
    }
    async fn mock_inventory(server: &MockServer, tool: Value, endpoints: Vec<Value>) {
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(me(false)))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"services":[tool]})))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/services/tool-id/endpoints"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"endpoints":endpoints})))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn preflight_all_rows_rejects_unproven_eligibility_without_writes() {
        for eligibility in [Value::Null, json!(false)] {
            let server = MockServer::start().await;
            let mut later = tool();
            later["id"] = "later-id".into();
            later["slug"] = "tools-later".into();
            later["tools_manager_eligible"] = eligibility;
            Mock::given(method("GET"))
                .and(path("/api/v1/services"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"services":[tool(),later]})),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/api/v1/services/tool-id/endpoints"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"endpoints":[endpoint("published")]})),
                )
                .mount(&server)
                .await;
            let mut m = manifest();
            m.tools[0].name = Some("would write first".into());
            let mut later = m.tools[0].clone();
            later.slug = "tools-later".into();
            m.tools.push(later);
            let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
            assert!(plan(&mut api, &m, &me(false)).await.is_err());
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method == "GET")
            );
        }
    }

    #[tokio::test]
    async fn validation_and_icon_identity_reject_ambiguous_inventory() {
        let server = MockServer::start().await;
        let mut ready = tool();
        ready["platform_key"] = json!({"enabled":true,"audience":"public"});
        let mut duplicate = ready.clone();
        duplicate["id"] = "duplicate-id".into();
        duplicate["is_active"] = false.into();
        let inventory = vec![ready, duplicate];
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(me(true)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"services":inventory})))
            .mount(&server)
            .await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let report = validate(&mut api, &manifest()).await.unwrap();
        assert_eq!(report[0].status, "failed");
        assert_eq!(report[0].reasons, vec!["ambiguous_existing_tool"]);
        assert_eq!(report[0].ordinary_user_eligibility, "not_ready");
        assert!(super::super::admin_catalog::unambiguous_row(&inventory, "tools-test").is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && !r.url.path().ends_with("/endpoints"))
        );
    }

    #[tokio::test]
    async fn validation_reports_ineligible_manager_and_retains_real_vendor_gate() {
        let server = MockServer::start().await;
        let mut row = tool();
        row["tools_manager_eligible"] = false.into();
        row["platform_key"] = json!({"enabled":true,"audience":"public"});
        mock_inventory(&server, row, vec![endpoint("published")]).await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let report = validate(&mut api, &manifest()).await.unwrap();
        assert_eq!(report[0].status, "pending_admin");
        assert!(report[0].reasons.contains(&"manager_ineligible_shape"));
        assert_eq!(report[0].verification["status"], "not_verified");
        assert_eq!(
            report[0].verification["pending_gates"],
            json!(["outside_user_platform_call_admin_confirmed"])
        );
    }

    #[tokio::test]
    async fn planner_uses_current_server_topic_vocabulary() {
        let server = MockServer::start().await;
        mock_inventory(&server, tool(), vec![endpoint("published")]).await;
        Mock::given(method("GET"))
            .and(path("/api/v1/tools/topics"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"slug":"new-server-topic","label":"New topic"}])),
            )
            .mount(&server)
            .await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let mut m = manifest();
        m.tools[0].topics = Some(vec!["new-server-topic".into()]);
        assert!(plan(&mut api, &m, &me(false)).await.is_ok());
        m.tools[0].topics = Some(vec!["removed-topic".into()]);
        assert!(plan(&mut api, &m, &me(false)).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }
    #[test]
    fn secret_free_allowlist_rejects_malformed_documents_without_echo() {
        let base = serde_json::to_value(manifest()).unwrap();
        for key in [
            "credential",
            "authorization",
            "headers",
            "provider_config_id",
            "billing",
            "platform_key",
            "tools_manager_created",
            "created_by",
        ] {
            let mut v = base.clone();
            v["tools"][0][key] = "DO_NOT_ECHO_FAKE_SECRET".into();
            let error = parse(&v.to_string()).unwrap_err().to_string();
            assert!(!error.contains("DO_NOT_ECHO_FAKE_SECRET"), "{key}");
        }
        for (key, value) in [
            (
                "base_url",
                json!("https://user:DO_NOT_ECHO_FAKE_SECRET@vendor.example"),
            ),
            (
                "base_url",
                json!(format!("https://vendor.example/{}", "x".repeat(2048))),
            ),
            ("base_url", json!(" https://vendor.example/api ")),
            ("openapi_spec_url", json!("https://vendor.example/\nspec")),
            ("auth_method", json!("token_exchange")),
            ("topics", json!(["INVALID TOPIC"])),
            ("operations", json!(["search", "search"])),
            ("credential_env", json!("secret=value")),
        ] {
            let mut v = base.clone();
            v["tools"][0][key] = value;
            assert!(parse(&v.to_string()).is_err());
        }
        for raw in [
            "version: 3\ntools: []",
            "version: 1\ntools: DO_NOT_ECHO_FAKE_SECRET",
            "credential: DO_NOT_ECHO_FAKE_SECRET\n---\nversion: 1",
            "version: 1\ntools:\n - slug: x\n   name: bad\n   twin_of: api-x\n   base_url: https://vendor.example",
        ] {
            assert!(
                !parse(raw)
                    .unwrap_err()
                    .to_string()
                    .contains("DO_NOT_ECHO_FAKE_SECRET")
            );
        }
        let mut duplicate = base.clone();
        duplicate["tools"]
            .as_array_mut()
            .unwrap()
            .push(base["tools"][0].clone());
        assert!(parse(&duplicate.to_string()).is_err());
        let mut none = base;
        none["tools"][0]["operations"] = json!([]);
        assert!(parse(&none.to_string()).is_ok());
    }
    #[test]
    fn templates_auth_and_non_destructive_omission() {
        let m = parse(include_str!("../../templates/tools/firecrawl.yaml")).unwrap();
        assert_eq!(m.tools[0].twin_of.as_deref(), Some("api-firecrawl"));
        let raw = r#"{"version":1,"tools":[{"slug":"tools-new","name":"New","base_url":"https://api.example","auth_method":"header","auth_key_name":"X-API-Key"}]}"#;
        let m = parse(raw).unwrap();
        assert!(m.tools[0].operations.is_none());
        assert!(!selected(&m.tools[0], "search"));
        assert!(!valid_url("https://vendor.example/path#secret", true));
        assert!(!valid_operation("Bad"));
        assert!(!valid_slug("../bad"));
    }
    #[tokio::test]
    async fn twin_name_omission_inherits_source_and_preserves_existing_name() {
        let m = parse(r#"{"version":1,"tools":[{"slug":"tools-test","twin_of":"api-source"}]}"#)
            .unwrap();
        assert!(m.tools[0].name.is_none());
        assert!(parse(r#"{"version":1,"tools":[{"slug":"tools-new","base_url":"https://api.example","auth_method":"none"}]}"#).is_err());
        let server = MockServer::start().await;
        let mut source = tool();
        source["slug"] = "api-source".into();
        source["id"] = "source-id".into();
        source["offering_kind"] = "ai_service".into();
        Mock::given(method("GET"))
            .and(path("/api/v1/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"services":[source]})))
            .mount(&server)
            .await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let planned = plan(&mut api, &m, &me(false)).await.unwrap();
        assert!(planned[0].create.get("name").is_none());
        let retry = MockServer::start().await;
        let mut current = tool();
        current["name"] = "Inherited vendor name".into();
        current["import_source"] = json!({"kind":"catalog_twin","reference":"api-source"});
        mock_inventory(&retry, current.clone(), vec![endpoint("published")]).await;
        let mut api = ApiClient::new(&retry.uri(), "fake-session".into()).unwrap();
        let planned = plan(&mut api, &m, &me(false)).await.unwrap();
        assert!(planned[0].update.as_object().unwrap().is_empty());
        let report = readiness(&m.tools[0], Some(&current), &planned[0].endpoints);
        assert_eq!(report.lark["name"], "Inherited vendor name");
        assert!(
            retry
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }
    #[tokio::test]
    async fn draft_planning_keeps_admin_audience_deliberate_and_no_auth_secret_free() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"services":[]})))
            .mount(&server)
            .await;
        let mut m = manifest();
        m.tools[0].auth_method = Some("none".into());
        m.tools[0].credential_env = None;
        m.tools[0].operations = None;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let admin = plan(&mut api, &m, &me(true)).await.unwrap();
        assert_eq!(admin[0].create["platform_key"]["audience"], "restricted");
        assert_eq!(admin[0].row.credential, "not_required");
        let manager = plan(&mut api, &m, &me(false)).await.unwrap();
        assert!(manager[0].create.get("platform_key").is_none());
        assert_eq!(manager[0].row.credential, "not_required");
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }

    #[tokio::test]
    async fn dry_plan_has_zero_writes_and_never_needs_credential_env() {
        let server = MockServer::start().await;
        let mut current = tool();
        current["credential_configured"] = false.into();
        current["tools_credential_frozen"] = false.into();
        mock_inventory(&server, current, vec![endpoint("draft")]).await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let plans = plan(&mut api, &manifest(), &me(false)).await.unwrap();
        assert_eq!(plans[0].row.credential, "pending_credential");
        assert!(!plans[0].row.admin_work.is_empty());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }
    #[tokio::test]
    async fn complete_rerun_is_write_free_and_preserves_secret() {
        let server = MockServer::start().await;
        mock_inventory(&server, tool(), vec![endpoint("published")]).await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let plans = plan(&mut api, &manifest(), &me(true)).await.unwrap();
        let (rows, _) = apply(&mut api, plans, &me(true), false).await.unwrap();
        assert_eq!(rows[0].action, "unchanged");
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }
    #[tokio::test]
    async fn manager_prepares_without_publishing_and_pauses_deselected() {
        let server = MockServer::start().await;
        mock_inventory(
            &server,
            tool(),
            vec![
                endpoint("draft"),
                json!({"name":"other","publication":"published","is_active":true}),
            ],
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/tool-id/publication"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"endpoints":[]})))
            .mount(&server)
            .await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let plans = plan(&mut api, &manifest(), &me(false)).await.unwrap();
        let (report, _) = apply(&mut api, plans, &me(false), false).await.unwrap();
        assert_eq!(report[0].action, "updated");
        let requests = server.received_requests().await.unwrap();
        let states: Vec<_> = requests
            .iter()
            .filter(|r| r.method == "POST")
            .map(|r| r.body_json::<Value>().unwrap()["state"].clone())
            .collect();
        assert_eq!(states, vec![json!("validated"), json!("paused")]);
    }
    #[tokio::test]
    async fn incompatible_foreign_and_missing_selection_fail_before_writes() {
        for change in [
            json!({"offering_kind":"ai_service"}),
            json!({"created_by":"foreign"}),
            json!({"tools_manager_created":false}),
        ] {
            let server = MockServer::start().await;
            let mut v = tool();
            for (k, val) in change.as_object().unwrap() {
                v[k] = val.clone();
            }
            mock_inventory(&server, v, vec![endpoint("draft")]).await;
            let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
            assert!(plan(&mut api, &manifest(), &me(false)).await.is_err());
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method == "GET")
            );
        }
        let server = MockServer::start().await;
        mock_inventory(
            &server,
            tool(),
            vec![json!({"name":"other","publication":"draft"})],
        )
        .await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        assert!(plan(&mut api, &manifest(), &me(true)).await.is_err());
    }
    #[tokio::test]
    async fn inactive_rows_match_without_duplicate_create() {
        let server = MockServer::start().await;
        let mut v = tool();
        v["is_active"] = false.into();
        mock_inventory(&server, v, vec![endpoint("validated")]).await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let p = plan(&mut api, &manifest(), &me(false)).await.unwrap();
        assert!(p[0].current.is_some());
        let (report, _) = apply(&mut api, p, &me(false), false).await.unwrap();
        assert_eq!(report[0].action, "unchanged");
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }
    #[tokio::test]
    async fn ambiguous_server_operations_fail_before_any_write() {
        let server = MockServer::start().await;
        mock_inventory(
            &server,
            tool(),
            vec![endpoint("draft"), endpoint("validated")],
        )
        .await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let error = plan(&mut api, &manifest(), &me(true)).await.err().unwrap();
        assert!(error.to_string().contains("duplicate operation names"));
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }
    #[tokio::test]
    async fn secret_setting_failure_discards_reflected_upstream_body() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/services/id"))
            .respond_with(ResponseTemplate::new(403).set_body_string("DO_NOT_ECHO_FAKE_SECRET"))
            .mount(&server)
            .await;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let error = set_credential(&mut api, "id", "DO_NOT_ECHO_FAKE_SECRET")
            .await
            .unwrap_err();
        assert!(!format!("{error:#}").contains("DO_NOT_ECHO"));
        assert!(!format!("{error:?}").contains("DO_NOT_ECHO"));
    }
    #[test]
    fn readiness_does_not_treat_admin_preview_as_public_acl() {
        let m = manifest();
        let t = &m.tools[0];
        let mut v = tool();
        let r = readiness(t, Some(&v), &[endpoint("published")]);
        assert_eq!(r.status, "pending_admin");
        assert_eq!(r.ordinary_user_eligibility, "not_ready");
        v["platform_key"]["allowed_owner_ids"] = json!(["someone"]);
        let r = readiness(t, Some(&v), &[endpoint("published")]);
        assert_eq!(r.status, "ready");
        assert_eq!(
            r.ordinary_user_eligibility,
            "restricted_membership_not_evaluated"
        );
        v["credential_configured"] = false.into();
        assert_eq!(
            readiness(t, Some(&v), &[endpoint("published")]).status,
            "pending_credential"
        );
        assert_eq!(readiness(t, Some(&v), &[]).status, "failed");
    }
    #[test]
    fn admin_handoff_preserves_approved_audience_and_lists_only_missing_actions() {
        use clap::Parser;
        let mut t = manifest().tools.remove(0);
        t.operations = None;
        let mut v = tool();
        v["platform_key"]["allowed_owner_ids"] = json!(["approved-person"]);
        let free = readiness(&t, Some(&v), &[endpoint("published")]);
        assert_eq!(
            free.status, "ready",
            "intentional free access remains ready"
        );
        assert!(
            free.admin_commands
                .iter()
                .any(|command| command.starts_with("Review billing.platform_key_pricing"))
        );
        // A configured price needs no advisory; retain coverage of the exact
        // missing publication/activation/credential commands below.
        v["billing"] = json!({"platform_key_pricing":{"metric":"requests","credits_per_unit":"1","sync_status":"synced"}});
        let r = readiness(&t, Some(&v), &[endpoint("validated")]);
        assert_eq!(
            r.admin_commands,
            ["nyxid catalog publish tools-test --operation search"]
        );
        assert!(
            readiness(&t, Some(&v), &[endpoint("published")])
                .admin_commands
                .is_empty()
        );
        v["credential_configured"] = false.into();
        v["is_active"] = false.into();
        let r = readiness(&t, Some(&v), &[endpoint("published")]);
        assert_eq!(r.admin_commands.len(), 2);
        assert!(
            r.admin_commands
                .iter()
                .all(|command| !command.contains("audience"))
        );
        for command in &r.admin_commands {
            crate::cli::Cli::try_parse_from(command.split_whitespace()).unwrap();
        }
        assert!(r.admin_commands[1].ends_with("--credential-env THIS_ENV_MUST_NEVER_BE_READ"));
    }
    #[test]
    fn tools_arguments_parse_and_no_terminal_is_required_for_file_modes() {
        use clap::Parser;
        for args in [
            vec![
                "nyxid",
                "admin",
                "tools",
                "apply",
                "--file",
                "m.yaml",
                "--dry-run",
            ],
            vec![
                "nyxid", "admin", "tools", "validate", "--file", "m.yaml", "--output", "json",
            ],
            vec![
                "nyxid",
                "admin",
                "tools",
                "add",
                "--template",
                "firecrawl",
                "--yes",
            ],
        ] {
            assert!(crate::cli::Cli::try_parse_from(args).is_ok());
        }
        assert!(
            crate::cli::Cli::try_parse_from([
                "nyxid",
                "admin",
                "tools",
                "add",
                "--file",
                "m.yaml",
                "--template",
                "firecrawl"
            ])
            .is_err()
        );
    }
    #[test]
    fn variables_precedence_and_secret_free_saved_expansion() {
        let raw = r#"{"version":1,"variables":{"suffix":{"default":"one","allowed":["one","two"]}},"tools":[{"slug":"tools-${suffix}","name":"Test","base_url":"https://api.example","auth_method":"none","homepage_url":"https://${suffix}.example"}]}"#;
        assert_eq!(parse(raw).unwrap().tools[0].slug, "tools-one");
        let expanded = parse_expanded(raw, &["suffix=two".into()], false).unwrap();
        assert_eq!(expanded.tools[0].slug, "tools-two");
        let saved = serde_json::to_string(&expanded).unwrap();
        assert!(!saved.contains("variables"));
        assert!(!saved.contains("${"));
        for overrides in [
            vec!["unknown=two".into()],
            vec!["suffix=DO_NOT_ECHO_SECRET!".into()],
            vec!["suffix=one".into(), "suffix=two".into()],
        ] {
            assert!(
                !format!("{:#}", parse_expanded(raw, &overrides, false).unwrap_err())
                    .contains("DO_NOT_ECHO_SECRET")
            );
        }
        assert!(parse(&raw.replace("${suffix}", "${unknown}")).is_err());
    }
    #[test]
    fn effective_prices_follow_primary_component_and_operation_sync() {
        let t = &manifest().tools[0];
        let mut v = tool();
        v["billing"] = json!({"platform_key_pricing":{"metric":"requests","credits_per_unit":"1","sync_status":"synced","components":[{"metric":"bytes","credits_per_unit":"2","sync_status":"pending"}],"operations":[{"operation":"search","credits_per_unit":"3","sync_status":"pending"}]}});
        let r = readiness(t, Some(&v), &[endpoint("published")]);
        assert_eq!(
            r.operations[0]["effective_price"]["primary"]["credits_per_unit"],
            "1"
        );
        assert_eq!(
            r.pricing["components"][0]["effective"]["credits_per_unit"],
            "0"
        );
        v["billing"]["platform_key_pricing"]["operations"][0]["sync_status"] = "synced".into();
        let r = readiness(t, Some(&v), &[endpoint("published")]);
        assert_eq!(r.operations[0]["effective_price"]["credits_per_unit"], "3");
        v["billing"]["platform_key_pricing"]["sync_status"] = "pending".into();
        let r = readiness(t, Some(&v), &[endpoint("published")]);
        assert_eq!(r.pricing["effective"], "free");
        v["auth_method"] = "none".into();
        assert_eq!(
            readiness(t, Some(&v), &[endpoint("published")]).pricing["effective"],
            "free"
        );
    }
    #[tokio::test]
    async fn create_without_credentials_and_partial_retry_are_safe() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"services":[]})))
            .mount(&server)
            .await;
        let mut created = tool();
        created["credential_configured"] = false.into();
        created["tools_credential_frozen"] = false.into();
        Mock::given(method("POST"))
            .and(path("/api/v1/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&created))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/services/tool-id/endpoints"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"endpoints":[]})))
            .mount(&server)
            .await;
        let mut m = manifest();
        m.tools[0].operations = None;
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let plans = plan(&mut api, &m, &me(false)).await.unwrap();
        let (report, saved) = apply_options(&mut api, plans, &me(false), false, true)
            .await
            .unwrap();
        assert_eq!(report[0].action, "created");
        assert_eq!(report[0].credential, "pending_credential");
        assert!(
            !serde_json::to_string(&saved)
                .unwrap()
                .contains("credential\":")
        );
        server.verify().await;
        let retry = MockServer::start().await;
        mock_inventory(&retry, created, vec![]).await;
        let mut api = ApiClient::new(&retry.uri(), "fake-session".into()).unwrap();
        let plans = plan(&mut api, &m, &me(false)).await.unwrap();
        assert!(apply(&mut api, plans, &me(false), false).await.is_err());
        assert!(
            retry
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }
    #[tokio::test]
    async fn metadata_drift_is_one_minimal_update() {
        let server = MockServer::start().await;
        mock_inventory(&server, tool(), vec![endpoint("published")]).await;
        let mut changed = tool();
        changed["name"] = "New name".into();
        Mock::given(method("PUT"))
            .and(path("/api/v1/services/tool-id"))
            .and(wiremock::matchers::body_json(json!({"name":"New name"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(changed))
            .expect(1)
            .mount(&server)
            .await;
        let mut m = manifest();
        m.tools[0].name = Some("New name".into());
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        let plans = plan(&mut api, &m, &me(true)).await.unwrap();
        let (report, _) = apply(&mut api, plans, &me(true), false).await.unwrap();
        assert_eq!(report[0].action, "updated");
        assert_eq!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method != "GET")
                .count(),
            1
        );
    }
    #[test]
    fn declared_lanes_are_strict_and_do_not_reach_api_payloads() {
        let mut value = serde_json::to_value(manifest()).unwrap();
        for bad in [json!([]), json!(["byok", "byok"]), json!(["oauth"])] {
            value["tools"][0]["lanes"] = bad;
            assert!(parse(&value.to_string()).is_err());
        }
        value["tools"][0]["lanes"] = json!(["platform", "byok"]);
        let m = parse(&value.to_string()).unwrap();
        assert_eq!(m.tools[0].declared_lanes(), [Lane::Platform, Lane::Byok]);
        value["tools"][0]["auth_method"] = "none".into();
        value["tools"][0]
            .as_object_mut()
            .unwrap()
            .remove("credential_env");
        assert!(parse(&value.to_string()).is_err());
        assert_eq!(manifest().tools[0].declared_lanes(), [Lane::Platform]);
    }

    #[test]
    fn byok_readiness_is_independent_of_platform_credential_and_empty_audience() {
        let mut t = manifest().tools.remove(0);
        t.lanes = Some(vec![Lane::Platform, Lane::Byok]);
        let mut v = tool();
        v["credential_configured"] = false.into();
        let report = readiness(&t, Some(&v), &[endpoint("published")]);
        assert_eq!(report.status, "pending_credential");
        assert_eq!(report.lanes["platform"]["status"], "pending_credential");
        assert_eq!(report.lanes["byok"]["status"], "ready");
        assert_eq!(report.lanes["byok"]["pricing"]["effective"], "free");
        assert!(
            report
                .reasons
                .contains(&"byok_pricing_unconfigured_effective_free")
        );
        assert_eq!(
            report.verification["pending_gates"],
            json!([
                "outside_user_platform_call_admin_confirmed",
                "outside_user_byok_call_admin_confirmed"
            ])
        );
        assert_eq!(report.lark["lanes"], json!(["platform", "byok"]));
        t.lanes = Some(vec![Lane::Byok]);
        let report = readiness(&t, Some(&v), &[endpoint("published")]);
        assert_eq!(report.status, "ready");
        assert_eq!(
            report.ordinary_user_eligibility,
            "requires_user_key_connection"
        );
        assert!(!report.reasons.contains(&"credential_missing"));
        assert!(!report.reasons.contains(&"admin_audience_setup_required"));
        let paused = readiness(&t, Some(&v), &[endpoint("paused")]);
        assert_eq!(paused.lanes["byok"]["status"], "pending_admin");
        v["billing"] = json!({"byok_pricing":{"metric":"requests","credits_per_unit":"2","sync_status":"synced"}});
        let priced = readiness(&t, Some(&v), &[endpoint("published")]);
        assert_eq!(priced.lanes["byok"]["pricing"]["effective"], "configured");
        assert!(
            !priced
                .reasons
                .contains(&"byok_pricing_unconfigured_effective_free")
        );
    }

    #[tokio::test]
    async fn planner_rejects_no_auth_byok_twin_before_any_writes() {
        let server = MockServer::start().await;
        let mut source = tool();
        source["slug"] = "api-source".into();
        source["auth_method"] = "none".into();
        mock_inventory(&server, source, vec![]).await;
        let m=parse(r#"{"version":1,"tools":[{"slug":"tools-new","twin_of":"api-source","lanes":["byok"]}]}"#).unwrap();
        let mut api = ApiClient::new(&server.uri(), "fake-session".into()).unwrap();
        assert!(plan(&mut api, &m, &me(true)).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method == "GET")
        );
    }
}
