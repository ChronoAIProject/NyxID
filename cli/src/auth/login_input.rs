//! Hidden one-time-code input. Never attach IO messages or entered bytes to errors.
use zeroize::{Zeroize, Zeroizing};

use super::login_exchange::{LoginError, LoginFailure};
use crate::net_diagnostics::{Diagnostic, Stage};

fn unavailable(reason: &'static str) -> LoginFailure {
    let mut diagnostic = Diagnostic::new(Stage::Input, None, None, reason);
    diagnostic.reason = Some(reason);
    diagnostic.causes.clear();
    LoginFailure {
        kind: LoginError::InputUnavailable,
        diagnostic: Some(Box::new(diagnostic)),
    }
}

#[cfg(test)]
mod tests;

#[cfg(unix)]
pub(super) async fn read_code() -> Result<Zeroizing<String>, LoginFailure> {
    let mut terminal = unix::StdinTerminal::open().map_err(unavailable)?;
    // Installed before echo changes, dropped after read_hidden restores it.
    // Restore previous handlers too: Ctrl-C during the later HTTP exchange
    // must not be swallowed by process-global Tokio signal registrations.
    let signals = unix::Signals::install().map_err(|_| unavailable("hidden_input_unavailable"))?;
    let result = read_hidden(&mut terminal, signals.interrupted()).await;
    // A broken prompt stream must not panic before JSON can report the error.
    let _ = std::io::Write::write_all(&mut std::io::stderr(), b"\n");
    result
}

#[cfg(not(unix))]
pub(super) async fn read_code() -> Result<Zeroizing<String>, LoginFailure> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(unavailable("stdin_not_terminal"));
    }
    // Retain the existing native console implementation on non-Unix platforms.
    rpassword::prompt_password("One-time login code: ")
        .map(Zeroizing::new)
        .map_err(|_| unavailable("hidden_input_unavailable"))
}

#[cfg(any(unix, test))]
enum Input {
    Pending,
    Byte(u8),
    Eof,
}

/// This seam deliberately has no controlling-terminal dependency. Both an
/// ordinary terminal and a managed PTY use the same stdin-fd operations.
#[cfg(any(unix, test))]
trait Terminal {
    type Mode;
    fn mode(&mut self) -> std::io::Result<Self::Mode>;
    /// Must verify no-echo mode after applying it; fail before reading otherwise.
    fn hide(&mut self, original: &Self::Mode) -> std::io::Result<()>;
    fn restore(&mut self, original: &Self::Mode) -> std::io::Result<()>;
    fn prompt(&mut self) -> std::io::Result<()>;
    /// Nonblocking, so cancellation never leaves an abandoned blocking reader.
    fn read(&mut self) -> std::io::Result<Input>;
}

#[cfg(any(unix, test))]
struct Restore<'a, T: Terminal> {
    terminal: &'a mut T,
    original: Option<T::Mode>,
}

#[cfg(any(unix, test))]
impl<T: Terminal> Restore<'_, T> {
    fn restore(&mut self) -> Result<(), LoginFailure> {
        if let Some(original) = &self.original {
            self.terminal
                .restore(original)
                .map_err(|_| unavailable("terminal_restore_failed"))?;
            self.original = None;
        }
        Ok(())
    }
}

#[cfg(any(unix, test))]
impl<T: Terminal> Drop for Restore<'_, T> {
    fn drop(&mut self) {
        // Covers cancellation/panic and retries a failed explicit restoration.
        let _ = self.restore();
    }
}

#[cfg(any(unix, test))]
async fn read_hidden(
    terminal: &mut impl Terminal,
    interrupted: impl std::future::Future<Output = ()>,
) -> Result<Zeroizing<String>, LoginFailure> {
    let original = terminal
        .mode()
        .map_err(|_| unavailable("hidden_input_unavailable"))?;
    let mut guard = Restore {
        terminal,
        original: Some(original),
    };
    // Arm restoration BEFORE the attempted mode change, including partial errors.
    let result = async {
        guard
            .terminal
            .hide(guard.original.as_ref().expect("saved terminal mode"))
            .map_err(|_| unavailable("hidden_input_unavailable"))?;
        guard
            .terminal
            .prompt()
            .map_err(|_| unavailable("prompt_failed"))?;
        tokio::select! {
            biased;
            _ = interrupted => Err(unavailable("input_interrupted")),
            result = read_bytes(guard.terminal) => result,
        }
    }
    .await;
    guard.restore()?;
    result
}

#[cfg(any(unix, test))]
async fn read_bytes(terminal: &mut impl Terminal) -> Result<Zeroizing<String>, LoginFailure> {
    // Bounded allocation: never reallocate a secret-bearing buffer.
    let mut bytes: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(256));
    loop {
        match terminal
            .read()
            .map_err(|_| unavailable("input_read_failed"))?
        {
            Input::Pending => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
            Input::Eof | Input::Byte(4) => return Err(unavailable("input_closed")),
            Input::Byte(3 | 26 | 28) => return Err(unavailable("input_interrupted")),
            Input::Byte(21) => {
                bytes.zeroize();
                bytes.clear();
            }
            Input::Byte(23) => {
                let mut end = bytes.len();
                while end > 0 && bytes[end - 1].is_ascii_whitespace() {
                    end -= 1;
                }
                while end > 0 {
                    end -= 1;
                    if bytes[end].is_ascii_whitespace() {
                        break;
                    }
                }
                bytes[end..].zeroize();
                bytes.truncate(end);
            }
            Input::Byte(b'\n' | b'\r') => {
                return std::str::from_utf8(&bytes)
                    .map(|code| Zeroizing::new(code.to_owned()))
                    .map_err(|_| unavailable("input_encoding_invalid"));
            }
            Input::Byte(8 | 127) => {
                while let Some(byte) = bytes.pop() {
                    if byte & 0xc0 != 0x80 {
                        break;
                    }
                }
            }
            Input::Byte(byte) if byte < 0x20 => {}
            Input::Byte(byte) => {
                if bytes.len() == 256 {
                    return Err(unavailable("input_too_long"));
                }
                bytes.push(byte);
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::{Input, Terminal};
    use std::{
        io::{self, IsTerminal, Write},
        os::fd::{AsFd, AsRawFd, OwnedFd},
        sync::atomic::{AtomicBool, Ordering},
    };

    static INPUT_BUSY: AtomicBool = AtomicBool::new(false);
    static INTERRUPTED: AtomicBool = AtomicBool::new(false);

    extern "C" fn interrupt(_: libc::c_int) {
        // Lock-free and signal-safe: all cleanup happens in the read future.
        INTERRUPTED.store(true, Ordering::SeqCst);
    }

    pub(super) struct Signals {
        previous: Vec<(libc::c_int, libc::sigaction)>,
    }

    impl Signals {
        pub(super) fn install() -> io::Result<Self> {
            INPUT_BUSY
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .map_err(|_| io::Error::other("input_busy"))?;
            INTERRUPTED.store(false, Ordering::SeqCst);
            let mut guard = Self {
                previous: Vec::new(),
            };
            for signal in [
                libc::SIGINT,
                libc::SIGTERM,
                libc::SIGHUP,
                libc::SIGQUIT,
                libc::SIGTSTP,
                libc::SIGTTIN,
                libc::SIGTTOU,
            ] {
                // SAFETY: zero initialization plus sigemptyset initializes the
                // handler, flags and mask. Both pointers remain valid throughout.
                let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
                action.sa_sigaction = interrupt as *const () as usize;
                let mut previous = std::mem::MaybeUninit::uninit();
                unsafe {
                    libc::sigemptyset(&mut action.sa_mask);
                    if libc::sigaction(signal, &action, previous.as_mut_ptr()) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                    guard.previous.push((signal, previous.assume_init()));
                }
            }
            Ok(guard)
        }

        pub(super) async fn interrupted(&self) {
            while !INTERRUPTED.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
    }

    impl Drop for Signals {
        fn drop(&mut self) {
            for (signal, action) in self.previous.iter().rev() {
                // SAFETY: restore the exact saved handler while input is exclusive.
                unsafe {
                    libc::sigaction(*signal, action, std::ptr::null_mut());
                }
            }
            INPUT_BUSY.store(false, Ordering::SeqCst);
        }
    }

    pub(super) struct StdinTerminal(OwnedFd);

    impl StdinTerminal {
        pub(super) fn open() -> Result<Self, &'static str> {
            let stdin = io::stdin();
            if !stdin.is_terminal() {
                return Err("stdin_not_terminal");
            }
            stdin
                .as_fd()
                .try_clone_to_owned()
                .map(Self)
                .map_err(|_| "hidden_input_unavailable")
        }

        fn set_mode(&self, mode: &libc::termios) -> io::Result<()> {
            // Flush unread type-ahead before restoring echo, including Ctrl-C.
            // SAFETY: the owned fd and termios reference remain valid for the call.
            loop {
                if unsafe { libc::tcsetattr(self.0.as_raw_fd(), libc::TCSAFLUSH, mode) } == 0 {
                    return Ok(());
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted || INTERRUPTED.load(Ordering::SeqCst)
                {
                    return Err(error);
                }
            }
        }
    }

    const HIDDEN_FLAGS: libc::tcflag_t =
        libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN;

    impl Terminal for StdinTerminal {
        type Mode = libc::termios;

        fn mode(&mut self) -> io::Result<Self::Mode> {
            let mut mode = std::mem::MaybeUninit::uninit();
            // SAFETY: tcgetattr writes a complete termios on success.
            if unsafe { libc::tcgetattr(self.0.as_raw_fd(), mode.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(unsafe { mode.assume_init() })
        }

        fn hide(&mut self, original: &Self::Mode) -> io::Result<()> {
            let mut hidden = *original;
            hidden.c_lflag &= !HIDDEN_FLAGS;
            hidden.c_cc[libc::VMIN] = 0;
            hidden.c_cc[libc::VTIME] = 0;
            self.set_mode(&hidden)?;
            let actual = self.mode()?;
            if actual.c_lflag & HIDDEN_FLAGS != 0
                || actual.c_cc[libc::VMIN] != 0
                || actual.c_cc[libc::VTIME] != 0
            {
                return Err(io::Error::other("hidden_input_unavailable"));
            }
            Ok(())
        }

        fn restore(&mut self, original: &Self::Mode) -> io::Result<()> {
            self.set_mode(original)?;
            let actual = self.mode()?;
            if actual.c_lflag != original.c_lflag
                || actual.c_cc[libc::VMIN] != original.c_cc[libc::VMIN]
                || actual.c_cc[libc::VTIME] != original.c_cc[libc::VTIME]
            {
                return Err(io::Error::other("terminal_restore_failed"));
            }
            Ok(())
        }

        fn prompt(&mut self) -> io::Result<()> {
            let mut stderr = io::stderr().lock();
            stderr.write_all(b"One-time login code (hidden): ")?;
            stderr.flush()
        }

        fn read(&mut self) -> io::Result<Input> {
            let fd = self.0.as_raw_fd();
            let mut event = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: event points to one initialized pollfd; zero timeout.
            match unsafe { libc::poll(&mut event, 1, 0) } {
                -1 => return Err(io::Error::last_os_error()),
                0 => return Ok(Input::Pending),
                _ => {}
            }
            let mut byte = [0u8];
            // SAFETY: buffer is writable and bounded, fd owned; VMIN/VTIME=0.
            match unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) } {
                1 => Ok(Input::Byte(byte[0])),
                0 => Ok(Input::Eof),
                _ => Err(io::Error::last_os_error()),
            }
        }
    }
}
