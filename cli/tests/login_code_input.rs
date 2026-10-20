//! CLI-level regression for #1803. Synthetic codes only; never dump captured IO.
use std::{path::Path, process::Stdio};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn command(home: &Path, server: &str, json: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
    command
        .args(["login", "--code", "--base-url", server])
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_TELEMETRY_DSN")
        .env_remove("NYXID_SHARE_ANALYTICS")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_URL")
        .env_remove("NYXID_API_KEY")
        .env_remove("NYXID_ACCESS_TOKEN")
        .env_remove("NYXID_LOGIN_NO_DEVICE_FALLBACK")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if json {
        command.args(["--output", "json"]);
    }
    command
}

#[tokio::test]
async fn noninteractive_code_input_is_a_local_error_without_a_request_in_both_formats() {
    for json in [false, true] {
        for piped in [false, true] {
            let server = MockServer::start().await;
            let home = tempfile::tempdir().unwrap();
            let mut command = command(home.path(), &server.uri(), json);
            if piped {
                command.stdin(Stdio::piped());
            }
            let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(output.status.code(), Some(22));
            assert!(server.received_requests().await.unwrap().is_empty());
            let message = if json {
                let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(value["error"]["code"], "login_input_unavailable");
                assert_eq!(value["error"]["diagnostic"]["stage"], "input");
                assert_eq!(value["error"]["diagnostic"]["reason"], "stdin_not_terminal");
                value["error"]["diagnostic"]["hint"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            } else {
                assert!(output.stdout.is_empty());
                let message = String::from_utf8(output.stderr).unwrap();
                assert!(message.contains("stdin_not_terminal"));
                message
            };
            assert!(message.contains("interactive terminal"));
            assert!(message.contains("nyxid login (device flow)"));
            assert!(message.contains("nyxid login --callback"));
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs::File,
        io::{Read, Write},
        os::fd::{AsRawFd, FromRawFd},
        os::unix::process::ExitStatusExt,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    use tokio::io::AsyncReadExt;

    fn pty() -> (File, File) {
        let (mut master, mut slave) = (-1, -1);
        // SAFETY: output pointers are valid; null inputs request default settings.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            0
        );
        // SAFETY: successful openpty returns two new owned descriptors.
        unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) }
    }

    fn mode(file: &File) -> libc::termios {
        let mut mode = std::mem::MaybeUninit::uninit();
        // SAFETY: valid fd and output pointer, initialized on success.
        assert_eq!(
            unsafe { libc::tcgetattr(file.as_raw_fd(), mode.as_mut_ptr()) },
            0
        );
        unsafe { mode.assume_init() }
    }

    // Real PTYs verify the syscall adapter. Mode/error/cancellation edge cases
    // use the injected Terminal seam in auth/login_input/tests.rs.
    #[derive(Clone, Copy, PartialEq)]
    enum Finish {
        InvalidCode,
        CtrlC,
        SignalDuringInput,
        SignalDuringRedeem,
    }

    async fn attempt(controlling: bool, json: bool, finish: Finish) {
        let server = MockServer::start().await;
        let redeem_started = Arc::new(AtomicBool::new(false));
        let started = redeem_started.clone();
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/login-code/redeem"))
            .respond_with(move |_: &wiremock::Request| {
                started.store(true, Ordering::SeqCst);
                let response = ResponseTemplate::new(400).set_body_json(serde_json::json!({
                    "error_code": 12000, "message": "must not reflect AAAA-BBBB"
                }));
                if finish == Finish::SignalDuringRedeem {
                    response.set_delay(std::time::Duration::from_secs(60))
                } else {
                    response
                }
            })
            .mount(&server)
            .await;
        let home = tempfile::tempdir().unwrap();
        let (mut master, slave) = pty();
        let original = mode(&slave);
        let mut command = command(home.path(), &server.uri(), json);
        command.stdin(slave.try_clone().unwrap());
        // SAFETY: only async-signal-safe syscalls between fork and exec; no secrets.
        unsafe {
            command.pre_exec(move || {
                // Test restoration against a known original disposition even
                // when the test runner itself inherited an ignored SIGINT.
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if controlling && libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let tty = libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR);
                if tty >= 0 {
                    libc::close(tty);
                }
                if (tty >= 0) != controlling {
                    return Err(std::io::Error::other("unexpected controlling terminal"));
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        let mut prompt = vec![0; b"One-time login code (hidden): ".len()];
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            child.stderr.as_mut().unwrap().read_exact(&mut prompt),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(prompt == b"One-time login code (hidden): ");
        assert_eq!(mode(&slave).c_lflag & (libc::ECHO | libc::ECHONL), 0);
        match finish {
            Finish::InvalidCode => master.write_all(b"AAAA-BBBB\n").unwrap(),
            Finish::CtrlC => master.write_all(b"AAAA\x03").unwrap(),
            Finish::SignalDuringInput | Finish::SignalDuringRedeem => {
                if finish == Finish::SignalDuringRedeem {
                    master.write_all(b"AAAA-BBBB\n").unwrap();
                    tokio::time::timeout(std::time::Duration::from_secs(30), async {
                        while !redeem_started.load(Ordering::SeqCst) {
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        }
                    })
                    .await
                    .unwrap();
                } else {
                    master.write_all(b"AAAA").unwrap();
                }
                // SAFETY: signal only this owned test child.
                assert_eq!(
                    unsafe { libc::kill(child.id().unwrap() as _, libc::SIGINT) },
                    0
                );
            }
        }
        let output =
            tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
                .await
                .unwrap()
                .unwrap();
        // On macOS the session leader's exit revokes the slave descriptor.
        // The master still exposes the same PTY settings, so read back there.
        let after = mode(&master);
        assert_eq!(after.c_lflag, original.c_lflag);
        assert_eq!(after.c_iflag, original.c_iflag);
        assert_eq!(after.c_cc, original.c_cc);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("AAAA"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("AAAA"));
        // No typed bytes were echoed to the PTY, including on cancellation.
        // SAFETY: master is an owned, valid descriptor.
        unsafe {
            libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
        }
        let mut echoed = [0u8; 128];
        match master.read(&mut echoed) {
            Ok(count) => assert_eq!(count, 0), // Revoked controlling PTY: EOF.
            Err(error) => assert!(
                error.kind() == std::io::ErrorKind::WouldBlock
                    || (controlling && error.raw_os_error() == Some(libc::EIO))
            ),
        }
        let requests = server.received_requests().await.unwrap();
        if finish == Finish::SignalDuringRedeem {
            assert_eq!(output.status.signal(), Some(libc::SIGINT));
        } else if finish != Finish::InvalidCode {
            assert_eq!(output.status.code(), Some(22));
            assert!(requests.is_empty());
            if json {
                let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(value["error"]["code"], "login_input_unavailable");
                assert_eq!(value["error"]["diagnostic"]["reason"], "input_interrupted");
            }
        } else {
            assert_eq!(output.status.code(), Some(18));
            assert_eq!(requests.len(), 1);
            let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert!(body["code"] == "AAAA-BBBB");
            if json {
                let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(value["error"]["code"], "login_code_invalid");
            } else {
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("The login code is invalid or cancelled.")
                );
            }
        }
    }

    #[tokio::test]
    async fn controlling_tty_reads_without_echo_and_preserves_server_invalid_code() {
        for json in [false, true] {
            attempt(true, json, Finish::InvalidCode).await;
        }
    }

    #[tokio::test]
    async fn pty_without_dev_tty_reads_without_echo_in_both_output_formats() {
        for json in [false, true] {
            attempt(false, json, Finish::InvalidCode).await;
        }
    }

    #[tokio::test]
    async fn ctrl_c_and_external_sigint_restore_echo_without_redeeming() {
        for controlling in [false, true] {
            for finish in [Finish::CtrlC, Finish::SignalDuringInput] {
                attempt(controlling, true, finish).await;
            }
        }
    }

    #[tokio::test]
    async fn original_signal_handling_is_restored_before_redeeming() {
        attempt(false, true, Finish::SignalDuringRedeem).await;
    }
}
