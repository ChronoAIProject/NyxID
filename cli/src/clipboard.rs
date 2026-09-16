//! Best-effort copying of the human login code using platform clipboard tools.

use std::io::Write;
use std::process::{Command, Stdio};

type Tool = (&'static str, &'static [&'static str]);

fn candidates(os: &str, wsl: bool) -> &'static [Tool] {
    if wsl || os == "windows" {
        &[("clip.exe", &[])]
    } else {
        match os {
            "macos" => &[("pbcopy", &[])],
            "linux" => &[
                ("wl-copy", &[]),
                ("xclip", &["-selection", "clipboard"]),
                ("xsel", &["--clipboard", "--input"]),
            ],
            _ => &[],
        }
    }
}

pub fn copy_user_code(code: &str) {
    if !copy_with(
        code,
        candidates(std::env::consts::OS, crate::browser::is_wsl()),
        run_tool,
    ) {
        eprintln!("The user code could not be copied to the clipboard. Enter it manually.");
    }
}

fn copy_with(code: &str, tools: &[Tool], mut run: impl FnMut(Tool, &str) -> bool) -> bool {
    tools.iter().any(|tool| run(*tool, code))
}

fn run_tool((program, args): Tool, code: &str) -> bool {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // Pass only the code, through stdin, never through shell text or arguments.
    // Drop stdin to send EOF, and reap the child even when writing failed.
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(code.as_bytes()).is_ok());
    let succeeded = child.wait().is_ok_and(|status| status.success());
    written && succeeded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_platform_tools_in_order() {
        assert_eq!(candidates("macos", false), &[("pbcopy", &[][..])]);
        assert_eq!(candidates("windows", false), &[("clip.exe", &[][..])]);
        assert_eq!(candidates("linux", true), candidates("windows", false));
        assert_eq!(
            candidates("linux", false),
            &[
                ("wl-copy", &[][..]),
                ("xclip", &["-selection", "clipboard"][..]),
                ("xsel", &["--clipboard", "--input"][..]),
            ]
        );
        assert!(candidates("unknown", false).is_empty());
    }

    #[test]
    fn falls_back_after_failures_and_stops_on_success() {
        let mut attempted = Vec::new();
        assert!(copy_with(
            "2-ABCD-EFGH",
            candidates("linux", false),
            |tool, input| {
                assert_eq!(input, "2-ABCD-EFGH");
                attempted.push(tool.0);
                tool.0 == "xclip"
            }
        ));
        assert_eq!(attempted, ["wl-copy", "xclip"]);
    }

    #[test]
    fn reports_failure_when_no_tool_succeeds() {
        let mut count = 0;
        assert!(!copy_with(
            "ABCD-EFGH",
            candidates("linux", false),
            |_, _| {
                count += 1;
                false
            }
        ));
        assert_eq!(count, 3);
        assert!(!copy_with("ABCD-EFGH", &[], |_, _| panic!("no candidates")));
    }
}
