//! Account/server-bound local narrowing. Legacy global positives never grant
//! this permission; env signals can only further restrict an explicit choice.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub base_url: String,
    pub account_id: String,
    pub login_generation: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Choice {
    enabled: bool,
    revision: String,
}

pub fn deployment_base(base: &str) -> Option<String> {
    let url = reqwest::Url::parse(base).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(url.as_str().trim_end_matches('/').to_owned())
}
pub fn saved_identity(profile: Option<&str>) -> Option<Identity> {
    if crate::auth::agent_key::is_agent_key_profile(profile) {
        return None;
    }
    let token = crate::auth::read_saved_token_for(profile)?;
    let account_id = crate::auth::jwt_sub_from_token(&token)?;
    uuid::Uuid::parse_str(&account_id).ok()?;
    Some(Identity {
        base_url: deployment_base(&crate::auth::read_saved_base_url_for(profile)?)?,
        account_id,
        login_generation: crate::auth::login_generation(profile),
    })
}
fn choice_path(profile: Option<&str>, identity: &Identity) -> Option<PathBuf> {
    let mut directory = dirs::home_dir()?.join(".nyxid");
    if let Some(profile) = profile {
        crate::auth::validate_profile_name(profile).ok()?;
        directory = directory.join("profiles").join(profile);
    }
    let digest = hex::encode(Sha256::digest(
        format!("{}\n{}", identity.base_url, identity.account_id).as_bytes(),
    ));
    Some(
        directory
            .join("analytics_choices")
            .join(format!("{digest}.json")),
    )
}
pub fn permitted_revision(profile: Option<&str>, identity: &Identity) -> Option<String> {
    if !super::consent::resolve_consent_preferring_profile(profile).enabled
        || saved_identity(profile).as_ref() != Some(identity)
    {
        return None;
    }
    let choice: Choice =
        serde_json::from_slice(&std::fs::read(choice_path(profile, identity)?).ok()?).ok()?;
    if !choice.enabled || uuid::Uuid::parse_str(&choice.revision).is_err() {
        return None;
    }
    Some(choice.revision)
}
pub fn persist_choice(profile: Option<&str>, enabled: bool) -> Result<()> {
    let identity = saved_identity(profile).context(
        "Sign in to the selected profile before changing this device's analytics choice",
    )?;
    let path = choice_path(profile, &identity).context("analytics choice path unavailable")?;
    let directory = path
        .parent()
        .context("analytics choice directory unavailable")?;
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    use std::io::Write;
    temporary.write_all(&serde_json::to_vec(&Choice {
        enabled,
        revision: uuid::Uuid::new_v4().to_string(),
    })?)?;
    temporary.as_file().sync_all()?;
    if saved_identity(profile).as_ref() != Some(&identity) {
        bail!("Account changed; retry analytics choice");
    }
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
/// Recomputed for every request, including refresh retries; never cached in a
/// reqwest client. A different process's withdrawal stops future admissions.
pub fn device_headers(
    profile: Option<&str>,
    base: &str,
    token: &str,
) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    let Some(identity) = saved_identity(profile) else {
        return headers;
    };
    if deployment_base(base).as_deref() != Some(identity.base_url.as_str())
        || crate::auth::jwt_sub_from_token(token).as_deref() != Some(identity.account_id.as_str())
        || permitted_revision(profile, &identity).is_none()
    {
        return headers;
    }
    headers.insert("x-nyxid-client", "cli".parse().unwrap());
    headers.insert(
        "x-nyxid-client-version",
        env!("CARGO_PKG_VERSION").parse().unwrap(),
    );
    headers.insert("x-nyxid-analytics", "enabled".parse().unwrap());
    if let Ok(value) = identity.account_id.parse() {
        headers.insert("x-nyxid-analytics-account", value);
    }
    headers
}
