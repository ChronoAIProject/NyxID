use super::*;
use std::{collections::VecDeque, future::pending, io};

#[derive(Default)]
struct FakeTerminal {
    hidden: bool,
    bytes: VecDeque<u8>,
    fail: Option<&'static str>,
    events: Vec<&'static str>,
}

impl Terminal for FakeTerminal {
    type Mode = bool;

    fn mode(&mut self) -> io::Result<bool> {
        self.events.push("save");
        self.check("save")?;
        Ok(self.hidden)
    }
    fn hide(&mut self, _: &bool) -> io::Result<()> {
        self.events.push("hide_and_verify");
        self.hidden = true;
        self.check("hide")
    }
    fn restore(&mut self, original: &bool) -> io::Result<()> {
        self.events.push("restore");
        self.hidden = *original;
        self.check("restore")
    }
    fn prompt(&mut self) -> io::Result<()> {
        assert!(self.hidden);
        self.events.push("prompt");
        self.check("prompt")
    }
    fn read(&mut self) -> io::Result<Input> {
        assert!(self.hidden);
        self.events.push("read");
        self.check("read")?;
        Ok(self.bytes.pop_front().map_or(Input::Pending, Input::Byte))
    }
}

impl FakeTerminal {
    fn check(&self, phase: &str) -> io::Result<()> {
        if self.fail == Some(phase) {
            // Even a hostile IO error containing input must not escape.
            Err(io::Error::other("private fixture input"))
        } else {
            Ok(())
        }
    }
    fn with_input(input: &[u8]) -> Self {
        Self {
            bytes: input.iter().copied().collect(),
            ..Self::default()
        }
    }
}

#[tokio::test]
async fn terminal_input_is_hidden_before_prompt_and_restored_after_read() {
    let mut terminal = FakeTerminal::with_input(b"AAAA-BBBC\x7fB\n");
    let code = read_hidden(&mut terminal, pending()).await.unwrap();
    assert!(code.as_str() == "AAAA-BBBB");
    assert_eq!(&terminal.events[..3], ["save", "hide_and_verify", "prompt"]);
    assert_eq!(terminal.events.last(), Some(&"restore"));
    assert!(!terminal.hidden);
}

#[tokio::test]
async fn no_echo_failure_never_prompts_or_reads_and_restores_partial_change() {
    let mut terminal = FakeTerminal {
        fail: Some("hide"),
        ..FakeTerminal::default()
    };
    let error = read_hidden(&mut terminal, pending()).await.err().unwrap();
    assert_eq!(error.kind, LoginError::InputUnavailable);
    assert_eq!(
        error.json()["error"]["diagnostic"]["reason"],
        "hidden_input_unavailable"
    );
    assert_eq!(terminal.events, ["save", "hide_and_verify", "restore"]);
    assert!(!terminal.hidden);
}

#[tokio::test]
async fn terminal_errors_restore_settings_and_never_expose_raw_io_messages() {
    for (phase, reason) in [
        ("save", "hidden_input_unavailable"),
        ("prompt", "prompt_failed"),
        ("read", "input_read_failed"),
        ("restore", "terminal_restore_failed"),
    ] {
        let mut terminal = FakeTerminal {
            fail: Some(phase),
            ..FakeTerminal::with_input(b"AAAA-BBBB\n")
        };
        let error = read_hidden(&mut terminal, pending()).await.err().unwrap();
        assert_eq!(error.json()["error"]["diagnostic"]["reason"], reason);
        assert!(!error.text().contains("private fixture"));
        assert!(!format!("{error:?}").contains("AAAA-BBBB"));
        assert!(!terminal.hidden);
        if phase != "save" {
            assert_eq!(terminal.events.last(), Some(&"restore"));
        }
    }
}

#[tokio::test]
async fn ctrl_c_eof_and_external_interruption_restore_echo() {
    for (input, reason) in [
        (b"AAAA\x03".as_slice(), "input_interrupted"),
        (b"AAAA\x04", "input_closed"),
    ] {
        let mut terminal = FakeTerminal::with_input(input);
        let error = read_hidden(&mut terminal, pending()).await.err().unwrap();
        assert_eq!(error.json()["error"]["diagnostic"]["reason"], reason);
        assert!(!terminal.hidden);
    }
    let mut terminal = FakeTerminal::default();
    let error = read_hidden(&mut terminal, std::future::ready(()))
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.json()["error"]["diagnostic"]["reason"],
        "input_interrupted"
    );
    assert!(!terminal.hidden);
    assert!(!terminal.events.contains(&"read"));
}

#[tokio::test]
async fn dropped_input_future_restores_original_terminal_mode() {
    let mut terminal = FakeTerminal::default();
    {
        let mut read = Box::pin(read_hidden(&mut terminal, pending()));
        assert!(futures::poll!(&mut read).is_pending());
    }
    assert!(!terminal.hidden);
    assert_eq!(terminal.events.last(), Some(&"restore"));
}

#[tokio::test]
async fn overlong_or_non_utf8_input_is_bounded_and_never_redeemed() {
    for input in [vec![b'A'; 257], vec![0xff, b'\n']] {
        let mut terminal = FakeTerminal::with_input(&input);
        assert!(read_hidden(&mut terminal, pending()).await.is_err());
        assert!(!terminal.hidden);
    }
}

#[tokio::test]
async fn line_editing_controls_are_private_and_other_c0_bytes_are_ignored() {
    for (input, expected) in [
        (b"alpha beta\x17\n".as_slice(), "alpha"),
        (b"alpha beta   \x17\n", "alpha"),
        (b"discarded\x17kept\n", "kept"),
        (b"discarded\x15kept\x01\n", "kept"),
        (b"a\x08b\x7fc\x00\x02\x05\x06\x07\x09\x0b\x0c\x0e\x0f\x10\x11\x12\x13\x14\x16\x18\x19\x1b\x1d\x1e\x1f\n", "c"),
    ] {
        let mut terminal = FakeTerminal::with_input(input);
        let code = read_hidden(&mut terminal, pending()).await.unwrap();
        assert!(code.as_str() == expected);
        assert!(!terminal.hidden);
    }
}

#[test]
fn input_error_contract_is_safe_and_equivalent_in_text_and_json() {
    let error = unavailable("stdin_not_terminal");
    assert_eq!(error.kind.exit_code(), 22);
    assert_eq!(error.json()["error"]["code"], "login_input_unavailable");
    let diagnostic = &error.json()["error"]["diagnostic"];
    for field in ["stage", "reason", "hint"] {
        assert!(error.text().contains(diagnostic[field].as_str().unwrap()));
    }
    assert!(error.text().contains("nyxid login (device flow)"));
    assert!(error.text().contains("nyxid login --callback"));
    assert_eq!(LoginError::InvalidCode.exit_code(), 18);
    assert_eq!(LoginError::InvalidCode.code(), "login_code_invalid");
}
