#[path = "../build-support/source_identity.rs"]
mod source_identity;

fn main() {
    source_identity::emit();

    // macOS integration tests link the real node runtime's ScreenCaptureKit
    // bridge. Cargo does not propagate dependency build-script rpaths.
    if std::env::var("TARGET").is_ok_and(|target| target.contains("apple-darwin")) {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
