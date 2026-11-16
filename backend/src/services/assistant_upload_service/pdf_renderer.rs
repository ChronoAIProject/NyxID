//! Poppler is an explicit runtime dependency, executed without secrets or a shell.
//! One page per worker bounds pixel memory even for hostile document dimensions.
use std::{path::Path, process::Stdio, sync::LazyLock, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Semaphore,
};

pub const VERSION: &str = "poppler-jpeg-2048-v1";
pub const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PAGES: usize = 8;
pub const RESERVATION: usize = 80 * 1024 * 1024; // weight=160 MiB: worker 128 + PDF 20 + buffers
static WORKERS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));

#[derive(Clone, Copy, Debug)]
pub(super) enum Failure {
    Unavailable,
    Timeout,
    Limit,
    Invalid,
}
impl Failure {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "renderer_unavailable",
            Self::Timeout => "render_timeout",
            Self::Limit => "render_limit",
            Self::Invalid => "render_failed",
        }
    }
}
type RenderResult<T> = Result<T, Failure>;
fn failed() -> Failure {
    Failure::Invalid
}

/// Kill descendants as well as the converter on timeout, Stop or future drop.
struct ProcessGroup(u32);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}

pub(super) async fn render(bytes: &[u8], page: u32) -> RenderResult<Vec<u8>> {
    let path = std::env::var_os("NYXID_PDF_RENDERER").unwrap_or_else(|| "/usr/bin/pdftoppm".into());
    render_with(Path::new(&path), bytes, page, Duration::from_secs(8)).await
}

async fn render_with(
    path: &Path,
    bytes: &[u8],
    page: u32,
    deadline: Duration,
) -> RenderResult<Vec<u8>> {
    if !path.is_absolute()
        || page == 0
        || page > 200
        || bytes.len() > super::attachment_extraction::MAX_BYTES
    {
        return Err(failed());
    }
    let _worker = WORKERS.acquire().await.map_err(|_| failed())?;
    let mut command = tokio::process::Command::new(path);
    command
        .args([
            "-f",
            &page.to_string(),
            "-l",
            &page.to_string(),
            "-singlefile",
            "-scale-to",
            "2048",
            "-jpeg",
            "-jpegopt",
            "quality=85",
            "-",
        ])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    unsafe {
        command.process_group(0).pre_exec(|| {
            let cpu = libc::rlimit {
                rlim_cur: 6,
                rlim_max: 6,
            };
            if libc::setrlimit(libc::RLIMIT_CPU, &cpu) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            {
                let memory = libc::rlimit {
                    rlim_cur: 128 * 1024 * 1024,
                    rlim_max: 128 * 1024 * 1024,
                };
                if libc::setrlimit(libc::RLIMIT_AS, &memory) != 0
                    || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|_| Failure::Unavailable)?;
    let group = ProcessGroup(child.id().ok_or_else(failed)?);
    let mut stdin = child.stdin.take().ok_or_else(failed)?;
    let stdout = child.stdout.take().ok_or_else(failed)?;
    let result = tokio::time::timeout(deadline, async {
        let write = async {
            stdin.write_all(bytes).await?;
            stdin.shutdown().await?;
            drop(stdin);
            Ok::<_, std::io::Error>(())
        };
        let read = async {
            let mut output = Vec::new();
            stdout
                .take((MAX_PAGE_BYTES + 1) as u64)
                .read_to_end(&mut output)
                .await?;
            Ok::<_, std::io::Error>(output)
        };
        let (_, output) = tokio::try_join!(write, read).map_err(|_| failed())?;
        if output.len() > MAX_PAGE_BYTES {
            return Err(Failure::Limit);
        }
        if !child.wait().await.map_err(|_| failed())?.success() {
            return Err(Failure::Limit);
        }
        let reader = image::ImageReader::with_format(
            std::io::Cursor::new(&output),
            image::ImageFormat::Jpeg,
        );
        let (w, h) = reader.into_dimensions().map_err(|_| failed())?;
        if w == 0 || h == 0 || w > 2048 || h > 2048 {
            return Err(failed());
        }
        Ok(output)
    })
    .await
    .unwrap_or(Err(Failure::Timeout));
    drop(group);
    // Reap on every explicit exit; kill_on_drop also handles cancellation.
    if child.id().is_some() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_invalid_pages_missing_converter_and_timeout() {
        assert!(
            render_with(Path::new("relative"), b"pdf", 1, Duration::from_secs(1))
                .await
                .is_err()
        );
        assert!(render(b"pdf", 0).await.is_err());
        assert!(
            render_with(
                Path::new("/missing-nyxid-renderer"),
                b"pdf",
                1,
                Duration::from_secs(1)
            )
            .await
            .is_err()
        );
        assert!(
            render_with(Path::new("/bin/cat"), b"pdf", 1, Duration::ZERO)
                .await
                .is_err()
        );
    }
}

#[cfg(all(test, unix))]
mod cancellation_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn pdf_stop_and_timeout_kill_converter_process_tree_and_release_slot() {
        for stop in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("converter");
            let pid_file = dir.path().join("child.pid");
            std::fs::write(
                &script,
                format!(
                    "#!/bin/sh\n/bin/sleep 60 &\necho $! > '{}'\nwait\n",
                    pid_file.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            let task = tokio::spawn(async move {
                render_with(&script, b"%PDF-test", 1, Duration::from_secs(5)).await
            });
            let child = tokio::time::timeout(Duration::from_secs(4), async {
                loop {
                    if let Ok(pid) = std::fs::read_to_string(&pid_file)
                        && let Ok(pid) = pid.trim().parse::<i32>()
                    {
                        break pid;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            if stop {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert!(task.await.unwrap().is_err());
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if unsafe { libc::kill(child, 0) } != 0 {
                        break;
                    }
                    // On Linux a killed grandchild may briefly remain as a zombie.
                    #[cfg(target_os = "linux")]
                    if std::fs::read_to_string(format!("/proc/{child}/stat"))
                        .is_ok_and(|s| s.contains(") Z "))
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("converter child must not survive Stop/timeout");
        }
        assert_eq!(WORKERS.available_permits(), 2);
    }

    #[tokio::test]
    async fn pdf_real_converter_rotates_and_bounds_pixels_and_encoded_bytes() {
        let pdf = super::super::super::attachment_extraction::pdf_fixture(&[None], 90);
        let _budget = super::super::byte_budget::reserve(RESERVATION)
            .await
            .unwrap();
        let bytes = render(&pdf, 1).await.unwrap();
        assert!(bytes.len() <= MAX_PAGE_BYTES);
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (2048, 1536));
    }
}
