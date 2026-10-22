use std::process::Command;

fn git_output(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
}

fn watch_git_metadata() {
    println!("cargo:rerun-if-changed=build.rs");
    if let Some(head) = git_output(&["rev-parse", "--git-path", "HEAD"])
        && std::path::Path::new(&head).exists()
    {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(reference) = git_output(&["symbolic-ref", "--quiet", "HEAD"])
        && let Some(path) = git_output(&["rev-parse", "--git-path", &reference])
    {
        let mut path = std::path::PathBuf::from(path);
        if !path.exists() {
            if let Some(packed) = git_output(&["rev-parse", "--git-path", "packed-refs"])
                && std::path::Path::new(&packed).exists()
            {
                println!("cargo:rerun-if-changed={packed}");
            }
            // A commit creates a loose reference when the branch is packed.
            while !path.exists() && path.pop() {}
        }
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn main() {
    watch_git_metadata();
    println!("cargo:rerun-if-env-changed=NYXID_GIT_HASH");

    // macOS integration tests link the real node runtime's ScreenCaptureKit
    // bridge. Cargo does not propagate dependency build-script rpaths.
    if std::env::var("TARGET").is_ok_and(|target| target.contains("apple-darwin")) {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }

    // If the caller already provided NYXID_GIT_HASH (e.g. Docker build arg in
    // CI, where .git is not in the build context), honor it verbatim.
    let full = std::env::var("NYXID_GIT_HASH")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(resolve_from_git);

    println!("cargo:rustc-env=NYXID_GIT_HASH={full}");
}

fn resolve_from_git() -> String {
    let hash =
        git_output(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".to_string());

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
