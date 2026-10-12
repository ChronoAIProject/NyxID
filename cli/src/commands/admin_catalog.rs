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

use crate::api::ApiClient;
use crate::cli::{AdminCatalogCommands, AdminProviderCommands, AuthArgs, OutputFormat};

/// Built-in manifests for `nyxid admin catalog add --template`.
const TEMPLATES: &[(&str, &str)] = &[(
    "zoho-mail",
    include_str!("../../templates/catalog/zoho-mail.yaml"),
)];

/// Fields that would put an OAuth client secret into a file.
const PROVIDER_SECRET_FIELDS: &[&str] = &["client_id", "client_secret", "clear_client_credentials"];

/// Service fields the update route cannot change.
const SERVICE_CREATE_ONLY: &[&str] = &["service_type", "auth_method", "auth_key_name"];

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
            let mut body = match file {
                Some(path) => read_document(&path)?,
                None => {
                    require_terminal("Pass --file without a terminal")?;
                    eprintln!("New provider. Press Enter to accept a [default].");
                    provider_wizard(None)?
                }
            };
            reject_fields(&body, PROVIDER_SECRET_FIELDS, "provider file")?;
            if client_required_at_creation(&body) {
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
                body["client_id"] = id.into();
                if let Some(secret) = secret {
                    body["client_secret"] = secret.into();
                }
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let created: Value = api.post("/providers", &body).await?;
            print_provider(&created, &auth)?;
            if wizard && !client_configured(&created) {
                offer_client(&mut api, &created, &auth).await?;
            }
            Ok(())
        }
        AdminProviderCommands::Update {
            provider,
            file,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
            let updated: Value = if body.as_object().is_some_and(Map::is_empty) {
                eprintln!("No changes.");
                current
            } else {
                api.put(&format!("/providers/{}", text(&current, "id")), &body)
                    .await?
            };
            print_provider(&updated, &auth)?;
            if file.is_none() {
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
        AdminCatalogCommands::Apply {
            manifest,
            dry_run,
            auth,
        } => {
            let manifest = Manifest::load(&manifest.file, &manifest.vars, false)?;
            let clients = clients_from_env(&manifest)?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let rows = apply(&mut api, &manifest, dry_run, &clients).await?;
            print_apply(&rows, dry_run, auth.output)?;
            fail_on_errors(&rows)
        }
        AdminCatalogCommands::Validate { manifest, auth } => {
            let manifest = Manifest::load(&manifest.file, &manifest.vars, false)?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
            if (client_id_env.is_some() || client_secret_env.is_some())
                && manifest.providers.len() != 1
            {
                bail!("--client-id-env and --client-secret-env need a manifest with one provider");
            }

            let mut clients = clients_from_env(&manifest)?;
            if let Some(id_env) = client_id_env.as_deref() {
                let desired = &manifest.providers[0];
                let needs_secret = desired["provider_type"] != "device_code";
                let secret = match (client_secret_env.as_deref(), needs_secret) {
                    (Some(name), _) => Some(env_secret(name)?),
                    (None, true) => bail!("--client-id-env needs --client-secret-env"),
                    (None, false) => None,
                };
                clients.insert(text(desired, "slug"), (env_secret(id_env)?, secret));
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
            // Managed (admin-mode) providers need their client before they
            // can be created.
            for row in plan
                .iter()
                .filter(|row| row.kind == "provider" && row.note.is_some())
            {
                if skip_credentials || !interactive {
                    bail!("{}: {}", row.slug, row.note.as_deref().unwrap_or_default());
                }
                let desired = manifest
                    .providers
                    .iter()
                    .find(|provider| provider["slug"] == row.slug.as_str())
                    .ok_or_else(|| anyhow!("Provider {} is not in the manifest", row.slug))?;
                let needs_secret = desired["provider_type"] != "device_code";
                let client = read_client(&row.slug, needs_secret, &redirect, None, None)?;
                clients.insert(row.slug.clone(), client);
            }
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
                    if clients.contains_key(&slug) {
                        continue;
                    }
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
                    eprintln!(
                        "Test {} as a user: nyxid service add {} --oauth",
                        row.service, row.service
                    );
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
        }
        for service in &services {
            require_slug(service, "service")?;
            reject_fields(
                service,
                &["credential", "provider_config_id"],
                "manifest service",
            )?;
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
) -> Result<BTreeMap<String, String>> {
    let declared = match declared {
        Value::Null => Map::new(),
        Value::Object(map) => map.clone(),
        _ => bail!("`variables` must be a map of name to {{ prompt, default, allowed }}"),
    };
    if let Some(unknown) = provided.keys().find(|name| !declared.contains_key(*name)) {
        bail!("--set {unknown}: the manifest declares no such variable");
    }
    let mut values = BTreeMap::new();
    for (name, spec) in &declared {
        let default = spec.get("default").and_then(Value::as_str);
        let allowed: Vec<&str> = spec
            .get("allowed")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let value = match provided.get(name) {
            Some(value) => value.clone(),
            None if interactive => {
                let label = spec.get("prompt").and_then(Value::as_str).unwrap_or(name);
                let label = if allowed.is_empty() {
                    label.to_string()
                } else {
                    format!("{label} ({})", allowed.join(", "))
                };
                ask(&label, default)?
            }
            None => default
                .map(str::to_string)
                .ok_or_else(|| anyhow!("Set the manifest variable with --set {name}=<value>"))?,
        };
        if !allowed.is_empty() && !allowed.contains(&value.as_str()) {
            bail!("{name} must be one of: {}", allowed.join(", "));
        }
        if value.is_empty()
            || !value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            bail!("{name} may contain only letters, digits, '.', '-' and '_'");
        }
        values.insert(name.clone(), value);
    }
    Ok(values)
}

fn substitute(value: &mut Value, values: &BTreeMap<String, String>) -> Result<()> {
    match value {
        Value::String(text) => {
            for (name, replacement) in values {
                *text = text.replace(&format!("${{{name}}}"), replacement);
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
    let mut rows = Vec::new();
    let mut provider_ids: BTreeMap<String, Option<String>> = BTreeMap::new();
    let existing_providers = list_providers(api).await?;

    for desired in &manifest.providers {
        let slug = text(desired, "slug");
        let current = existing_providers
            .iter()
            .find(|provider| provider["slug"] == desired["slug"]);
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
                        note: missing.then(hint),
                        ..ApplyRow::new("provider", &slug, "create")
                    },
                    (false, true) => ApplyRow::failed("provider", &slug, hint()),
                    (false, false) => {
                        let mut body = desired.clone();
                        if let Some((id, secret)) = client {
                            body["client_id"] = id.clone().into();
                            if let Some(secret) = secret {
                                body["client_secret"] = secret.clone().into();
                            }
                        }
                        match api.post::<Value, _>("/providers", &body).await {
                            Ok(created) => ApplyRow {
                                id: created["id"].as_str().map(str::to_string),
                                ..ApplyRow::new("provider", &slug, "created")
                            },
                            Err(error) => ApplyRow::failed("provider", &slug, error),
                        }
                    }
                }
            }
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
        let usable = row.action != "failed";
        rows.push(row);
        if usable {
            provider_ids.insert(slug, id);
        }
    }

    let existing_services = list_services(api).await?;
    for desired in &manifest.services {
        let slug = text(desired, "slug");
        let provider_ref = desired.get("provider").and_then(Value::as_str);
        let provider_id = match provider_ref {
            None => None,
            Some(reference) => match provider_ids.get(reference) {
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
                None => match existing_providers
                    .iter()
                    .find(|provider| provider["slug"] == reference || provider["id"] == reference)
                {
                    Some(provider) => Some(text(provider, "id")),
                    None => {
                        rows.push(ApplyRow::failed(
                            "service",
                            &slug,
                            format!("provider {reference} not found"),
                        ));
                        continue;
                    }
                },
            },
        };
        let mut body = desired.clone();
        if let Some(object) = body.as_object_mut() {
            object.remove("provider");
            if let Some(id) = &provider_id {
                object.insert("provider_config_id".into(), id.clone().into());
            }
        }
        let current = existing_services
            .iter()
            .find(|service| service["slug"] == desired["slug"]);
        let row = match current {
            None if dry_run => ApplyRow::new("service", &slug, "create"),
            None => match api.post::<Value, _>("/services", &body).await {
                Ok(created) => ApplyRow {
                    id: created["id"].as_str().map(str::to_string),
                    ..ApplyRow::new("service", &slug, "created")
                },
                Err(error) => ApplyRow::failed("service", &slug, error),
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
        rows.push(row);
    }
    Ok(rows)
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
    let mut skip = vec!["slug", "provider_config_id"];
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
            ..ApplyRow::new("service", slug, "unchanged")
        };
    }
    if !dry_run {
        if !changed.is_empty()
            && let Err(error) = api
                .put::<Value, _>(&format!("/services/{id}"), &pick(body, &changed))
                .await
        {
            return ApplyRow::failed("service", slug, error);
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
        ..ApplyRow::new("service", slug, if dry_run { "update" } else { "updated" })
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
                    Some(existing) => existing != *value,
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
    let failed = rows.iter().filter(|row| row.action == "failed").count();
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
            table.set_header(["Kind", "Slug", "Action", "Details"]);
            for row in rows {
                let details = row
                    .error
                    .clone()
                    .or_else(|| row.note.clone())
                    .unwrap_or_else(|| row.changed.join(", "));
                table.add_row([row.kind, row.slug.as_str(), row.action, details.as_str()]);
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
}

async fn validate(
    api: &mut ApiClient,
    manifest: &Manifest,
    redirect: &str,
) -> Result<Vec<ValidationRow>> {
    let mut report = Vec::new();
    for desired in &manifest.services {
        let slug = text(desired, "slug");
        let mut failures = Vec::new();
        let mut pending = Vec::new();
        let mut notes = Vec::new();
        let service = find_service(api, &slug).await?;
        let provider_ref = desired
            .get("provider")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                service
                    .as_ref()
                    .and_then(|row| row["provider_config_id"].as_str().map(str::to_string))
            });
        let provider = match &provider_ref {
            Some(reference) => find_provider(api, reference).await?,
            None => None,
        };
        let desired_provider = provider_ref.as_ref().and_then(|reference| {
            manifest
                .providers
                .iter()
                .find(|row| row["slug"] == reference.as_str())
        });

        match &service {
            None => failures.push("service not found".to_string()),
            Some(service) => {
                if service["is_active"] != true {
                    failures.push("service is inactive".to_string());
                }
                if let Some(url) = desired.get("base_url")
                    && &service["base_url"] != url
                {
                    failures.push(format!(
                        "base_url is {} (manifest: {})",
                        service["base_url"], url
                    ));
                }
                if let Some(provider) = &provider
                    && service["provider_config_id"] != provider["id"]
                {
                    failures.push(format!(
                        "service is not linked to provider {}",
                        text(provider, "slug")
                    ));
                }
            }
        }
        match (&provider_ref, &provider) {
            (Some(reference), None) => failures.push(format!("provider {reference} not found")),
            (_, Some(provider)) => {
                if provider["is_active"] != true {
                    failures.push("provider is inactive".to_string());
                }
                if let Some(desired) = desired_provider {
                    for field in [
                        "authorization_url",
                        "token_url",
                        "default_scopes",
                        "extra_auth_params",
                        "token_endpoint_auth_method",
                        "supports_pkce",
                    ] {
                        if let Some(value) = desired.get(field)
                            && &provider[field] != value
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
                    notes.push(
                        "credential_mode is user: each user brings their own OAuth app".to_string(),
                    );
                } else if oauth && !client_configured(provider) {
                    pending.push(format!(
                        "OAuth client not set: nyxid admin provider set-credentials {}",
                        text(provider, "slug")
                    ));
                } else if oauth && provider["has_oauth_config"] != true {
                    failures.push("provider OAuth configuration is incomplete".to_string());
                }
            }
            (None, None) => {}
        }

        let status = if !failures.is_empty() {
            "failed"
        } else if !pending.is_empty() {
            "pending_credentials"
        } else {
            "ready"
        };
        let scopes: Vec<String> = provider
            .as_ref()
            .and_then(|row| row["default_scopes"].as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let provider_slug = provider.as_ref().map(|row| text(row, "slug"));
        let mut issues = failures;
        issues.extend(pending);
        issues.extend(notes);
        let remarks = format!(
            "NyxID service {slug}; scopes: {}; managed OAuth client: {}; validation: {status}",
            if scopes.is_empty() {
                "none".to_string()
            } else {
                scopes.join(" ")
            },
            provider.as_ref().map_or("n/a", client_status),
        );
        report.push(ValidationRow {
            connect_command: format!("nyxid service add {slug} --oauth"),
            lark: json!({
                "platform": provider_slug.as_ref().map(|slug| format!("platform-{slug}")),
                "type": "Platform services",
                "remarks": remarks,
            }),
            service: slug,
            provider: provider_slug,
            status,
            issues,
            scopes,
            redirect_uri: redirect.to_string(),
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
            table.set_header(["Service", "Provider", "Status", "Issues"]);
            for row in report {
                let issues = row.issues.join("; ");
                table.add_row([
                    row.service.as_str(),
                    row.provider.as_deref().unwrap_or("-"),
                    row.status,
                    issues.as_str(),
                ]);
            }
            eprintln!("{table}");
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
    let (client_id, client_secret) = read_client(
        &slug,
        needs_secret,
        redirect,
        client_id_env,
        client_secret_env,
    )?;
    let mut body = json!({ "client_id": client_id });
    if let Some(secret) = client_secret {
        body["client_secret"] = secret.into();
    }
    let updated: Value = api
        .put(&format!("/providers/{}", text(provider, "id")), &body)
        .await?;
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
type Clients = BTreeMap<String, (String, Option<String>)>;

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
    let mut clients = Clients::new();
    for desired in manifest
        .providers
        .iter()
        .filter(|desired| client_required_at_creation(desired))
    {
        let slug = text(desired, "slug");
        let (id_env, secret_env) = client_env_names(&slug);
        if std::env::var_os(&id_env).is_none() {
            continue;
        }
        let secret = if desired["provider_type"] == "device_code" {
            None
        } else {
            Some(env_secret(&secret_env)?)
        };
        clients.insert(slug, (env_secret(&id_env)?, secret));
    }
    Ok(clients)
}

/// Read a client ID and (unless the provider is device-code) a secret from
/// environment variables, or ask: the ID visibly, the secret hidden.
fn read_client(
    slug: &str,
    needs_secret: bool,
    redirect: &str,
    client_id_env: Option<&str>,
    client_secret_env: Option<&str>,
) -> Result<(String, Option<String>)> {
    eprintln!("{slug}: register this redirect URI in the vendor's OAuth app: {redirect}");
    let client_id = match client_id_env {
        Some(name) => env_secret(name)?,
        None => ask_required("Client ID", None)?,
    };
    let client_secret = match (client_secret_env, needs_secret) {
        (Some(name), _) => Some(env_secret(name)?),
        (None, true) => Some(
            rpassword::prompt_password("Client secret (hidden): ")
                .context("Failed to read the client secret")?,
        ),
        (None, false) => None,
    };
    if client_secret
        .as_deref()
        .is_some_and(|secret| secret.trim().is_empty())
    {
        bail!("The client secret is empty");
    }
    Ok((
        client_id.trim().to_string(),
        client_secret.map(|secret| secret.trim().to_string()),
    ))
}

fn env_secret(name: &str) -> Result<String> {
    let value =
        std::env::var(name).with_context(|| format!("Environment variable {name} is not set"))?;
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
    let provider = provider_wizard(None)?;
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
    }
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
    let listing: Value = api.get("/providers").await?;
    Ok(listing["providers"].as_array().cloned().unwrap_or_default())
}

async fn list_services(api: &mut ApiClient) -> Result<Vec<Value>> {
    let listing: Value = api.get("/services").await?;
    Ok(listing["services"].as_array().cloned().unwrap_or_default())
}

async fn find_provider(api: &mut ApiClient, reference: &str) -> Result<Option<Value>> {
    if uuid::Uuid::parse_str(reference).is_ok() {
        return api.get_optional(&format!("/providers/{reference}")).await;
    }
    Ok(list_providers(api)
        .await?
        .into_iter()
        .find(|provider| provider["slug"] == reference))
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
    Ok(list_services(api)
        .await?
        .into_iter()
        .find(|service| service["slug"] == reference))
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
            eprintln!("Authorize URL: {}", text(provider, "authorization_url"));
            eprintln!("Token URL:     {}", text(provider, "token_url"));
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
        let mut row = manifest.services[0].clone();
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

    #[tokio::test]
    async fn apply_creates_provider_then_links_the_new_service_to_it() {
        let server = MockServer::start().await;
        listing(&server, json!([]), json!([])).await;
        let manifest = zoho(&[]).unwrap();
        let mut provider_body = manifest.providers[0].clone();
        provider_body["client_id"] = "zoho-id".into();
        provider_body["client_secret"] = "zoho-secret".into();
        Mock::given(method("POST"))
            .and(path("/api/v1/providers"))
            .and(body_json(&provider_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(provider_row(true)))
            .expect(1)
            .mount(&server)
            .await;
        let mut service = manifest.services[0].clone();
        service.as_object_mut().unwrap().remove("provider");
        service["provider_config_id"] = provider_row(false)["id"].clone();
        Mock::given(method("POST"))
            .and(path("/api/v1/services"))
            .and(body_json(&service))
            .respond_with(ResponseTemplate::new(200).set_body_json(service_row()))
            .expect(1)
            .mount(&server)
            .await;

        let mut api = ApiClient::from_auth(&mock_auth(server.uri())).unwrap();
        // Without a client the managed provider is planned with a note and
        // refused on a real run.
        let plan = apply(&mut api, &manifest, true, &Clients::new())
            .await
            .unwrap();
        assert_eq!(
            plan.iter().map(|row| row.action).collect::<Vec<_>>(),
            ["create", "create"]
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

        let clients = Clients::from([(
            "zoho-mail".to_string(),
            ("zoho-id".to_string(), Some("zoho-secret".to_string())),
        )]);
        let rows = apply(&mut api, &manifest, false, &clients).await.unwrap();
        assert_eq!(
            rows.iter().map(|row| row.action).collect::<Vec<_>>(),
            ["created", "created"]
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
        assert!(clients_from_env(&manifest).unwrap().is_empty());
        unsafe {
            std::env::set_var("ZOHO_MAIL_CLIENT_ID", "id-from-env");
        }
        let missing_secret = clients_from_env(&manifest);
        unsafe {
            std::env::set_var("ZOHO_MAIL_CLIENT_SECRET", "secret-from-env");
        }
        let clients = clients_from_env(&manifest);
        unsafe {
            std::env::remove_var("ZOHO_MAIL_CLIENT_ID");
            std::env::remove_var("ZOHO_MAIL_CLIENT_SECRET");
        }
        assert!(missing_secret.is_err());
        assert_eq!(
            clients.unwrap()["zoho-mail"],
            (
                "id-from-env".to_string(),
                Some("secret-from-env".to_string())
            )
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
        assert_eq!(rows[1].action, "updated");
        assert_eq!(rows[1].changed, ["description"]);
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
        assert_eq!(plan[1].action, "update");
        assert_eq!(plan[1].changed, ["provider"]);
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
}
