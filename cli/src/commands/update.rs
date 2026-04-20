use anyhow::{Context, Result};

use crate::cli::UpdateArgs;
use crate::commands::repo::REPO_URL;

pub async fn run(args: UpdateArgs) -> Result<()> {
    if args.skills_only {
        return update_skills(&args.base_url).await;
    }

    update_cli().await?;
    update_skills(&args.base_url).await?;

    eprintln!();
    eprintln!("All up to date.");
    Ok(())
}

async fn update_cli() -> Result<()> {
    eprintln!("Updating NyxID CLI...");

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

    eprintln!("CLI updated.");
    Ok(())
}

async fn update_skills(base_url: &Option<String>) -> Result<()> {
    // Reuse the ai-setup update logic (updates all installed tools)
    super::ai_setup::run(crate::cli::AiSetupCommands::Update {
        tool: None,
        base_url: base_url.clone(),
    })
    .await
}
