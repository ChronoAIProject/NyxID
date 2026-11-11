//! `nyxid admin provider` and `nyxid admin catalog`: create providers and
//! catalog services, set the NyxID-managed OAuth client, and check that a
//! service is ready to connect. Admins and Catalog Managers may run these;
//! the server enforces what a Catalog Manager may change.

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde::Serialize;
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::admin_catalog_playbook as playbook;
use crate::api::ApiClient;
use crate::cli::{AdminCatalogCommands, AdminProviderCommands, AuthArgs, OutputFormat};

/// Built-in manifests for `nyxid admin catalog add --template`.
const TEMPLATES: &[(&str, &str)] = &[
    (
        "zoho-mail",
        include_str!("../../templates/catalog/zoho-mail.yaml"),
    ),
    (
        "telnyx-platform",
        include_str!("../../templates/catalog/telnyx-platform.yaml"),
    ),
];

/// Fields that would put an OAuth client secret into a file.
const PROVIDER_SECRET_FIELDS: &[&str] = &["client_id", "client_secret", "clear_client_credentials"];

/// Service fields the update route cannot change.
const SERVICE_CREATE_ONLY: &[&str] = &["service_type", "auth_method", "auth_key_name"];

/// UUID references select exactly that identity. Slugs must be unambiguous even
/// when inactive rows are present; inventory order never decides a write target.
pub(crate) fn unambiguous_row<'a>(rows: &'a [Value], reference: &str) -> Result<Option<&'a Value>> {
    let explicit_id = uuid::Uuid::parse_str(reference).is_ok();
    let mut matches = rows
        .iter()
        .filter(|row| row["id"] == reference || (!explicit_id && row["slug"] == reference));
    let first = matches.next();
    if matches.next().is_some() {
        bail!(
            "Ambiguous existing catalog identity; use an explicit UUID or resolve duplicate slugs"
        );
    }
    Ok(first)
}

pub async fn run_provider(command: AdminProviderCommands) -> Result<()> {
    match command {
        AdminProviderCommands::List { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let providers = list_providers(&mut api).await?;
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&providers)?),
                OutputFormat::Table => {
                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL_CONDENSED);
                    table.set_header(["Slug", "Name", "Type", "OAuth client", "Ready", "ID"]);
                    for provider in &providers {
                        table.add_row([
                            text(provider, "slug"),
                            text(provider, "name"),
                            text(provider, "provider_type"),
                            client_status(provider).to_string(),
                            yes_no(provider["has_oauth_config"].as_bool()).to_string(),
                            text(provider, "id"),
                        ]);
                    }
                    eprintln!("{table}");
                }
            }
            Ok(())
        }
        AdminProviderCommands::Show { provider, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let provider = find_provider(&mut api, &provider)
                .await?
                .ok_or_else(|| anyhow!("Provider not found: {provider}"))?;
            print_provider(&provider, &auth)?;
            Ok(())
        }
        AdminProviderCommands::Create {
            file,
            client_id_env,
            client_secret_env,
            auth,
        } => {
            let wizard = file.is_none();
            let body = match file {
                Some(path) => read_document(&path)?,
                None => {
                    require_terminal("Pass --file without a terminal")?;
                    eprintln!("New provider. Press Enter to accept a [default].");
                    provider_wizard(None)?
                }
            };
            reject_fields(&body, PROVIDER_SECRET_FIELDS, "provider file")?;
            playbook::validate_authorization_nonce(&body)?;
            let mut api = ApiClient::from_auth_with_credential_transport(&auth)?;
            let client = if client_required_at_creation(&body) {
                if client_id_env.is_some()
                    && client_secret_env.is_none()
                    && body["provider_type"] != "device_code"
                {
                    bail!("--client-id-env needs --client-secret-env");
                }
                if client_id_env.is_none() {
                    require_terminal(
                        "A managed (credential_mode admin) provider needs its OAuth client at creation; pass --client-id-env and --client-secret-env",
                    )?;
                }
                let (id, secret) = read_client(
                    &text(&body, "slug"),
                    body["provider_type"] != "device_code",
                    &redirect_uri(&auth)?,
                    client_id_env.as_deref(),
                    client_secret_env.as_deref(),
                )?;
                Some((id, secret))
            } else {
                None
            };
            let created = post_provider(&mut api, &body, client.as_ref()).await?;
            print_provider(&created, &auth)?;
            if wizard && !client_configured(&created) {
                offer_client(&mut api, &created, &auth).await?;
            }
            Ok(())
        }
        AdminProviderCommands::Update {
            provider,
            file,
            with_client,
            client_id_env,
            client_secret_env,
            auth,
        } => {
            let mut api = ApiClient::from_auth_with_credential_transport(&auth)?;
            let current = find_provider(&mut api, &provider)
                .await?
                .ok_or_else(|| anyhow!("Provider not found: {provider}"))?;
            let body = match &file {
                Some(path) => read_document(path)?,
                None => {
                    require_terminal("Pass --file without a terminal")?;
                    eprintln!(
                        "Editing {}. Press Enter to keep a [current value].",
                        text(&current, "slug")
                    );
                    let desired = provider_wizard(Some(&current))?;
                    pick(
                        &desired,
                        &changed_fields(&desired, &current, &["slug", "provider_type"]),
                    )
                }
            };
            reject_fields(&body, PROVIDER_SECRET_FIELDS, "provider file")?;
            let mut effective = current.clone();
            if let (Some(fields), Some(changes)) = (effective.as_object_mut(), body.as_object()) {
                fields.extend(changes.clone());
            }
            playbook::validate_authorization_nonce(&effective)?;
            let client = if with_client {
                api.require_credential_transport()?;
                Some(read_client_values(
                    current["provider_type"] != "device_code",
                    client_id_env.as_deref(),
                    client_secret_env.as_deref(),
                )?)
            } else {
                None
            };
            let updated: Value = if body.as_object().is_some_and(Map::is_empty) && client.is_none()
            {
                eprintln!("No changes.");
                current
            } else {
                let request = ClientRequest {
                    fields: &body,
                    client_id: client.as_ref().map(|(id, _)| id.as_str()),
                    client_secret: client
                        .as_ref()
                        .and_then(|(_, secret)| secret.as_ref().map(|s| s.as_str())),
                };
                let result = api
                    .put(&format!("/providers/{}", text(&current, "id")), &request)
                    .await;
                if client.is_some() {
                    result.map_err(redacted_client_error)?
                } else {
                    result?
                }
            };
            print_provider(&updated, &auth)?;
            if file.is_none() && !with_client {
                offer_client(&mut api, &updated, &auth).await?;
            }
            Ok(())
        }
        AdminProviderCommands::SetCredentials {
            provider,
            client_id_env,
            client_secret_env,
            auth,
        } => {
            let mut api = ApiClient::from_auth_with_credential_transport(&auth)?;
            let current = find_provider(&mut api, &provider)
                .await?
                .ok_or_else(|| anyhow!("Provider not found: {provider}"))?;
            let redirect = redirect_uri(&auth)?;
            let updated = set_client(
                &mut api,
                &current,
                &redirect,
                client_id_env.as_deref(),
                client_secret_env.as_deref(),
            )
            .await?;
            print_provider(&updated, &auth)?;
            Ok(())
        }
        AdminProviderCommands::Link {
            provider,
            service,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let provider_id = provider_id(&mut api, &provider).await?;
            let service = find_service(&mut api, &service)
                .await?
                .ok_or_else(|| anyhow!("Catalog service not found: {service}"))?;
            let linked: Value = api
                .put(
                    &format!("/providers/{provider_id}/services/{}", text(&service, "id")),
                    &json!({}),
                )
                .await?;
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&linked)?),
                OutputFormat::Table => {
                    eprintln!("Linked {} to provider {provider}.", text(&linked, "slug"))
                }
            }
            Ok(())
        }
        AdminProviderCommands::Delete {
            provider,
            yes,
            auth,
        } => {
            if !yes {
                bail!(
                    "Deleting a provider revokes every user's tokens for it; pass --yes to confirm"
                );
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let id = provider_id(&mut api, &provider).await?;
            let result: Value = api.delete(&format!("/providers/{id}")).await?;
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Table => eprintln!("{}", text(&result, "message")),
            }
            Ok(())
        }
    }
}

pub async fn run_catalog(command: AdminCatalogCommands) -> Result<()> {
    match command {
        AdminCatalogCommands::Icon {
            provider,
            file,
            auth,
        } => {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(&file)?
                .take(128 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 128 * 1024 {
                bail!("SVG exceeds 128 KiB");
            }
            let svg = String::from_utf8(bytes)?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let row = find_provider(&mut api, &provider)
                .await?
                .ok_or_else(|| anyhow!("Provider not found: {provider}"))?;
            let id = row["id"]
                .as_str()
                .ok_or_else(|| anyhow!("Provider has no ID"))?;
            let uploaded: Value = api
                .post(
                    &format!("/providers/{id}/icon"),
                    &serde_json::json!({"svg": svg}),
                )
                .await?;
            let updated: Value = api
                .put(
                    &format!("/providers/{id}"),
                    &serde_json::json!({"icon_url": uploaded["icon_url"]}),
                )
                .await?;
            print_provider(&updated, &auth)?;
            Ok(())
        }
        AdminCatalogCommands::Apply {
            manifest,
            dry_run,
            allow_access_change,
            auth,
        } => {
            let manifest = Manifest::load(&manifest.file, &manifest.vars, false)?;
            let clients = clients_from_env(&manifest)?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            if !dry_run {
                api.require_credential_transport()?;
            }
            let rows = apply_with_access_change(
                &mut api,
                &manifest,
                dry_run,
                &clients,
                allow_access_change,
            )
            .await?;
            print_apply(&rows, dry_run, auth.output)?;
            fail_on_errors(&rows)
        }
        AdminCatalogCommands::Validate { manifest, auth } => {
            let manifest = Manifest::load(&manifest.file, &manifest.vars, false)?;
            let mut api = ApiClient::from_auth(&auth)?.without_token_refresh();
            let report = validate(&mut api, &manifest, &redirect_uri(&auth)?).await?;
            print_validation(&report, auth.output)?;
            if report.iter().any(|row| row.status == "failed") {
                bail!("Validation failed for at least one service");
            }
            Ok(())
        }
        AdminCatalogCommands::Add {
            file,
            template,
            vars,
            save,
            client_id_env,
            client_secret_env,
            skip_credentials,
            yes,
            dry_run,
            auth,
        } => {
            let interactive = std::io::stdin().is_terminal();
            let document = match (file, template) {
                (Some(_), Some(_)) => bail!("Use either --file or --template, not both"),
                (Some(path), None) => read_document(&path)?,
                (None, Some(name)) => template_document(&name)?,
                (None, None) if interactive => wizard_document()?,
                (None, None) => bail!(
                    "Without a terminal, pass --file or --template (templates: {})",
                    template_names()
                ),
            };
            let manifest = Manifest::from_document(document, &vars, interactive)?;
            if let Some(path) = save {
                std::fs::write(
                    &path,
                    serde_json::to_string_pretty(&manifest.to_document())?,
                )
                .with_context(|| format!("Failed to write {}", path.display()))?;
                eprintln!("Saved manifest to {}", path.display());
            }
            if dry_run {
                let mut api = ApiClient::from_auth_checked(&auth).await?;
                let rows = apply(&mut api, &manifest, true, &BTreeMap::new()).await?;
                print_apply(&rows, true, auth.output)?;
                return fail_on_errors(&rows);
            }
            if (client_id_env.is_some() || client_secret_env.is_some())
                && manifest.providers.len() != 1
            {
                bail!("--client-id-env and --client-secret-env need a manifest with one provider");
            }

            let mut clients = clients_from_env(&manifest)?;
            if let Some(id_env) = client_id_env.as_deref() {
                let desired = &manifest.providers[0];
                let needs_secret = desired["provider_type"] != "device_code";
                if needs_secret && client_secret_env.is_none() {
                    bail!("--client-id-env needs --client-secret-env");
                }
                clients.insert(
                    text(desired, "slug"),
                    ClientSource::Environment {
                        id: id_env.to_owned(),
                        secret: client_secret_env.clone(),
                        prompt_redirect: None,
                    },
                );
            }

            let redirect = redirect_uri(&auth)?;

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let plan = apply(&mut api, &manifest, true, &clients).await?;
            print_apply(&plan, true, OutputFormat::Table)?;
            fail_on_errors(&plan)?;
            if !yes {
                if !interactive {
                    bail!("Pass --yes to apply without a terminal");
                }
                if !confirm("Apply these changes?")? {
                    bail!("Cancelled");
                }
            }
            // Environment sources hold names only. A prompt fallback is read at
            // the mutation that needs it, after the plan is accepted.
            if interactive && !skip_credentials {
                for source in clients.values_mut() {
                    let ClientSource::Environment {
                        prompt_redirect, ..
                    } = source;
                    *prompt_redirect = Some(redirect.clone());
                }
            }
            api.require_credential_transport()?;
            let rows = apply(&mut api, &manifest, false, &clients).await?;
            print_apply(&rows, false, OutputFormat::Table)?;
            fail_on_errors(&rows)?;

            if !skip_credentials {
                for desired in &manifest.providers {
                    if !matches!(
                        desired["provider_type"].as_str(),
                        Some("oauth2" | "device_code")
                    ) {
                        continue;
                    }
                    let slug = text(desired, "slug");
                    let current = find_provider(&mut api, &slug)
                        .await?
                        .ok_or_else(|| anyhow!("Provider {slug} was not created"))?;
                    if client_configured(&current) {
                        eprintln!(
                            "{slug}: OAuth client already set. Use `nyxid admin provider set-credentials {slug}` to replace it."
                        );
                        continue;
                    }
                    if client_id_env.is_none() && !interactive {
                        bail!(
                            "Pass --client-id-env and --client-secret-env, or --skip-credentials, without a terminal"
                        );
                    }
                    set_client(
                        &mut api,
                        &current,
                        &redirect,
                        client_id_env.as_deref(),
                        client_secret_env.as_deref(),
                    )
                    .await?;
                }
            }

            let report = validate(&mut api, &manifest, &redirect).await?;
            print_validation(&report, auth.output)?;
            if matches!(auth.output, OutputFormat::Table) {
                for row in &report {
                    eprintln!("Connect {} as a user: {}", row.service, row.connect_command);
                }
            }
            if report.iter().any(|row| row.status == "failed") {
                bail!("Validation failed for at least one service");
            }
            Ok(())
        }
    }
}

// --- Manifest ---

#[derive(Debug, Clone, PartialEq)]
struct Manifest {
    providers: Vec<Value>,
    services: Vec<Value>,
}

impl Manifest {
    fn load(path: &Path, vars: &[String], interactive: bool) -> Result<Self> {
        Self::from_document(read_document(path)?, vars, interactive)
    }

    fn from_document(mut document: Value, vars: &[String], interactive: bool) -> Result<Self> {
        let object = document
            .as_object_mut()
            .ok_or_else(|| anyhow!("A manifest must be an object with providers and services"))?;
        let declared = object.remove("variables").unwrap_or(Value::Null);
        let values = resolve_variables(&declared, &parse_vars(vars)?, interactive)?;
        let mut take = |key: &str| -> Result<Vec<Value>> {
            match object.remove(key) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(rows)) => Ok(rows),
                Some(_) => bail!("`{key}` must be a list"),
            }
        };
        let mut providers = take("providers")?;
        let mut services = take("services")?;
        if let Some(extra) = object.keys().next() {
            bail!("Unknown manifest key `{extra}`; expected variables, providers, services");
        }
        for row in providers.iter_mut().chain(services.iter_mut()) {
            substitute(row, &values)?;
        }
        for provider in &providers {
            require_slug(provider, "provider")?;
            reject_fields(provider, PROVIDER_SECRET_FIELDS, "manifest provider")?;
            playbook::validate(provider, false)?;
        }
        for service in &services {
            require_slug(service, "service")?;
            reject_fields(
                service,
                &["credential", "provider_config_id"],
                "manifest service",
            )?;
            playbook::validate(service, true)?;
        }
        if providers.is_empty() && services.is_empty() {
            bail!("The manifest has no providers or services");
        }
        Ok(Self {
            providers,
            services,
        })
    }

    fn to_document(&self) -> Value {
        json!({ "providers": self.providers, "services": self.services })
    }
}

fn require_slug(row: &Value, kind: &str) -> Result<()> {
    match row.get("slug").and_then(Value::as_str) {
        Some(slug) if !slug.is_empty() => Ok(()),
        _ => bail!("Every manifest {kind} needs a slug"),
    }
}

fn reject_fields(row: &Value, fields: &[&str], context: &str) -> Result<()> {
    let present: Vec<&str> = fields
        .iter()
        .copied()
        .filter(|field| row.get(*field).is_some())
        .collect();
    if present.is_empty() {
        return Ok(());
    }
    bail!(
        "Remove {} from the {context}. Secrets never go in files: use `nyxid admin provider set-credentials`, and link providers with the `provider` field",
        present.join(", ")
    )
}

fn parse_vars(vars: &[String]) -> Result<BTreeMap<String, String>> {
    vars.iter()
        .map(|pair| {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| anyhow!("--set expects name=value, got `{pair}`"))?;
            Ok((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Resolve `variables: { name: { prompt, default, allowed } }` from `--set`,
/// an interactive answer, or the default, in that order.
fn resolve_variables(
    declared: &Value,
    provided: &BTreeMap<String, String>,
    interactive: bool,
) -> Result<BTreeMap<String, Value>> {
    let declared = match declared {
        Value::Null => Map::new(),
        Value::Object(map) => map.clone(),
        _ => bail!("variables must be a map of variable specifications"),
    };
    if let Some(unknown) = provided.keys().find(|name| !declared.contains_key(*name)) {
        bail!("--set {unknown}: the manifest declares no such variable");
    }
    let mut values = BTreeMap::new();
    for (name, spec) in &declared {
        let kind = spec.get("type").and_then(Value::as_str).unwrap_or("string");
        if !["string", "model_id", "int"].contains(&kind) {
            bail!("Unknown variable type for {name}");
        }
        let default = spec.get("default").filter(|v| !v.is_null()).map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
        });
        let allowed: Vec<String> = spec
            .get("allowed")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            })
            .collect();
        let value = match provided.get(name) {
            Some(value) => value.clone(),
            None if interactive => {
                let label = spec.get("prompt").and_then(Value::as_str).unwrap_or(name);
                ask(
                    &if allowed.is_empty() {
                        label.into()
                    } else {
                        format!("{label} ({})", allowed.join(", "))
                    },
                    default.as_deref(),
                )?
            }
            None => default
                .ok_or_else(|| anyhow!("Set the manifest variable with --set {name}=<value>"))?,
        };
        if !allowed.is_empty() && !allowed.contains(&value) {
            bail!("{name} must be one of: {}", allowed.join(", "));
        }
        let resolved = match kind {
            "int" => {
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    bail!("{name} must be a nonnegative integer");
                }
                let number = value
                    .parse::<u64>()
                    .with_context(|| format!("{name} is out of integer range"))?;
                if spec
                    .get("minimum")
                    .and_then(Value::as_u64)
                    .is_some_and(|min| number < min)
                    || spec
                        .get("maximum")
                        .and_then(Value::as_u64)
                        .is_some_and(|max| number > max)
                {
                    bail!("{name} is outside the declared integer bounds");
                }
                json!(number)
            }
            _ => {
                if value.is_empty()
                    || (kind == "model_id" && value.len() > 200)
                    || !value.bytes().all(|b| {
                        b.is_ascii_alphanumeric()
                            || b"._-".contains(&b)
                            || (kind == "model_id" && b"/:".contains(&b))
                    })
                {
                    bail!("{name} contains characters outside its declared {kind} type");
                }
                json!(value)
            }
        };
        values.insert(name.clone(), resolved);
    }
    Ok(values)
}

fn substitute(value: &mut Value, values: &BTreeMap<String, Value>) -> Result<()> {
    match value {
        Value::String(text) => {
            if let Some((_, replacement)) = values
                .iter()
                .find(|(name, _)| text == &format!("${{{name}}}"))
            {
                *value = replacement.clone();
                return Ok(());
            }
            for (name, replacement) in values {
                *text = text.replace(
                    &format!("${{{name}}}"),
                    &replacement
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| replacement.to_string()),
                );
            }
            if let Some(start) = text.find("${") {
                bail!(
                    "Undeclared manifest variable in `{}`",
                    &text[start..text.len().min(start + 40)]
                );
            }
        }
        Value::Array(items) => {
            for item in items {
                substitute(item, values)?;
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                substitute(item, values)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn read_document(path: &Path) -> Result<Value> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    parse_document(&raw).with_context(|| format!("Failed to parse {}", path.display()))
}

fn parse_document(raw: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_str(raw) {
        return Ok(value);
    }
    serde_saphyr::from_str::<Value>(raw).map_err(|error| anyhow!("{error}"))
}

fn template_names() -> String {
    TEMPLATES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn template_document(name: &str) -> Result<Value> {
    let (_, raw) = TEMPLATES
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .ok_or_else(|| anyhow!("Unknown template `{name}`; available: {}", template_names()))?;
    parse_document(raw)
}

// --- Apply ---

#[derive(Debug, Serialize, PartialEq)]
struct ApplyRow {
    kind: &'static str,
    slug: String,
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    changed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

impl ApplyRow {
    fn new(kind: &'static str, slug: &str, action: &'static str) -> Self {
        Self {
            kind,
            slug: slug.to_string(),
            action,
            id: None,
            changed: Vec::new(),
            error: None,
            note: None,
        }
    }

    fn failed(kind: &'static str, slug: &str, error: impl std::fmt::Display) -> Self {
        Self {
            error: Some(error.to_string()),
            ..Self::new(kind, slug, "failed")
        }
    }
}

async fn apply(
    api: &mut ApiClient,
    manifest: &Manifest,
    dry_run: bool,
    clients: &Clients,
) -> Result<Vec<ApplyRow>> {
    apply_with_access_change(api, manifest, dry_run, clients, false).await
}

async fn apply_with_access_change(
    api: &mut ApiClient,
    manifest: &Manifest,
    dry_run: bool,
    clients: &Clients,
    allow_access_change: bool,
) -> Result<Vec<ApplyRow>> {
    let mut rows = Vec::new();
    let mut provider_ids: BTreeMap<String, Option<String>> = BTreeMap::new();
    let existing_providers = list_providers(api).await?;
    let existing_services = list_services(api).await?;
    // Resolve all authored identities before writes; duplicate inventory is not
    // an ordering rule, including when one of the duplicate rows is inactive.
    for (kind, desired_rows, inventory) in [
        ("provider", &manifest.providers, &existing_providers),
        ("service", &manifest.services, &existing_services),
    ] {
        for desired in desired_rows {
            if unambiguous_row(inventory, &text(desired, "slug")).is_err() {
                rows.push(ApplyRow::failed(
                    kind,
                    &text(desired, "slug"),
                    "ambiguous_existing_catalog_identity",
                ));
            }
        }
    }
    if !rows.is_empty() {
        return Ok(rows);
    }

    let mut warnings = Vec::new();
    for desired in &manifest.services {
        if let Some(current) = unambiguous_row(&existing_services, &text(desired, "slug"))?
            && let Some(warning) = platform_access_warning(desired, current)
        {
            warnings.push(warning);
        }
    }
    authorize_access_changes(
        &warnings,
        dry_run,
        allow_access_change,
        std::io::stdin().is_terminal(),
        || confirm("Allow these live platform access changes?"),
    )?;

    for authored in &manifest.providers {
        let fields = playbook::api_fields(authored);
        let desired = &fields;
        let slug = text(desired, "slug");
        let current = unambiguous_row(&existing_providers, &slug)?;
        let row = match current {
            None => {
                let client = clients.get(&slug);
                let missing = client_required_at_creation(desired) && client.is_none();
                let hint = || {
                    let (id_env, secret_env) = client_env_names(&slug);
                    format!(
                        "needs its OAuth client at creation (credential_mode admin): set {id_env} and {secret_env}, or run `nyxid admin catalog add`"
                    )
                };
                match (dry_run, missing) {
                    (true, _) => ApplyRow {
                        note: client_required_at_creation(desired).then(hint),
                        ..ApplyRow::new("provider", &slug, "create")
                    },
                    (false, true) => ApplyRow::failed("provider", &slug, hint()),
                    (false, false) => {
                        api.require_credential_transport()?;
                        let credentials = client.map(|source| source.load(&slug)).transpose();
                        let result = match credentials {
                            Ok(credentials) => {
                                post_provider(api, desired, credentials.as_ref()).await
                            }
                            Err(_) => {
                                Err(anyhow!("OAuth client input missing or invalid; {}", hint()))
                            }
                        };
                        match result {
                            Ok(created) => ApplyRow {
                                id: created["id"].as_str().map(str::to_string),
                                ..ApplyRow::new("provider", &slug, "created")
                            },
                            Err(error) => ApplyRow::failed("provider", &slug, error),
                        }
                    }
                }
            }
            Some(current) if current["is_active"] == false => ApplyRow {
                id: current["id"].as_str().map(str::to_owned),
                note: Some(format!(
                    "Ask an Admin: nyxid admin provider update {} --file reactivate-provider.json (file contains {{\"is_active\":true}})",
                    text(current, "id")
                )),
                ..ApplyRow::new("provider", &slug, "inactive")
            },
            Some(current) => {
                if desired.get("provider_type").is_some()
                    && desired["provider_type"] != current["provider_type"]
                {
                    ApplyRow::failed(
                        "provider",
                        &slug,
                        "provider_type cannot change; create a provider with a new slug",
                    )
                } else {
                    let changed = changed_fields(desired, current, &["slug", "provider_type"]);
                    let id = text(current, "id");
                    let action = match (changed.is_empty(), dry_run) {
                        (true, _) => Ok("unchanged"),
                        (false, true) => Ok("update"),
                        (false, false) => api
                            .put::<Value, _>(&format!("/providers/{id}"), &pick(desired, &changed))
                            .await
                            .map(|_| "updated"),
                    };
                    match action {
                        Ok(action) => ApplyRow {
                            id: Some(id),
                            changed,
                            ..ApplyRow::new("provider", &slug, action)
                        },
                        Err(error) => ApplyRow::failed("provider", &slug, error),
                    }
                }
            }
        };
        let id = row.id.clone();
        let usable = !["failed", "inactive"].contains(&row.action);
        rows.push(row);
        if usable {
            provider_ids.insert(slug, id);
        }
    }

    for desired in &manifest.services {
        let slug = text(desired, "slug");
        let provider_ref = desired.get("provider").and_then(Value::as_str);
        let provider_id = match provider_ref {
            None => None,
            Some(reference) => {
                match provider_ids.get(reference) {
                    Some(id) => id.clone(),
                    None if manifest
                        .providers
                        .iter()
                        .any(|provider| provider["slug"] == reference) =>
                    {
                        rows.push(ApplyRow::failed(
                            "service",
                            &slug,
                            format!("provider {reference} failed"),
                        ));
                        continue;
                    }
                    None => {
                        match unambiguous_row(&existing_providers, reference) {
                            Ok(Some(provider)) if provider["is_active"] != false => {
                                Some(text(provider, "id"))
                            }
                            Ok(Some(_)) => {
                                rows.push(ApplyRow::failed("service",&slug,"referenced provider is inactive; ask an Admin to reactivate it"));
                                continue;
                            }
                            Ok(None) => {
                                rows.push(ApplyRow::failed(
                                    "service",
                                    &slug,
                                    format!("provider {reference} not found"),
                                ));
                                continue;
                            }
                            Err(_) => {
                                rows.push(ApplyRow::failed(
                                    "service",
                                    &slug,
                                    "ambiguous_existing_provider",
                                ));
                                continue;
                            }
                        }
                    }
                }
            }
        };
        let mut body = playbook::api_fields(desired);
        if let Some(object) = body.as_object_mut() {
            object.remove("provider");
            if let Some(id) = &provider_id {
                object.insert("provider_config_id".into(), id.clone().into());
            }
        }
        let current = unambiguous_row(&existing_services, &slug)?;
        let row = match current {
            None if dry_run => ApplyRow::new("service", &slug, "create"),
            None => match create_service(api, &body).await {
                Ok(created) => created,
                Err(error) => ApplyRow::failed("service", &slug, error),
            },
            Some(current) if current["is_active"] == false => ApplyRow {
                id: current["id"].as_str().map(str::to_owned),
                note: Some(format!(
                    "Ask an Admin: nyxid service update {} --catalog-admin --active",
                    text(current, "id")
                )),
                ..ApplyRow::new("service", &slug, "inactive")
            },
            Some(current) => {
                // In a dry run a provider about to be created has no ID yet;
                // linking to it is still a change.
                let new_provider = provider_ref.is_some() && provider_id.is_none();
                apply_service_update(
                    api,
                    &slug,
                    &body,
                    current,
                    &provider_id,
                    new_provider,
                    dry_run,
                )
                .await
            }
        };
        if !["failed", "inactive"].contains(&row.action)
            && row.error.is_none()
            && desired.get("operations").is_some()
        {
            rows.extend(reconcile_operations(api, desired, row.id.as_deref(), dry_run).await?);
        }
        rows.push(row);
    }
    Ok(rows)
}

async fn reconcile_operations(
    api: &mut ApiClient,
    desired: &Value,
    service_id: Option<&str>,
    dry_run: bool,
) -> Result<Vec<ApplyRow>> {
    let existing = match service_id {
        Some(id) => list_operations(api, id).await?,
        None => vec![],
    };
    let operations = playbook::operations(desired)?;
    let mut rows = Vec::new();
    for operation in &operations {
        let slug = format!("{}/{}", text(desired, "slug"), operation.name);
        let matches: Vec<_> = existing
            .iter()
            .filter(|row| row["name"] == operation.name)
            .collect();
        if matches.len() > 1 {
            rows.push(ApplyRow::failed(
                "operation",
                &slug,
                "ambiguous_existing_operation",
            ));
            continue;
        }
        let body = serde_json::to_value(operation)?;
        let current = matches.first().copied();
        let changed = current
            .map(|row| changed_fields(&body, row, &["name"]))
            .unwrap_or_default();
        let action = if current.is_none() {
            "create"
        } else if changed.is_empty() {
            "unchanged"
        } else {
            "update"
        };
        let mut row = ApplyRow::new("operation", &slug, action);
        row.changed = changed.clone();
        row.id = current
            .and_then(|row| row["id"].as_str())
            .map(str::to_owned);
        if !dry_run && action != "unchanged" {
            let id = service_id.ok_or_else(|| anyhow!("Service creation returned no ID"))?;
            let result = if let Some(endpoint) = current {
                api.put::<Value, _>(
                    &format!("/services/{id}/endpoints/{}", text(endpoint, "id")),
                    &pick(&body, &changed),
                )
                .await
            } else {
                api.post::<Value, _>(&format!("/services/{id}/endpoints"), &body)
                    .await
            };
            row = match result {
                Ok(value) => ApplyRow {
                    id: value["id"].as_str().map(str::to_owned).or(row.id),
                    action: if current.is_some() {
                        "updated"
                    } else {
                        "created"
                    },
                    ..row
                },
                Err(error) => ApplyRow::failed("operation", &slug, error),
            };
        }
        rows.push(row);
    }
    for endpoint in &existing {
        if !operations
            .iter()
            .any(|operation| endpoint["name"] == operation.name)
        {
            rows.push(ApplyRow {
                id: endpoint["id"].as_str().map(str::to_owned),
                ..ApplyRow::new(
                    "operation",
                    &format!("{}/{}", text(desired, "slug"), text(endpoint, "name")),
                    "unmanaged",
                )
            });
        }
    }
    Ok(rows)
}

async fn list_operations(api: &mut ApiClient, service_id: &str) -> Result<Vec<Value>> {
    let response: Value = api
        .get(&format!("/services/{service_id}/endpoints"))
        .await?;
    Ok(response["endpoints"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

async fn create_service(api: &mut ApiClient, desired: &Value) -> Result<ApplyRow> {
    let mut body = desired.clone();
    let fields = body.as_object_mut().context("Service must be an object")?;
    let update_only = ["identity_propagation_mode", "inject_delegation_token"];
    fields.remove("requires_user_credential");
    for key in update_only {
        fields.remove(key);
    }
    let created: Value = api.post("/services", &body).await?;
    let mut row = ApplyRow {
        id: created["id"].as_str().map(str::to_owned),
        ..ApplyRow::new("service", &text(desired, "slug"), "created")
    };
    let mut errors = Vec::new();
    if let Some(error) = credential_requirement_error(desired, &created) {
        errors.push(format!("created; {error}"));
    }
    let changed: Vec<_> = changed_fields(desired, &created, &[])
        .into_iter()
        .filter(|key| update_only.contains(&key.as_str()))
        .collect();
    if !changed.is_empty() {
        let result = match row.id.as_deref() {
            Some(id) => api
                .put::<Value, _>(&format!("/services/{id}"), &pick(desired, &changed))
                .await
                .map(|_| ()),
            None => Err(anyhow!("created response has no service id")),
        };
        if let Err(error) = result {
            errors.push(format!(
                "created; follow-up update of {} failed: {error}; rerun apply",
                changed.join(", ")
            ));
        }
    }
    if !errors.is_empty() {
        row.error = Some(errors.join("; "));
    }
    Ok(row)
}

fn credential_requirement_error(desired: &Value, actual: &Value) -> Option<String> {
    desired
        .get("requires_user_credential")
        .filter(|value| *value != &actual["requires_user_credential"])
        .map(|_| {
            "requires_user_credential differs from the server response and cannot be set directly; correct the manifest"
                .to_string()
        })
}

async fn apply_service_update(
    api: &mut ApiClient,
    slug: &str,
    body: &Value,
    current: &Value,
    provider_id: &Option<String>,
    new_provider: bool,
    dry_run: bool,
) -> ApplyRow {
    let fixed: Vec<&str> = SERVICE_CREATE_ONLY
        .iter()
        .copied()
        .filter(|field| {
            let Some(desired) = body.get(*field) else {
                return false;
            };
            let same = desired == &current[*field]
                || (*field == "auth_method" && desired == &current["auth_type"]);
            !same
        })
        .collect();
    if !fixed.is_empty() {
        return ApplyRow::failed(
            "service",
            slug,
            format!(
                "{} cannot change on an existing service; ask an Admin",
                fixed.join(", ")
            ),
        );
    }
    let mut skip = vec!["slug", "provider_config_id", "requires_user_credential"];
    skip.extend_from_slice(SERVICE_CREATE_ONLY);
    let mut changed = changed_fields(body, current, &skip);
    let relink = new_provider
        || provider_id
            .as_ref()
            .is_some_and(|id| current["provider_config_id"].as_str() != Some(id.as_str()));
    let id = text(current, "id");
    if changed.is_empty() && !relink {
        return ApplyRow {
            id: Some(id),
            error: credential_requirement_error(body, current),
            ..ApplyRow::new("service", slug, "unchanged")
        };
    }
    let mut actual = current.clone();
    if !dry_run {
        if !changed.is_empty() {
            match api
                .put::<Value, _>(&format!("/services/{id}"), &pick(body, &changed))
                .await
            {
                Ok(updated) => actual = updated,
                Err(error) => return ApplyRow::failed("service", slug, error),
            }
        }
        if let (true, Some(provider_id)) = (relink, provider_id)
            && let Err(error) = api
                .put::<Value, _>(
                    &format!("/providers/{provider_id}/services/{id}"),
                    &json!({}),
                )
                .await
        {
            return ApplyRow::failed("service", slug, error);
        }
    }
    if relink {
        changed.push("provider".to_string());
    }
    ApplyRow {
        id: Some(id),
        changed,
        error: if dry_run {
            None
        } else {
            credential_requirement_error(body, &actual)
        },
        ..ApplyRow::new("service", slug, if dry_run { "update" } else { "updated" })
    }
}

fn authorize_access_changes(
    warnings: &[String],
    dry_run: bool,
    allowed: bool,
    interactive: bool,
    confirm_change: impl FnOnce() -> Result<bool>,
) -> Result<()> {
    for warning in warnings {
        eprintln!("Warning: {warning}");
    }
    if warnings.is_empty() || dry_run || allowed {
        return Ok(());
    }
    if interactive && confirm_change()? {
        return Ok(());
    }
    bail!(
        "Live platform access change requires confirmation; use `nyxid admin catalog apply --allow-access-change --file <manifest>`"
    );
}

fn platform_access_warning(desired: &Value, current: &Value) -> Option<String> {
    if current["is_active"] != true {
        return None;
    }
    let next = desired.get("platform_key")?;
    let disabled = next.get("enabled") == Some(&Value::Bool(false))
        && current["platform_key"]["enabled"] != false;
    let clears = next
        .get("allowed_owner_ids")
        .is_some_and(|v| v.as_array().is_some_and(Vec::is_empty))
        && current["platform_key"]["allowed_owner_ids"]
            .as_array()
            .is_some_and(|v| !v.is_empty());
    if disabled || clears {
        Some(format!(
            "Applying this manifest to {} {}. Review pilot access before applying.",
            text(current, "slug"),
            match (disabled, clears) {
                (true, true) => "disables the platform key and clears its allowlist",
                (true, false) => "disables the platform key",
                _ => "clears the platform-key allowlist",
            }
        ))
    } else {
        None
    }
}

fn changed_fields(desired: &Value, current: &Value, skip: &[&str]) -> Vec<String> {
    desired
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter(|(key, _)| !skip.contains(&key.as_str()))
                .filter(|(key, value)| match current.get(key.as_str()) {
                    Some(existing) if key.as_str() == "billing" && !value.is_null() => {
                        super::admin_billing::normalize_billing(existing)
                            != super::admin_billing::normalize_billing(
                                &super::admin_billing::merged_billing(existing, value),
                            )
                    }
                    Some(existing) => existing != *value,
                    None if key.as_str() == "authorization_nonce_required"
                        && *value == &Value::Bool(false) =>
                    {
                        false
                    }
                    None => !value.is_null(),
                })
                .map(|(key, _)| key.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn pick(source: &Value, fields: &[String]) -> Value {
    Value::Object(
        fields
            .iter()
            .filter_map(|field| {
                source
                    .get(field)
                    .map(|value| (field.clone(), value.clone()))
            })
            .collect(),
    )
}

fn fail_on_errors(rows: &[ApplyRow]) -> Result<()> {
    let failed = rows
        .iter()
        .filter(|row| row.error.is_some() || ["failed", "inactive"].contains(&row.action))
        .count();
    if failed > 0 {
        bail!("{failed} manifest row(s) failed");
    }
    Ok(())
}

fn print_apply(rows: &[ApplyRow], dry_run: bool, output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "dry_run": dry_run, "results": rows }))?
        ),
        OutputFormat::Table => {
            if dry_run {
                eprintln!("Plan (nothing has changed yet):");
            }
            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["Kind", "Slug", "Action", "ID", "Details"]);
            for row in rows {
                let details = row
                    .error
                    .clone()
                    .or_else(|| row.note.clone())
                    .unwrap_or_else(|| row.changed.join(", "));
                table.add_row([
                    row.kind,
                    row.slug.as_str(),
                    row.action,
                    row.id.as_deref().unwrap_or("-"),
                    details.as_str(),
                ]);
            }
            eprintln!("{table}");
        }
    }
    Ok(())
}

// --- Validate ---

#[derive(Debug, Serialize)]
struct ValidationRow {
    service: String,
    provider: Option<String>,
    /// ready, pending_credentials or failed
    status: &'static str,
    issues: Vec<String>,
    scopes: Vec<String>,
    redirect_uri: String,
    connect_command: String,
    /// Values for the Lark Platform Accounts row (no secrets).
    lark: Value,
    playbook: Value,
    verification: Value,
    smoke_command: Option<String>,
    operation_count: usize,
    unmanaged_operations: Vec<String>,
}

async fn validate(
    api: &mut ApiClient,
    manifest: &Manifest,
    redirect: &str,
) -> Result<Vec<ValidationRow>> {
    let me: Value = api.get("/users/me").await?;
    let services = list_services(api).await?;
    let providers = list_providers(api).await?;
    let mut report = Vec::new();
    let mut icon_presence = std::collections::HashMap::new();
    for desired in &manifest.services {
        let slug = text(desired, "slug");
        let mut failures = Vec::new();
        let mut pending = Vec::new();
        let mut notes = Vec::new();
        let service = match unambiguous_row(&services, &slug) {
            Ok(row) => row,
            Err(_) => {
                failures.push("ambiguous_existing_service".to_owned());
                None
            }
        };
        let provider_ref = desired
            .get("provider")
            .and_then(Value::as_str)
            .or_else(|| service.and_then(|row| row["provider_config_id"].as_str()));
        let provider = match provider_ref {
            Some(reference) => match unambiguous_row(&providers, reference) {
                Ok(row) => row,
                Err(_) => {
                    failures.push("ambiguous_existing_provider".to_owned());
                    None
                }
            },
            None => None,
        };
        let desired_provider = provider_ref.and_then(|reference| {
            manifest.providers.iter().find(|row| {
                row["slug"] == reference || provider.is_some_and(|live| live["slug"] == row["slug"])
            })
        });
        if let Some(service) = service {
            if service["is_active"] != true {
                failures.push("service is inactive".into());
            }
            let body = playbook::api_fields(desired);
            for (field, value) in body.as_object().into_iter().flatten() {
                if field == "provider" {
                    continue;
                }
                let live = if field == "auth_method" && service[field].is_null() {
                    &service["auth_type"]
                } else {
                    &service[field]
                };
                if !playbook::matches_fields(value, live) {
                    failures.push(format!("service {field} differs from the manifest"));
                }
            }
            if let Some(provider) = provider
                && service["provider_config_id"] != provider["id"]
            {
                failures.push("service is not linked to the declared provider".into());
            }
        } else {
            failures.push("service not found".into());
        }
        if provider_ref.is_some() && provider.is_none() {
            failures.push("referenced provider not found".into());
        }
        if let Some(provider) = provider {
            if provider["is_active"] != true {
                failures.push("provider is inactive".into());
            }
            if let Some(desired) = desired_provider {
                let fields = playbook::api_fields(desired);
                for field in [
                    "authorization_url",
                    "token_url",
                    "device_code_url",
                    "device_token_url",
                    "default_scopes",
                    "extra_auth_params",
                    "token_endpoint_auth_method",
                    "supports_pkce",
                    "authorization_nonce_required",
                    "supports_oauth_scopes",
                    "credential_mode",
                    "revocation_url",
                    "revocation",
                ] {
                    let live = if field == "authorization_nonce_required"
                        && provider.get(field).is_none()
                    {
                        &Value::Bool(false)
                    } else {
                        &provider[field]
                    };
                    if let Some(value) = fields.get(field)
                        && !playbook::matches_fields(value, live)
                    {
                        failures.push(format!("provider {field} differs from the manifest"));
                    }
                }
            }
            let oauth = matches!(
                provider["provider_type"].as_str(),
                Some("oauth2" | "device_code")
            );
            if oauth && provider["credential_mode"] == "user" {
                notes.push("credential_mode is user: each user brings their own OAuth app; playbook C4 is missing".into());
            } else if oauth && !client_configured(provider) {
                pending.push(format!(
                    "OAuth client not set: nyxid admin provider set-credentials {}",
                    text(provider, "slug")
                ));
            } else if oauth && provider["has_oauth_config"] != true {
                failures.push("provider OAuth configuration is incomplete".into());
            }
        }
        // An ambiguous identity must never choose arbitrary endpoints.
        let endpoints = if let Some(service) = service {
            list_operations(api, &text(service, "id")).await?
        } else {
            vec![]
        };
        let authored_operations = playbook::operations(desired)?;
        for operation in &authored_operations {
            let matches: Vec<_> = endpoints
                .iter()
                .filter(|row| row["name"] == operation.name)
                .collect();
            if matches.len() != 1 {
                failures.push(format!(
                    "operation {} is missing or ambiguous",
                    operation.name
                ));
            } else {
                let fields = serde_json::to_value(operation)?;
                if !playbook::matches_fields(&fields, matches[0]) {
                    failures.push(format!(
                        "operation {} differs from the manifest",
                        operation.name
                    ));
                }
            }
        }
        let unmanaged_operations = if desired.get("operations").is_some() {
            endpoints
                .iter()
                .filter(|row| {
                    !authored_operations
                        .iter()
                        .any(|operation| row["name"] == operation.name)
                })
                .filter_map(|row| row["name"].as_str().map(str::to_owned))
                .collect()
        } else {
            vec![]
        };
        let icon_present = if let Some(provider) = provider {
            let id = text(provider, "id");
            if let Some(present) = icon_presence.get(&id) {
                *present
            } else {
                let present = playbook::served_icon(api.base_url_root(), provider).await?;
                icon_presence.insert(id, present);
                present
            }
        } else {
            false
        };
        let (mut checklist, verification, smoke_command) = playbook::report(
            desired,
            desired_provider,
            provider,
            provider_ref.is_some(),
            &endpoints,
            &me,
            icon_present,
        );
        if failures
            .iter()
            .any(|reason| reason.starts_with("ambiguous_existing"))
        {
            checklist["complete"] = false.into();
        }
        let status = if !failures.is_empty() {
            "failed"
        } else if !pending.is_empty() {
            "pending_credentials"
        } else {
            "ready"
        };
        let scopes = provider
            .and_then(|row| row["default_scopes"].as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let provider_slug = provider.map(|row| text(row, "slug"));
        let mut issues = failures;
        issues.extend(pending);
        issues.extend(notes);
        if checklist["complete"] == false {
            issues.push("playbook incomplete; configuration readiness is not verification".into());
        }
        report.push(ValidationRow {
            connect_command: playbook::connect_command(&slug,provider),
            lark: json!({"platform":provider_slug.as_ref().map(|slug|format!("platform-{slug}")),"type":"Platform services","configuration":status,"remarks":format!("NyxID service {slug}; configured; not verified; playbook complete: {}",checklist["complete"])}),
            service:slug, provider:provider_slug, status, issues, scopes,
            redirect_uri:redirect.to_owned(), playbook:checklist, verification, smoke_command,
            operation_count:endpoints.iter().filter(|row|row["is_active"]==true).count(),
            unmanaged_operations,
        });
    }
    Ok(report)
}

fn print_validation(report: &[ValidationRow], output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(report)?),
        OutputFormat::Table => {
            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["Service", "Provider", "Status", "Playbook", "Issues"]);
            for row in report {
                let issues = row.issues.join("; ");
                table.add_row([
                    row.service.as_str(),
                    row.provider.as_deref().unwrap_or("-"),
                    row.status,
                    if row.playbook["complete"] == true {
                        "complete"
                    } else {
                        "incomplete"
                    },
                    issues.as_str(),
                ]);
            }
            eprintln!("{table}");
            for row in report {
                eprintln!("{}: Connect: {}", row.service, row.connect_command);
                if let Some(command) = &row.smoke_command {
                    eprintln!("  Smoke: {command}");
                }
                eprintln!("  Playbook: {}", row.playbook);
                eprintln!("  Verification: {}", row.verification);
                eprintln!("  Lark: {}", row.lark);
            }
            if let Some(row) = report.first() {
                eprintln!("OAuth redirect URI for vendor apps: {}", row.redirect_uri);
            }
        }
    }
    Ok(())
}

// --- Credentials ---

async fn set_client(
    api: &mut ApiClient,
    provider: &Value,
    redirect: &str,
    client_id_env: Option<&str>,
    client_secret_env: Option<&str>,
) -> Result<Value> {
    let slug = text(provider, "slug");
    let needs_secret = provider["provider_type"] != "device_code";
    api.require_credential_transport()?;
    let (client_id, client_secret) = read_client(
        &slug,
        needs_secret,
        redirect,
        client_id_env,
        client_secret_env,
    )?;
    let fields = json!({});
    let body = ClientRequest {
        fields: &fields,
        client_id: Some(client_id.as_str()),
        client_secret: client_secret.as_ref().map(|value| value.as_str()),
    };
    let updated: Value = api
        .put(&format!("/providers/{}", text(provider, "id")), &body)
        .await
        .map_err(redacted_client_error)?;
    eprintln!(
        "{slug}: OAuth client {}.",
        if client_configured(&updated) {
            "saved"
        } else {
            "not complete"
        }
    );
    Ok(updated)
}

/// OAuth client values for providers created in this run, keyed by provider
/// slug. They are sent to the API only, never written to a file or printed.
type ClientValues = (Zeroizing<String>, Option<Zeroizing<String>>);
// Only environment variable names survive planning. Never inspect env in dry-run.
enum ClientSource {
    Environment {
        id: String,
        secret: Option<String>,
        prompt_redirect: Option<String>,
    },
}
type Clients = BTreeMap<String, ClientSource>;

impl ClientSource {
    fn load(&self, slug: &str) -> Result<ClientValues> {
        let Self::Environment {
            id,
            secret,
            prompt_redirect,
        } = self;
        if std::env::var_os(id).is_none()
            && let Some(redirect) = prompt_redirect
        {
            return read_client(slug, secret.is_some(), redirect, None, None);
        }
        Ok((
            env_secret(id)?,
            secret.as_deref().map(env_secret).transpose()?,
        ))
    }
}

#[derive(Serialize)]
struct ClientRequest<'a> {
    #[serde(flatten)]
    fields: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<&'a str>,
}
async fn post_provider(
    api: &mut ApiClient,
    fields: &Value,
    client: Option<&ClientValues>,
) -> Result<Value> {
    if client.is_some() {
        api.require_credential_transport()?;
    }
    let body = ClientRequest {
        fields,
        client_id: client.map(|(id, _)| id.as_str()),
        client_secret: client.and_then(|(_, secret)| secret.as_ref().map(|s| s.as_str())),
    };
    let result = api.post("/providers", &body).await;
    if client.is_some() {
        result.map_err(redacted_client_error)
    } else {
        result
    }
}

fn redacted_client_error(error: anyhow::Error) -> anyhow::Error {
    if let Some(api) = error.downcast_ref::<crate::api::ApiError>() {
        let code = api.body_json().and_then(|body| body["error_code"].as_i64());
        return anyhow!(
            "OAuth client request failed (HTTP {}, error_code {}); response details redacted",
            api.status().as_u16(),
            code.map_or_else(|| "unavailable".into(), |n| n.to_string())
        );
    }
    anyhow!("OAuth client request failed; transport details redacted")
}

/// The server refuses to create an OAuth or device-code provider in admin
/// mode (the default) without its client.
fn client_required_at_creation(desired: &Value) -> bool {
    matches!(
        desired["provider_type"].as_str(),
        Some("oauth2" | "device_code")
    ) && desired
        .get("credential_mode")
        .and_then(Value::as_str)
        .unwrap_or("admin")
        == "admin"
}

/// `zoho-mail` reads `ZOHO_MAIL_CLIENT_ID` and `ZOHO_MAIL_CLIENT_SECRET`.
fn client_env_names(slug: &str) -> (String, String) {
    let base: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    (format!("{base}_CLIENT_ID"), format!("{base}_CLIENT_SECRET"))
}

fn clients_from_env(manifest: &Manifest) -> Result<Clients> {
    Ok(manifest
        .providers
        .iter()
        .filter(|desired| client_required_at_creation(desired))
        .map(|desired| {
            let slug = text(desired, "slug");
            let (id, secret) = client_env_names(&slug);
            (
                slug,
                ClientSource::Environment {
                    id,
                    secret: (desired["provider_type"] != "device_code").then_some(secret),
                    prompt_redirect: None,
                },
            )
        })
        .collect())
}

/// Read a client ID and (unless the provider is device-code) a secret from
/// environment variables, or ask: the ID visibly, the secret hidden.
fn read_client(
    slug: &str,
    needs_secret: bool,
    redirect: &str,
    client_id_env: Option<&str>,
    client_secret_env: Option<&str>,
) -> Result<ClientValues> {
    eprintln!("{slug}: register this redirect URI in the vendor's OAuth app: {redirect}");
    read_client_values(needs_secret, client_id_env, client_secret_env)
}

fn read_client_values(
    needs_secret: bool,
    client_id_env: Option<&str>,
    client_secret_env: Option<&str>,
) -> Result<ClientValues> {
    let client_id = match client_id_env {
        Some(name) => env_secret(name)?,
        None => Zeroizing::new(ask_required("Client ID", None)?),
    };
    let client_secret = match (client_secret_env, needs_secret) {
        (Some(name), _) => Some(env_secret(name)?),
        (None, true) => Some(Zeroizing::new(
            rpassword::prompt_password("Client secret (hidden): ")
                .map_err(|_| anyhow!("Failed to read the client secret"))?,
        )),
        (None, false) => None,
    };
    if client_secret
        .as_deref()
        .is_some_and(|secret| secret.trim().is_empty())
    {
        bail!("The client secret is empty");
    }
    Ok((
        Zeroizing::new(client_id.trim().to_string()),
        client_secret.map(|secret| Zeroizing::new(secret.trim().to_string())),
    ))
}

fn env_secret(name: &str) -> Result<Zeroizing<String>> {
    let value = Zeroizing::new(
        std::env::var(name).map_err(|_| anyhow!("Environment variable {name} is not set"))?,
    );
    if value.trim().is_empty() {
        bail!("Environment variable {name} is empty");
    }
    Ok(value)
}

fn client_configured(provider: &Value) -> bool {
    provider["has_client_id"] == true
        && (provider["provider_type"] == "device_code" || provider["has_client_secret"] == true)
}

fn client_status(provider: &Value) -> &'static str {
    match provider["provider_type"].as_str() {
        Some("oauth2" | "device_code") if client_configured(provider) => "set",
        Some("oauth2" | "device_code") => "missing",
        _ => "n/a",
    }
}

// --- Wizard ---

fn wizard_document() -> Result<Value> {
    eprintln!("New catalog service. Press Enter to accept a [default].");
    let mut provider = provider_wizard(None)?;
    if matches!(
        provider["provider_type"].as_str(),
        Some("oauth2" | "device_code")
    ) {
        let scope_kind = if provider["supports_oauth_scopes"] == false {
            "no_scopes".to_owned()
        } else {
            ask_choice(
                "Scope source",
                &["vendor_cli", "documented_minimum"],
                "documented_minimum",
            )?
        };
        let revocation_kind = ask_choice(
            "Revocation applicability",
            &["configured", "vendor_unsupported", "nyxid_incompatible"],
            if provider["revocation_url"].is_string() {
                "configured"
            } else {
                "vendor_unsupported"
            },
        )?;
        if revocation_kind == "configured" && !provider["revocation_url"].is_string() {
            provider["revocation_url"] = ask_required("Token revocation URL", None)?.into();
        }
        provider["playbook"] = json!({
            "scope_source":{"kind":scope_kind,"url":ask_required("HTTPS scope evidence URL",None)?},
            "revocation":{"kind":revocation_kind,"source":ask_required("HTTPS revocation evidence source",None)?}
        });
    }
    let provider_slug = text(&provider, "slug");
    let mut service = json!({
        "name": provider["name"],
        "slug": ask_required("Service slug", Some(&format!("api-{provider_slug}")))?,
        "provider": provider_slug,
        "base_url": ask_required("API base URL", None)?,
    });
    if provider["provider_type"] == "api_key" {
        service["auth_method"] = "header".into();
        service["auth_key_name"] =
            ask_required("Header that carries the API key", Some("X-API-Key"))?.into();
    } else {
        service["auth_method"] = "bearer".into();
    }
    let description = ask("Service description (optional)", None)?;
    if !description.is_empty() {
        service["description"] = description.into();
    }
    let spec = ask("OpenAPI spec URL (optional)", None)?;
    if !spec.is_empty() {
        service["openapi_spec_url"] = spec.into();
        service["smoke_test"] = json!({"method":"GET","path":ask_required("Literal read-only GET smoke path (no query or placeholders)",None)?});
    } else {
        let name = ask_required("Read-only operation name", Some("list_accounts"))?;
        let path = ask_required("Literal GET operation path", None)?;
        service["operations"] = json!([{"name":name,"method":"GET","path":path}]);
        service["smoke_test"] = json!({"operation":name});
    }
    eprintln!(
        "After apply, attach a permitted static SVG with nyxid admin catalog icon {provider_slug} --file <brand.svg>."
    );
    Ok(json!({ "providers": [provider], "services": [service] }))
}

/// Ask for every provider setting. With `current`, each question offers the
/// stored value and the slug and type stay fixed.
fn provider_wizard(current: Option<&Value>) -> Result<Value> {
    let had = |key: &str| {
        current
            .and_then(|row| row.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let name = ask_required("Display name", had("name").as_deref())?;
    let (slug, provider_type) = match current {
        Some(row) => {
            eprintln!(
                "Slug {} and type {} cannot change.",
                text(row, "slug"),
                text(row, "provider_type")
            );
            (text(row, "slug"), text(row, "provider_type"))
        }
        None => (
            ask_required("Provider slug", Some(&slugify(&name)))?,
            ask_choice(
                "Provider type",
                &["oauth2", "device_code", "api_key"],
                "oauth2",
            )?,
        ),
    };
    let mut provider = json!({ "name": name, "slug": slug, "provider_type": provider_type });
    let optional = |provider: &mut Value, key: &str, label: &str| -> Result<()> {
        let answer = ask(label, had(key).as_deref())?;
        if !answer.is_empty() {
            provider[key] = answer.into();
        }
        Ok(())
    };
    optional(&mut provider, "description", "Description (optional)")?;
    if provider_type == "api_key" {
        optional(
            &mut provider,
            "api_key_url",
            "Where users create API keys (URL, optional)",
        )?;
        optional(
            &mut provider,
            "api_key_instructions",
            "Instructions shown to users (optional)",
        )?;
        optional(
            &mut provider,
            "documentation_url",
            "Documentation URL (optional)",
        )?;
        return Ok(provider);
    }

    provider["authorization_url"] =
        ask_required("Authorization URL", had("authorization_url").as_deref())?.into();
    provider["token_url"] = ask_required("Token URL", had("token_url").as_deref())?.into();
    if provider_type == "device_code" {
        provider["device_code_url"] =
            ask_required("Device code URL", had("device_code_url").as_deref())?.into();
        provider["device_token_url"] =
            ask_required("Device token URL", had("device_token_url").as_deref())?.into();
        optional(
            &mut provider,
            "device_verification_url",
            "Device verification URL (optional)",
        )?;
    }
    let uses_now = current
        .and_then(|row| row["supports_oauth_scopes"].as_bool())
        .map_or("yes", |on| if on { "yes" } else { "no" });
    let uses_scopes = ask_choice(
        "Does the vendor use OAuth scopes (no = never send a scope parameter)",
        &["yes", "no"],
        uses_now,
    )? == "yes";
    provider["supports_oauth_scopes"] = uses_scopes.into();
    if uses_scopes {
        let scopes_now = current
            .and_then(|row| row["default_scopes"].as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            });
        let scopes = ask(
            "Scopes, matching the vendor's own CLI (space separated)",
            scopes_now.as_deref().filter(|value| !value.is_empty()),
        )?;
        provider["default_scopes"] = json!(scopes.split_whitespace().collect::<Vec<_>>());
    }
    let params_now = current
        .and_then(|row| row["extra_auth_params"].as_object())
        .map(|params| {
            params
                .iter()
                .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join(",")
        });
    let extra = ask(
        "Extra authorization parameters (name=value, comma separated)",
        params_now.as_deref().filter(|value| !value.is_empty()),
    )?;
    if !extra.trim().is_empty() {
        let mut params = Map::new();
        for pair in extra.split(',') {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| anyhow!("Expected name=value, got `{pair}`"))?;
            params.insert(key.trim().into(), value.trim().into());
        }
        provider["extra_auth_params"] = Value::Object(params);
    }
    provider["token_endpoint_auth_method"] = ask_choice(
        "Token endpoint authentication",
        &["client_secret_post", "client_secret_basic"],
        had("token_endpoint_auth_method")
            .as_deref()
            .unwrap_or("client_secret_post"),
    )?
    .into();
    let pkce_now = current
        .and_then(|row| row["supports_pkce"].as_bool())
        .map_or("yes", |on| if on { "yes" } else { "no" });
    provider["supports_pkce"] = (ask_choice("PKCE", &["yes", "no"], pkce_now)? == "yes").into();
    if provider_type == "oauth2" {
        let nonce_now = current
            .and_then(|row| row["authorization_nonce_required"].as_bool())
            .map_or("no", |on| if on { "yes" } else { "no" });
        provider["authorization_nonce_required"] = (ask_choice(
            "Requires a generated authorization nonce (hosted OAuth only; upgrade all backend replicas first)",
            &["yes", "no"],
            nonce_now,
        )? == "yes").into();
    }
    provider["credential_mode"] = ask_choice(
        "Who supplies the OAuth app (admin = NyxID managed, user = each user's own, both)",
        &["admin", "user", "both"],
        had("credential_mode").as_deref().unwrap_or("admin"),
    )?
    .into();
    optional(
        &mut provider,
        "revocation_url",
        "Token revocation URL (optional)",
    )?;
    optional(
        &mut provider,
        "documentation_url",
        "Documentation URL (optional)",
    )?;
    Ok(provider)
}

/// After the provider wizard, offer to set the NyxID-managed OAuth client.
async fn offer_client(api: &mut ApiClient, provider: &Value, auth: &AuthArgs) -> Result<()> {
    let oauth = matches!(
        provider["provider_type"].as_str(),
        Some("oauth2" | "device_code")
    );
    if !oauth || provider["credential_mode"] == "user" {
        return Ok(());
    }
    let question = if client_configured(provider) {
        "Replace the NyxID-managed OAuth client now?"
    } else {
        "Set the NyxID-managed OAuth client now?"
    };
    if confirm(question)? {
        let updated = set_client(api, provider, &redirect_uri(auth)?, None, None).await?;
        print_provider(&updated, auth)?;
    }
    Ok(())
}

fn require_terminal(message: &str) -> Result<()> {
    if std::io::stdin().is_terminal() {
        Ok(())
    } else {
        bail!("{message}")
    }
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

fn ask(label: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(default) => eprint!("{label} [{default}]: "),
        None => eprint!("{label}: "),
    }
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("Failed to read input")?;
    let answer = line.trim();
    Ok(match (answer.is_empty(), default) {
        (true, Some(default)) => default.to_string(),
        _ => answer.to_string(),
    })
}

fn ask_required(label: &str, default: Option<&str>) -> Result<String> {
    loop {
        let answer = ask(label, default)?;
        if !answer.is_empty() {
            return Ok(answer);
        }
        eprintln!("A value is required.");
    }
}

fn ask_choice(label: &str, choices: &[&str], default: &str) -> Result<String> {
    loop {
        let answer = ask(&format!("{label} ({})", choices.join("/")), Some(default))?;
        if choices.contains(&answer.as_str()) {
            return Ok(answer);
        }
        eprintln!("Choose one of: {}", choices.join(", "));
    }
}

fn confirm(question: &str) -> Result<bool> {
    Ok(matches!(
        ask(&format!("{question} [y/N]"), None)?
            .to_ascii_lowercase()
            .as_str(),
        "y" | "yes"
    ))
}

// --- API helpers ---

async fn list_providers(api: &mut ApiClient) -> Result<Vec<Value>> {
    let listing: Value = api.get("/providers?include_inactive=true").await?;
    Ok(listing["providers"].as_array().cloned().unwrap_or_default())
}

async fn list_services(api: &mut ApiClient) -> Result<Vec<Value>> {
    let listing: Value = api.get("/services?include_inactive=true").await?;
    Ok(listing["services"].as_array().cloned().unwrap_or_default())
}

async fn find_provider(api: &mut ApiClient, reference: &str) -> Result<Option<Value>> {
    if uuid::Uuid::parse_str(reference).is_ok() {
        return api.get_optional(&format!("/providers/{reference}")).await;
    }
    Ok(unambiguous_row(&list_providers(api).await?, reference)?.cloned())
}

async fn provider_id(api: &mut ApiClient, reference: &str) -> Result<String> {
    find_provider(api, reference)
        .await?
        .map(|provider| text(&provider, "id"))
        .ok_or_else(|| anyhow!("Provider not found: {reference}"))
}

async fn find_service(api: &mut ApiClient, reference: &str) -> Result<Option<Value>> {
    if uuid::Uuid::parse_str(reference).is_ok() {
        return api.get_optional(&format!("/services/{reference}")).await;
    }
    Ok(unambiguous_row(&list_services(api).await?, reference)?.cloned())
}

fn redirect_uri(auth: &AuthArgs) -> Result<String> {
    Ok(format!(
        "{}/api/v1/providers/callback",
        auth.resolved_base_url()?.trim_end_matches('/')
    ))
}

fn print_provider(provider: &Value, auth: &AuthArgs) -> Result<()> {
    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(provider)?),
        OutputFormat::Table => {
            eprintln!(
                "Provider:      {} ({})",
                text(provider, "name"),
                text(provider, "slug")
            );
            eprintln!("ID:            {}", text(provider, "id"));
            eprintln!("Type:          {}", text(provider, "provider_type"));
            eprintln!("Icon URL:      {}", text(provider, "icon_url"));
            eprintln!("Authorize URL: {}", text(provider, "authorization_url"));
            eprintln!("Token URL:     {}", text(provider, "token_url"));
            eprintln!(
                "Auth nonce:    {} (hosted OAuth only)",
                yes_no(Some(
                    provider["authorization_nonce_required"]
                        .as_bool()
                        .unwrap_or(false)
                ))
            );
            let scopes = provider["default_scopes"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            eprintln!("Scopes:        {scopes}");
            eprintln!("OAuth client:  {}", client_status(provider));
            eprintln!(
                "Ready:         {}",
                yes_no(provider["has_oauth_config"].as_bool())
            );
            eprintln!("Redirect URI:  {}", redirect_uri(auth)?);
        }
    }
    Ok(())
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("-").to_string()
}

fn yes_no(value: Option<bool>) -> &'static str {
    if value == Some(true) { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{env_lock, mock_auth};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    fn zoho(vars: &[&str]) -> Result<Manifest> {
        let vars: Vec<String> = vars.iter().map(|value| value.to_string()).collect();
        Manifest::from_document(template_document("zoho-mail")?, &vars, false)
    }

    #[test]
    fn zoho_template_resolves_its_data_center_and_keeps_secrets_out() {
        let manifest = zoho(&[]).unwrap();
        assert_eq!(
            manifest.providers[0]["token_url"],
            "https://accounts.zoho.com/oauth/v2/token"
        );
        assert_eq!(
            manifest.services[0]["base_url"],
            "https://mail.zoho.com/api"
        );
        assert_eq!(manifest.services[0]["auth_scheme"], "Zoho-oauthtoken");
        assert_eq!(
            playbook::operations(&manifest.services[0]).unwrap().len(),
            3
        );
        assert_eq!(
            playbook::smoke_command(&manifest.services[0]).as_deref(),
            Some("nyxid proxy request api-zoho-mail /accounts --method GET")
        );
        assert_eq!(
            manifest.providers[0]["playbook"]["revocation"]["kind"],
            "nyxid_incompatible"
        );
        let eu = zoho(&["zoho_domain=zoho.eu"]).unwrap();
        assert_eq!(eu.services[0]["base_url"], "https://mail.zoho.eu/api");
        assert!(zoho(&["zoho_domain=evil.example"]).is_err());
        assert!(zoho(&["zoho_region=eu"]).is_err());
    }

    #[test]
    fn manifests_reject_secrets_and_undeclared_variables() {
        let with_secret = json!({
            "providers": [{ "slug": "p", "provider_type": "oauth2", "client_secret": "x" }],
        });
        let error = Manifest::from_document(with_secret, &[], false).unwrap_err();
        assert!(error.to_string().contains("client_secret"), "{error}");
        let undeclared = json!({ "services": [{ "slug": "s", "base_url": "https://${host}" }] });
        assert!(Manifest::from_document(undeclared, &[], false).is_err());
        let unsafe_value = json!({
            "variables": { "host": {} },
            "services": [{ "slug": "s", "base_url": "https://${host}" }],
        });
        assert!(Manifest::from_document(unsafe_value, &["host=a/b?x".to_string()], false).is_err());
        let yaml = parse_document("providers:\n  - slug: p\n    provider_type: oauth2\n").unwrap();
        assert_eq!(yaml["providers"][0]["slug"], "p");
    }

    fn provider_row(client: bool) -> Value {
        json!({
            "id": "11111111-1111-4111-8111-111111111111",
            "slug": "zoho-mail",
            "name": "Zoho Mail",
            "description": "Zoho Mail OAuth (single data center)",
            "provider_type": "oauth2",
            "credential_mode": "admin",
            "authorization_url": "https://accounts.zoho.com/oauth/v2/auth",
            "token_url": "https://accounts.zoho.com/oauth/v2/token",
            "default_scopes": ["ZohoMail.accounts.READ,ZohoMail.folders.READ,ZohoMail.messages.READ"],
            "supports_oauth_scopes": true,
            "supports_pkce": false,
            "token_endpoint_auth_method": "client_secret_post",
            "extra_auth_params": { "access_type": "offline", "prompt": "consent" },
            "documentation_url": "https://www.zoho.com/mail/help/api/",
            "is_active": true,
            "has_client_id": client,
            "has_client_secret": client,
            "has_oauth_config": client,
        })
    }

    fn service_row() -> Value {
        let manifest = zoho(&[]).unwrap();
        let mut row = playbook::api_fields(&manifest.services[0]);
        let object = row.as_object_mut().unwrap();
        object.remove("provider");
        object.insert("id".into(), "22222222-2222-4222-8222-222222222222".into());
        object.insert(
            "provider_config_id".into(),
            provider_row(false)["id"].clone(),
        );
        object.insert("auth_method".into(), "bearer".into());
        object.insert("auth_type".into(), "bearer".into());
        object.insert("is_active".into(), true.into());
        row
    }

    async fn listing(server: &MockServer, providers: Value, services: Value) {
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id":"admin","is_admin":true})),
            )
            .mount(server)
            .await;
        for service in services.as_array().into_iter().flatten() {
            if let Some(id) = service["id"].as_str() {
                Mock::given(method("GET"))
                    .and(path(format!("/api/v1/services/{id}/endpoints")))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(json!({"endpoints":template_operations()})),
                    )
                    .mount(server)
                    .await;
            }
        }
        if services.as_array().is_some_and(Vec::is_empty) {
            Mock::given(method("GET"))
                .and(path(
                    "/api/v1/services/22222222-2222-4222-8222-222222222222/endpoints",
                ))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"endpoints":template_operations()})),
                )
                .mount(server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/api/v1/providers"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "providers": providers })),
            )
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "services": services })))
            .mount(server)
            .await;
    }

    fn template_operations() -> Vec<Value> {
        playbook::operations(&zoho(&[]).unwrap().services[0])
            .unwrap()
            .into_iter()
            .map(|operation| {
                let mut row = serde_json::to_value(operation).unwrap();
                row["id"] = format!("ep-{}", row["name"].as_str().unwrap()).into();
                row["is_active"] = true.into();
                row
            })
            .collect()
    }

    #[test]
    fn catalog_auth_scheme_manifest_validation_is_strict_and_passes_api_field() {
        assert!(
            Manifest::from_document(
                json!({"services":[{"slug":"scheme","auth_scheme":"Zoho-oauthtoken"}]}),
                &[],
                false
            )
            .is_err()
        );
        for scheme in [
            json!(""),
            json!("two words"),
            json!("Bearer\r\n"),
            json!("a".repeat(65)),
            json!(3),
        ] {
            assert!(Manifest::from_document(json!({"services":[{"slug":"scheme","auth_method":"bearer","auth_scheme":scheme}]}), &[], false).is_err());
        }
        assert!(Manifest::from_document(json!({"services":[{"slug":"scheme","auth_method":"header","auth_scheme":"Zoho-oauthtoken"}]}), &[], false).is_err());
        let row = &zoho(&[]).unwrap().services[0];
        assert_eq!(playbook::api_fields(row)["auth_scheme"], "Zoho-oauthtoken");
        assert!(playbook::api_fields(row).get("operations").is_none());
    }

    #[tokio::test]
    async fn icon_upload_sets_provider_url() {
        let server = MockServer::start().await;
        listing(&server, json!([provider_row(true)]), json!([])).await;
        let svg = r#"<svg viewBox="0 0 24 24"><path d="M0 0h24v24z"/></svg>"#;
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), svg).unwrap();
        let icon_url = format!("{}/catalog-icons/hash", server.uri());
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/providers/11111111-1111-4111-8111-111111111111/icon",
            ))
            .and(body_json(json!({"svg": svg})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"icon_url": icon_url})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(
                "/api/v1/providers/11111111-1111-4111-8111-111111111111",
            ))
            .and(body_json(json!({"icon_url": icon_url})))
            .respond_with(ResponseTemplate::new(200).set_body_json(provider_row(true)))
            .expect(1)
            .mount(&server)
            .await;
        run_catalog(AdminCatalogCommands::Icon {
            provider: "zoho-mail".into(),
            file: file.path().into(),
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn provider_update_with_client_sends_the_client_with_the_change() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let server = MockServer::start().await;
        listing(&server, json!([provider_row(true)]), json!([])).await;
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "token_url: https://accounts.zoho.eu/oauth/v2/token\n",
        )
        .unwrap();
        Mock::given(method("PUT"))
            .and(path(
                "/api/v1/providers/11111111-1111-4111-8111-111111111111",
            ))
            .and(body_json(json!({
                "token_url": "https://accounts.zoho.eu/oauth/v2/token",
                "client_id": "current-id",
                "client_secret": "current-secret",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(provider_row(true)))
            .expect(1)
            .mount(&server)
            .await;
        unsafe {
            std::env::set_var("NYXID_TEST_WITH_CLIENT_ID", "current-id");
            std::env::set_var("NYXID_TEST_WITH_CLIENT_SECRET", " current-secret ");
        }
        let result = run_provider(AdminProviderCommands::Update {
            provider: "zoho-mail".into(),
            file: Some(file.path().into()),
            with_client: true,
            client_id_env: Some("NYXID_TEST_WITH_CLIENT_ID".into()),
            client_secret_env: Some("NYXID_TEST_WITH_CLIENT_SECRET".into()),
            auth: mock_auth(server.uri()),
        })
        .await;
        unsafe {
            std::env::remove_var("NYXID_TEST_WITH_CLIENT_ID");
            std::env::remove_var("NYXID_TEST_WITH_CLIENT_SECRET");
        }
        result.unwrap();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn apply_creates_provider_then_links_the_new_service_to_it() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let server = MockServer::start().await;
        listing(&server, json!([]), json!([])).await;
        let manifest = zoho(&[]).unwrap();
        let mut provider_body = playbook::api_fields(&manifest.providers[0]);
        provider_body["client_id"] = "zoho-id".into();
        provider_body["client_secret"] = "zoho-secret".into();
        Mock::given(method("POST"))
            .and(path("/api/v1/providers"))
            .and(body_json(&provider_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(provider_row(true)))
            .expect(1)
            .mount(&server)
            .await;
        let mut service = playbook::api_fields(&manifest.services[0]);
        service.as_object_mut().unwrap().remove("provider");
        service["provider_config_id"] = provider_row(false)["id"].clone();
        Mock::given(method("POST"))
            .and(path("/api/v1/services"))
            .and(body_json(&service))
            .respond_with(ResponseTemplate::new(200).set_body_json(service_row()))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(
                "/api/v1/services/22222222-2222-4222-8222-222222222222/endpoints",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"endpoints":[]})))
            .with_priority(1)
            .mount(&server)
            .await;
        for operation in playbook::operations(&manifest.services[0]).unwrap() {
            let body = serde_json::to_value(operation).unwrap();
            let mut response = body.clone();
            response["id"] = format!("ep-{}", response["name"].as_str().unwrap()).into();
            response["is_active"] = true.into();
            Mock::given(method("POST"))
                .and(path(
                    "/api/v1/services/22222222-2222-4222-8222-222222222222/endpoints",
                ))
                .and(body_json(body))
                .respond_with(ResponseTemplate::new(200).set_body_json(response))
                .expect(1)
                .mount(&server)
                .await;
        }

        let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
        // Without a client the managed provider is planned with a note and
        // refused on a real run.
        let plan = apply(&mut api, &manifest, true, &Clients::new())
            .await
            .unwrap();
        assert_eq!(
            plan.iter().map(|row| row.action).collect::<Vec<_>>(),
            ["create", "create", "create", "create", "create"]
        );
        assert!(
            plan[0]
                .note
                .as_deref()
                .unwrap()
                .contains("ZOHO_MAIL_CLIENT_ID")
        );
        let refused = apply(&mut api, &manifest, false, &Clients::new())
            .await
            .unwrap();
        assert_eq!(refused[0].action, "failed");
        assert_eq!(refused[1].action, "failed");

        unsafe {
            std::env::set_var("NYXID_CATALOG_TEST_CLIENT_ID", "zoho-id");
            std::env::set_var("NYXID_CATALOG_TEST_CLIENT_SECRET", "zoho-secret");
        }
        let clients = Clients::from([(
            "zoho-mail".to_string(),
            ClientSource::Environment {
                id: "NYXID_CATALOG_TEST_CLIENT_ID".into(),
                secret: Some("NYXID_CATALOG_TEST_CLIENT_SECRET".into()),
                prompt_redirect: None,
            },
        )]);
        let result = apply(&mut api, &manifest, false, &clients).await;
        unsafe {
            std::env::remove_var("NYXID_CATALOG_TEST_CLIENT_ID");
            std::env::remove_var("NYXID_CATALOG_TEST_CLIENT_SECRET");
        }
        let rows = result.unwrap();
        assert_eq!(
            rows.iter().map(|row| row.action).collect::<Vec<_>>(),
            ["created", "created", "created", "created", "created"]
        );
    }

    #[test]
    #[allow(clippy::await_holding_lock)]
    fn clients_come_from_slug_named_environment_variables() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(
            client_env_names("zoho-mail"),
            (
                "ZOHO_MAIL_CLIENT_ID".to_string(),
                "ZOHO_MAIL_CLIENT_SECRET".to_string()
            )
        );
        let manifest = zoho(&[]).unwrap();
        let sources = clients_from_env(&manifest).unwrap();
        assert!(sources.contains_key("zoho-mail"));
        unsafe {
            std::env::set_var("ZOHO_MAIL_CLIENT_ID", "id-from-env");
        }
        let missing_secret = sources["zoho-mail"].load("zoho-mail");
        unsafe {
            std::env::set_var("ZOHO_MAIL_CLIENT_SECRET", "secret-from-env");
        }
        let client = sources["zoho-mail"].load("zoho-mail");
        unsafe {
            std::env::remove_var("ZOHO_MAIL_CLIENT_ID");
            std::env::remove_var("ZOHO_MAIL_CLIENT_SECRET");
        }
        assert!(missing_secret.is_err());
        let (id, secret) = client.unwrap();
        assert_eq!(id.as_str(), "id-from-env");
        assert_eq!(
            secret.as_ref().map(|value| value.as_str()),
            Some("secret-from-env")
        );
    }

    #[tokio::test]
    async fn apply_is_unchanged_when_rows_match_and_updates_only_drift() {
        let server = MockServer::start().await;
        let mut drifted = service_row();
        drifted["description"] = "old".into();
        listing(&server, json!([provider_row(true)]), json!([drifted])).await;
        Mock::given(method("PUT"))
            .and(path(
                "/api/v1/services/22222222-2222-4222-8222-222222222222",
            ))
            .and(body_json(
                json!({ "description": "Read Zoho Mail accounts, folders and messages." }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(service_row()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let manifest = zoho(&[]).unwrap();
        let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
        let rows = apply(&mut api, &manifest, false, &Clients::new())
            .await
            .unwrap();
        assert_eq!(rows[0].action, "unchanged");
        let service = rows.iter().find(|row| row.kind == "service").unwrap();
        assert_eq!(service.action, "updated");
        assert_eq!(service.changed, ["description"]);
    }

    #[tokio::test]
    async fn apply_refuses_to_change_create_only_service_fields() {
        let server = MockServer::start().await;
        let mut other_auth = service_row();
        other_auth["auth_method"] = "header".into();
        other_auth["auth_type"] = "api_key".into();
        listing(&server, json!([provider_row(true)]), json!([other_auth])).await;
        let manifest = zoho(&[]).unwrap();
        let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
        let rows = apply(&mut api, &manifest, true, &Clients::new())
            .await
            .unwrap();
        assert_eq!(rows[1].action, "failed");
        assert!(rows[1].error.as_deref().unwrap().contains("auth_method"));
        assert!(fail_on_errors(&rows).is_err());
    }

    #[tokio::test]
    async fn dry_run_plans_the_relink_to_a_provider_it_will_create() {
        let server = MockServer::start().await;
        listing(&server, json!([]), json!([service_row()])).await;
        let manifest = zoho(&[]).unwrap();
        let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
        let plan = apply(&mut api, &manifest, true, &Clients::new())
            .await
            .unwrap();
        assert_eq!(plan[0].action, "create");
        let service = plan.iter().find(|row| row.kind == "service").unwrap();
        assert_eq!(service.action, "update");
        assert_eq!(service.changed, ["provider"]);
    }

    #[test]
    fn authorization_nonce_cli_validation_is_scoped_and_manifest_roundtrips() {
        for required in [false, true] {
            let mut row = provider_row(true);
            row["authorization_nonce_required"] = required.into();
            // Standalone validation does not impose catalog playbook evidence.
            row["playbook"] = json!({"unknown_legacy_field": true});
            assert!(playbook::validate_authorization_nonce(&row).is_ok());
            row.as_object_mut().unwrap().remove("playbook");
            let manifest =
                Manifest::from_document(json!({"providers":[row.clone()]}), &[], false).unwrap();
            assert_eq!(
                playbook::api_fields(&manifest.providers[0])["authorization_nonce_required"],
                required
            );
            for (url, param) in [
                ("https://example.com/auth?nonce=x", "client_id"),
                ("https://example.com/auth?%6eonce=x", "client_id"),
                ("https://example.com/auth#fragment", "client_id"),
                ("https://example.com/auth", " nonce "),
            ] {
                row["authorization_url"] = url.into();
                row["client_id_param_name"] = param.into();
                assert_eq!(
                    playbook::validate_authorization_nonce(&row).is_ok(),
                    !required
                );
            }
            row["provider_type"] = "api_key".into();
            assert_eq!(
                playbook::validate_authorization_nonce(&row).is_ok(),
                !required
            );
        }
        let partial = Manifest::from_document(
            json!({"providers":[{"slug":"hubstaff","authorization_nonce_required":true}]}),
            &[],
            false,
        )
        .unwrap();
        assert_eq!(partial.providers[0]["authorization_nonce_required"], true);
        assert!(
            playbook::validate_authorization_nonce(&json!({"authorization_nonce_required":"true"}))
                .is_err()
        );
        assert!(
            changed_fields(
                &json!({"authorization_nonce_required":false}),
                &json!({}),
                &[]
            )
            .is_empty()
        );
        assert_eq!(
            changed_fields(
                &json!({"authorization_nonce_required":true}),
                &json!({}),
                &[]
            ),
            ["authorization_nonce_required"]
        );
    }

    #[tokio::test]
    async fn authorization_nonce_catalog_readback_treats_legacy_absence_as_false() {
        for (live, desired, drift) in [
            (None, false, false),
            (None, true, true),
            (Some(false), false, false),
            (Some(true), true, false),
            (Some(true), false, true),
        ] {
            let server = MockServer::start().await;
            let mut provider = provider_row(true);
            if let Some(live) = live {
                provider["authorization_nonce_required"] = live.into();
            }
            listing(&server, json!([provider]), json!([service_row()])).await;
            let mut manifest = zoho(&[]).unwrap();
            manifest.providers[0]["authorization_nonce_required"] = desired.into();
            let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
            let report = validate(&mut api, &manifest, "https://nyx.example/cb")
                .await
                .unwrap();
            assert_eq!(
                report[0]
                    .issues
                    .iter()
                    .any(|issue| issue.contains("provider authorization_nonce_required differs")),
                drift,
                "{:?}",
                report[0].issues
            );
        }
    }

    #[tokio::test]
    async fn validate_accepts_user_supplied_oauth_apps() {
        let server = MockServer::start().await;
        let mut provider = provider_row(false);
        provider["credential_mode"] = "user".into();
        provider["has_oauth_config"] = true.into();
        listing(&server, json!([provider]), json!([service_row()])).await;
        let mut manifest = zoho(&[]).unwrap();
        manifest.providers[0]["credential_mode"] = "user".into();
        let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
        let report = validate(&mut api, &manifest, "https://nyx.example/cb")
            .await
            .unwrap();
        assert_eq!(report[0].status, "ready", "{:?}", report[0].issues);
        assert!(report[0].issues[0].contains("own OAuth app"));
    }

    #[tokio::test]
    async fn validate_reports_pending_client_then_ready() {
        for (client, status) in [(false, "pending_credentials"), (true, "ready")] {
            let server = MockServer::start().await;
            listing(
                &server,
                json!([provider_row(client)]),
                json!([service_row()]),
            )
            .await;
            let manifest = zoho(&[]).unwrap();
            let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
            let report = validate(
                &mut api,
                &manifest,
                "https://nyx.example/api/v1/providers/callback",
            )
            .await
            .unwrap();
            assert_eq!(report[0].status, status, "{:?}", report[0].issues);
            assert_eq!(report[0].lark["platform"], "platform-zoho-mail");
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn set_credentials_reads_env_and_sends_both_values() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let server = MockServer::start().await;
        listing(&server, json!([provider_row(false)]), json!([])).await;
        Mock::given(method("PUT"))
            .and(path(
                "/api/v1/providers/11111111-1111-4111-8111-111111111111",
            ))
            .and(body_json(
                json!({ "client_id": "zoho-id", "client_secret": "zoho-secret" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(provider_row(true)))
            .expect(1)
            .mount(&server)
            .await;
        unsafe {
            std::env::set_var("NYXID_TEST_ZOHO_ID", "zoho-id");
            std::env::set_var("NYXID_TEST_ZOHO_SECRET", "zoho-secret");
        }
        let result = run_provider(AdminProviderCommands::SetCredentials {
            provider: "zoho-mail".into(),
            client_id_env: Some("NYXID_TEST_ZOHO_ID".into()),
            client_secret_env: Some("NYXID_TEST_ZOHO_SECRET".into()),
            auth: mock_auth(server.uri()),
        })
        .await;
        unsafe {
            std::env::remove_var("NYXID_TEST_ZOHO_ID");
            std::env::remove_var("NYXID_TEST_ZOHO_SECRET");
        }
        result.unwrap();
    }

    #[test]
    fn manifest_evidence_is_strict_and_never_in_api_fields() {
        let manifest=Manifest::from_document(json!({
            "providers":[{"slug":"p","playbook":{"scope_source":{"kind":"vendor_cli","url":"https://vendor.example/scopes"},"revocation":{"kind":"vendor_unsupported","source":"https://vendor.example/revoke"}}}],
            "services":[{"slug":"s","provider":"p","operations":[{"name":"accounts","method":"GET","path":"/accounts"}],"smoke_test":{"operation":"accounts"}}]
        }),&[],false).unwrap();
        assert!(manifest.to_document()["providers"][0]["playbook"].is_object());
        assert!(
            playbook::api_fields(&manifest.providers[0])
                .get("playbook")
                .is_none()
        );
        assert!(
            playbook::api_fields(&manifest.services[0])
                .get("operations")
                .is_none()
        );
        assert!(
            playbook::api_fields(&manifest.services[0])
                .get("smoke_test")
                .is_none()
        );
        for bad in [
            json!({"scope_source":{"kind":"vendor_cli","source":"https://vendor.example"}}),
            json!({"revocation":{"kind":"configured","url":"https://vendor.example"}}),
            json!({"scope_source":{"kind":"vendor_cli","url":"https://user:secret@vendor.example"}}),
            json!({"revocation":{"kind":"unknown","source":"https://vendor.example"}}),
        ] {
            assert!(
                Manifest::from_document(
                    json!({"providers":[{"slug":"p","playbook":bad}]}),
                    &[],
                    false
                )
                .is_err()
            );
        }
        for bad in [
            json!({"operation":"accounts","headers":{"Authorization":"canary"}}),
            json!({"method":"POST","path":"/accounts"}),
            json!({"method":"GET","path":"/accounts?token=canary"}),
            json!({"method":"GET","path":"/accounts/{id}"}),
        ] {
            assert!(
                Manifest::from_document(
                    json!({"services":[{"slug":"s","smoke_test":bad}]}),
                    &[],
                    false
                )
                .is_err()
            );
        }
        let both = json!({"services":[{"slug":"s","openapi_spec_url":"https://vendor.example/spec","operations":[]}]});
        assert!(Manifest::from_document(both, &[], false).is_err());
    }

    fn evidenced_manifest() -> Manifest {
        Manifest::from_document(json!({
            "providers":[{"slug":"zoho-mail","default_scopes":["read"],"playbook":{"scope_source":{"kind":"documented_minimum","url":"https://vendor.example/scopes"},"revocation":{"kind":"configured","source":"https://vendor.example/revocation"}},"revocation_url":"https://vendor.example/revoke"}],
            "services":[{"slug":"api-zoho-mail","provider":"zoho-mail","operations":[{"name":"accounts","method":"GET","path":"/accounts"}],"smoke_test":{"operation":"accounts"}}]
        }),&[],false).unwrap()
    }

    #[tokio::test]
    async fn catalog_readiness_reports_applicability_and_pending_real_gates() {
        for (provider_type, mode, client, expected_c1, expected_c4) in [
            ("oauth2", "admin", true, "met", "met"),
            ("oauth2", "both", false, "missing", "met"),
            ("oauth2", "user", false, "missing", "missing"),
            ("device_code", "admin", true, "met", "met"),
            (
                "api_key",
                "admin",
                false,
                "not_applicable",
                "not_applicable",
            ),
        ] {
            let server = MockServer::start().await;
            let manifest = evidenced_manifest();
            let mut provider = provider_row(client);
            provider["provider_type"] = provider_type.into();
            provider["credential_mode"] = mode.into();
            provider["has_oauth_config"] = true.into();
            provider["default_scopes"] = json!(["read"]);
            provider["revocation_url"] = "https://vendor.example/revoke".into();
            provider["device_code_url"] = "https://vendor.example/device".into();
            provider["device_token_url"] = "https://vendor.example/token".into();
            if provider_type == "device_code" {
                provider["has_client_secret"] = false.into();
            }
            provider["icon_url"] = format!(
                "{}/catalog-icons/33333333-3333-4333-8333-333333333333",
                server.uri()
            )
            .into();
            Mock::given(method("GET"))
                .and(path("/catalog-icons/33333333-3333-4333-8333-333333333333"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw("<svg viewBox=\"0 0 1 1\"/>", "image/svg+xml"),
                )
                .mount(&server)
                .await;
            listing(&server, json!([provider]), json!([service_row()])).await;
            Mock::given(method("GET")).and(path("/api/v1/services/22222222-2222-4222-8222-222222222222/endpoints"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"endpoints":[{"id":"ep","name":"accounts","method":"GET","path":"/accounts","is_active":true}]}))).with_priority(1).mount(&server).await;
            let mut api = ApiClient::from_auth(&mock_auth(server.uri()))
                .unwrap()
                .without_token_refresh();
            let report = validate(&mut api, &manifest, "https://nyx.example/cb")
                .await
                .unwrap();
            let row = &report[0];
            assert_eq!(row.playbook["c1"]["state"], expected_c1, "{row:?}");
            assert_eq!(row.playbook["c4"]["state"], expected_c4, "{row:?}");
            assert_eq!(row.playbook["c5"]["state"], "met");
            assert_eq!(row.playbook["c6"]["state"], "met");
            assert_eq!(
                row.verification,
                json!({"status":"not_verified","pending_gates":["api_smoke_test_real_vendor","outside_user_connection_admin_confirmed"]})
            );
            assert_eq!(
                row.smoke_command.as_deref(),
                Some("nyxid proxy request api-zoho-mail /accounts --method GET")
            );
            let option = match provider_type {
                "oauth2" => "--oauth",
                "device_code" => "--device-code",
                _ => "--credential-env VENDOR_API_KEY",
            };
            assert!(row.connect_command.ends_with(option));
            assert!(
                row.lark["remarks"]
                    .as_str()
                    .unwrap()
                    .contains("configured; not verified")
            );
            let requests = server.received_requests().await.unwrap();
            assert!(
                requests
                    .iter()
                    .all(|request| request.method == wiremock::http::Method::GET)
            );
        }
    }

    #[test]
    fn playbook_missing_provider_and_no_scope_evidence_fail_closed() {
        let manifest = evidenced_manifest();
        let me = json!({"id":"manager","is_admin":false,"capabilities":{"manage_catalog":true}});
        let endpoints = vec![json!({"is_active":true})];
        let desired = &manifest.services[0];
        let (missing, _, _) = playbook::report(desired, None, None, true, &endpoints, &me, false);
        assert_eq!(missing["complete"], false);
        assert_eq!(missing["c1"]["reason"], "referenced_provider_missing");
        let (na, verification, _) = playbook::report(
            &json!({"slug":"internal"}),
            None,
            None,
            false,
            &[],
            &me,
            false,
        );
        assert_eq!(na["c1"]["state"], "not_applicable");
        assert_eq!(verification["pending_gates"], json!([]));
        let mut provider = provider_row(true);
        provider["supports_oauth_scopes"] = false.into();
        provider["default_scopes"] = json!([]);
        let mut authored = manifest.providers[0].clone();
        authored["default_scopes"] = json!([]);
        authored["playbook"]["scope_source"]["kind"] = "no_scopes".into();
        let (proof, _, _) = playbook::report(
            desired,
            Some(&authored),
            Some(&provider),
            true,
            &endpoints,
            &me,
            false,
        );
        assert_eq!(proof["c2"]["state"], "not_applicable");
        assert_eq!(proof["c6"]["requires_admin"], false);
        assert!(
            proof["client_bound_write_requirement"]
                .as_str()
                .unwrap()
                .contains("existing_client_reentry")
        );
        authored["playbook"]["scope_source"]["kind"] = "documented_minimum".into();
        let (missing, _, _) = playbook::report(
            desired,
            Some(&authored),
            Some(&provider),
            true,
            &endpoints,
            &me,
            false,
        );
        assert_eq!(missing["c2"]["state"], "missing");
        provider["created_by"] = "manager".into();
        let (own, _, _) = playbook::report(
            desired,
            Some(&authored),
            Some(&provider),
            true,
            &endpoints,
            &me,
            false,
        );
        assert_eq!(own["c6"]["requires_admin"], false);
    }

    #[tokio::test]
    async fn catalog_icon_readiness_requires_owned_served_asset_without_credentials_or_redirects() {
        for case in [
            "valid",
            "missing",
            "foreign",
            "redirect",
            "not_svg",
            "oversized",
            "query",
        ] {
            let server = MockServer::start().await;
            let foreign = MockServer::start().await;
            let asset = "/catalog-icons/33333333-3333-4333-8333-333333333333";
            let mut provider = provider_row(true);
            let icon_url = match case {
                "foreign" => format!("{}{asset}", foreign.uri()),
                "query" => format!("{}{asset}?ignored=1", server.uri()),
                _ => format!("{}{asset}", server.uri()),
            };
            provider["icon_url"] = icon_url.into();
            let response = match case {
                "missing" => ResponseTemplate::new(404),
                "redirect" => ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/redirected", foreign.uri())),
                "not_svg" => ResponseTemplate::new(200).set_body_json(json!({})),
                "oversized" => ResponseTemplate::new(200)
                    .set_body_raw("x".repeat(128 * 1024 + 1), "image/svg+xml"),
                _ => ResponseTemplate::new(200)
                    .set_body_raw("<svg viewBox=\"0 0 1 1\"/>", "image/svg+xml"),
            };
            Mock::given(method("GET"))
                .and(path(asset))
                .respond_with(response)
                .mount(&server)
                .await;
            listing(&server, json!([provider]), json!([service_row()])).await;
            let mut api = ApiClient::from_auth(&mock_auth(server.uri()))
                .unwrap()
                .without_token_refresh();
            let report = validate(&mut api, &evidenced_manifest(), "https://nyx.example/cb")
                .await
                .unwrap();
            assert_eq!(
                report[0].playbook["c6"]["state"],
                if case == "valid" { "met" } else { "missing" },
                "{case}"
            );
            assert_eq!(report[0].verification["status"], "not_verified");
            let requests = server.received_requests().await.unwrap();
            assert!(
                requests
                    .iter()
                    .all(|request| request.method == wiremock::http::Method::GET)
            );
            let assets: Vec<_> = requests
                .iter()
                .filter(|request| request.url.path() == asset)
                .collect();
            assert_eq!(
                assets.len(),
                usize::from(!matches!(case, "foreign" | "query")),
                "{case}"
            );
            assert!(
                assets
                    .iter()
                    .all(|request| !request.headers.contains_key("authorization"))
            );
            assert!(
                foreign.received_requests().await.unwrap().is_empty(),
                "{case}"
            );
        }
    }

    #[tokio::test]
    async fn catalog_inactive_or_ambiguous_rows_never_create_and_validate_fails_closed() {
        for ambiguous in [false, true] {
            let server = MockServer::start().await;
            let mut inactive = service_row();
            inactive["is_active"] = false.into();
            let services = if ambiguous {
                json!([service_row(), inactive])
            } else {
                json!([inactive])
            };
            listing(&server, json!([provider_row(true)]), services).await;
            let manifest = zoho(&[]).unwrap();
            let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
            let rows = apply(&mut api, &manifest, false, &Clients::new())
                .await
                .unwrap();
            assert!(rows.iter().any(|row| if ambiguous {
                row.action == "failed" && row.error.as_deref().unwrap().contains("ambiguous")
            } else {
                row.action == "inactive"
            }));
            let validation = validate(&mut api, &manifest, "https://nyx.example/cb")
                .await
                .unwrap();
            assert_eq!(validation[0].status, "failed");
            assert_eq!(validation[0].playbook["complete"], false);
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|request| request.method == wiremock::http::Method::GET)
            );
        }
        let server = MockServer::start().await;
        let mut inactive = provider_row(true);
        inactive["is_active"] = false.into();
        listing(&server, json!([inactive]), json!([])).await;
        let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
        let rows = apply(&mut api, &zoho(&[]).unwrap(), false, &Clients::new())
            .await
            .unwrap();
        assert_eq!(rows[0].action, "inactive");
        assert!(
            rows[0]
                .note
                .as_deref()
                .unwrap()
                .contains("--file reactivate-provider.json")
        );
        assert_eq!(rows[1].action, "failed");
    }

    #[tokio::test]
    async fn explicit_operations_create_update_noop_and_retain_unmanaged_rows() {
        let server = MockServer::start().await;
        let desired = json!({"slug":"api-example","operations":[{"name":"accounts","method":"GET","path":"/accounts","description":"new"},{"name":"folders","method":"GET","path":"/folders"}]});
        let id = "service-id";
        Mock::given(method("GET")).and(path("/api/v1/services/service-id/endpoints"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"endpoints":[{"id":"one","name":"accounts","method":"GET","path":"/accounts","description":"old"},{"id":"extra","name":"unmanaged","method":"GET","path":"/other"}]}))).mount(&server).await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/services/service-id/endpoints/one"))
            .and(body_json(json!({"description":"new"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"one"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/service-id/endpoints"))
            .and(body_json(
                json!({"name":"folders","method":"GET","path":"/folders"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"two"})))
            .expect(1)
            .mount(&server)
            .await;
        let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
        let rows = reconcile_operations(&mut api, &desired, Some(id), false)
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|row| row.action).collect::<Vec<_>>(),
            ["updated", "created", "unmanaged"]
        );
        server.verify().await;
        server.reset().await;
        Mock::given(method("GET")).and(path("/api/v1/services/service-id/endpoints"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"endpoints":[{"id":"one","name":"accounts","method":"GET","path":"/accounts","description":"new"},{"id":"two","name":"folders","method":"GET","path":"/folders"},{"id":"extra","name":"unmanaged","method":"GET","path":"/other"}]}))).mount(&server).await;
        let rows = reconcile_operations(&mut api, &desired, Some(id), false)
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|row| row.action).collect::<Vec<_>>(),
            ["unchanged", "unchanged", "unmanaged"]
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method == wiremock::http::Method::GET)
        );
    }

    #[tokio::test]
    async fn catalog_client_errors_redact_only_secret_requests_and_keep_status_code() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/v1/providers"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error":"forbidden","error_code":1003,"message":"server echoed client-canary"}))).mount(&server).await;
        let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
        let client = (
            Zeroizing::new("id-canary".into()),
            Some(Zeroizing::new("secret-canary".into())),
        );
        let error = post_provider(&mut api, &json!({"slug":"p"}), Some(&client))
            .await
            .unwrap_err();
        let rendered = format!("{error:#}");
        assert!(rendered.contains("403"), "{rendered}");
        assert!(rendered.contains("1003"), "{rendered}");
        assert!(!rendered.contains("canary"), "{rendered}");
        let error = post_provider(&mut api, &json!({"slug":"p"}), None)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("server echoed client-canary"));
        let auth = mock_auth("http://example.invalid");
        assert!(ApiClient::from_auth_with_credential_transport(&auth).is_err());
    }

    #[tokio::test]
    async fn catalog_client_create_does_not_follow_redirects() {
        let origin = MockServer::start().await;
        let target = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/providers"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("Location", format!("{}/sink", target.uri())),
            )
            .expect(1)
            .mount(&origin)
            .await;
        let mut api = ApiClient::from_auth(&mock_auth(origin.uri())).unwrap();
        let client = (
            Zeroizing::new("fake-id".into()),
            Some(Zeroizing::new("redirect-canary".into())),
        );
        assert!(
            post_provider(&mut api, &json!({"slug":"p"}), Some(&client))
                .await
                .is_err()
        );
        assert!(target.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn catalog_dry_run_ignores_poisoned_client_environment() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let server = MockServer::start().await;
        listing(&server, json!([]), json!([])).await;
        let manifest = zoho(&[]).unwrap();
        let mut api = ApiClient::from_auth(&mock_auth(server.uri()))
            .unwrap()
            .without_token_refresh();
        unsafe {
            std::env::remove_var("ZOHO_MAIL_CLIENT_ID");
            std::env::remove_var("ZOHO_MAIL_CLIENT_SECRET");
        }
        let sources = clients_from_env(&manifest).unwrap();
        let empty = apply(&mut api, &manifest, true, &sources).await.unwrap();
        unsafe {
            std::env::set_var("ZOHO_MAIL_CLIENT_ID", "poisoned-client-canary");
            std::env::set_var("ZOHO_MAIL_CLIENT_SECRET", "");
        }
        let poisoned = apply(
            &mut api,
            &manifest,
            true,
            &clients_from_env(&manifest).unwrap(),
        )
        .await;
        unsafe {
            std::env::remove_var("ZOHO_MAIL_CLIENT_ID");
            std::env::remove_var("ZOHO_MAIL_CLIENT_SECRET");
        }
        let poisoned = poisoned.unwrap();
        assert_eq!(empty, poisoned);
        assert!(!serde_json::to_string(&poisoned).unwrap().contains("canary"));
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method == wiremock::http::Method::GET)
        );
    }

    #[test]
    fn catalog_billing_sync_and_bson_dates_do_not_create_drift() {
        let raw: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/billing/managed-service.json"
        ))
        .unwrap();
        let desired: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/billing/set-managed.golden.json"
        ))
        .unwrap();
        assert!(changed_fields(&desired, &raw, &[]).is_empty());
    }

    #[test]
    fn catalog_billing_normalization_preserves_explicit_clear() {
        assert_eq!(
            changed_fields(
                &json!({"billing":null}),
                &json!({"billing":{"platform_billable":true}}),
                &[]
            ),
            vec!["billing"]
        );
    }

    #[test]
    fn catalog_billing_normalization_keeps_authored_strings_exact() {
        let before = json!({"managed_execution":{"tariff_version":"2026-10-20T00:00:00Z"}});
        let after = json!({"managed_execution":{"tariff_version":"2026-10-19T19:00:00-05:00"}});
        assert_ne!(
            super::super::admin_billing::normalize_billing(&before),
            super::super::admin_billing::normalize_billing(&after)
        );
    }

    #[test]
    fn telnyx_template_pins_one_disabled_restricted_operation_with_typed_variables() {
        let manifest = Manifest::from_document(
            template_document("telnyx-platform").unwrap(),
            &["model=org/model:1".into(), "max_output_tokens=1024".into()],
            false,
        )
        .unwrap();
        assert!(manifest.providers.is_empty());
        assert_eq!(manifest.services.len(), 1);
        let service = &manifest.services[0];
        assert_eq!(service["service_category"], "internal");
        for field in [
            "requires_user_credential",
            "identity_propagation_mode",
            "inject_delegation_token",
        ] {
            assert!(service.get(field).is_none(), "{field} uses server defaults");
        }
        assert_eq!(service["forward_access_token"], false);
        assert_eq!(
            service["platform_key"],
            json!({"enabled":false,"audience":"restricted","allowed_owner_ids":[]})
        );
        assert!(service.get("billing").is_none());
        let operations = service["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0]["method"], "POST");
        assert_eq!(operations[0]["path"], "/ai/openai/chat/completions");
        assert_eq!(
            operations[0]["request_body_schema"]["properties"]["model"]["enum"],
            json!(["org/model:1"])
        );
        assert_eq!(
            operations[0]["request_body_schema"]["properties"]["max_tokens"]["maximum"],
            1024
        );
        for bad in ["-1", "0", "32769", "1.5", "1e3"] {
            assert!(
                Manifest::from_document(
                    template_document("telnyx-platform").unwrap(),
                    &[
                        "model=org/model:1".into(),
                        format!("max_output_tokens={bad}")
                    ],
                    false
                )
                .is_err()
            );
        }
    }

    #[test]
    fn typed_variables_preserve_default_charset_and_reject_unknown_types() {
        for spec in [
            json!({}),
            json!({"type":"string"}),
            json!({"type":"unknown"}),
        ] {
            assert!(
                resolve_variables(
                    &json!({"model":spec}),
                    &BTreeMap::from([("model".into(), "org/model:1".into())]),
                    false
                )
                .is_err()
            );
        }
        assert!(
            resolve_variables(
                &json!({"model":{"type":"model_id"}}),
                &BTreeMap::from([("model".into(), "org/model:1".into())]),
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn catalog_apply_warns_before_disabling_or_clearing_live_platform_access() {
        let live = json!({"slug":"telnyx-platform","is_active":true,"platform_key":{"enabled":true,"allowed_owner_ids":["person"]}});
        for patch in [
            json!({"platform_key":{"enabled":false}}),
            json!({"platform_key":{"allowed_owner_ids":[]}}),
        ] {
            assert!(platform_access_warning(&patch, &live).is_some());
        }
        assert!(
            platform_access_warning(&json!({"platform_key":{"enabled":true}}), &live).is_none()
        );
        assert!(
            platform_access_warning(
                &json!({"platform_key":{"enabled":false}}),
                &json!({"is_active":false})
            )
            .is_none()
        );
    }

    #[test]
    fn live_access_changes_require_confirmation_flag_or_dry_run() {
        let warnings = vec!["Disables live access".into()];
        assert!(
            authorize_access_changes(&warnings, false, false, false, || panic!("no terminal"))
                .is_err()
        );
        assert!(authorize_access_changes(&warnings, false, false, true, || Ok(false)).is_err());
        authorize_access_changes(&warnings, false, false, true, || Ok(true)).unwrap();
        authorize_access_changes(&warnings, false, true, false, || panic!("explicit flag"))
            .unwrap();
        authorize_access_changes(&warnings, true, false, false, || panic!("dry run")).unwrap();
    }
}
