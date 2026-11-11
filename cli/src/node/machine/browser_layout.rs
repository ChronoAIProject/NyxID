//! Package-backed Linux policy locations. Never infer a layout from an arbitrary
//! executable's basename or install a compatibility symlink.
use anyhow::{Context, Result, bail, ensure};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    DebianChromium,
    SnapChromium,
    GoogleChrome,
}
impl Layout {
    pub fn policies(self) -> &'static str {
        match self {
            Self::DebianChromium => "etc/chromium/policies/managed",
            Self::SnapChromium => "etc/chromium-browser/policies/managed",
            Self::GoogleChrome => "etc/opt/chrome/policies/managed",
        }
    }
    pub fn native_hosts(self) -> &'static str {
        match self {
            Self::DebianChromium | Self::SnapChromium => "etc/chromium/native-messaging-hosts",
            Self::GoogleChrome => "etc/opt/chrome/native-messaging-hosts",
        }
    }
    pub fn detect(binary: &Path) -> Result<Self> {
        Self::detect_at(Path::new("/"), binary)
    }
    fn detect_at(root: &Path, binary: &Path) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let root = root.canonicalize()?;
        let root = root.as_path();
        // The real system root is owned by root even when an owner invokes
        // setup before its sudo installation step. Tests use a private root.
        let owner = root.metadata()?.uid();
        let resolved = root
            .join(binary.strip_prefix("/")?)
            .canonicalize()
            .context(
                "managed_browser_policy_layout_unknown: configured browser cannot be resolved",
            )?;
        let relative = resolved.strip_prefix(root)?.to_string_lossy();
        let snap = relative
            .strip_prefix("snap/chromium/")
            .and_then(|p| p.strip_suffix("/usr/lib/chromium-browser/chrome"));
        if let Some(revision) =
            snap.filter(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
        {
            let metadata = root.join(format!("snap/chromium/{revision}/meta/snap.yaml"));
            if trusted_text(&metadata, owner)?
                .lines()
                .any(|line| line == "name: chromium")
            {
                return Ok(Self::SnapChromium);
            }
        }
        for (package, paths, layout) in [
            (
                "chromium",
                &["usr/bin/chromium", "usr/lib/chromium/chromium"][..],
                Self::DebianChromium,
            ),
            (
                "google-chrome-stable",
                &[
                    "opt/google/chrome/google-chrome",
                    "opt/google/chrome/chrome",
                ][..],
                Self::GoogleChrome,
            ),
        ] {
            if !paths.contains(&relative.as_ref()) {
                continue;
            }
            let directory = root.join("var/lib/dpkg/info");
            for entry in std::fs::read_dir(&directory).into_iter().flatten() {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name != format!("{package}.list")
                    && !(name.starts_with(&format!("{package}:")) && name.ends_with(".list"))
                {
                    continue;
                }
                if trusted_text(&entry.path(), owner)?
                    .lines()
                    .any(|line| line.strip_prefix('/') == Some(relative.as_ref()))
                {
                    return Ok(layout);
                }
            }
        }
        bail!(
            "managed_browser_policy_layout_unknown: the configured executable is not a recognized Debian Chromium, Ubuntu Snap Chromium or packaged Google Chrome layout; install a supported browser package and rerun setup"
        )
    }
}
fn trusted_text(path: &Path, owner: u32) -> Result<String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == owner
            && metadata.mode() & 0o022 == 0
            && metadata.len() < 8 * 1024 * 1024,
        "managed_browser_package_metadata_untrusted"
    );
    use std::io::Read;
    let mut text = String::new();
    file.take(8 * 1024 * 1024).read_to_string(&mut text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn put(root: &Path, path: &str, text: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        // Package metadata is immutable to other users, independent of the
        // test runner's umask (Lima's regular login defaults to group writable).
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    #[test]
    fn package_layouts_follow_resolved_executables_and_reject_unknowns() {
        let root = tempfile::tempdir().unwrap();
        for (binary, package, layout) in [
            (
                "usr/lib/chromium/chromium",
                "chromium",
                Layout::DebianChromium,
            ),
            (
                "opt/google/chrome/chrome",
                "google-chrome-stable",
                Layout::GoogleChrome,
            ),
        ] {
            put(root.path(), binary, "binary");
            assert!(Layout::detect_at(root.path(), &PathBuf::from(format!("/{binary}"))).is_err());
            put(
                root.path(),
                &format!("var/lib/dpkg/info/{package}.list"),
                &format!("/{binary}\n"),
            );
            assert_eq!(
                Layout::detect_at(root.path(), &PathBuf::from(format!("/{binary}"))).unwrap(),
                layout
            );
            use std::os::unix::fs::PermissionsExt;
            let metadata = root
                .path()
                .join(format!("var/lib/dpkg/info/{package}.list"));
            std::fs::set_permissions(&metadata, std::fs::Permissions::from_mode(0o666)).unwrap();
            assert!(Layout::detect_at(root.path(), &PathBuf::from(format!("/{binary}"))).is_err());
            std::fs::set_permissions(metadata, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        put(
            root.path(),
            "snap/chromium/123/usr/lib/chromium-browser/chrome",
            "binary",
        );
        put(
            root.path(),
            "snap/chromium/123/meta/snap.yaml",
            "name: chromium\n",
        );
        put(root.path(), "usr/bin/unknown-chromium", "binary");
        std::os::unix::fs::symlink(
            "../../snap/chromium/123/usr/lib/chromium-browser/chrome",
            root.path().join("usr/bin/chromium"),
        )
        .unwrap();
        assert_eq!(
            Layout::detect_at(root.path(), Path::new("/usr/bin/chromium")).unwrap(),
            Layout::SnapChromium
        );
        assert!(Layout::detect_at(root.path(), Path::new("/usr/bin/unknown-chromium")).is_err());
        put(
            root.path(),
            "snap/chromium/123/meta/snap.yaml",
            "name: another-browser\n",
        );
        assert!(Layout::detect_at(root.path(), Path::new("/usr/bin/chromium")).is_err());
    }
}
