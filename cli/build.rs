use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
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

    let full = if dirty && hash != "unknown" {
        format!("{hash}-dirty")
    } else {
        hash
    };

    println!("cargo:rustc-env=NYXID_GIT_HASH={full}");
}
