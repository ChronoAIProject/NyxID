use anyhow::Result;

use crate::cli::RepoArgs;

pub const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");
pub const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_HASH: &str = env!("NYXID_GIT_HASH");

pub fn issues_url() -> String {
    format!("{REPO_URL}/issues")
}

pub async fn run_repo(args: RepoArgs) -> Result<()> {
    println!("{REPO_URL}");
    if args.open
        && let Err(e) = open::that(REPO_URL)
    {
        eprintln!("Failed to open browser: {e}");
    }
    Ok(())
}

pub async fn run_info() -> Result<()> {
    println!("NyxID CLI v{CLI_VERSION} ({GIT_HASH})");
    println!("Repo:   {REPO_URL}");
    println!("Issues: {}", issues_url());
    Ok(())
}
