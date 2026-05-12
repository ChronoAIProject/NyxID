use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::cli::UpdateArgs;
use crate::commands::repo::REPO_URL;

pub(crate) const GITHUB_API_URL: &str = "https://api.github.com";
pub(crate) const GITHUB_OWNER: &str = "ChronoAIProject";
pub(crate) const GITHUB_REPO: &str = "NyxID";
const RELEASE_WORKFLOW_PATH: &str = ".github/workflows/release.yml";
const DIST_PACKAGE_NAME: &str = "nyxid-cli";
const INSTALL_ROOT_ENV: &str = "NYXID_INSTALL_ROOT";
const ACTIVE_SYMLINK_ENV: &str = "NYXID_ACTIVE_SYMLINK";
const RETAINED_VERSION_COUNT: usize = 3;

pub async fn run(args: UpdateArgs) -> Result<()> {
    if args.list_versions {
        return list_versions();
    }

    if args.rollback {
        let target = rollback_cli()?;
        eprintln!("Rolled back nyxid to {}.", target.display());
        return Ok(());
    }

    if args.skills_only {
        return update_skills(&args.base_url).await;
    }

    if args.check {
        return check_cli_update(&args).await;
    }

    let replaced_binary = update_cli(&args).await?;

    // Hand off the skills phase to the freshly-installed binary so it always
    // runs against the latest fetch / install logic, even when the running
    // process was launched from an older binary that predates new skill paths.
    if let Some(new_bin) = find_new_binary(replaced_binary.as_deref()) {
        eprintln!("Handing off to {} for skill update...", new_bin.display());
        return exec_skills_update(&new_bin, &args.base_url);
    }

    eprintln!(
        "Warning: could not locate the freshly-installed nyxid binary; \
         continuing with the in-process skill update."
    );
    update_skills(&args.base_url).await?;

    eprintln!();
    eprintln!("All up to date.");
    Ok(())
}

async fn check_cli_update(args: &UpdateArgs) -> Result<()> {
    let client = github_client()?;
    let release = resolve_release(&client, args.version.as_deref()).await?;
    let installed = format!("v{}", env!("CARGO_PKG_VERSION"));

    println!("Installed: {installed}");

    let Some(release) = release else {
        println!("Latest: unavailable");
        println!("Status: no NyxID GitHub releases were found yet");
        return Ok(());
    };

    let target = current_target();
    let asset_name = asset_name_for_target(target)?;
    let asset_available = release_asset(&release, &asset_name).is_some();
    let label = if args.version.is_some() {
        "Requested"
    } else {
        "Latest"
    };

    println!("{label}: {}", release.tag_name);
    println!("Target: {target}");
    println!(
        "Binary asset: {}",
        if asset_available {
            asset_name
        } else {
            format!("{asset_name} (not found)")
        }
    );

    println!(
        "Status: {}",
        cli_update_status(&installed, &release.tag_name, asset_available)
    );

    Ok(())
}

fn cli_update_status(installed: &str, release_tag: &str, asset_available: bool) -> &'static str {
    if release_tag == installed && !asset_available {
        "up to date (no prebuilt asset for this target; future updates will fall back to --from-source)"
    } else if release_tag == installed {
        "up to date"
    } else if asset_available {
        "update available"
    } else {
        "prebuilt binary unavailable; update will fall back to --from-source"
    }
}

async fn update_cli(args: &UpdateArgs) -> Result<Option<PathBuf>> {
    if args.from_source {
        update_cli_from_source().await?;
        return Ok(None);
    }

    eprintln!("Updating NyxID CLI...");

    let client = github_client()?;
    let Some(release) = resolve_release(&client, args.version.as_deref()).await? else {
        eprintln!("No NyxID GitHub releases were found yet; falling back to source install.");
        update_cli_from_source().await?;
        return Ok(None);
    };

    let target = current_target();
    let asset_name = asset_name_for_target(target)?;
    let Some(asset) = release_asset(&release, &asset_name) else {
        eprintln!(
            "No prebuilt binary asset named {asset_name} exists on {}; falling back to source install.",
            release.tag_name
        );
        update_cli_from_source().await?;
        return Ok(None);
    };

    let tmp = tempfile::tempdir().context("Failed to create temporary update directory")?;
    let archive_path = tmp.path().join(&asset.name);

    eprintln!("Downloading {}...", asset.name);
    download_asset(&client, asset, &archive_path).await?;

    match verify_release_attestation(&client, &archive_path, &release.tag_name).await {
        Ok(()) => eprintln!("Release attestation verified."),
        Err(err) if args.insecure_skip_verify => {
            eprintln!();
            eprintln!("WARNING: release attestation verification failed.");
            eprintln!("WARNING: continuing because --insecure-skip-verify was provided.");
            eprintln!("WARNING: provenance error: {err:#}");
            eprintln!();
        }
        Err(err) => {
            return Err(err).context(
                "Release attestation verification failed; refusing to install this binary",
            );
        }
    }

    let installed_path = install_release_binary(&archive_path, &release.tag_name)?;

    eprintln!("CLI updated to {}.", release.tag_name);
    Ok(Some(installed_path))
}

async fn update_cli_from_source() -> Result<()> {
    eprintln!("Updating NyxID CLI from source...");

    let status = tokio::process::Command::new("cargo")
        .args([
            "install",
            "--git",
            REPO_URL,
            "nyxid-cli",
            "--force",
            "--locked",
        ])
        .status()
        .await
        .context("Failed to run cargo install. Is cargo available?")?;

    if !status.success() {
        anyhow::bail!("cargo install failed with exit code {}", status);
    }

    eprintln!("CLI updated from source.");
    Ok(())
}

async fn update_skills(base_url: &Option<String>) -> Result<()> {
    // When invoked via the exec hand-off from `update_cli`, the running binary
    // *is* the freshly-installed versioned binary -- which means we're
    // executing inside the install versions root. That's our cue to clean up
    // any legacy symlinks that older binaries (which lacked the in-process
    // cleanup) left dangling at previous versions. Downgrade-safe via tag
    // comparison, so a manual invocation against an older versioned binary
    // can't accidentally move PATH symlinks backwards.
    #[cfg(unix)]
    if let Ok(current_exe) = std::env::current_exe()
        && let Ok(versions_root) = install_versions_root()
        && let Ok(canonical_root) = versions_root.canonicalize()
        && let Ok(canonical_exe) = current_exe.canonicalize()
        && canonical_exe.starts_with(&canonical_root)
    {
        retarget_secondary_symlinks(&canonical_exe, Path::new(""));
    }

    // Reuse the ai-setup update logic (updates all installed tools)
    super::ai_setup::run(crate::cli::AiSetupCommands::Update {
        tool: None,
        base_url: base_url.clone(),
    })
    .await
}

#[derive(Debug, Deserialize)]
pub(crate) struct GitHubRelease {
    pub(crate) tag_name: String,
    pub(crate) assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GitHubAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
}

pub(crate) fn github_client() -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(&format!(
            "nyxid/{} ({GITHUB_OWNER}/{GITHUB_REPO})",
            env!("CARGO_PKG_VERSION")
        ))
        .context("Failed to build GitHub User-Agent header")?,
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "x-github-api-version",
        HeaderValue::from_static("2022-11-28"),
    );

    if let Some(token) = github_token() {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .context("Failed to build GitHub Authorization header")?,
        );
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .context("Failed to build GitHub HTTP client")
}

fn github_token() -> Option<String> {
    ["GITHUB_TOKEN", "GH_TOKEN"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.trim().is_empty())
}

pub(crate) async fn resolve_release(
    client: &reqwest::Client,
    version: Option<&str>,
) -> Result<Option<GitHubRelease>> {
    let url = if let Some(version) = version {
        let tag = normalize_release_tag(version)?;
        format!("{GITHUB_API_URL}/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/tags/{tag}")
    } else {
        format!("{GITHUB_API_URL}/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest")
    };

    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Failed to query GitHub release API: {url}"))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        if version.is_some() {
            anyhow::bail!("Requested NyxID release was not found: {url}");
        }
        return Ok(None);
    }

    let response = response
        .error_for_status()
        .with_context(|| format!("GitHub release API returned an error for {url}"))?;

    response
        .json::<GitHubRelease>()
        .await
        .context("Failed to parse GitHub release response")
        .map(Some)
}

fn release_asset<'a>(release: &'a GitHubRelease, asset_name: &str) -> Option<&'a GitHubAsset> {
    release.assets.iter().find(|asset| asset.name == asset_name)
}

async fn download_asset(
    client: &reqwest::Client,
    asset: &GitHubAsset,
    destination: &Path,
) -> Result<()> {
    let mut response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .with_context(|| format!("Failed to download {}", asset.name))?
        .error_for_status()
        .with_context(|| format!("GitHub returned an error while downloading {}", asset.name))?;

    let mut file = tokio::fs::File::create(destination)
        .await
        .with_context(|| format!("Failed to create {}", destination.display()))?;

    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("Failed while reading {}", asset.name))?
    {
        file.write_all(&chunk)
            .await
            .with_context(|| format!("Failed while writing {}", destination.display()))?;
    }

    file.flush()
        .await
        .with_context(|| format!("Failed to flush {}", destination.display()))?;
    Ok(())
}

async fn verify_release_attestation(
    client: &reqwest::Client,
    archive_path: &Path,
    tag: &str,
) -> Result<()> {
    eprintln!("Verifying GitHub artifact attestation...");

    let digest = sha256_file_hex(archive_path)
        .with_context(|| format!("Failed to hash {}", archive_path.display()))?;
    let expected_identity = expected_workflow_identity(tag);
    super::update_attestation::verify_release_attestation(
        client,
        GITHUB_OWNER,
        GITHUB_REPO,
        &digest,
        &expected_identity,
    )
    .await
    .context("Sigstore bundle verification failed")
}

fn install_release_binary(archive_path: &Path, tag: &str) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let extract_dir = tempfile::tempdir().context("Failed to create extraction directory")?;
        let new_bin = extract_binary(archive_path, extract_dir.path())?;
        return replace_current_binary(&new_bin);
    }

    #[cfg(unix)]
    {
        let versioned_bin = extract_binary_to_version_dir(archive_path, tag)?;
        let active_path = active_binary_path()?;
        retarget_active_symlink(&active_path, &versioned_bin)?;
        retarget_secondary_symlinks(&versioned_bin, &active_path);
        cleanup_old_versions(
            &install_versions_root()?,
            Some(&versioned_bin),
            RETAINED_VERSION_COUNT,
        )?;
        Ok(versioned_bin)
    }
}

#[cfg(unix)]
fn extract_binary_to_version_dir(archive_path: &Path, tag: &str) -> Result<PathBuf> {
    let tag = normalize_release_tag(tag)?;
    let version_dir = install_versions_root()?.join(&tag);
    fs::create_dir_all(&version_dir)
        .with_context(|| format!("Failed to create {}", version_dir.display()))?;
    let destination = version_dir.join(archive_binary_name());
    if destination.exists() {
        fs::remove_file(&destination)
            .with_context(|| format!("Failed to replace {}", destination.display()))?;
    }

    extract_binary(archive_path, &version_dir)
}

fn extract_binary(archive_path: &Path, extract_dir: &Path) -> Result<PathBuf> {
    let bin_name = archive_binary_name();
    // cargo-dist nests the binary inside a directory named after the package
    // and target triple, e.g. `nyxid-cli-aarch64-apple-darwin/nyxid`.
    // self_update::Extract::extract_file matches the in-archive path
    // exactly, so we have to ask for the nested path.
    let archive_dir = archive_root_dir(current_target());
    let in_archive_path = format!("{archive_dir}/{bin_name}");
    self_update::Extract::from_source(archive_path)
        .extract_file(extract_dir, &in_archive_path)
        .with_context(|| {
            format!(
                "Failed to extract {in_archive_path} from {}",
                archive_path.display()
            )
        })?;

    let new_bin = extract_dir.join(&archive_dir).join(bin_name);
    if !new_bin.exists() {
        anyhow::bail!(
            "Release archive did not contain expected binary path {}",
            new_bin.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&new_bin, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("Failed to set executable mode on {}", new_bin.display()))?;
    }

    Ok(new_bin)
}

fn archive_root_dir(target: &str) -> String {
    format!("{DIST_PACKAGE_NAME}-{target}")
}

#[cfg(windows)]
fn replace_current_binary(new_bin: &Path) -> Result<PathBuf> {
    let current_exe = std::env::current_exe().context("Failed to locate current nyxid binary")?;
    self_replace::self_replace(new_bin)
        .with_context(|| format!("Failed to replace {}", current_exe.display()))?;
    Ok(current_exe)
}

pub(crate) fn install_versions_root() -> Result<PathBuf> {
    if let Ok(root) = std::env::var(INSTALL_ROOT_ENV)
        && !root.trim().is_empty()
    {
        return Ok(PathBuf::from(root));
    }

    #[cfg(windows)]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            return Ok(local_app_data.join("nyxid").join("versions"));
        }
        anyhow::bail!("Could not determine LOCALAPPDATA for nyxid install root");
    }

    #[cfg(not(windows))]
    {
        if let Ok(data_home) = std::env::var("XDG_DATA_HOME")
            && !data_home.trim().is_empty()
        {
            return Ok(PathBuf::from(data_home).join("nyxid").join("versions"));
        }

        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home
            .join(".local")
            .join("share")
            .join("nyxid")
            .join("versions"))
    }
}

pub(crate) fn active_binary_path() -> Result<PathBuf> {
    active_binary_path_with_current(std::env::current_exe().ok().as_deref())
}

fn active_binary_path_with_current(current_exe: Option<&Path>) -> Result<PathBuf> {
    if let Ok(path) = std::env::var(ACTIVE_SYMLINK_ENV)
        && !path.trim().is_empty()
    {
        return Ok(PathBuf::from(path));
    }

    if let Some(current_exe) = current_exe
        && current_exe.file_name().and_then(|name| name.to_str()) == Some(archive_binary_name())
        && let Some(parent) = current_exe.parent()
        && path_contains_dir(parent)
        && !parent_dir_is_under(current_exe, &install_versions_root()?)
    {
        return Ok(current_exe.to_path_buf());
    }

    #[cfg(windows)]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            return Ok(local_app_data.join("nyxid").join(archive_binary_name()));
        }
        anyhow::bail!("Could not determine LOCALAPPDATA for nyxid active binary path");
    }

    #[cfg(not(windows))]
    {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".local").join("bin").join(archive_binary_name()))
    }
}

pub(crate) fn path_contains_dir(dir: &Path) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|entry| paths_equivalent(&entry, dir))
}

/// True when `path` is *located* inside `parent` (looking at where the file lives,
/// not what it resolves to). A symlink at `~/.cargo/bin/nyxid` whose target is
/// inside the versions root is NOT "under" the versions root for our purposes —
/// it's the legacy PATH entry we want to keep retargeting.
fn parent_dir_is_under(path: &Path, parent: &Path) -> bool {
    let Some(path_parent) = path.parent() else {
        return false;
    };
    path_parent
        .canonicalize()
        .ok()
        .zip(parent.canonicalize().ok())
        .is_some_and(|(p, root)| p.starts_with(root))
}

fn paths_equivalent(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    left.canonicalize()
        .ok()
        .zip(right.canonicalize().ok())
        .is_some_and(|(left, right)| left == right)
}

/// Walk `$PATH` and retarget every `nyxid` symlink that points into our
/// install versions root, except the primary one we just handled. This keeps
/// legacy install locations (e.g. `~/.cargo/bin/nyxid` left behind from an
/// initial `cargo install --path cli`) in sync with the active version
/// instead of letting them rot at whichever release the legacy entry first
/// pointed to.
///
/// Failures on individual entries are logged and skipped — a permission-denied
/// PATH dir should never abort the update.
#[cfg(unix)]
fn retarget_secondary_symlinks(versioned_bin: &Path, primary: &Path) {
    let Ok(versions_root) = install_versions_root() else {
        return;
    };
    let Ok(canonical_versions) = versions_root.canonicalize() else {
        return;
    };
    let Some(path_env) = std::env::var_os("PATH") else {
        return;
    };
    let bin_name = archive_binary_name();

    for dir in std::env::split_paths(&path_env) {
        let candidate = dir.join(bin_name);

        if paths_equivalent(&candidate, primary) {
            continue;
        }

        let Ok(meta) = fs::symlink_metadata(&candidate) else {
            continue;
        };
        if !meta.file_type().is_symlink() {
            continue;
        }

        let Ok(target) = candidate.canonicalize() else {
            continue;
        };
        if !target.starts_with(&canonical_versions) {
            continue;
        }

        if paths_equivalent(&target, versioned_bin) {
            continue;
        }

        // Downgrade guard: if the existing target points at a strictly newer
        // version than the binary we'd retarget to, leave it alone. Otherwise
        // a stray invocation of an older versioned binary (e.g.
        // `versions/v0.5.2/.../nyxid update --skills-only`) could silently
        // downgrade every legacy symlink in PATH.
        let target_tag = tag_for_path_in_versions(&target, &canonical_versions);
        let new_tag = versioned_bin
            .canonicalize()
            .ok()
            .as_deref()
            .and_then(|v| tag_for_path_in_versions(v, &canonical_versions))
            .or_else(|| tag_for_path_in_versions(versioned_bin, &canonical_versions));
        if let (Some(existing), Some(new_tag)) = (target_tag.as_deref(), new_tag.as_deref())
            && matches!(
                compare_release_tags(existing, new_tag),
                Ok(Ordering::Greater)
            )
        {
            continue;
        }

        match retarget_active_symlink(&candidate, versioned_bin) {
            Ok(()) => eprintln!(
                "Updated legacy symlink {} -> {}",
                candidate.display(),
                versioned_bin.display()
            ),
            Err(err) => eprintln!(
                "Warning: failed to update legacy symlink {}: {err:#}",
                candidate.display()
            ),
        }
    }
}

/// Extract the version tag from a path inside the install versions root.
/// `<versions_root>/v0.5.3/nyxid-cli-aarch64-apple-darwin/nyxid` -> `Some("v0.5.3")`.
fn tag_for_path_in_versions(path: &Path, canonical_versions: &Path) -> Option<String> {
    path.strip_prefix(canonical_versions)
        .ok()?
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .map(|s| s.to_string())
}

#[cfg(unix)]
pub(crate) fn retarget_active_symlink(active_path: &Path, versioned_bin: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    let parent = active_path
        .parent()
        .context("Active nyxid symlink path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("Failed to create {}", parent.display()))?;

    let temp_name = format!(
        "{}.tmp.{}.{}",
        archive_binary_name(),
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos()
    );
    let temp_path = parent.join(temp_name);
    if temp_path.exists() {
        fs::remove_file(&temp_path)
            .with_context(|| format!("Failed to remove stale {}", temp_path.display()))?;
    }

    symlink(versioned_bin, &temp_path).with_context(|| {
        format!(
            "Failed to create temporary symlink {} -> {}",
            temp_path.display(),
            versioned_bin.display()
        )
    })?;

    fs::rename(&temp_path, active_path).with_context(|| {
        format!(
            "Failed to retarget {} to {}",
            active_path.display(),
            versioned_bin.display()
        )
    })?;
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct InstalledVersion {
    pub(crate) tag: String,
    pub(crate) dir: PathBuf,
    pub(crate) binary: PathBuf,
    pub(crate) modified: SystemTime,
    pub(crate) active: bool,
}

pub(crate) fn installed_versions() -> Result<Vec<InstalledVersion>> {
    installed_versions_in(
        &install_versions_root()?,
        active_binary_path().ok().as_deref(),
    )
}

pub(crate) fn installed_versions_in(
    root: &Path,
    active_path: Option<&Path>,
) -> Result<Vec<InstalledVersion>> {
    let active_target = active_path
        .and_then(|path| fs::read_link(path).ok())
        .map(|target| {
            if target.is_absolute() {
                target
            } else {
                active_path
                    .and_then(Path::parent)
                    .unwrap_or_else(|| Path::new("."))
                    .join(target)
            }
        });

    installed_versions_with_active_target(root, active_target.as_deref())
}

fn installed_versions_with_active_target(
    root: &Path,
    active_target: Option<&Path>,
) -> Result<Vec<InstalledVersion>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut versions = Vec::new();
    for entry in fs::read_dir(root).with_context(|| format!("Failed to read {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(tag) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if normalize_release_tag(tag).is_err() {
            continue;
        }

        let binary = path.join(archive_binary_name());
        if !binary.is_file() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let active = active_target.is_some_and(|target| paths_equivalent(target, &binary));

        versions.push(InstalledVersion {
            tag: tag.to_string(),
            dir: path,
            binary,
            modified,
            active,
        });
    }

    versions.sort_by(compare_installed_versions);
    Ok(versions)
}

fn compare_installed_versions(left: &InstalledVersion, right: &InstalledVersion) -> Ordering {
    right
        .modified
        .cmp(&left.modified)
        .then_with(|| compare_release_tags(&right.tag, &left.tag).unwrap_or(Ordering::Equal))
}

pub(crate) fn format_installed_versions(versions: &[InstalledVersion], root: &Path) -> String {
    if versions.is_empty() {
        return format!("No nyxid versions installed yet at {}.", root.display());
    }

    let mut lines = vec!["Installed nyxid versions:".to_string()];
    for version in versions {
        let marker = if version.active { "*" } else { " " };
        let active = if version.active { " (active)" } else { "" };
        lines.push(format!(
            "{marker} {:<18} {}{active}",
            version.tag,
            version.binary.display()
        ));
    }
    lines.join("\n")
}

fn list_versions() -> Result<()> {
    let root = install_versions_root()?;
    let versions = installed_versions()?;
    println!("{}", format_installed_versions(&versions, &root));
    Ok(())
}

#[cfg(unix)]
fn cleanup_old_versions(
    root: &Path,
    active_binary: Option<&Path>,
    keep_total: usize,
) -> Result<()> {
    let versions = installed_versions_with_active_target(root, active_binary)?;
    if versions.len() <= keep_total {
        return Ok(());
    }

    let mut keep = HashSet::new();
    if let Some(active) = active_binary {
        for version in &versions {
            if paths_equivalent(&version.binary, active) {
                keep.insert(version.dir.clone());
                break;
            }
        }
    }

    for version in &versions {
        if keep.len() >= keep_total {
            break;
        }
        keep.insert(version.dir.clone());
    }

    for version in versions {
        if !keep.contains(&version.dir) {
            fs::remove_dir_all(&version.dir)
                .with_context(|| format!("Failed to prune {}", version.dir.display()))?;
        }
    }

    Ok(())
}

fn rollback_cli() -> Result<PathBuf> {
    #[cfg(not(unix))]
    {
        anyhow::bail!("nyxid update --rollback is only supported for versioned Unix installs");
    }

    #[cfg(unix)]
    {
        let root = install_versions_root()?;
        let active_path = active_binary_path()?;
        let versions = installed_versions_in(&root, Some(&active_path))?;
        if versions.len() <= 1 {
            anyhow::bail!("No previous nyxid version is available to roll back to");
        }

        let active_index = versions
            .iter()
            .position(|version| version.active)
            .context("No active nyxid version symlink was found")?;
        let Some(target) = versions.get(active_index + 1) else {
            anyhow::bail!("No previous nyxid version is available to roll back to");
        };
        retarget_active_symlink(&active_path, &target.binary)?;
        Ok(target.binary.clone())
    }
}

fn sha256_file_hex(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

fn expected_workflow_identity(tag: &str) -> String {
    format!(
        "https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/{RELEASE_WORKFLOW_PATH}@refs/tags/{tag}"
    )
}

pub(crate) fn normalize_release_tag(version: &str) -> Result<String> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);

    if !is_valid_semver_tag(version) {
        anyhow::bail!(
            "Invalid release version `{version}`; expected SemVer X.Y.Z with optional pre-release/build metadata"
        );
    }

    Ok(format!("v{version}"))
}

fn is_valid_semver_tag(version: &str) -> bool {
    let Some((without_build, build_metadata)) = split_optional_once(version, '+') else {
        return false;
    };
    if let Some(build_metadata) = build_metadata
        && !valid_semver_identifiers(build_metadata, false)
    {
        return false;
    }

    let (core, pre_release) = match without_build.split_once('-') {
        Some((core, pre_release)) => (core, Some(pre_release)),
        None => (without_build, None),
    };
    valid_semver_core(core)
        && pre_release
            .map(|pre_release| valid_semver_identifiers(pre_release, true))
            .unwrap_or(true)
}

fn split_optional_once(input: &str, separator: char) -> Option<(&str, Option<&str>)> {
    match input.split_once(separator) {
        Some((_, right)) if right.contains(separator) => None,
        Some((left, right)) => Some((left, Some(right))),
        None => Some((input, None)),
    }
}

fn valid_semver_core(core: &str) -> bool {
    let parts = core.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| valid_numeric_identifier(part, true))
}

fn valid_semver_identifiers(identifiers: &str, reject_numeric_leading_zero: bool) -> bool {
    !identifiers.is_empty()
        && identifiers.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
                && (!reject_numeric_leading_zero
                    || !identifier.chars().all(|ch| ch.is_ascii_digit())
                    || valid_numeric_identifier(identifier, true))
        })
}

fn valid_numeric_identifier(identifier: &str, reject_leading_zero: bool) -> bool {
    !identifier.is_empty()
        && identifier.chars().all(|ch| ch.is_ascii_digit())
        && (!reject_leading_zero || identifier.len() == 1 || !identifier.starts_with('0'))
}

pub(crate) fn compare_release_tags(left: &str, right: &str) -> Result<Ordering> {
    let left = ParsedReleaseTag::parse(left)?;
    let right = ParsedReleaseTag::parse(right)?;
    Ok(left.cmp(&right))
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ParsedReleaseTag {
    major: u64,
    minor: u64,
    patch: u64,
    pre_release: Vec<PreReleaseIdentifier>,
}

impl ParsedReleaseTag {
    fn parse(tag: &str) -> Result<Self> {
        let normalized = normalize_release_tag(tag)?;
        let version = normalized.trim_start_matches('v');
        let Some((without_build, _)) = split_optional_once(version, '+') else {
            anyhow::bail!("Invalid release version `{tag}`");
        };
        let (core, pre_release) = match without_build.split_once('-') {
            Some((core, pre_release)) => (core, pre_release),
            None => (without_build, ""),
        };
        let parts = core.split('.').collect::<Vec<_>>();
        if parts.len() != 3 {
            anyhow::bail!("Invalid release version `{tag}`");
        }

        let pre_release = if pre_release.is_empty() {
            Vec::new()
        } else {
            pre_release
                .split('.')
                .map(PreReleaseIdentifier::parse)
                .collect::<Result<Vec<_>>>()?
        };

        Ok(Self {
            major: parts[0].parse()?,
            minor: parts[1].parse()?,
            patch: parts[2].parse()?,
            pre_release,
        })
    }
}

impl Ord for ParsedReleaseTag {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| compare_pre_release(&self.pre_release, &other.pre_release))
    }
}

impl PartialOrd for ParsedReleaseTag {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum PreReleaseIdentifier {
    Numeric(u64),
    Alpha(String),
}

impl PreReleaseIdentifier {
    fn parse(value: &str) -> Result<Self> {
        if value.chars().all(|ch| ch.is_ascii_digit()) {
            Ok(Self::Numeric(value.parse()?))
        } else {
            Ok(Self::Alpha(value.to_string()))
        }
    }
}

fn compare_pre_release(left: &[PreReleaseIdentifier], right: &[PreReleaseIdentifier]) -> Ordering {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }

    for (left, right) in left.iter().zip(right) {
        let ordering = match (left, right) {
            (PreReleaseIdentifier::Numeric(left), PreReleaseIdentifier::Numeric(right)) => {
                left.cmp(right)
            }
            (PreReleaseIdentifier::Alpha(left), PreReleaseIdentifier::Alpha(right)) => {
                left.cmp(right)
            }
            (PreReleaseIdentifier::Numeric(_), PreReleaseIdentifier::Alpha(_)) => Ordering::Less,
            (PreReleaseIdentifier::Alpha(_), PreReleaseIdentifier::Numeric(_)) => Ordering::Greater,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    left.len().cmp(&right.len())
}

fn asset_name_for_target(target: &str) -> Result<String> {
    if target.is_empty() || target == "unknown" {
        anyhow::bail!("Current build target is unavailable");
    }

    let extension = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    Ok(format!("{DIST_PACKAGE_NAME}-{target}.{extension}"))
}

fn current_target() -> &'static str {
    env!("TARGET")
}

fn archive_binary_name() -> &'static str {
    if cfg!(windows) { "nyxid.exe" } else { "nyxid" }
}

/// Locate the freshly-installed `nyxid` binary.
///
/// For prebuilt updates this prefers the current executable path that was just
/// replaced in place. For `--from-source`, it falls back to the cargo install
/// location.
fn find_new_binary(preferred: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = preferred
        && path.exists()
    {
        return Some(path.to_path_buf());
    }

    if let Ok(path) = std::env::current_exe()
        && path.exists()
    {
        return Some(path);
    }

    let cargo_home = std::env::var("CARGO_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))?;

    let path = cargo_home.join("bin").join(archive_binary_name());
    path.exists().then_some(path)
}

/// Replace the current process with `<new_bin> update --skills-only [--base-url X]`.
/// On Unix this `exec`s in place; on Windows we spawn + wait + propagate the
/// exit code since `exec` semantics aren't available.
fn exec_skills_update(new_bin: &PathBuf, base_url: &Option<String>) -> Result<()> {
    let mut cmd = std::process::Command::new(new_bin);
    cmd.arg("update").arg("--skills-only");
    if let Some(url) = base_url {
        cmd.arg("--base-url").arg(url);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // On success, exec replaces the process and never returns.
        let err = cmd.exec();
        Err(anyhow::anyhow!(
            "Failed to exec {}: {err}",
            new_bin.display()
        ))
    }
    #[cfg(not(unix))]
    {
        let status = cmd
            .status()
            .with_context(|| format!("Failed to spawn {}", new_bin.display()))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn extracts_nested_binary_from_dist_tarball() {
        // cargo-dist tarballs nest the binary inside `{package}-{target}/`,
        // and self_update::Extract matches paths exactly. Verify the nested
        // path resolution end-to-end against a tarball that mirrors the
        // real release layout.
        let target = "aarch64-apple-darwin";
        let archive_dir = archive_root_dir(target);
        assert_eq!(archive_dir, "nyxid-cli-aarch64-apple-darwin");

        let tmp = tempfile::tempdir().unwrap();
        let archive_path = tmp.path().join("nyxid-cli-aarch64-apple-darwin.tar.gz");
        let extract_dir = tmp.path().join("extract");
        std::fs::create_dir_all(&extract_dir).unwrap();

        let payload = b"#!/bin/sh\necho fake-nyxid\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();

        let tar_gz = std::fs::File::create(&archive_path).unwrap();
        let gz_writer = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut tar_builder = tar::Builder::new(gz_writer);
        tar_builder
            .append_data(
                &mut header,
                format!("{archive_dir}/nyxid"),
                payload.as_slice(),
            )
            .unwrap();
        // Toss in a sibling so we know the path-match logic is exact, not
        // prefix-based.
        let mut readme = tar::Header::new_gnu();
        readme.set_size(8);
        readme.set_mode(0o644);
        readme.set_cksum();
        tar_builder
            .append_data(
                &mut readme,
                format!("{archive_dir}/README.md"),
                b"readme\n\n".as_slice(),
            )
            .unwrap();
        tar_builder.finish().unwrap();
        drop(tar_builder);

        let archive_dir_clone = archive_dir.clone();
        let in_archive_path = format!("{archive_dir_clone}/nyxid");
        self_update::Extract::from_source(&archive_path)
            .extract_file(&extract_dir, &in_archive_path)
            .unwrap();

        let extracted = extract_dir.join(&archive_dir).join("nyxid");
        assert!(extracted.exists(), "expected {extracted:?} to exist");
        assert_eq!(std::fs::read(&extracted).unwrap(), payload);
    }

    #[test]
    fn builds_dist_asset_names() {
        assert_eq!(
            asset_name_for_target("x86_64-unknown-linux-gnu").unwrap(),
            "nyxid-cli-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            asset_name_for_target("aarch64-unknown-linux-gnu").unwrap(),
            "nyxid-cli-aarch64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            asset_name_for_target("x86_64-apple-darwin").unwrap(),
            "nyxid-cli-x86_64-apple-darwin.tar.gz"
        );
        assert_eq!(
            asset_name_for_target("aarch64-apple-darwin").unwrap(),
            "nyxid-cli-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(
            asset_name_for_target("x86_64-pc-windows-msvc").unwrap(),
            "nyxid-cli-x86_64-pc-windows-msvc.zip"
        );
    }

    #[test]
    fn parses_release_version_tags() {
        assert_eq!(normalize_release_tag("0.4.0").unwrap(), "v0.4.0");
        assert_eq!(normalize_release_tag("v1.2.3").unwrap(), "v1.2.3");
        assert_eq!(
            normalize_release_tag("0.4.0-beta.1").unwrap(),
            "v0.4.0-beta.1"
        );
        assert_eq!(
            normalize_release_tag("1.2.3-rc.1+build.42").unwrap(),
            "v1.2.3-rc.1+build.42"
        );
        assert!(normalize_release_tag("1.2").is_err());
        assert!(normalize_release_tag("1.2.x").is_err());
        assert!(normalize_release_tag("1.2.3-0123").is_err());
        assert!(normalize_release_tag("1.2.3-").is_err());
        assert!(normalize_release_tag("release-1.2.3").is_err());
    }

    #[test]
    fn compares_release_tags() {
        assert_eq!(
            compare_release_tags("v0.5.0", "v0.4.9").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_release_tags("v1.0.0", "v1.0.0-rc.1").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_release_tags("v1.0.0-rc.2", "v1.0.0-rc.10").unwrap(),
            Ordering::Less
        );
        assert_eq!(
            compare_release_tags("v1.0.0+build.2", "v1.0.0+build.1").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn check_status_mentions_missing_asset_when_installed_release_has_no_binary() {
        assert_eq!(
            cli_update_status("v0.4.0", "v0.4.0", false),
            "up to date (no prebuilt asset for this target; future updates will fall back to --from-source)"
        );
        assert_eq!(cli_update_status("v0.4.0", "v0.4.0", true), "up to date");
        assert_eq!(
            cli_update_status("v0.4.0", "v0.5.0", true),
            "update available"
        );
        assert_eq!(
            cli_update_status("v0.4.0", "v0.5.0", false),
            "prebuilt binary unavailable; update will fall back to --from-source"
        );
    }

    #[test]
    fn builds_expected_workflow_identity() {
        assert_eq!(
            expected_workflow_identity("v1.2.3"),
            "https://github.com/ChronoAIProject/NyxID/.github/workflows/release.yml@refs/tags/v1.2.3"
        );
    }

    #[test]
    fn verifies_fixture_attestation_subject_digest() {
        let payload =
            include_bytes!("../../tests/fixtures/update-attestation-statement.json").as_slice();
        super::super::update_attestation::verify_payload_subject_digest(
            payload,
            "8c5b8a213a6d3d0c74a1f3a1c9dbd9ed93094b2b2ca8c7a4d00365bd7a9a6a6b",
        )
        .unwrap();
        assert!(
            super::super::update_attestation::verify_payload_subject_digest(payload, "0000")
                .is_err()
        );
    }

    #[test]
    fn creates_dsse_pae() {
        assert_eq!(
            super::super::update_attestation::dsse_pae("application/vnd.in-toto+json", b"hello"),
            b"DSSEv1 28 application/vnd.in-toto+json 5 hello"
        );
    }

    #[test]
    fn install_root_honors_env_and_xdg_then_home() {
        let _lock = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root_override = tmp.path().join("override");
        let xdg_home = tmp.path().join("xdg");
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let _install = EnvGuard::set(INSTALL_ROOT_ENV, root_override.as_os_str());
        let _xdg = EnvGuard::set("XDG_DATA_HOME", xdg_home.as_os_str());
        let _home = EnvGuard::set("HOME", home.as_os_str());
        assert_eq!(install_versions_root().unwrap(), root_override);

        drop(_install);
        assert_eq!(
            install_versions_root().unwrap(),
            xdg_home.join("nyxid").join("versions")
        );

        drop(_xdg);
        assert_eq!(
            install_versions_root().unwrap(),
            home.join(".local")
                .join("share")
                .join("nyxid")
                .join("versions")
        );
    }

    #[test]
    fn list_versions_format_marks_active_and_handles_empty() {
        let root = PathBuf::from("/tmp/nyxid/versions");
        assert_eq!(
            format_installed_versions(&[], &root),
            "No nyxid versions installed yet at /tmp/nyxid/versions."
        );

        let versions = vec![
            InstalledVersion {
                tag: "v0.5.0".to_string(),
                dir: root.join("v0.5.0"),
                binary: root.join("v0.5.0").join("nyxid"),
                modified: SystemTime::UNIX_EPOCH,
                active: true,
            },
            InstalledVersion {
                tag: "v0.4.1".to_string(),
                dir: root.join("v0.4.1"),
                binary: root.join("v0.4.1").join("nyxid"),
                modified: SystemTime::UNIX_EPOCH,
                active: false,
            },
        ];
        let rendered = format_installed_versions(&versions, &root);
        assert!(rendered.contains("* v0.5.0"));
        assert!(rendered.contains("(active)"));
        assert!(rendered.contains("  v0.4.1"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_retarget_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let active = tmp.path().join("bin").join("nyxid");
        let versioned = write_version_binary(tmp.path().join("versions").as_path(), "v0.5.0");

        retarget_active_symlink(&active, &versioned).unwrap();
        assert_eq!(fs::read_link(&active).unwrap(), versioned);

        let versioned = write_version_binary(tmp.path().join("versions").as_path(), "v0.5.0");
        retarget_active_symlink(&active, &versioned).unwrap();
        assert_eq!(fs::read_link(&active).unwrap(), versioned);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_retarget_migrates_legacy_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let active = tmp.path().join("bin").join("nyxid");
        fs::create_dir_all(active.parent().unwrap()).unwrap();
        fs::write(&active, b"legacy").unwrap();
        let versioned = write_version_binary(tmp.path().join("versions").as_path(), "v0.5.0");

        retarget_active_symlink(&active, &versioned).unwrap();

        assert!(
            fs::symlink_metadata(&active)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&active).unwrap(), versioned);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_keeps_current_and_two_previous_versions_by_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("versions");
        let mut binaries = Vec::new();
        for patch in 0..5 {
            binaries.push(write_version_binary(&root, &format!("v0.1.{patch}")));
            std::thread::sleep(Duration::from_millis(15));
        }
        let active = binaries.last().unwrap().clone();

        cleanup_old_versions(&root, Some(&active), 3).unwrap();

        assert!(!root.join("v0.1.0").exists());
        assert!(!root.join("v0.1.1").exists());
        assert!(root.join("v0.1.2").exists());
        assert!(root.join("v0.1.3").exists());
        assert!(root.join("v0.1.4").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rollback_errors_with_one_version_and_succeeds_with_two() {
        let _lock = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("versions");
        let active = tmp.path().join("bin").join("nyxid");
        let _install = EnvGuard::set(INSTALL_ROOT_ENV, root.as_os_str());
        let _active = EnvGuard::set(ACTIVE_SYMLINK_ENV, active.as_os_str());

        let first = write_version_binary(&root, "v0.4.0");
        retarget_active_symlink(&active, &first).unwrap();
        assert!(rollback_cli().is_err());

        std::thread::sleep(Duration::from_millis(15));
        let second = write_version_binary(&root, "v0.5.0");
        retarget_active_symlink(&active, &second).unwrap();

        let rolled_back = rollback_cli().unwrap();
        assert!(paths_equivalent(&rolled_back, &first));
        assert!(paths_equivalent(&fs::read_link(&active).unwrap(), &first));
    }

    #[cfg(unix)]
    #[test]
    fn active_path_resolves_symlink_in_path_pointing_into_versions_root() {
        use std::os::unix::fs::symlink;
        let _lock = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let versions_root = tmp.path().join("versions");
        let bin_dir = tmp.path().join("legacy-bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let versioned = write_version_binary(&versions_root, "v0.5.3");
        let legacy_symlink = bin_dir.join(archive_binary_name());
        symlink(&versioned, &legacy_symlink).unwrap();

        let _install = EnvGuard::set(INSTALL_ROOT_ENV, versions_root.as_os_str());
        let mut path_value = OsString::from(bin_dir.as_os_str());
        if let Some(existing) = std::env::var_os("PATH") {
            path_value.push(":");
            path_value.push(&existing);
        }
        let _path = EnvGuard::set("PATH", &path_value);

        let resolved = active_binary_path_with_current(Some(&legacy_symlink)).unwrap();
        assert_eq!(resolved, legacy_symlink);
    }

    #[cfg(unix)]
    #[test]
    fn active_path_skips_real_binary_inside_versions_root() {
        let _lock = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let versions_root = tmp.path().join("versions");
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let versioned = write_version_binary(&versions_root, "v0.5.3");

        let _install = EnvGuard::set(INSTALL_ROOT_ENV, versions_root.as_os_str());
        let _home = EnvGuard::set("HOME", home.as_os_str());

        // Even though the versioned binary's parent is in PATH (we add it
        // below), `parent_dir_is_under` should detect that it lives inside
        // the versions root and fall through to the default install path.
        let mut path_value = OsString::from(versioned.parent().unwrap().as_os_str());
        if let Some(existing) = std::env::var_os("PATH") {
            path_value.push(":");
            path_value.push(&existing);
        }
        let _path = EnvGuard::set("PATH", &path_value);

        let resolved = active_binary_path_with_current(Some(&versioned)).unwrap();
        assert_eq!(
            resolved,
            home.join(".local").join("bin").join(archive_binary_name())
        );
    }

    #[cfg(unix)]
    #[test]
    fn retarget_secondary_symlinks_updates_stale_legacy_symlinks() {
        use std::os::unix::fs::symlink;
        let _lock = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let versions_root = tmp.path().join("versions");
        let old_bin = write_version_binary(&versions_root, "v0.5.2");
        let new_bin = write_version_binary(&versions_root, "v0.5.3");

        let primary_dir = tmp.path().join("primary-bin");
        let legacy_dir = tmp.path().join("legacy-bin");
        let other_dir = tmp.path().join("other-bin");
        fs::create_dir_all(&primary_dir).unwrap();
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();

        let primary = primary_dir.join(archive_binary_name());
        let legacy = legacy_dir.join(archive_binary_name());
        let outside_target = tmp.path().join("outside-target");
        fs::write(&outside_target, b"other").unwrap();
        let unrelated = other_dir.join(archive_binary_name());

        // Primary already points to the new binary (post primary retarget).
        symlink(&new_bin, &primary).unwrap();
        // Legacy symlink still points at the previous version.
        symlink(&old_bin, &legacy).unwrap();
        // A nyxid symlink that points outside the versions root should be left
        // alone -- it might be a deliberate dev-tree symlink, not ours.
        symlink(&outside_target, &unrelated).unwrap();

        let _install = EnvGuard::set(INSTALL_ROOT_ENV, versions_root.as_os_str());
        let path_value = std::env::join_paths([&primary_dir, &legacy_dir, &other_dir]).unwrap();
        let _path = EnvGuard::set("PATH", &path_value);

        retarget_secondary_symlinks(&new_bin, &primary);

        assert_eq!(fs::read_link(&primary).unwrap(), new_bin);
        assert_eq!(fs::read_link(&legacy).unwrap(), new_bin);
        assert_eq!(fs::read_link(&unrelated).unwrap(), outside_target);
    }

    #[cfg(unix)]
    #[test]
    fn retarget_secondary_symlinks_refuses_to_downgrade() {
        use std::os::unix::fs::symlink;
        let _lock = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let versions_root = tmp.path().join("versions");
        let older = write_version_binary(&versions_root, "v0.5.2");
        let newer = write_version_binary(&versions_root, "v0.5.4");

        let legacy_dir = tmp.path().join("legacy-bin");
        fs::create_dir_all(&legacy_dir).unwrap();
        let legacy = legacy_dir.join(archive_binary_name());
        // Legacy symlink already at the newer version.
        symlink(&newer, &legacy).unwrap();

        let _install = EnvGuard::set(INSTALL_ROOT_ENV, versions_root.as_os_str());
        let _path = EnvGuard::set("PATH", legacy_dir.as_os_str());

        // Simulate a stray hand-off from the older versioned binary --
        // versioned_bin is the older path, primary is meaningless.
        retarget_secondary_symlinks(&older, Path::new(""));

        // Legacy symlink must still point at the newer binary.
        assert_eq!(fs::read_link(&legacy).unwrap(), newer);
    }

    #[cfg(unix)]
    #[test]
    fn retarget_secondary_symlinks_leaves_regular_files_alone() {
        let _lock = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let versions_root = tmp.path().join("versions");
        let new_bin = write_version_binary(&versions_root, "v0.5.3");

        let path_dir = tmp.path().join("bin");
        fs::create_dir_all(&path_dir).unwrap();
        let regular = path_dir.join(archive_binary_name());
        fs::write(&regular, b"some-other-tool").unwrap();

        let _install = EnvGuard::set(INSTALL_ROOT_ENV, versions_root.as_os_str());
        let _path = EnvGuard::set("PATH", path_dir.as_os_str());

        let bogus_primary = tmp.path().join("nonexistent-primary");
        retarget_secondary_symlinks(&new_bin, &bogus_primary);

        let meta = fs::symlink_metadata(&regular).unwrap();
        assert!(meta.file_type().is_file());
        assert_eq!(fs::read(&regular).unwrap(), b"some-other-tool");
    }

    #[cfg(unix)]
    fn write_version_binary(root: &Path, tag: &str) -> PathBuf {
        let dir = root.join(tag);
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join(archive_binary_name());
        fs::write(&binary, b"nyxid").unwrap();
        binary
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
            let old = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.old {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
}
