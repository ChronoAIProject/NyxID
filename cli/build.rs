use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=NYXID_GIT_HASH");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-changed=build.rs");
    watch_git_state();

    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET={target}");

    // ScreenCaptureKit's Swift bridge needs the toolchain compatibility
    // archives as well as the system Swift runtime. The SDK's lib/swift path
    // alone does not contain them on Command Line Tools installations.
    if target.contains("apple-darwin") {
        for variable in ["DEVELOPER_DIR", "TOOLCHAINS", "SDKROOT"] {
            println!("cargo:rerun-if-env-changed={variable}");
        }
        // Dependency build-script link arguments do not propagate to binaries.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        let swift = Command::new("xcrun")
            .args(["--find", "swiftc"])
            .output()
            .expect("macOS builds require the Xcode Swift toolchain");
        assert!(swift.status.success(), "xcrun could not locate swiftc");
        let executable = std::path::PathBuf::from(
            String::from_utf8(swift.stdout)
                .expect("Swift path is UTF-8")
                .trim(),
        );
        let libraries = executable
            .parent()
            .and_then(std::path::Path::parent)
            .expect("Swift toolchain directory")
            .join("lib/swift/macosx");
        println!("cargo:rustc-link-search=native={}", libraries.display());
    }

    let full = resolve_from_git();
    println!("cargo:rustc-env=NYXID_GIT_HASH={full}");
}

fn watch_git_state() {
    if git_output(&["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return;
    }
    // HEAD and its reflog belong to this worktree. Watching the shared refs
    // directory or packed-refs would rebuild for unrelated branch updates.
    watch_git_path("HEAD");
    if !watch_git_path("logs/HEAD")
        && !git_output(&["symbolic-ref", "-q", "HEAD"])
            .is_some_and(|reference| watch_git_path(&reference))
    {
        // With no reflog or loose ref, a commit can leave every watched file
        // unchanged. A missing reflog watch makes Cargo rerun until one exists
        // or the commit creates a loose ref that we can watch instead.
        let reflog = git_output(&["rev-parse", "--git-path", "logs/HEAD"])
            // build.rs is a file, so this child cannot exist even if Git fails.
            .unwrap_or_else(|| "build.rs/git-stamp-unavailable".to_string());
        println!("cargo:rerun-if-changed={reflog}");
    }
}

fn watch_git_path(name: &str) -> bool {
    if let Some(path) = git_output(&["rev-parse", "--git-path", name])
        .and_then(|path| std::fs::canonicalize(path).ok())
        .filter(|path| path.is_file())
    {
        println!("cargo:rerun-if-changed={}", path.display());
        return true;
    }
    false
}

fn git_output(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
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
