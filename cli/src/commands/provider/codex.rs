use std::{
    io::{Read, Write},
    path::Path,
};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    api::ApiClient,
    cli::{CodexConnectArgs, OutputFormat},
};

const ENDPOINT: &str = "/providers/codex-connection";
const MAX_SOURCE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Deserialize, Serialize)]
struct ConnectionVersion {
    id: uuid::Uuid,
    state_version: i64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ConnectionState {
    NotConnected,
    Saved,
    Usable,
    ReconnectRequired,
}

#[derive(Deserialize, Serialize)]
struct Connection {
    account_id: uuid::Uuid,
    account_email: String,
    provider_id: uuid::Uuid,
    provider_slug: String,
    connection: Option<ConnectionVersion>,
    status: ConnectionState,
    service_id: Option<uuid::Uuid>,
}

pub async fn run(args: CodexConnectArgs) -> Result<()> {
    if args.skip {
        return report(&args, "skipped", None, None);
    }
    let mut api = ApiClient::from_auth_checked(&args.auth)
        .await?
        .for_credential_transfer()?;
    let instance = normalize_instance(api.base_url_root())?;
    let before: Connection = api.get(ENDPOINT).await.map_err(|_| anyhow!("Cannot inspect the connection. Sign in to the selected NyxID instance with a first-party account, then retry."))?;
    if before.provider_slug != "openai" {
        bail!("The selected instance does not offer a compatible OpenAI API-key connection");
    }
    if args.status {
        return report(&args, "status", Some(&instance), Some(&before));
    }
    if args.verify {
        return verify(&args, &mut api, &instance, before).await;
    }
    let approved = match (&args.approve_instance, &args.approve_account) {
        (Some(destination), Some(account)) => {
            normalize_instance(destination)? == instance
                && uuid::Uuid::parse_str(account).ok() == Some(before.account_id)
        }
        _ => false,
    };
    if !approved {
        return report(&args, "consent_required", Some(&instance), Some(&before));
    }
    if let Some(existing) = &before.connection {
        if args
            .replace_connection
            .as_deref()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            != Some(existing.id)
            || args.replace_version != Some(existing.state_version)
        {
            return report(
                &args,
                "replacement_consent_required",
                Some(&instance),
                Some(&before),
            );
        }
    } else if args.replace_connection.is_some() {
        bail!(
            "The approved replacement no longer exists. Review the destination and account again."
        );
    }
    let home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .ok_or_else(|| {
            anyhow!("Cannot locate Codex configuration; use separate authorization in AI Services")
        })?;
    let key = match read_api_key(&home) {
        Ok(key) => key,
        Err(reason) => return report(&args, reason, Some(&instance), Some(&before)),
    };
    #[derive(Serialize)]
    struct Transfer<'a> {
        account_id: uuid::Uuid,
        api_key: &'a str,
        expected_connection: &'a Option<ConnectionVersion>,
    }
    let mut saved: Connection = api.post(ENDPOINT, &Transfer {account_id:before.account_id,api_key:&key,expected_connection:&before.connection})
        .await.map_err(|_| anyhow!("Credential upload did not complete. No response body was displayed. Check connection status before retrying; confirm replacement again if it was saved."))?;
    drop(key);
    if saved.account_id != before.account_id
        || saved.provider_id != before.provider_id
        || saved.provider_slug != "openai"
    {
        bail!(
            "Unexpected connection response. Review the selected account's AI Services before retrying."
        );
    }
    saved.account_email = before.account_email;
    verify(&args, &mut api, &instance, saved).await
}

async fn verify(
    args: &CodexConnectArgs,
    api: &mut ApiClient,
    instance: &str,
    saved: Connection,
) -> Result<()> {
    let Some(version) = saved.connection.as_ref() else {
        return report(args, "not_connected", Some(instance), Some(&saved));
    };
    #[derive(Serialize)]
    struct Verify<'a> {
        connection: &'a ConnectionVersion,
        model: &'a str,
    }
    let verified = api
        .post::<Connection, _>(
            &format!("{ENDPOINT}/verify"),
            &Verify {
                connection: version,
                model: &args.verification_model,
            },
        )
        .await;
    match verified {
        Ok(mut value)
            if value.account_id == saved.account_id
                && value.provider_id == saved.provider_id
                && value.provider_slug == "openai"
                && value.connection.as_ref().is_some_and(|v| {
                    v.id == version.id && v.state_version == version.state_version
                }) =>
        {
            value.account_email = saved.account_email;
            report(args, "verification_finished", Some(instance), Some(&value))
        }
        _ => {
            let saved = Connection {
                status: ConnectionState::Saved,
                ..saved
            };
            report(
                args,
                "saved_verification_pending",
                Some(instance),
                Some(&saved),
            )
        }
    }
}

fn normalize_instance(raw: &str) -> Result<String> {
    let url = url::Url::parse(raw).map_err(|_| anyhow!("Invalid NyxID instance URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Use a NyxID instance URL without credentials, query parameters, or a fragment");
    }
    Ok(url.as_str().trim_end_matches('/').into())
}

fn read_source(path: &Path) -> std::result::Result<Zeroizing<String>, &'static str> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "source_unavailable")?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES {
        return Err("source_unsupported");
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| "source_unavailable")?;
    let mut content = Zeroizing::new(String::new());
    file.take(MAX_SOURCE_BYTES + 1)
        .read_to_string(&mut content)
        .map_err(|_| "source_unsupported")?;
    if content.len() as u64 > MAX_SOURCE_BYTES {
        return Err("source_unsupported");
    }
    Ok(content)
}

fn read_api_key(home: &Path) -> std::result::Result<Zeroizing<String>, &'static str> {
    #[derive(Deserialize)]
    struct Config {
        cli_auth_credentials_store: Option<String>,
    }
    let path = home.join("config.toml");
    let config_exists = match std::fs::symlink_metadata(&path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err("source_unavailable"),
    };
    if config_exists {
        let content = read_source(&path)?;
        let config: Config = toml::from_str(&content).map_err(|_| "source_unsupported")?;
        match config
            .cli_auth_credentials_store
            .as_deref()
            .unwrap_or("file")
        {
            "file" => {}
            "keyring" | "auto" => return Err("separate_authorization_required"),
            _ => return Err("source_unsupported"),
        }
    }
    #[derive(Deserialize)]
    struct AuthFile<'a> {
        #[serde(rename = "OPENAI_API_KEY", borrow)]
        api_key: Option<&'a str>,
        #[serde(borrow)]
        auth_mode: Option<&'a str>,
        tokens: Option<serde::de::IgnoredAny>,
    }
    let content = read_source(&home.join("auth.json"))?;
    let auth: AuthFile<'_> = serde_json::from_str(&content).map_err(|_| "source_unsupported")?;
    if auth.tokens.is_some() || auth.auth_mode.is_some_and(|mode| mode != "apikey") {
        return Err("separate_authorization_required");
    }
    let key = auth
        .api_key
        .filter(|key| {
            key.starts_with("sk-")
                && (8..=4096).contains(&key.len())
                && !key.chars().any(char::is_whitespace)
        })
        .ok_or("source_unsupported")?;
    Ok(Zeroizing::new(key.to_string()))
}

fn report(
    args: &CodexConnectArgs,
    outcome: &str,
    instance: Option<&str>,
    connection: Option<&Connection>,
) -> Result<()> {
    let message = match outcome {
        "skipped" => "Credential connection skipped. NyxID remains installed.",
        "consent_required" => {
            "Confirm this instance and account before reading or uploading Codex credentials. API keys enable paid OpenAI API access; connection verification makes a small billed Responses request. Use --approve-instance and --approve-account only after explicit approval."
        }
        "replacement_consent_required" => {
            "A connection already exists. Confirm --replace-connection and --replace-version with its reviewed ID and version before replacing it."
        }
        "source_unavailable" => {
            "Codex file credentials are missing or inaccessible. Retry after fixing file access, authorize a separate provider in AI Services, or skip."
        }
        "source_unsupported" => {
            "This credential source cannot be imported safely. Use separate provider authorization in AI Services or skip."
        }
        "separate_authorization_required" => {
            "Use separate provider authorization in AI Services. ChatGPT OAuth refresh rotation cannot be shared with local Codex; keyring and automatic storage are not imported. Local Codex remains unchanged."
        }
        "saved_verification_pending" => {
            "Credential saved; AI access is not verified. Retry with --verify or reconnect in AI Services."
        }
        _ => {
            "Connection status checked. Only usable confirms a completed request with this saved credential."
        }
    };
    // Account and destination details belong in consent output, never diagnostics.
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    match args.auth.output {
        OutputFormat::Json => writeln!(
            output,
            "{}",
            serde_json::to_string(
                &serde_json::json!({"outcome":outcome,"instance":instance,"connection":connection,"message":message})
            )?
        )?,
        OutputFormat::Table => {
            writeln!(output, "{message}")?;
            if let Some(instance) = instance {
                writeln!(output, "Instance: {instance}")?;
            }
            if let Some(connection) = connection {
                writeln!(
                    output,
                    "Account: {} ({})",
                    connection.account_email, connection.account_id
                )?;
                writeln!(
                    output,
                    "State: {}",
                    serde_json::to_value(&connection.status)?
                        .as_str()
                        .unwrap_or("saved")
                )?;
                if let Some(version) = &connection.connection {
                    writeln!(
                        output,
                        "Connection: {} (version {})",
                        version.id, version.state_version
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
