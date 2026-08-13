//! Bounded byte-oriented decoder for streamed Chrono Chat Completions hops.

use std::collections::BTreeMap;

use serde::Deserialize;

pub const MAX_SSE_EVENT_BYTES: usize = 256 * 1024;
/// Retained model-visible output from one hop is bounded to the same size as
/// the next-hop request preflight. SSE framing has its own per-event bound.
pub const MAX_HOP_DECODED_BYTES: usize = super::MAX_UPSTREAM_BODY_BYTES;

#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("upstream SSE event exceeded the configured limit")]
    EventTooLarge,
    #[error("upstream hop output exceeded the configured limit")]
    HopTooLarge,
    #[error("upstream SSE event was not valid UTF-8")]
    InvalidUtf8,
    #[error("upstream SSE data was not valid JSON")]
    InvalidJson,
    #[error("upstream returned an error frame")]
    UpstreamError,
    #[error("upstream SSE frames violated the Chrono terminal protocol")]
    Protocol,
    #[error("upstream stream ended before its terminal frames")]
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SseData {
    Json(String),
    Done,
}

#[derive(Default)]
struct SseByteDecoder {
    buffer: Vec<u8>,
}

impl SseByteDecoder {
    fn push(
        &mut self,
        mut bytes: &[u8],
        mut on_event: impl FnMut(SseData) -> Result<(), DecodeError>,
    ) -> Result<(), DecodeError> {
        while !bytes.is_empty() {
            let remaining_capacity = MAX_SSE_EVENT_BYTES
                .checked_sub(self.buffer.len())
                .ok_or(DecodeError::EventTooLarge)?;
            if remaining_capacity == 0 {
                return Err(DecodeError::EventTooLarge);
            }
            let take = remaining_capacity.min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            self.drain(false, &mut on_event)?;
        }
        Ok(())
    }

    fn finish(
        &mut self,
        mut on_event: impl FnMut(SseData) -> Result<(), DecodeError>,
    ) -> Result<(), DecodeError> {
        self.drain(true, &mut on_event)
    }

    fn drain(
        &mut self,
        eof: bool,
        on_event: &mut impl FnMut(SseData) -> Result<(), DecodeError>,
    ) -> Result<(), DecodeError> {
        while let Some(end) = find_event_end(&self.buffer, eof) {
            if end > MAX_SSE_EVENT_BYTES {
                return Err(DecodeError::EventTooLarge);
            }
            let event = self.buffer.drain(..end).collect::<Vec<_>>();
            if let Some(data) = parse_event(&event)? {
                on_event(data)?;
            }
        }

        if eof && !self.buffer.is_empty() {
            if self.buffer.len() > MAX_SSE_EVENT_BYTES {
                return Err(DecodeError::EventTooLarge);
            }
            let event = std::mem::take(&mut self.buffer);
            if let Some(data) = parse_event(&event)? {
                on_event(data)?;
            }
        } else if self.buffer.len() > MAX_SSE_EVENT_BYTES {
            return Err(DecodeError::EventTooLarge);
        }
        Ok(())
    }
}

fn find_event_end(bytes: &[u8], eof: bool) -> Option<usize> {
    let mut cursor = 0;
    let mut line_start = 0;
    while cursor < bytes.len() {
        let newline_len = match bytes[cursor] {
            b'\n' => 1,
            b'\r' if cursor + 1 < bytes.len() && bytes[cursor + 1] == b'\n' => 2,
            b'\r' if cursor + 1 < bytes.len() || eof => 1,
            b'\r' => return None,
            _ => {
                cursor += 1;
                continue;
            }
        };
        let line_is_empty = cursor == line_start;
        cursor += newline_len;
        if line_is_empty {
            return Some(cursor);
        }
        line_start = cursor;
    }
    None
}

fn parse_event(bytes: &[u8]) -> Result<Option<SseData>, DecodeError> {
    let text = std::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)?;
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let data = normalized
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return Ok(None);
    }
    if data == "[DONE]" {
        Ok(Some(SseData::Done))
    } else {
        Ok(Some(SseData::Json(data)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReassembledToolCall {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopResult {
    pub text: String,
    pub tool_calls: Vec<ReassembledToolCall>,
    pub finish_reason: String,
    pub saw_usage: bool,
}

#[derive(Default)]
struct ToolCallParts {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Default)]
pub struct ChronoHopDecoder {
    sse: SseByteDecoder,
    text: String,
    tool_calls: BTreeMap<usize, ToolCallParts>,
    decoded_bytes: usize,
    finish_reason: Option<String>,
    saw_usage: bool,
    saw_done: bool,
    protocol_error: Option<DecodeError>,
}

impl ChronoHopDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, DecodeError> {
        let mut deltas = Vec::new();
        let mut sse = std::mem::take(&mut self.sse);
        let result = sse.push(bytes, |event| {
            self.apply_and_remember_protocol_error(event, &mut deltas)
        });
        self.sse = sse;
        result?;
        Ok(deltas)
    }

    pub fn finish(mut self) -> Result<HopResult, DecodeError> {
        let mut discarded_deltas = Vec::new();
        let mut sse = std::mem::take(&mut self.sse);
        let result = sse
            .finish(|event| self.apply_and_remember_protocol_error(event, &mut discarded_deltas));
        self.sse = sse;
        result?;
        if let Some(error) = self.protocol_error {
            return Err(error);
        }
        if !self.saw_done || !self.saw_usage {
            return Err(DecodeError::Truncated);
        }
        let finish_reason = self.finish_reason.ok_or(DecodeError::Truncated)?;
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|(index, parts)| {
                Ok(ReassembledToolCall {
                    index,
                    id: parts.id.ok_or(DecodeError::Truncated)?,
                    name: parts.name.ok_or(DecodeError::Truncated)?,
                    arguments: parts.arguments,
                })
            })
            .collect::<Result<Vec<_>, DecodeError>>()?;
        Ok(HopResult {
            text: self.text,
            tool_calls,
            finish_reason,
            saw_usage: self.saw_usage,
        })
    }

    fn apply_and_remember_protocol_error(
        &mut self,
        event: SseData,
        deltas: &mut Vec<String>,
    ) -> Result<(), DecodeError> {
        match self.apply(event, deltas) {
            Err(error @ (DecodeError::Protocol | DecodeError::HopTooLarge)) => {
                self.protocol_error.get_or_insert(error);
                Ok(())
            }
            result => result,
        }
    }

    fn apply(&mut self, event: SseData, deltas: &mut Vec<String>) -> Result<(), DecodeError> {
        match event {
            SseData::Done => {
                if self.saw_done {
                    return Err(DecodeError::Protocol);
                }
                self.saw_done = true;
                Ok(())
            }
            SseData::Json(data) => {
                if self.saw_done {
                    return Err(DecodeError::Protocol);
                }
                let frame: CompletionFrame =
                    serde_json::from_str(&data).map_err(|_| DecodeError::InvalidJson)?;
                if frame.error.is_some() {
                    return Err(DecodeError::UpstreamError);
                }
                if let Some(usage) = frame.usage {
                    if self.saw_usage
                        || self.finish_reason.is_none()
                        || !frame.choices.is_empty()
                        || !usage.is_object()
                    {
                        return Err(DecodeError::Protocol);
                    }
                    self.saw_usage = true;
                    return Ok(());
                }
                if self.saw_usage || (self.finish_reason.is_some() && !frame.choices.is_empty()) {
                    return Err(DecodeError::Protocol);
                }
                for choice in frame.choices {
                    if let Some(content) = choice.delta.content
                        && !content.is_empty()
                        && self.reserve_output(content.len())
                    {
                        self.text.push_str(&content);
                        deltas.push(content);
                    }
                    for fragment in choice.delta.tool_calls {
                        let index = fragment.index;
                        if let Some(id) = fragment.id
                            && self
                                .tool_calls
                                .get(&index)
                                .is_none_or(|parts| parts.id.is_none())
                            && self.reserve_output(id.len())
                        {
                            self.tool_calls.entry(index).or_default().id = Some(id);
                        }
                        if let Some(function) = fragment.function {
                            if let Some(name) = function.name
                                && !name.is_empty()
                                && self
                                    .tool_calls
                                    .get(&index)
                                    .is_none_or(|parts| parts.name.is_none())
                                && self.reserve_output(name.len())
                            {
                                self.tool_calls.entry(index).or_default().name = Some(name);
                            }
                            if let Some(arguments) = function.arguments
                                && self.reserve_output(arguments.len())
                            {
                                self.tool_calls
                                    .entry(index)
                                    .or_default()
                                    .arguments
                                    .push_str(&arguments);
                            }
                        }
                    }
                    if let Some(reason) = choice.finish_reason {
                        if self.finish_reason.is_some() {
                            return Err(DecodeError::Protocol);
                        }
                        self.finish_reason = Some(normalize_finish_reason(&reason).to_string());
                    }
                }
                Ok(())
            }
        }
    }

    fn reserve_output(&mut self, additional: usize) -> bool {
        if self.protocol_error == Some(DecodeError::HopTooLarge) {
            return false;
        }
        let Some(next) = self.decoded_bytes.checked_add(additional) else {
            self.protocol_error = Some(DecodeError::HopTooLarge);
            return false;
        };
        if next > MAX_HOP_DECODED_BYTES {
            self.protocol_error = Some(DecodeError::HopTooLarge);
            return false;
        }
        self.decoded_bytes = next;
        true
    }
}

fn normalize_finish_reason(reason: &str) -> &'static str {
    match reason {
        "stop" => "stop",
        "tool_calls" => "tool_calls",
        "length" => "length",
        "content_filter" => "content_filter",
        _ => "other",
    }
}

#[derive(Deserialize)]
struct CompletionFrame {
    #[serde(default)]
    choices: Vec<CompletionChoice>,
    usage: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    #[serde(default)]
    delta: CompletionDelta,
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct CompletionDelta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallFragment>,
}

#[derive(Deserialize)]
struct ToolCallFragment {
    index: usize,
    id: Option<String>,
    function: Option<FunctionFragment>,
}

#[derive(Deserialize)]
struct FunctionFragment {
    name: Option<String>,
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(reason: &str) -> String {
        format!(
            "data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"{reason}\"}}]}}\n\ndata: {{\"choices\":[],\"usage\":{{\"total_tokens\":1}}}}\n\ndata: [DONE]\n\n"
        )
    }

    #[test]
    fn handles_split_crlf_and_split_utf8() {
        let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"},\"finish_reason\":null}]}\r\n\r\n";
        let bytes = stream.as_bytes();
        let split_utf8 = stream.find('好').unwrap() + 1;
        let mut decoder = ChronoHopDecoder::default();
        decoder.push(&bytes[..split_utf8]).unwrap();
        decoder.push(&bytes[split_utf8..bytes.len() - 1]).unwrap();
        decoder.push(&bytes[bytes.len() - 1..]).unwrap();
        decoder.push(terminal("stop").as_bytes()).unwrap();
        let result = decoder.finish().unwrap();
        assert_eq!(result.text, "你好");
        assert_eq!(result.finish_reason, "stop");
    }

    #[test]
    fn reassembles_empty_name_continuations_and_drains_usage_done() {
        let fixture = include_bytes!(
            "../../../../frontend/src/lib/assistant/__fixtures__/chrono-llm-agent-tool-call-upstream.sse"
        );
        let mut decoder = ChronoHopDecoder::default();
        for chunk in fixture.chunks(37) {
            decoder.push(chunk).unwrap();
        }
        let result = decoder.finish().unwrap();
        assert_eq!(result.finish_reason, "tool_calls");
        assert!(result.saw_usage);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "get_weather");
        assert_eq!(result.tool_calls[0].arguments, r#"{"city":"Singapore"}"#);
    }

    #[test]
    fn rejects_oversized_event() {
        let mut decoder = ChronoHopDecoder::default();
        let error = decoder
            .push(&vec![b'x'; MAX_SSE_EVENT_BYTES + 1])
            .unwrap_err();
        assert_eq!(error, DecodeError::EventTooLarge);
    }

    #[test]
    fn rejects_one_large_transport_chunk_before_unbounded_buffer_growth() {
        let mut decoder = ChronoHopDecoder::default();
        let error = decoder
            .push(&vec![b'x'; MAX_SSE_EVENT_BYTES * 4])
            .unwrap_err();
        assert_eq!(error, DecodeError::EventTooLarge);
        assert!(decoder.sse.buffer.len() <= MAX_SSE_EVENT_BYTES);
    }

    #[test]
    fn rejects_cumulative_text_from_many_individually_bounded_events() {
        let content = "x".repeat(1024);
        let frame = format!(
            "data: {}\n\n",
            serde_json::json!({
                "choices":[{"delta":{"content":content},"finish_reason":null}]
            })
        );
        assert!(frame.len() < MAX_SSE_EVENT_BYTES);
        let mut decoder = ChronoHopDecoder::default();
        for _ in 0..=(MAX_HOP_DECODED_BYTES / 1024) {
            decoder.push(frame.as_bytes()).unwrap();
        }
        decoder.push(terminal("stop").as_bytes()).unwrap();
        assert_eq!(decoder.finish().unwrap_err(), DecodeError::HopTooLarge);
    }

    #[test]
    fn rejects_cumulative_tool_arguments_from_many_bounded_events() {
        let arguments = "x".repeat(1024);
        let first = format!(
            "data: {}\n\n",
            serde_json::json!({
                "choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"nyx_call_tool","arguments":arguments}}]},"finish_reason":null}]
            })
        );
        let continuation = format!(
            "data: {}\n\n",
            serde_json::json!({
                "choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"","arguments":arguments}}]},"finish_reason":null}]
            })
        );
        assert!(first.len() < MAX_SSE_EVENT_BYTES);
        assert!(continuation.len() < MAX_SSE_EVENT_BYTES);
        let mut decoder = ChronoHopDecoder::default();
        decoder.push(first.as_bytes()).unwrap();
        for _ in 0..=(MAX_HOP_DECODED_BYTES / 1024) {
            decoder.push(continuation.as_bytes()).unwrap();
        }
        decoder.push(terminal("tool_calls").as_bytes()).unwrap();
        assert_eq!(decoder.finish().unwrap_err(), DecodeError::HopTooLarge);
    }

    #[test]
    fn done_without_finish_or_usage_is_truncated() {
        let mut decoder = ChronoHopDecoder::default();
        decoder.push(b"data: [DONE]\n\n").unwrap();
        assert_eq!(decoder.finish().unwrap_err(), DecodeError::Truncated);
    }

    #[test]
    fn finish_and_done_without_requested_usage_is_truncated_after_drain() {
        let mut decoder = ChronoHopDecoder::default();
        decoder
            .push(
                b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            )
            .unwrap();
        assert_eq!(decoder.finish().unwrap_err(), DecodeError::Truncated);
    }

    #[test]
    fn supports_bare_cr_event_boundaries() {
        let mut decoder = ChronoHopDecoder::default();
        decoder
            .push(terminal("content_filter").replace('\n', "\r").as_bytes())
            .unwrap();
        assert_eq!(decoder.finish().unwrap().finish_reason, "content_filter");
    }

    #[test]
    fn rejects_duplicate_done_and_data_after_done_after_full_drain() {
        for suffix in [
            "data: [DONE]\n\n",
            "data: {\"choices\":[],\"usage\":{\"total_tokens\":2}}\n\n",
        ] {
            let mut decoder = ChronoHopDecoder::default();
            decoder.push(terminal("stop").as_bytes()).unwrap();
            // Protocol drift is remembered so callers can continue draining.
            decoder.push(suffix.as_bytes()).unwrap();
            assert_eq!(decoder.finish().unwrap_err(), DecodeError::Protocol);
        }
    }

    #[test]
    fn rejects_repeated_or_conflicting_finish_reason() {
        for second in ["stop", "length"] {
            let stream = format!(
                "data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"{second}\"}}]}}\n\ndata: {{\"choices\":[],\"usage\":{{\"total_tokens\":1}}}}\n\ndata: [DONE]\n\n"
            );
            let mut decoder = ChronoHopDecoder::default();
            decoder.push(stream.as_bytes()).unwrap();
            assert_eq!(decoder.finish().unwrap_err(), DecodeError::Protocol);
        }
    }

    #[test]
    fn normalizes_unknown_finish_reason_before_metadata_use() {
        let hostile = terminal("accessToken=super-secret");
        let mut decoder = ChronoHopDecoder::default();
        decoder.push(hostile.as_bytes()).unwrap();
        let result = decoder.finish().unwrap();
        assert_eq!(result.finish_reason, "other");
        assert!(!result.finish_reason.contains("super-secret"));
    }

    #[test]
    fn rejects_duplicate_or_malformed_usage_terminal() {
        let malformed = [
            concat!(
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"choices\":[{\"delta\":{}}],\"usage\":{\"total_tokens\":1}}\n\n",
                "data: [DONE]\n\n"
            ),
            concat!(
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"total_tokens\":1}}\n\n",
                "data: {\"choices\":[],\"usage\":{\"total_tokens\":2}}\n\n",
                "data: [DONE]\n\n"
            ),
            concat!(
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":7}\n\n",
                "data: [DONE]\n\n"
            ),
        ];
        for stream in malformed {
            let mut decoder = ChronoHopDecoder::default();
            decoder.push(stream.as_bytes()).unwrap();
            assert_eq!(decoder.finish().unwrap_err(), DecodeError::Protocol);
        }
    }
}
