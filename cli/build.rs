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
    println!("cargo:rerun-if-env-changed=TARGET");

    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET={target}");

    // ScreenCaptureKit's Swift bridge needs the toolchain compatibility
    // archives as well as the system Swift runtime. The SDK's lib/swift path
    // alone does not contain them on Command Line Tools installations.
    if target.contains("apple-darwin") {
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

    let hash =
        git_output(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".to_string());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    let full = if dirty && hash != "unknown" {
        format!("{hash}-dirty")
    } else {
        hash
    };

    println!("cargo:rustc-env=NYXID_GIT_HASH={full}");
}
