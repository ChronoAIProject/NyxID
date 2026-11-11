use super::*;
use clap::Parser;
fn key() -> String {
    Uuid::new_v4().to_string()
}
fn receipt(key: &str, status: &str) -> Receipt {
    Receipt {
        invocation_id: key.into(),
        conversation_id: "nyxa-test".into(),
        turn_id: Uuid::new_v4().to_string(),
        status: status.into(),
        terminal: matches!(status, "completed" | "failed" | "cancelled" | "deleted"),
        details: Default::default(),
    }
}

#[test]
fn nyxbot_arguments_input_and_protocol_contract() {
    for args in [
        vec!["nyxid", "nyxbot", "run"],
        vec!["nyxid", "nyxbot", "run", "--text", "x", "--file", "-"],
        vec![
            "nyxid",
            "nyxbot",
            "run",
            "--text",
            "x",
            "--conversation",
            "nyxa-x",
            "--agent",
            "a",
        ],
        vec!["nyxid", "nyxbot", "wait", "invalid"],
        vec![
            "nyxid",
            "nyxbot",
            "wait",
            "e104bf09-0d68-4f2e-07aa-81e352cc3e66",
        ],
    ] {
        assert!(crate::cli::Cli::try_parse_from(args).is_err());
    }
    assert!(
        crate::cli::Cli::try_parse_from(["nyxid", "nyxbot", "run", "--text", "x", "--no-wait"])
            .is_ok()
    );
    assert!(read_input(&b"\xff"[..]).is_err());
    assert!(read_input(vec![b'x'; MAX_INPUT_BYTES + 1].as_slice()).is_err());
    assert!(validate_input(&"é".repeat(MAX_INPUT_CHARS)).is_ok());
    assert!(validate_input(&"x".repeat(MAX_INPUT_CHARS + 1)).is_err());
    assert!(validate_input("  \n").is_err());
    let key = key();
    for (status, exit) in [
        ("completed", 0),
        ("failed", 20),
        ("cancelled", 21),
        ("requires_action", 22),
        ("requires_review", 22),
        ("waiting", 0),
    ] {
        let row = receipt(&key, status);
        assert!(row.validate(&key).is_ok());
        assert_eq!(row.outcome().1, exit);
    }
    let mut invalid = receipt(&key, "completed");
    invalid.terminal = false;
    assert!(invalid.validate(&key).is_err());
    assert!(receipt(&key, "unknown").validate(&key).is_err());
    assert!(receipt(&key, "running").validate(&self::key()).is_err());
}

#[test]
fn nyxbot_actual_rendering_removes_terminal_controls_and_json_is_complete() {
    let key = key();
    let mut row = receipt(&key, "completed");
    row.details.insert(
        "replies".into(),
        json!([{"text":"visible\u{1b}]52;c;payload\u{7}\u{1b}[2J\r\u{85}\u{202e}\u{061c}\u{200e}\u{200f}end\nline\ttab"}]),
    );
    let report = report(&key, row);
    let text = render(&report, OutputFormat::Table).unwrap();
    assert!(text.contains("visible"));
    assert!(text.contains("end\nline\ttab"));
    assert!(
        !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    );
    for control in ['\u{202e}', '\u{061c}', '\u{200e}', '\u{200f}'] {
        assert!(!text.contains(control));
    }
    let encoded = render(&report, OutputFormat::Json).unwrap();
    for control in [
        '\u{1b}', '\u{85}', '\u{202e}', '\u{061c}', '\u{200e}', '\u{200f}',
    ] {
        assert!(!encoded.contains(control));
    }
    let parsed: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(parsed["exit_code"], 0);
    assert_eq!(parsed["invocation_id"], key);
    assert_eq!(parsed, serde_json::to_value(&report).unwrap());

    let controls: String = (0x7f..=0x9f)
        .chain([0x061c, 0x200e, 0x200f])
        .chain(0x202a..=0x202e)
        .chain(0x2066..=0x2069)
        .map(|value| char::from_u32(value).unwrap())
        .collect();
    let discovery = json!({"agent_name": format!("safe{controls}name")});
    let encoded = render_discovery(&discovery, OutputFormat::Json).unwrap();
    assert!(!encoded.chars().any(|c| controls.contains(c)));
    assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), discovery);
}

#[test]
fn nyxbot_typed_auth_and_http_errors_retain_identity_without_raw_bodies() {
    let key = key();
    for (error, exit) in [
        (
            anyhow::Error::new(crate::auth::ReauthRequired {
                code: "session_expired",
                message: "ignored".into(),
            }),
            3,
        ),
        (
            anyhow::Error::new(crate::auth::RefreshUnavailable("ignored".into())),
            4,
        ),
    ] {
        let result = failure(&key, &error, None);
        assert_eq!(result.exit_code, exit);
        assert_eq!(result.invocation_id, key);
        assert!(result.guidance.is_some());
    }
    let error=ApiError::new("test",reqwest::StatusCode::FORBIDDEN,json!({"error":"forbidden","error_code":1003,"message":"do not display raw server prose","details":{"reason":"credential_type_unsupported"}}).to_string());
    let result = failure(&key, &error.into(), None);
    assert_eq!(result.error_code, Some(1003));
    assert_eq!(
        result.reason.as_deref(),
        Some("credential_type_unsupported")
    );
    assert!(result.guidance.unwrap().contains("account login"));
    assert!(
        !render(&result, OutputFormat::Json)
            .unwrap()
            .contains("raw server prose")
    );
    let error = serde_json::from_str::<Receipt>("{}").err().unwrap();
    assert_eq!(failure(&key, &error.into(), None).exit_code, 25);
}

async fn http_fixture(response: &[u8], stall: bool) -> (ApiClient, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let response = response.to_vec();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        // Do not format or retain credential-bearing request bytes.
        let _ = socket.read(&mut request).await;
        socket.write_all(&response).await.unwrap();
        socket.flush().await.unwrap();
        if stall {
            std::future::pending::<()>().await;
        }
    });
    let api = ApiClient::new(&format!("http://{address}"), "fake-loopback-fixture".into()).unwrap();
    (api, server)
}

#[tokio::test]
async fn nyxbot_discovery_http_errors_render_safe_codes_without_raw_body_or_identity() {
    let untrusted = "raw-upstream-body\u{1b}]52;c;payload\u{7}\u{9b}2J\u{202e}\u{200f}";
    for (status, content_type, body, expected_code) in [
        ("502 Bad Gateway", "text/plain", untrusted.to_owned(), None),
        (
            "403 Forbidden",
            "application/json",
            json!({
                "error": "forbidden",
                "error_code": 1003,
                "message": untrusted,
                "details": {"reason": "credential_type_unsupported"}
            })
            .to_string(),
            Some(1003),
        ),
    ] {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let (mut api, server) = http_fixture(response.as_bytes(), false).await;
        let failure = read_discovery(&mut api, "/assistant/nyxagent/agents")
            .await
            .unwrap_err();
        assert_eq!(failure.exit_code, 1);
        assert_eq!(failure.error_code, expected_code);
        assert_eq!(
            failure.http_status,
            Some(if expected_code.is_some() { 403 } else { 502 })
        );
        for output in [OutputFormat::Table, OutputFormat::Json] {
            let rendered = render_discovery_failure(&failure, output).unwrap();
            assert!(!rendered.contains("raw-upstream-body"));
            assert!(!rendered.contains("payload"));
            assert!(!rendered.contains("invocation"));
            assert!(
                !rendered
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
            );
            assert!(!rendered.contains(['\u{202e}', '\u{200f}']));
            if matches!(output, OutputFormat::Json) {
                let parsed: Value = serde_json::from_str(&rendered).unwrap();
                assert!(parsed.get("invocation_id").is_none());
                assert_eq!(parsed["exit_code"], 1);
                assert_eq!(parsed["http_status"], failure.http_status.unwrap());
            }
            if expected_code.is_some() {
                assert!(rendered.contains("account login"));
                assert!(rendered.contains("credential_type_unsupported"));
            }
        }
        server.await.unwrap();
    }
    for (error, expected_exit) in [
        (
            anyhow::Error::new(crate::auth::ReauthRequired {
                code: "session_expired",
                message: untrusted.into(),
            }),
            3,
        ),
        (
            anyhow::Error::new(crate::auth::RefreshUnavailable(untrusted.into())),
            4,
        ),
    ] {
        let failure = classify_failure(&error);
        assert_eq!(failure.exit_code, expected_exit);
        let rendered = render_discovery_failure(&failure, OutputFormat::Json).unwrap();
        assert!(!rendered.contains("raw-upstream-body"));
        assert!(
            serde_json::from_str::<Value>(&rendered)
                .unwrap()
                .get("invocation_id")
                .is_none()
        );
        assert!(failure.guidance.is_some());
    }
}

#[tokio::test]
async fn nyxbot_http_deadline_covers_inflight_body_and_keeps_identity() {
    let key = key();
    let (mut api, server) = http_fixture(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{",
        true,
    )
    .await;
    let started = Instant::now();
    let result = wait(&mut api, &key, started + Duration::from_millis(100), None).await;
    assert_eq!(result.exit_code, 23);
    assert_eq!(result.invocation_id, key);
    assert!(started.elapsed() < Duration::from_secs(2));
    server.abort();
}

#[tokio::test]
async fn nyxbot_http_decode_transport_rejection_and_redirect_have_distinct_outcomes() {
    let key = key();
    for (bytes, expected) in [
        (&b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\n\r\n{"[..],25),
        (&b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}"[..],25),
        (&b"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}"[..],1),
        (&b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/never\r\nContent-Length: 0\r\n\r\n"[..],1),
        (&b""[..],24),
    ] {
        let (mut api, server) = http_fixture(bytes,false).await;
        let result = wait(&mut api,&key,Instant::now()+Duration::from_secs(2),None).await;
        assert_eq!(result.exit_code,expected);
        assert_eq!(result.invocation_id,key);
        server.await.unwrap();
    }
}

#[test]
fn nyxbot_rendering_identifies_review_reason_child_and_bounded_history() {
    let key = key();
    let mut row = receipt(&key, "requires_review");
    row.details
        .insert("error_code".into(), json!("async_cancellation_unconfirmed"));
    row.details.insert("replies_truncated".into(), json!(true));
    row.details.insert("replies".into(),json!([{"text":"excerpt","text_truncated":true,"conversation_id":"nyxa-child","turn_id":"child-turn"}]));
    let text = render(&report(&key, row), OutputFormat::Table).unwrap();
    assert!(text.contains("Status reason: async_cancellation_unconfirmed"));
    assert!(text.contains("conversation nyxa-child, turn child-turn"));
    assert!(text.contains("Reply excerpt truncated"));
    assert!(text.contains("Additional replies omitted"));
    assert!(text.contains("nyxid nyxbot history nyxa-child"));
}
