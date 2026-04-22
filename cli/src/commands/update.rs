use std::path::Path;

use anyhow::{Context, Result, bail};
use tempfile::tempdir;

use crate::cli::UpdateArgs;
use crate::commands::repo::{CLI_VERSION, REPO_URL};
use crate::update_support::{
    CliReleaseManifest, ReleaseLookupError, VerificationContext, download_asset, extract_archive,
    fetch_github_release_assets, fetch_release_manifest, find_binary,
    handle_signature_verification_failure, host_release_target, is_newer_version,
    normalize_version, parse_sha256sums, sha256_file_hex, verify_checksums_signature,
};

pub async fn run(args: UpdateArgs) -> Result<()> {
    if args.skills_only {
        return update_skills(&args.resolved_base_url()?).await;
    }

    let base_url = args.resolved_base_url()?;

    if args.check {
        run_check(&base_url, &args).await
    } else {
        run_update(&base_url, &args).await
    }
}

async fn run_check(base_url: &str, args: &UpdateArgs) -> Result<()> {
    let manifest = desired_manifest(base_url, args).await?;
    let Some(target) = host_release_target() else {
        eprintln!(
            "No prebuilt nyxid binary is published for host target {}.",
            self_update::get_target()
        );
        std::process::exit(2);
    };

    let archive_name = manifest.archive_name_for_target(target)?;
    match fetch_github_release_assets(&manifest.version, target, &archive_name).await {
        Ok(_) => {}
        Err(ReleaseLookupError::MissingArchive { .. }) => {
            eprintln!("No prebuilt nyxid binary is published for host target {target}.");
            std::process::exit(2);
        }
        Err(error) => return Err(error.into()),
    }

    let version_changes_binary = if args.version.is_some() {
        normalize_version(CLI_VERSION) != normalize_version(&manifest.version)
    } else {
        is_newer_version(CLI_VERSION, &manifest.version)?
    };

    if version_changes_binary {
        eprintln!(
            "A newer nyxid is available: {} -> {}",
            CLI_VERSION, manifest.version
        );
        std::process::exit(1);
    } else {
        eprintln!("nyxid {CLI_VERSION} is up to date.");
        std::process::exit(0);
    }
}

async fn run_update(base_url: &str, args: &UpdateArgs) -> Result<()> {
    let cli_result = if args.from_source {
        update_from_source(args.version.as_deref()).await?
    } else {
        match update_prebuilt(base_url, args).await {
            Ok(result) => result,
            Err(PrebuiltUpdateError::NoBinary(target)) => {
                eprintln!(
                    "No prebuilt nyxid binary is published for host target {target}; falling back to a source build."
                );
                update_from_source(args.version.as_deref()).await?
            }
            Err(PrebuiltUpdateError::Other(error)) => return Err(error),
        }
    };

    match cli_result {
        CliUpdateResult::AlreadyCurrent(version) => {
            eprintln!("CLI is already up to date at {version}.");
        }
        CliUpdateResult::Updated(version) => {
            eprintln!("CLI updated to {version}.");
        }
        CliUpdateResult::UpdatedFromSource(version) => {
            eprintln!("CLI updated from source to {version}.");
        }
    }

    update_skills(base_url).await?;

    eprintln!();
    eprintln!("Update complete.");
    Ok(())
}

async fn desired_manifest(base_url: &str, args: &UpdateArgs) -> Result<CliReleaseManifest> {
    let latest = fetch_release_manifest(base_url).await?;
    Ok(match args.version.as_deref() {
        Some(version) => latest.for_version(version),
        None => latest,
    })
}

async fn update_prebuilt(
    base_url: &str,
    args: &UpdateArgs,
) -> std::result::Result<CliUpdateResult, PrebuiltUpdateError> {
    let target = host_release_target()
        .ok_or_else(|| PrebuiltUpdateError::NoBinary(self_update::get_target().to_string()))?;
    let manifest = desired_manifest(base_url, args)
        .await
        .map_err(PrebuiltUpdateError::Other)?;

    if args.version.is_none()
        && !is_newer_version(CLI_VERSION, &manifest.version).map_err(PrebuiltUpdateError::Other)?
    {
        return Ok(CliUpdateResult::AlreadyCurrent(manifest.version));
    }

    let archive_name = manifest
        .archive_name_for_target(target)
        .map_err(PrebuiltUpdateError::Other)?;
    let assets = fetch_github_release_assets(&manifest.version, target, &archive_name)
        .await
        .map_err(|error| match error {
            ReleaseLookupError::MissingArchive { .. } => {
                PrebuiltUpdateError::NoBinary(target.to_string())
            }
            other => PrebuiltUpdateError::Other(other.into()),
        })?;

    eprintln!("Downloading nyxid {} for {}...", manifest.version, target);

    let temp_dir = tempdir()
        .context("Failed to create temporary update directory")
        .map_err(PrebuiltUpdateError::Other)?;
    let archive_path = temp_dir.path().join(&archive_name);
    let checksums_path = temp_dir.path().join("SHA256SUMS");
    let signature_path = temp_dir.path().join("SHA256SUMS.sig");
    let certificate_path = temp_dir.path().join("SHA256SUMS.pem");

    download_asset(&assets.archive, &archive_path, true)
        .await
        .map_err(PrebuiltUpdateError::Other)?;
    download_asset(&assets.checksums, &checksums_path, false)
        .await
        .map_err(PrebuiltUpdateError::Other)?;
    download_asset(&assets.checksums_signature, &signature_path, false)
        .await
        .map_err(PrebuiltUpdateError::Other)?;
    download_asset(&assets.checksums_cert, &certificate_path, false)
        .await
        .map_err(PrebuiltUpdateError::Other)?;

    verify_downloaded_release(
        &manifest,
        &archive_path,
        &checksums_path,
        &signature_path,
        &certificate_path,
        args.allow_unverified,
    )
    .await
    .map_err(PrebuiltUpdateError::Other)?;

    let extract_dir = temp_dir.path().join("extract");
    extract_archive(&archive_path, &extract_dir)
        .await
        .map_err(PrebuiltUpdateError::Other)?;

    let new_binary = find_binary(&extract_dir, "nyxid").map_err(PrebuiltUpdateError::Other)?;
    replace_current_binary(&new_binary).map_err(PrebuiltUpdateError::Other)?;

    Ok(CliUpdateResult::Updated(manifest.version))
}

async fn verify_downloaded_release(
    manifest: &CliReleaseManifest,
    archive_path: &Path,
    checksums_path: &Path,
    signature_path: &Path,
    certificate_path: &Path,
    allow_unverified: bool,
) -> Result<()> {
    let checksums = std::fs::read_to_string(checksums_path)
        .with_context(|| format!("Failed to read {}", checksums_path.display()))?;
    let signature = std::fs::read_to_string(signature_path)
        .with_context(|| format!("Failed to read {}", signature_path.display()))?;
    let certificate = std::fs::read_to_string(certificate_path)
        .with_context(|| format!("Failed to read {}", certificate_path.display()))?;

    if let Err(failure) = verify_checksums_signature(&VerificationContext {
        blob: checksums.as_bytes(),
        certificate_pem: &certificate,
        signature: &signature,
        expected_identity: &manifest.cosign_identity,
        expected_issuer: &manifest.cosign_issuer,
    })
    .await
    {
        handle_signature_verification_failure(failure, allow_unverified)?;
    }

    let parsed_checksums = parse_sha256sums(&checksums)?;
    let archive_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid archive path {}", archive_path.display()))?;
    let expected_checksum = parsed_checksums.get(archive_name).ok_or_else(|| {
        anyhow::anyhow!("Verified SHA256SUMS did not contain an entry for {archive_name}")
    })?;
    let actual_checksum = sha256_file_hex(archive_path)?;

    if actual_checksum != *expected_checksum {
        bail!(
            "Archive checksum mismatch for {archive_name}: expected {expected_checksum}, got {actual_checksum}"
        );
    }

    Ok(())
}

fn replace_current_binary(new_binary: &Path) -> Result<()> {
    self_replace::self_replace(new_binary).with_context(|| {
        format!(
            "Failed to replace the current nyxid binary with {}",
            new_binary.display()
        )
    })?;

    Ok(())
}

async fn update_from_source(version: Option<&str>) -> Result<CliUpdateResult> {
    eprintln!("Falling back to source build via cargo install...");

    let mut command = tokio::process::Command::new("cargo");
    command.args(["install", "--git", REPO_URL]);

    if let Some(version) = version {
        command.args(["--tag", &format!("v{}", normalize_version(version))]);
    }

    command.args(["nyxid-cli", "--force", "--locked"]);

    let status = command
        .status()
        .await
        .context("Failed to run cargo install. Is cargo available?")?;

    if !status.success() {
        bail!("cargo install failed with exit code {}", status);
    }

    let version = version
        .map(normalize_version)
        .unwrap_or(CLI_VERSION)
        .to_string();

    Ok(CliUpdateResult::UpdatedFromSource(version))
}

async fn update_skills(base_url: &str) -> Result<()> {
    super::ai_setup::run(crate::cli::AiSetupCommands::Update {
        tool: None,
        base_url: Some(base_url.to_string()),
    })
    .await
}

enum CliUpdateResult {
    AlreadyCurrent(String),
    Updated(String),
    UpdatedFromSource(String),
}

enum PrebuiltUpdateError {
    NoBinary(String),
    Other(anyhow::Error),
}
