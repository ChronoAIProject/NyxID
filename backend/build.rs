use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-env-changed=NYXID_GIT_HASH");

    // If the caller already provided NYXID_GIT_HASH (e.g. Docker build arg in
    // CI, where .git is not in the build context), honor it verbatim.
    let full = std::env::var("NYXID_GIT_HASH")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(resolve_from_git);

    println!("cargo:rustc-env=NYXID_GIT_HASH={full}");
}

fn resolve_from_git() -> String {
    let hash = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    if dirty && hash != "unknown" {
        format!("{hash}-dirty")
    } else {
        hash
    }
}
