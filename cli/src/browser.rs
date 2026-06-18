//! WSL-aware browser launching.
//!
//! `open::that` is fine on native macOS / Linux / Windows, but under WSL
//! it only knows `wslview` — which ships in the `wslu` package and is
//! absent from a default Ubuntu WSL install. When `wslview` is missing
//! the `open` crate falls through to `xdg-open` / `gio` / `gnome-open` /
//! `kde-open`, none of which reach a Windows browser, so commands like
//! `nyxid service add` printed the wizard URL but never opened it
//! (issue #710).
//!
//! Under WSL we instead walk an explicit chain of Windows-side openers.
//! WSL is the documented Windows path for the CLI, so it should get the
//! same auto-open experience as macOS.

use std::process::Command;

pub type BrowserError = std::io::Error;

/// True when running inside Windows Subsystem for Linux (WSL 1 or 2).
///
/// Checks the interop env vars first — `WSL_INTEROP` / `WSL_DISTRO_NAME`
/// are set for the whole login session and are the cheapest signal. As a
/// fallback (e.g. a `sudo` shell that dropped the session env), the
/// kernel release string carries `microsoft` under both WSL 1 and WSL 2.
pub fn is_wsl() -> bool {
    if std::env::var_os("WSL_INTEROP").is_some() || std::env::var_os("WSL_DISTRO_NAME").is_some() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(osrelease) = std::fs::read_to_string("/proc/sys/kernel/osrelease") {
            return osrelease.to_ascii_lowercase().contains("microsoft");
        }
    }
    false
}

/// Open `url` in the user's default browser.
///
/// Native platforms go straight through `open::that`. Under WSL we use
/// [`open_browser_wsl`] so the URL actually reaches a Windows browser
/// instead of silently failing on a missing `wslview`.
pub fn open_browser(url: &str) -> Result<(), BrowserError> {
    if is_wsl() {
        open_browser_wsl(url)
    } else {
        open::that(url)
    }
}

/// WSL bridge: try each Windows-side opener until one launches the URL,
/// then fall back to the `open` crate's generic UNIX chain.
fn open_browser_wsl(url: &str) -> Result<(), BrowserError> {
    let mut last_err: Option<std::io::Error> = None;
    for mut cmd in wsl_open_commands(url) {
        match cmd.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                last_err = Some(std::io::Error::other(format!(
                    "{:?} exited with {status}",
                    cmd.get_program()
                )));
            }
            Err(e) => last_err = Some(e),
        }
    }
    // Last resort: hand off to `open`'s generic chain (xdg-open / gio /
    // gnome-open / kde-open). Only reached when both Windows-side openers
    // failed — e.g. WSL with interop disabled, where `powershell.exe` is
    // off PATH but a Linux browser is still reachable under WSLg.
    if open::that(url).is_ok() {
        return Ok(());
    }
    // Report the Windows-side failure rather than `open`'s: on WSL the
    // bridge is the expected path, so "powershell.exe exited with …" is
    // the more actionable signal than a generic "xdg-open not found".
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no browser opener succeeded (tried wslview, powershell.exe, xdg-open)",
        )
    }))
}

/// Windows-side open attempts for WSL, most-preferred first:
///   1. `wslview` — wslu's bridge; honors the Windows default browser.
///   2. `powershell.exe Start-Process` — always on the WSL-exposed PATH
///      even when `wslu` is not installed.
///
/// The PowerShell URL is single-quoted inside one `-Command` string so
/// the `&` between query params is treated as a literal rather than a
/// statement separator. Any literal `'` in the URL is doubled, which is
/// how PowerShell escapes a quote inside a single-quoted string.
fn wsl_open_commands(url: &str) -> Vec<Command> {
    let mut wslview = Command::new("wslview");
    wslview.arg(url);

    let ps_command = format!("Start-Process -- '{}'", url.replace('\'', "''"));
    let mut powershell = Command::new("powershell.exe");
    powershell.args(["-NoProfile", "-NonInteractive", "-Command", &ps_command]);

    vec![wslview, powershell]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_open_commands_tries_wslview_then_powershell() {
        let cmds = wsl_open_commands("http://127.0.0.1:5000/wizard");
        let programs: Vec<_> = cmds
            .iter()
            .map(|c| c.get_program().to_string_lossy().into_owned())
            .collect();
        assert_eq!(programs, vec!["wslview", "powershell.exe"]);
    }

    #[test]
    fn wslview_receives_the_raw_url() {
        let url = "http://127.0.0.1:5000/wizard?slug=openai&label=key";
        let cmds = wsl_open_commands(url);
        let args: Vec<_> = cmds[0]
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec![url]);
    }

    #[test]
    fn powershell_single_quotes_the_url_so_ampersands_stay_literal() {
        // Query strings carry `&`; unquoted, PowerShell would parse it as
        // a statement separator and only open the first param.
        let url = "http://127.0.0.1:5000/wizard?slug=openai&label=key";
        let cmds = wsl_open_commands(url);
        let args: Vec<_> = cmds[1]
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Process -- 'http://127.0.0.1:5000/wizard?slug=openai&label=key'",
            ]
        );
    }

    #[test]
    fn powershell_escapes_embedded_single_quotes() {
        let cmds = wsl_open_commands("http://x/?q='inject'");
        let args: Vec<_> = cmds[1]
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args.last().unwrap(),
            "Start-Process -- 'http://x/?q=''inject'''"
        );
    }
}
