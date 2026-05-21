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
        && let Err(e) = crate::browser::open_browser(REPO_URL)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_url_appends_issues_path() {
        assert_eq!(issues_url(), format!("{REPO_URL}/issues"));
        assert!(issues_url().ends_with("/issues"));
    }

    #[tokio::test]
    async fn run_repo_without_open_succeeds() {
        // open: false → prints the URL, no browser launch.
        run_repo(RepoArgs { open: false })
            .await
            .expect("repo should succeed");
    }

    #[tokio::test]
    async fn run_info_succeeds() {
        run_info().await.expect("info should succeed");
    }
}
