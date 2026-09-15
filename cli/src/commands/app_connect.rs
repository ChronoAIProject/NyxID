//! Developer and platform-admin commands for app requirements and hosted branding.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::{Value, json};

use crate::api::ApiClient;
use crate::cli::{
    AdminAppConnectCommands, AppBrandingCommands, AppConnectRolloutCommands, AppConnectRolloutMode,
    AppHandoffCommands, AppRequirementEnforcement, AppRequirementsCommands, DeveloperAppTarget,
    OutputFormat,
};
use crate::org_resolver::resolve_org_id;

async fn developer_target(target: &DeveloperAppTarget) -> Result<(ApiClient, String)> {
    let mut api = ApiClient::from_auth_checked(&target.auth).await?;
    if target.org.is_none() && uuid::Uuid::parse_str(&target.app).is_ok() {
        return Ok((api, target.app.clone()));
    }
    let path = match &target.org {
        Some(org) => {
            let id = resolve_org_id(&mut api, org).await?;
            format!(
                "/developer/oauth-clients?org_id={}",
                urlencoding::encode(&id)
            )
        }
        None => "/developer/oauth-clients".into(),
    };
    let clients: Value = api.get(&path).await?;
    let candidates: Vec<_> = clients["clients"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| {
            c["id"].as_str() == Some(&target.app) || c["client_name"].as_str() == Some(&target.app)
        })
        .filter_map(|c| c["id"].as_str())
        .collect();
    match candidates.as_slice() {
        [id] => Ok((api, (*id).into())),
        [] => {
            bail!("Developer app not found in this owner scope. Pass its client ID or use --org.")
        }
        _ => bail!(
            "App name is ambiguous. Use a client ID: {}",
            candidates.join(", ")
        ),
    }
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file =
        std::fs::File::open(path).with_context(|| format!("Could not open {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("Could not read {}", path.display()))?;
    if bytes.len() as u64 > limit {
        bail!("File exceeds {limit} bytes");
    }
    Ok(bytes)
}

pub async fn run_requirements(command: AppRequirementsCommands) -> Result<()> {
    match command {
        AppRequirementsCommands::List { target } => {
            let (mut api, id) = developer_target(&target).await?;
            let result: Value = api
                .get(&format!(
                    "/developer/oauth-clients/{}/requirements",
                    urlencoding::encode(&id)
                ))
                .await?;
            if matches!(target.auth.output, OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                let mut table = Table::new();
                table.load_preset(UTF8_FULL_CONDENSED);
                table.set_header(["Version", "Enforcement", "Requirements", "Published"]);
                for version in result["versions"].as_array().into_iter().flatten() {
                    table.add_row([
                        display(&version["version"]),
                        display(&version["enforcement"]),
                        version["requirements"]
                            .as_array()
                            .map_or(0, Vec::len)
                            .to_string(),
                        display(&version["published_at"]),
                    ]);
                }
                eprintln!("{table}");
            }
            Ok(())
        }
        AppRequirementsCommands::Publish {
            target,
            file,
            enforcement,
        } => {
            let bytes = read_bounded(&file, 2 * 1024 * 1024)?;
            let mut body: Value =
                serde_json::from_slice(&bytes).context("Manifest must be a JSON object")?;
            let object = body
                .as_object_mut()
                .context("Manifest must be a JSON object")?;
            if let Some(enforcement) = enforcement {
                object.insert(
                    "enforcement".into(),
                    json!(match enforcement {
                        AppRequirementEnforcement::Advise => "advise",
                        AppRequirementEnforcement::Gate => "gate",
                    }),
                );
            }
            let (mut api, id) = developer_target(&target).await?;
            let result: Value = api
                .post(
                    &format!(
                        "/developer/oauth-clients/{}/requirements",
                        urlencoding::encode(&id)
                    ),
                    &body,
                )
                .await?;
            print_response(&result, target.auth.output)
        }
    }
}

pub async fn run_handoff(command: AppHandoffCommands) -> Result<()> {
    let AppHandoffCommands::Set { target, text } = command;
    let (mut api, id) = developer_target(&target).await?;
    let result: Value = api
        .patch(
            &format!(
                "/developer/oauth-clients/{}/handoff",
                urlencoding::encode(&id)
            ),
            &json!({ "handoff_blurb": text }),
        )
        .await?;
    print_response(&result, target.auth.output)
}

pub async fn run_branding(command: AppBrandingCommands) -> Result<()> {
    let (result, output) = match command {
        AppBrandingCommands::Logo { target, file } => {
            let bytes = read_bounded(&file, 256 * 1024)?;
            let extension = file
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let (filename, content_type) = match extension.as_str() {
                "png" => ("logo.png", "image/png"),
                "webp" => ("logo.webp", "image/webp"),
                _ => bail!("Logo file must be PNG or WebP"),
            };
            let (mut api, id) = developer_target(&target).await?;
            let result: Value = api
                .post_file(
                    &format!(
                        "/developer/oauth-clients/{}/branding/logo",
                        urlencoding::encode(&id)
                    ),
                    "logo",
                    filename,
                    content_type,
                    &bytes,
                )
                .await?;
            (result, target.auth.output)
        }
        AppBrandingCommands::Homepage { target, url } => {
            let (mut api, id) = developer_target(&target).await?;
            let result: Value = api
                .patch(
                    &format!("/developer/oauth-clients/{}", urlencoding::encode(&id)),
                    &json!({ "homepage_url": url }),
                )
                .await?;
            (result, target.auth.output)
        }
    };
    print_response(&result, output)
}

pub async fn run_admin(command: AdminAppConnectCommands) -> Result<()> {
    let (result, output) = match command {
        AdminAppConnectCommands::Rollout { command } => match command {
            AppConnectRolloutCommands::Get { auth } => {
                let mut api = ApiClient::from_auth_checked(&auth).await?;
                let result: Value = api.get("/admin/settings/app-connect").await?;
                (result, auth.output)
            }
            AppConnectRolloutCommands::Set { mode, auth } => {
                let mut api = ApiClient::from_auth_checked(&auth).await?;
                let rollout = match mode {
                    AppConnectRolloutMode::Disabled => Some("disabled"),
                    AppConnectRolloutMode::Allowlist => Some("allowlist"),
                    AppConnectRolloutMode::Reset => None,
                };
                let result: Value = api
                    .patch(
                        "/admin/settings/app-connect",
                        &json!({ "rollout": rollout }),
                    )
                    .await?;
                (result, auth.output)
            }
        },
        AdminAppConnectCommands::Capability {
            app,
            enable,
            disable,
            auth,
        } => {
            if enable == disable {
                bail!("Choose exactly one of --enable or --disable");
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api
                .patch(
                    &format!(
                        "/admin/oauth-clients/{}/app-connect-capability",
                        urlencoding::encode(&app)
                    ),
                    &json!({ "enabled": enable }),
                )
                .await?;
            (result, auth.output)
        }
        AdminAppConnectCommands::VerifyBranding {
            app,
            revision,
            unverify,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api
                .post(
                    &format!(
                        "/admin/oauth-clients/{}/branding/verify",
                        urlencoding::encode(&app)
                    ),
                    &json!({ "branding_revision": revision, "verified": !unverify }),
                )
                .await?;
            (result, auth.output)
        }
    };
    print_response(&result, output)
}

fn display(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "-".into(),
        _ => value.to_string(),
    }
}

fn print_response(result: &Value, output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(result)?),
        OutputFormat::Table => {
            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["Field", "Value"]);
            for (key, value) in result
                .as_object()
                .context("Expected an API response object")?
            {
                table.add_row([key.clone(), display(value)]);
            }
            eprintln!("{table}");
        }
    }
    Ok(())
}
