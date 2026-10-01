//! cua's macOS screencapture fallback needs a filename. Give it a private RAM
//! volume, never the machine's disk. The driver still owns capture and input.
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub struct MemoryCapture {
    temporary: Option<tempfile::TempDir>,
    device: String,
}
impl MemoryCapture {
    pub async fn create() -> Result<Self> {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            Command::new("/usr/bin/hdiutil")
                .args(["attach", "-nomount", "ram://262144"])
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        if !output.status.success() {
            bail!("cua memory-only capture volume unavailable");
        }
        let device = device_name(&output.stdout)?;
        let mut memory = Self {
            temporary: None,
            device,
        };
        let label = format!("NyxID-Cua-{}", uuid::Uuid::new_v4().simple());
        let status = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            Command::new("/usr/sbin/diskutil")
                .args(["eraseVolume", "HFS+", &label, &memory.device])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await??;
        if !status.success() {
            bail!("cua memory-only capture volume could not be mounted");
        }
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            Command::new("/usr/sbin/diskutil")
                .args(["info", "-plist", &memory.device])
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        if !output.status.success() {
            bail!("cua memory-only capture volume metadata unavailable");
        }
        let info = plist::Value::from_reader(std::io::Cursor::new(output.stdout))?;
        let mount = info
            .as_dictionary()
            .and_then(|d| d.get("MountPoint"))
            .and_then(plist::Value::as_string)
            .context("cua memory-only capture mount unavailable")?;
        let path = PathBuf::from(mount);
        if path.file_name().and_then(|s| s.to_str()) != Some(label.as_str()) {
            bail!("cua memory-only capture mount changed");
        }
        memory.temporary = Some(
            tempfile::Builder::new()
                .prefix("capture-")
                .tempdir_in(path)?,
        );
        Ok(memory)
    }
    pub fn path(&self) -> &Path {
        self.temporary
            .as_ref()
            .expect("mounted capture volume")
            .path()
    }
}
fn device_name(bytes: &[u8]) -> Result<String> {
    let value = std::str::from_utf8(bytes)?.trim();
    if !value
        .strip_prefix("/dev/disk")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit()))
    {
        bail!("unexpected cua RAM volume device");
    }
    Ok(value.into())
}
impl Drop for MemoryCapture {
    fn drop(&mut self) {
        // Only the fresh ram:// device returned above can reach this command.
        // Remove temporary files before detaching; no disk-backed fallback.
        drop(self.temporary.take());
        let child = std::process::Command::new("/usr/bin/hdiutil")
            .args(["detach", "-force", &self.device])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            // Volume teardown must not block the async executor or shutdown.
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    if !matches!(child.try_wait(), Ok(None)) {
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_whole_device_returned_by_ram_attach_is_accepted() {
        assert_eq!(device_name(b"/dev/disk12  \n").unwrap(), "/dev/disk12");
        for invalid in [
            "/dev/disk",
            "/dev/disk1s1",
            "/dev/disk1\n/dev/disk2",
            "/dev/disk1;evil",
            "/",
        ] {
            assert!(device_name(invalid.as_bytes()).is_err());
        }
    }
    #[tokio::test]
    #[ignore = "mounts and detaches a private RAM disk; run alongside macOS desktop benchmark"]
    async fn capture_files_live_only_on_the_owned_ram_volume() {
        let capture = MemoryCapture::create().await.unwrap();
        let directory = capture.path().to_owned();
        std::fs::write(directory.join("synthetic-frame"), b"pixels").unwrap();
        assert!(directory.starts_with("/Volumes"));
        drop(capture);
        assert!(!directory.exists());
    }
}
