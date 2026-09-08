use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::ApiClient;

pub const TOKEN_FILE: &str = "token";
const AUTH_KIND_FILE: &str = "auth_kind";
const METADATA_FILE: &str = "agent_key.json";
pub const REJECTED_MESSAGE: &str = "Your Agent Key credential was rejected (revoked, expired, or the key was deleted). Run `nyxid login --agent-key` to authorize again.";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KeyMetadata {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub owner_type: String,
    pub owner_id: String,
    pub owner_name: String,
    pub scopes: String,
    pub allow_all_services: bool,
    pub allow_all_nodes: bool,
    pub allowed_service_ids: Vec<String>,
    pub allowed_node_ids: Vec<String>,
    pub expires_at: Option<String>,
    pub rate_limit_per_second: Option<u32>,
    pub rate_limit_burst: Option<u32>,
    pub platform: Option<String>,
    #[serde(default)]
    pub created_now: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Identity {
    #[serde(rename = "api_key")]
    pub key: KeyMetadata,
    #[serde(rename = "credential_id")]
    pub login_id: String,
    #[serde(rename = "credential_expires_at")]
    pub login_expires_at: Option<String>,
    pub label: String,
}

#[derive(Deserialize)]
pub(super) struct Delivery {
    pub(super) credential: String,
    #[serde(flatten)]
    pub(super) identity: Identity,
}

impl std::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentKeyDelivery").finish_non_exhaustive()
    }
}

pub fn is_agent_key_profile(profile: Option<&str>) -> bool {
    super::token_dir_for_profile(profile)
        .ok()
        .and_then(|dir| std::fs::read_to_string(dir.join(AUTH_KIND_FILE)).ok())
        .is_some_and(|value| value.trim() == "agent_key")
}

pub fn read_metadata(profile: Option<&str>) -> Option<Identity> {
    let path = super::token_dir_for_profile(profile)
        .ok()?
        .join(METADATA_FILE);
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn remove_if_present(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Failed to remove {}", path.display())),
    }
}

pub(super) fn clear_agent_files(profile: Option<&str>) -> Result<()> {
    let dir = super::token_dir_for_profile(profile)?;
    // Clear the mode marker last, after the restricted credential is gone.
    for name in [TOKEN_FILE, METADATA_FILE, AUTH_KIND_FILE] {
        remove_if_present(&dir.join(name))?;
    }
    Ok(())
}

pub(super) fn save(
    profile: Option<&str>,
    base_url: &str,
    credential: &str,
    identity: &Identity,
) -> Result<()> {
    super::replace_login_files(profile, || {
        let dir = super::token_dir_for_profile(profile)?;
        // Persist the mode first. Even an interrupted write must never expose a
        // stale account token as the fallback identity for this profile.
        super::write_token_file(&dir.join(AUTH_KIND_FILE), "agent_key")?;
        for name in [
            super::TOKEN_FILE_NAME,
            super::REFRESH_TOKEN_FILE_NAME,
            super::USER_ID_FILE_NAME,
        ] {
            remove_if_present(&dir.join(name))?;
        }
        super::write_token_file(
            &dir.join(METADATA_FILE),
            &serde_json::to_string_pretty(identity)?,
        )?;
        super::save_base_url_for(profile, base_url)?;
        super::write_token_file(&dir.join(TOKEN_FILE), credential)
    })
}

pub fn format_identity(identity: &Identity) -> String {
    let key = &identity.key;
    format!(
        "Authentication: Agent Key\nKey: {} ({})\nOwner: {} ({})\nScopes: {}\nServices: {} (allow all: {})\nNodes: {} (allow all: {})\nKey expiry: {}\nCredential expiry: {}\nCredential label: {}\nRate limit: {} requests/s; burst: {}",
        key.name,
        key.key_prefix,
        key.owner_name,
        key.owner_type,
        key.scopes,
        key.allowed_service_ids.len(),
        key.allow_all_services,
        key.allowed_node_ids.len(),
        key.allow_all_nodes,
        key.expires_at.as_deref().unwrap_or("None"),
        identity.login_expires_at.as_deref().unwrap_or("None"),
        identity.label,
        key.rate_limit_per_second
            .map_or_else(|| "Default".into(), |v| v.to_string()),
        key.rate_limit_burst
            .map_or_else(|| "Default".into(), |v| v.to_string())
    )
}

pub async fn show_identity(api: &mut ApiClient, output: crate::cli::OutputFormat) -> Result<()> {
    let identity: Identity = api.get("/auth/agent-key/self").await?;
    match output {
        crate::cli::OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"auth": {"kind": "agent_key", "api_key": identity.key, "credential_id": identity.login_id, "credential_expires_at": identity.login_expires_at, "label": identity.label}})
            )?
        ),
        crate::cli::OutputFormat::Table => eprintln!("{}", format_identity(&identity)),
    }
    Ok(())
}
