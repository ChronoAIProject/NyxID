use std::process::Command;

#[path = "../build-support/source_identity.rs"]
mod source_identity;

fn main() {
    source_identity::emit();

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
}
