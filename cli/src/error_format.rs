use serde_json::{Value, json};

/// Render an `anyhow::Error` for stderr.
///
/// When `json` is true, return a single-line JSON object with one of:
///   { "error": "http_error", "status": <int>, "path": "<string>", "body": <json|string> }
///   { "error": "cli_error", "message": "<string>" }
///
/// When `json` is false, fall back to the legacy `Error: {e:#}` shape so
/// non-JSON callers see no behavior change. The caller prints the returned
/// string with `eprintln!`, which supplies the trailing newline on stderr.
pub fn render_error(err: &anyhow::Error, json: bool) -> String {
    let message = format!("{err:#}");

    if !json {
        return format!("Error: {message}");
    }

    let payload = if let Some(http_error) = parse_http_error(&message) {
        let body = serde_json::from_str::<Value>(http_error.body)
            .unwrap_or_else(|_| Value::String(http_error.body.to_owned()));

        json!({
            "error": "http_error",
            "status": http_error.status,
            "path": http_error.path,
            "body": body,
        })
    } else {
        json!({
            "error": "cli_error",
            "message": message,
        })
    };

    serde_json::to_string(&payload).unwrap_or_else(|_| {
        "{\"error\":\"cli_error\",\"message\":\"failed to render error\"}".to_owned()
    })
}

pub(crate) fn detect_json_output_from_argv() -> bool {
    // This is intentionally approximate: before Clap parses the command tree,
    // a positional sequence that literally contains `--output json` is
    // indistinguishable from the flag form. The success path still uses Clap's
    // parsed `OutputFormat`; this only chooses the final error sink format.
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    detect_json_output_from_args(&args)
}

fn detect_json_output_from_args(args: &[String]) -> bool {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--output" {
            return matches!(args.next().map(String::as_str), Some("json"));
        }

        if let Some(value) = arg.strip_prefix("--output=") {
            return value == "json";
        }
    }

    false
}

struct HttpError<'a> {
    status: u16,
    path: &'a str,
    body: &'a str,
}

fn parse_http_error(message: &str) -> Option<HttpError<'_>> {
    let (request, rest) = message.split_once(" failed (HTTP ")?;
    let path = parse_request_path(request)?;
    let (status_and_reason, body) = rest.split_once("):")?;
    let status = status_and_reason
        .split_whitespace()
        .next()?
        .parse::<u16>()
        .ok()?;

    Some(HttpError {
        status,
        path,
        body: body.trim_start(),
    })
}

fn parse_request_path(request: &str) -> Option<&str> {
    let mut parts = request.split_whitespace();
    let first = parts.next()?;
    let second = parts.next();

    let path = if let Some(path) = second {
        if parts.next().is_some() {
            return None;
        }
        if !matches!(first, "GET" | "POST" | "PUT" | "DELETE" | "PATCH") {
            return None;
        }
        path
    } else {
        first
    };

    if path.is_empty() || path.chars().any(char::is_whitespace) {
        return None;
    }

    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string_args(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn renders_plain_bail_as_json_cli_error() {
        let err = anyhow::anyhow!("Provide a catalog slug or use --custom for a custom endpoint");

        let value: Value = serde_json::from_str(&render_error(&err, true)).unwrap();

        assert_eq!(value["error"].as_str(), Some("cli_error"));
        assert_eq!(
            value["message"].as_str(),
            Some("Provide a catalog slug or use --custom for a custom endpoint")
        );
    }

    #[test]
    fn renders_http_error_with_json_body() {
        let err = anyhow::anyhow!(
            "/keys failed (HTTP 400 Bad Request): {{\"error\":\"validation_error\",\"message\":\"bad URL\"}}"
        );

        let value: Value = serde_json::from_str(&render_error(&err, true)).unwrap();

        assert_eq!(value["error"].as_str(), Some("http_error"));
        assert_eq!(value["status"].as_u64(), Some(400));
        assert_eq!(value["path"].as_str(), Some("/keys"));
        assert!(value["body"].is_object());
        assert_eq!(value["body"]["error"].as_str(), Some("validation_error"));
    }

    #[test]
    fn renders_http_error_with_verb_prefix() {
        let err = anyhow::anyhow!(
            "POST /keys failed (HTTP 401 Unauthorized): {{\"error\":\"unauthorized\"}}"
        );

        let value: Value = serde_json::from_str(&render_error(&err, true)).unwrap();

        assert_eq!(value["error"].as_str(), Some("http_error"));
        assert_eq!(value["status"].as_u64(), Some(401));
        assert_eq!(value["path"].as_str(), Some("/keys"));
        assert!(value["body"].is_object());
        assert_eq!(value["body"]["error"].as_str(), Some("unauthorized"));
    }

    #[test]
    fn renders_http_error_with_text_body() {
        let err = anyhow::anyhow!("/proxy failed (HTTP 502 Bad Gateway): upstream timeout");

        let value: Value = serde_json::from_str(&render_error(&err, true)).unwrap();

        assert_eq!(value["error"].as_str(), Some("http_error"));
        assert_eq!(value["status"].as_u64(), Some(502));
        assert_eq!(value["path"].as_str(), Some("/proxy"));
        assert_eq!(value["body"].as_str(), Some("upstream timeout"));
    }

    #[test]
    fn renders_legacy_error_when_json_is_disabled() {
        let err = anyhow::anyhow!("Provide a catalog slug or use --custom for a custom endpoint");

        let rendered = render_error(&err, false);

        assert!(rendered.starts_with("Error: "));
        assert_eq!(
            rendered,
            "Error: Provide a catalog slug or use --custom for a custom endpoint"
        );
    }

    #[test]
    fn detects_json_output_from_argv() {
        assert!(detect_json_output_from_args(&string_args(&[
            "service",
            "add",
            "--base-url",
            "https://example.com",
            "--output",
            "json",
        ])));
        assert!(detect_json_output_from_args(&string_args(&[
            "service",
            "add",
            "--output=json",
        ])));
        assert!(!detect_json_output_from_args(&string_args(&[
            "service", "add", "--output", "table",
        ])));
    }
}
