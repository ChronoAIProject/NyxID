//! Human-only provider credential administration. Secret inputs never reach output.
use super::support;
use crate::{
    api::{ApiClient, ApiError},
    cli::AuthArgs,
};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::PathBuf};
use zeroize::Zeroizing;
const ROOT: &str = "/admin/platform-models/credentials";
const OFFERINGS: &str = "/admin/platform-models/offerings";
const MAX_KEY: u64 = 64 * 1024;
#[derive(Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    GcpServiceAccount,
    Openai,
    Anthropic,
}
#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct SecretInput {
    /// Private JSON file; group/world-readable files are refused
    #[arg(long)]
    file: Option<PathBuf>,
    /// Environment variable containing the API key (never pass the key itself)
    #[arg(long)]
    key_env: Option<String>,
}
#[derive(Subcommand)]
pub enum CredentialCommands {
    /// List credential metadata and current route references; never secrets
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
    Add {
        #[arg(long, value_enum)]
        kind: Kind,
        #[arg(long)]
        name: String,
        #[command(flatten)]
        source: SecretInput,
        #[command(flatten)]
        auth: AuthArgs,
    },
    Rotate {
        id: String,
        #[command(flatten)]
        source: SecretInput,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Refused while any enabled route still references the credential
    Disable {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
#[derive(Subcommand)]
pub enum OfferingCommands {
    /// Create an empty private model billing identity
    Create {
        #[arg(long)]
        name: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
#[derive(Deserialize, Serialize)]
struct Reference {
    model_id: String,
    public_id: String,
    display_name: String,
    route_id: Option<String>,
    enabled: bool,
}
#[derive(Deserialize, Serialize)]
struct Record {
    id: String,
    name: String,
    kind: Option<String>,
    enabled: bool,
    created_at: String,
    updated_at: String,
    references: Vec<Reference>,
}
#[derive(Deserialize, Serialize)]
struct Page {
    records: Vec<Record>,
    next_after: Option<String>,
}
fn read_secret(source: &SecretInput, kind: Option<&str>) -> Result<Zeroizing<String>> {
    if kind == Some("gcp-service-account") && source.file.is_none() {
        bail!("Google Cloud service accounts require --file <key.json>");
    }
    if matches!(kind, Some("openai" | "anthropic")) && source.key_env.is_none() {
        bail!("Provider API keys require --key-env <ENV>");
    }
    let raw = if let Some(path) = &source.file {
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            bail!("Choose a .json service-account key file");
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(path).map_err(|_| {
            anyhow::anyhow!("Cannot open credential file; use a regular private JSON file")
        })?;
        let metadata = file
            .metadata()
            .context("Cannot inspect credential file permissions")?;
        if !metadata.is_file() || metadata.len() > MAX_KEY {
            bail!("Credential file must be a regular file of at most 64 KiB");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                bail!(
                    "Credential file is group/world-readable or writable; run chmod 600 on it before retrying"
                );
            }
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_KEY as usize + 1));
        file.take(MAX_KEY + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow::anyhow!("Cannot read credential file"))?;
        if bytes.len() as u64 > MAX_KEY {
            bail!("Credential exceeds 64 KiB");
        }
        Zeroizing::new(
            std::str::from_utf8(&bytes)
                .map_err(|_| anyhow::anyhow!("Credential file must be UTF-8 JSON"))?
                .to_owned(),
        )
    } else {
        Zeroizing::new(
            std::env::var(
                source
                    .key_env
                    .as_deref()
                    .context("Supply --file or --key-env")?,
            )
            .map_err(|_| {
                anyhow::anyhow!("Credential environment variable is missing or not UTF-8")
            })?,
        )
    };
    if raw.trim().is_empty() || raw.len() as u64 > MAX_KEY {
        bail!("Credential must contain between 1 byte and 64 KiB");
    }
    // Authoritative closed JSON / RSA validation runs server-side, without token minting.
    Ok(raw)
}
fn safe_error(error: anyhow::Error) -> anyhow::Error {
    if let Some(api) = error.downcast_ref::<ApiError>() {
        ApiError::new(ROOT, api.status(),
            "Credential save refused or unconfirmed. Check input, admin permissions and the Platform Models flag, disable referencing routes if needed, and reload before retrying; no automatic retry was sent.".into()).into()
    } else {
        anyhow::anyhow!(
            "Credential save was not confirmed. Reload the inventory before retrying; no automatic retry was sent."
        )
    }
}
fn label(kind: Option<&str>) -> &str {
    match kind {
        Some("gcp-service-account") => "Google Cloud service account (Vertex AI)",
        Some("openai") => "OpenAI API key",
        Some("anthropic") => "Anthropic API key",
        _ => "Provider credential",
    }
}
fn render(records: &[Record]) -> String {
    let value = serde_json::to_value(records).unwrap_or_default();
    support::table(
        &[
            "ID",
            "Name",
            "Kind",
            "Enabled",
            "Created",
            "Updated",
            "References",
        ],
        value.as_array().into_iter().flatten().map(|row| {
            vec![
                support::text(&row["id"]),
                support::text(&row["name"]),
                label(row["kind"].as_str()).into(),
                support::text(&row["enabled"]),
                support::text(&row["created_at"]),
                support::text(&row["updated_at"]),
                row["references"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| {
                        format!(
                            "{} / {}{}",
                            support::text(&r["public_id"]),
                            support::text(&r["route_id"]),
                            if r["enabled"] == true {
                                " (enabled)"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            ]
        }),
    )
}
async fn current(api: &mut ApiClient, id: &str) -> Result<(String, Record)> {
    let path = format!("{ROOT}/{}", support::uuid(id)?);
    let record = api.get(&path).await?;
    Ok((path, record))
}
pub async fn run(command: CredentialCommands) -> Result<()> {
    let (records, auth) = match command {
        CredentialCommands::List { auth } => {
            let mut api = support::human_api(&auth).await?;
            let mut records = Vec::new();
            let mut after = None;
            let mut seen = std::collections::HashSet::new();
            loop {
                let path = after.as_ref().map_or_else(
                    || ROOT.to_string(),
                    |after: &String| format!("{ROOT}?after={}", urlencoding::encode(after)),
                );
                let page: Page = api.get(&path).await?;
                records.extend(page.records);
                let Some(next) = page.next_after else { break };
                if !seen.insert(next.clone()) {
                    bail!("Credential inventory returned a repeated cursor");
                }
                after = Some(next);
            }
            (records, auth)
        }
        CredentialCommands::Add {
            kind,
            name,
            source,
            auth,
        } => {
            let mut api = support::human_api(&auth).await?;
            let kind_name = serde_json::to_value(kind)?.as_str().unwrap().to_string();
            let credential = read_secret(&source, Some(&kind_name))?;
            #[derive(Serialize)]
            struct Body<'a> {
                name: &'a str,
                kind: Kind,
                credential: &'a str,
            }
            let row = api
                .post::<Record, _>(
                    ROOT,
                    &Body {
                        name: &name,
                        kind,
                        credential: &credential,
                    },
                )
                .await
                .map_err(safe_error)?;
            (vec![row], auth)
        }
        CredentialCommands::Rotate { id, source, auth } => {
            let mut api = support::human_api(&auth).await?;
            let (path, row) = current(&mut api, &id).await?;
            let credential = read_secret(&source, row.kind.as_deref())?;
            #[derive(Serialize)]
            struct Body<'a> {
                expected_updated_at: &'a str,
                credential: &'a str,
            }
            let row = api
                .put::<Record, _>(
                    &path,
                    &Body {
                        expected_updated_at: &row.updated_at,
                        credential: &credential,
                    },
                )
                .await
                .map_err(safe_error)?;
            (vec![row], auth)
        }
        CredentialCommands::Disable { id, auth } => {
            let mut api = support::human_api(&auth).await?;
            let (path, row) = current(&mut api, &id).await?;
            if row.references.iter().any(|r| r.enabled) {
                bail!("Disable the enabled routes referencing this credential first");
            }
            let row = api
                .put::<Record, _>(
                    &path,
                    &serde_json::json!({"expected_updated_at":row.updated_at,"enabled":false}),
                )
                .await
                .map_err(safe_error)?;
            (vec![row], auth)
        }
    };
    let value = serde_json::json!({"records":records});
    support::print(&value, auth.output, || render(&records))
}
pub async fn offering(command: OfferingCommands) -> Result<()> {
    let OfferingCommands::Create { name, auth } = command;
    let mut api = support::human_api(&auth).await?;
    let row: Record = api
        .post(OFFERINGS, &serde_json::json!({"name":name}))
        .await?;
    support::print(&serde_json::to_value(&row)?, auth.output, || {
        support::table(
            &["ID", "Billing identity", "Enabled"],
            [vec![row.id, row.name, row.enabled.to_string()]],
        )
    })
}
