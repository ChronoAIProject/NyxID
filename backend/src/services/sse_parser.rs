//! Shared SSE (Server-Sent Events) parser used by proxy, LLM gateway, and
//! ChatGPT translator modules.
//!
//! Each parser previously had its own near-identical implementation. This
//! module consolidates them into a single `parse_next_event` function that
//! extracts the next complete SSE event from a byte buffer.

/// A parsed SSE event from an upstream stream.
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
}

/// Extract the next complete SSE event from `buffer`.
///
/// SSE events are delimited by double newlines (`\n\n`). Each event may
/// contain:
/// - `event: <type>` line
/// - `data: <payload>` line(s) (multiple `data:` lines are joined with `\n`)
///
/// Returns `None` if no complete event is available yet. The consumed bytes
/// (including the `\n\n` delimiter) are drained from `buffer`.
///
/// Lines starting with `:` (comments) and fields like `id:` / `retry:` are
/// silently ignored. Empty/comment-only frames are skipped internally so
/// callers can keep using `while let Some(event) = parse_next_event(...)`.
///
/// When no explicit `event:` header is present but the data parses as JSON
/// with a `"type"` field, that value is used as `event_type`. This covers
/// providers that embed the event type inside the JSON payload (e.g. the
/// ChatGPT Responses API).
pub fn parse_next_event(buffer: &mut String) -> Option<SseEvent> {
    loop {
        let end = buffer.find("\n\n")?;
        let event_text = buffer[..end].to_string();
        buffer.drain(..end + 2);

        let mut event_type = None;
        let mut data_parts = Vec::new();

        for line in event_text.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                event_type = Some(rest.trim_start().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                data_parts.push(rest.trim_start().to_string());
            }
            // Ignore id:, retry:, and comment lines (starting with :)
        }

        if data_parts.is_empty() && event_type.is_none() {
            continue;
        }

        let data = data_parts.join("\n");

        // If no explicit event: header, try to extract type from JSON data
        if event_type.is_none()
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data)
        {
            event_type = parsed
                .get("type")
                .and_then(|t| t.as_str())
                .map(String::from);
        }

        return Some(SseEvent { event_type, data });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_event_with_type_and_data() {
        let mut buf =
            "event: response.created\ndata: {\"type\":\"response.created\"}\n\n".to_string();
        let event = parse_next_event(&mut buf).unwrap();
        assert_eq!(event.event_type.as_deref(), Some("response.created"));
        assert_eq!(event.data, "{\"type\":\"response.created\"}");
        assert!(buf.is_empty());
    }

    #[test]
    fn extracts_type_from_json_when_no_event_header() {
        let mut buf =
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n".to_string();
        let event = parse_next_event(&mut buf).unwrap();
        assert_eq!(
            event.event_type.as_deref(),
            Some("response.output_text.delta")
        );
        assert_eq!(
            event.data,
            "{\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}"
        );
    }

    #[test]
    fn returns_none_for_incomplete_event() {
        let mut buf = "event: response.created\ndata: {\"partial".to_string();
        assert!(parse_next_event(&mut buf).is_none());
        assert_eq!(buf, "event: response.created\ndata: {\"partial");
    }

    #[test]
    fn parses_multiple_events_sequentially() {
        let mut buf =
            "event: a\ndata: {\"type\":\"a\"}\n\nevent: b\ndata: {\"type\":\"b\"}\n\n".to_string();
        let e1 = parse_next_event(&mut buf).unwrap();
        assert_eq!(e1.event_type.as_deref(), Some("a"));
        let e2 = parse_next_event(&mut buf).unwrap();
        assert_eq!(e2.event_type.as_deref(), Some("b"));
        assert!(buf.is_empty());
    }

    #[test]
    fn skips_empty_blocks() {
        let mut buf = "\n\nevent: test\ndata: {}\n\n".to_string();
        let event = parse_next_event(&mut buf).unwrap();
        assert_eq!(event.event_type.as_deref(), Some("test"));
        assert_eq!(event.data, "{}");
        assert!(buf.is_empty());
    }

    #[test]
    fn skips_comment_only_frames_and_keeps_parsing() {
        let mut buf = ": keepalive\n\n:event comment\n\ndata: {\"type\":\"ready\"}\n\n".to_string();
        let event = parse_next_event(&mut buf).unwrap();
        assert_eq!(event.event_type.as_deref(), Some("ready"));
        assert_eq!(event.data, "{\"type\":\"ready\"}");
        assert!(buf.is_empty());
    }

    #[test]
    fn skips_id_retry_only_frames_and_keeps_parsing() {
        let mut buf = "id: 1\nretry: 1000\n\nevent: test\ndata: payload\n\n".to_string();
        let event = parse_next_event(&mut buf).unwrap();
        assert_eq!(event.event_type.as_deref(), Some("test"));
        assert_eq!(event.data, "payload");
        assert!(buf.is_empty());
    }

    #[test]
    fn comment_then_incomplete_event_waits_for_more_data() {
        let mut buf = ": keepalive\n\ndata: {\"partial\"".to_string();
        let event = parse_next_event(&mut buf);
        assert!(event.is_none());
        assert_eq!(buf, "data: {\"partial\"");
    }

    #[test]
    fn comment_then_complete_event_in_same_buffer_is_not_stranded() {
        let mut buf = ": keepalive\n\ndata: {\"type\":\"response.completed\"}\n\n".to_string();
        let event = parse_next_event(&mut buf).unwrap();
        assert_eq!(event.event_type.as_deref(), Some("response.completed"));
        assert_eq!(event.data, "{\"type\":\"response.completed\"}");
        assert!(buf.is_empty());
    }

    #[test]
    fn handles_data_only_without_json_type() {
        let mut buf = "data: plain text\n\n".to_string();
        let event = parse_next_event(&mut buf).unwrap();
        assert_eq!(event.event_type, None);
        assert_eq!(event.data, "plain text");
    }

    #[test]
    fn joins_multiple_data_lines() {
        let mut buf = "data: line1\ndata: line2\n\n".to_string();
        let event = parse_next_event(&mut buf).unwrap();
        assert_eq!(event.data, "line1\nline2");
    }

    #[test]
    fn handles_no_space_after_colon() {
        let mut buf = "event:test\ndata:{\"foo\":1}\n\n".to_string();
        let event = parse_next_event(&mut buf).unwrap();
        assert_eq!(event.event_type.as_deref(), Some("test"));
        assert_eq!(event.data, "{\"foo\":1}");
    }
}
