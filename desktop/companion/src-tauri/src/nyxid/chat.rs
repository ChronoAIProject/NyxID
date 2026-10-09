use reqwest::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use uuid::Version;

const CHAT_EVENT: &str = "companion://nyxid-chat";
const MAX_STREAM_BYTES: usize = 12 * 1024 * 1024;
const MAX_REPLY_BYTES: usize = 2 * 1024 * 1024;
const MAX_BUFFER_BYTES: usize = 512 * 1024;
const MAX_HISTORY_MESSAGES: usize = 50;
const MAX_HISTORY_TEXT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NyxIdChatRequest {
    pub request_id: String,
    pub conversation_id: Option<String>,
    pub text: String,
}

impl NyxIdChatRequest {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !valid_request_id(&self.request_id)
            || self
                .conversation_id
                .as_deref()
                .is_some_and(|id| !valid_conversation_id(id))
            || self.text.trim().is_empty()
            || self.text.chars().count() > 32_768
        {
            return Err("对话请求无效，请重试。".to_owned());
        }
        Ok(())
    }
}

pub(crate) fn valid_request_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.get_version() == Some(Version::Random))
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NyxIdChatCommandErrorKind {
    Rejected,
    AdmissionUnknown,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdChatCommandError {
    kind: NyxIdChatCommandErrorKind,
    message: String,
}

impl NyxIdChatCommandError {
    pub(crate) fn rejected(message: impl Into<String>) -> Self {
        Self {
            kind: NyxIdChatCommandErrorKind::Rejected,
            message: message.into(),
        }
    }

    pub(crate) fn admission_unknown(message: impl Into<String>) -> Self {
        Self {
            kind: NyxIdChatCommandErrorKind::AdmissionUnknown,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NyxIdChatEvent {
    Started {
        request_id: String,
        conversation_id: String,
        turn_id: String,
    },
    Delta {
        request_id: String,
        text: String,
    },
    Snapshot {
        request_id: String,
        text: String,
    },
    Completed {
        request_id: String,
        conversation_id: String,
        status: NyxIdChatStatus,
        error: Option<NyxIdChatError>,
    },
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NyxIdChatStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdChatError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdChatHistory {
    pub conversation: NyxIdChatHistoryConversation,
    pub messages: Vec<NyxIdChatHistoryMessage>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdChatHistoryConversation {
    pub id: String,
    pub active_turn: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdChatHistoryMessage {
    pub id: String,
    pub seq: i64,
    pub turn_id: String,
    pub role: NyxIdChatMessageRole,
    pub text: String,
    pub status: NyxIdChatMessageStatus,
    pub error_code: Option<String>,
}

impl NyxIdChatHistory {
    pub(crate) fn into_exact_admission_turn(
        self,
        turn_id: &str,
        active: bool,
    ) -> Result<Self, String> {
        if !valid_bounded_text(turn_id, 128, false) {
            return Err("NyxID 返回了无效的对话任务标识。".to_owned());
        }
        let messages = self
            .messages
            .into_iter()
            .filter(|message| message.turn_id == turn_id)
            .collect::<Vec<_>>();
        if messages.is_empty() {
            return Err("NyxID 对话受理记录仍在同步，请稍后重试。".to_owned());
        }
        if !active
            && !messages
                .iter()
                .any(|message| message.role == NyxIdChatMessageRole::Assistant)
        {
            return Err("NyxID 对话结束记录仍在同步，请稍后重试。".to_owned());
        }
        Ok(Self {
            conversation: NyxIdChatHistoryConversation {
                id: self.conversation.id,
                active_turn: active,
            },
            messages,
        })
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NyxIdChatRecovery {
    pub request_id: String,
    pub history: NyxIdChatHistory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NyxIdChatAdmission {
    pub(crate) client_request_id: String,
    pub(crate) conversation_id: String,
    pub(crate) turn_id: String,
    pub(crate) active: bool,
    pub(crate) conversation_deleted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NyxIdChatAdmissionWire {
    client_request_id: String,
    conversation_id: String,
    turn_id: String,
    active: bool,
    conversation_deleted: bool,
}

impl NyxIdChatAdmissionWire {
    pub(crate) fn into_public(
        self,
        expected_request_id: &str,
    ) -> Result<NyxIdChatAdmission, String> {
        if !valid_request_id(expected_request_id)
            || self.client_request_id != expected_request_id
            || !valid_conversation_id(&self.conversation_id)
            || !valid_bounded_text(&self.turn_id, 128, false)
            || (self.active && self.conversation_deleted)
        {
            return Err("NyxID 返回了无法识别的对话受理记录。".to_owned());
        }
        Ok(NyxIdChatAdmission {
            client_request_id: self.client_request_id,
            conversation_id: self.conversation_id,
            turn_id: self.turn_id,
            active: self.active,
            conversation_deleted: self.conversation_deleted,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NyxIdChatMessageRole {
    User,
    Assistant,
    Orchestrator,
    Event,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NyxIdChatMessageStatus {
    Completed,
    Failed,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NyxIdChatHistoryWire {
    conversation: NyxIdChatHistoryConversationWire,
    messages: Vec<NyxIdChatHistoryMessageWire>,
}

#[derive(Debug, Deserialize)]
struct NyxIdChatHistoryConversationWire {
    id: String,
    active_turn: Value,
}

#[derive(Debug, Deserialize)]
struct NyxIdChatHistoryMessageWire {
    id: String,
    seq: i64,
    turn_id: String,
    role: NyxIdChatMessageRole,
    text: String,
    status: NyxIdChatMessageStatus,
    error_code: Option<String>,
}

impl NyxIdChatHistoryWire {
    pub(crate) fn into_public(
        self,
        expected_conversation_id: &str,
    ) -> Result<NyxIdChatHistory, String> {
        if self.conversation.id != expected_conversation_id
            || !valid_conversation_id(&self.conversation.id)
            || !(self.conversation.active_turn.is_null()
                || self.conversation.active_turn.is_object())
            || self.messages.len() > MAX_HISTORY_MESSAGES
        {
            return Err("NyxID 返回了无法识别的对话记录。".to_owned());
        }

        let active_turn = self.conversation.active_turn.is_object();
        let mut text_bytes = 0_usize;
        let messages = self
            .messages
            .into_iter()
            .map(|message| {
                text_bytes = text_bytes
                    .checked_add(message.text.len())
                    .filter(|total| *total <= MAX_HISTORY_TEXT_BYTES)
                    .ok_or_else(|| "NyxID 对话记录超过了显示上限。".to_owned())?;
                if message.seq <= 0
                    || !valid_bounded_text(&message.id, 256, false)
                    || !valid_bounded_text(&message.turn_id, 128, false)
                    || message
                        .error_code
                        .as_deref()
                        .is_some_and(|code| !valid_bounded_text(code, 128, false))
                {
                    return Err("NyxID 返回了无法识别的对话记录。".to_owned());
                }
                Ok(NyxIdChatHistoryMessage {
                    id: message.id,
                    seq: message.seq,
                    turn_id: message.turn_id,
                    role: message.role,
                    text: message.text,
                    status: message.status,
                    error_code: message.error_code,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(NyxIdChatHistory {
            conversation: NyxIdChatHistoryConversation {
                id: self.conversation.id,
                active_turn,
            },
            messages,
        })
    }
}

#[derive(Default)]
struct TurnProjection {
    expected_conversation_id: Option<String>,
    conversation_id: Option<String>,
    turn_id: Option<String>,
    last_cursor: u64,
    reply_bytes: usize,
    terminal: Option<NyxIdChatEvent>,
}

impl TurnProjection {
    fn new(expected_conversation_id: Option<&str>) -> Self {
        Self {
            expected_conversation_id: expected_conversation_id.map(str::to_owned),
            ..Self::default()
        }
    }

    fn apply(&mut self, request_id: &str, value: Value) -> Result<Option<NyxIdChatEvent>, String> {
        let Some(event) = value.get("event").and_then(Value::as_str) else {
            return Ok(None);
        };
        let Some(cursor) = value.get("cursor").and_then(Value::as_u64) else {
            return Err("NyxID 对话流缺少事件序号。".to_owned());
        };
        if cursor <= self.last_cursor {
            return Ok(None);
        }
        self.last_cursor = cursor;

        match event {
            "turn.status" => {
                let conversation_id = bounded_string(&value, "conversation_id", 64)
                    .filter(|id| valid_conversation_id(id))
                    .ok_or_else(|| "NyxID 对话没有返回有效会话。".to_owned())?;
                let turn_id = bounded_string(&value, "turn_id", 128)
                    .ok_or_else(|| "NyxID 对话没有返回有效任务。".to_owned())?;
                if self
                    .expected_conversation_id
                    .as_deref()
                    .is_some_and(|expected| expected != conversation_id)
                    || self
                        .conversation_id
                        .as_deref()
                        .is_some_and(|current| current != conversation_id)
                    || self
                        .turn_id
                        .as_deref()
                        .is_some_and(|current| current != turn_id)
                {
                    return Err("NyxID 对话身份在响应中发生了变化。".to_owned());
                }
                if self.conversation_id.is_some() {
                    return Ok(None);
                }
                self.conversation_id = Some(conversation_id.to_owned());
                self.turn_id = Some(turn_id.to_owned());
                Ok(Some(NyxIdChatEvent::Started {
                    request_id: request_id.to_owned(),
                    conversation_id: conversation_id.to_owned(),
                    turn_id: turn_id.to_owned(),
                }))
            }
            "block.delta" => {
                if self.conversation_id.is_none() {
                    return Err("NyxID 在确认会话前返回了内容。".to_owned());
                }
                let text = value
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "NyxID 返回了无效的对话内容。".to_owned())?;
                self.reply_bytes = self
                    .reply_bytes
                    .checked_add(text.len())
                    .filter(|total| *total <= MAX_REPLY_BYTES)
                    .ok_or_else(|| "NyxID 回复超过了桌面宠物的显示上限。".to_owned())?;
                Ok(Some(NyxIdChatEvent::Delta {
                    request_id: request_id.to_owned(),
                    text: text.to_owned(),
                }))
            }
            "block.completed" => {
                if self.conversation_id.is_none() {
                    return Err("NyxID 在确认会话前返回了完整内容。".to_owned());
                }
                let Some(block) = value.get("block") else {
                    return Err("NyxID 返回了无效的完整对话内容。".to_owned());
                };
                if block.get("type").and_then(Value::as_str) != Some("text") {
                    return Ok(None);
                }
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| text.len() <= MAX_REPLY_BYTES)
                    .ok_or_else(|| "NyxID 返回了无效的完整对话内容。".to_owned())?;
                self.reply_bytes = text.len();
                Ok(Some(NyxIdChatEvent::Snapshot {
                    request_id: request_id.to_owned(),
                    text: text.to_owned(),
                }))
            }
            "turn.completed" => {
                let conversation_id = self
                    .conversation_id
                    .clone()
                    .ok_or_else(|| "NyxID 在确认会话前结束了响应。".to_owned())?;
                let turn_id = bounded_string(&value, "turn_id", 128)
                    .ok_or_else(|| "NyxID 对话结束事件缺少任务标识。".to_owned())?;
                if self.turn_id.as_deref() != Some(turn_id) {
                    return Err("NyxID 对话结束事件与当前任务不匹配。".to_owned());
                }
                let status = match value.get("status").and_then(Value::as_str) {
                    Some("completed") => NyxIdChatStatus::Completed,
                    Some("failed") => NyxIdChatStatus::Failed,
                    Some("cancelled") => NyxIdChatStatus::Cancelled,
                    _ => return Err("NyxID 返回了未知的对话结束状态。".to_owned()),
                };
                let error = (status == NyxIdChatStatus::Failed).then(|| {
                    let code = value
                        .get("error")
                        .and_then(|error| error.get("code"))
                        .and_then(Value::as_str)
                        .filter(|code| !code.is_empty() && code.len() <= 128)
                        .unwrap_or("assistant_failed");
                    public_error(code)
                });
                let terminal = NyxIdChatEvent::Completed {
                    request_id: request_id.to_owned(),
                    conversation_id,
                    status,
                    error,
                };
                self.terminal = Some(terminal.clone());
                Ok(Some(terminal))
            }
            _ => Ok(None),
        }
    }
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, String> {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > MAX_BUFFER_BYTES {
            return Err("NyxID 对话流包含过大的单个事件。".to_owned());
        }
        let mut values = Vec::new();
        while let Some((index, delimiter_length)) = record_boundary(&self.buffer) {
            let remainder = self.buffer.split_off(index + delimiter_length);
            let record = std::mem::replace(&mut self.buffer, remainder);
            if let Some(value) = decode_record(&record[..index])? {
                values.push(value);
            }
        }
        Ok(values)
    }

    fn finish(mut self) -> Result<Vec<Value>, String> {
        if self.buffer.is_empty() {
            return Ok(Vec::new());
        }
        decode_record(&std::mem::take(&mut self.buffer)).map(|value| value.into_iter().collect())
    }
}

pub(crate) async fn consume_turn_response<F>(
    app: &AppHandle,
    response: &mut Response,
    request_id: &str,
    expected_conversation_id: Option<&str>,
    mut on_started: F,
) -> Result<NyxIdChatEvent, String>
where
    F: FnMut(&str, &str) -> Result<(), String>,
{
    let mut decoder = SseDecoder::default();
    let mut projection = TurnProjection::new(expected_conversation_id);
    let mut total_bytes = 0_usize;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "与 NyxID 的对话连接中断了。".to_owned())?
    {
        total_bytes = total_bytes
            .checked_add(chunk.len())
            .filter(|total| *total <= MAX_STREAM_BYTES)
            .ok_or_else(|| "NyxID 对话流超过了大小上限。".to_owned())?;
        for value in decoder.push(&chunk)? {
            if let Some(event) = projection.apply(request_id, value)? {
                if let NyxIdChatEvent::Started {
                    conversation_id,
                    turn_id,
                    ..
                } = &event
                {
                    on_started(conversation_id, turn_id)?;
                }
                app.emit(CHAT_EVENT, &event)
                    .map_err(|_| "无法更新桌面宠物的对话状态。".to_owned())?;
            }
            if projection.terminal.is_some() {
                break;
            }
        }
        if projection.terminal.is_some() {
            break;
        }
    }

    if projection.terminal.is_none() {
        for value in decoder.finish()? {
            if let Some(event) = projection.apply(request_id, value)? {
                if let NyxIdChatEvent::Started {
                    conversation_id,
                    turn_id,
                    ..
                } = &event
                {
                    on_started(conversation_id, turn_id)?;
                }
                app.emit(CHAT_EVENT, &event)
                    .map_err(|_| "无法更新桌面宠物的对话状态。".to_owned())?;
            }
        }
    }

    projection
        .terminal
        .ok_or_else(|| "NyxID 对话在完成前中断了；请先查看原会话，不要重复发送。".to_owned())
}

fn decode_record(record: &[u8]) -> Result<Option<Value>, String> {
    let text = std::str::from_utf8(record)
        .map_err(|_| "NyxID 对话流包含无效文本。".to_owned())?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|line| line.strip_prefix(' ').unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    if data.trim().is_empty() || data.trim() == "[DONE]" {
        return Ok(None);
    }
    serde_json::from_str(&data)
        .map(Some)
        .map_err(|_| "NyxID 对话流包含无效事件。".to_owned())
}

fn record_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    (0..buffer.len()).find_map(|index| {
        if buffer.get(index..index + 4) == Some(b"\r\n\r\n") {
            Some((index, 4))
        } else if buffer.get(index..index + 2) == Some(b"\n\n")
            || buffer.get(index..index + 2) == Some(b"\r\r")
        {
            Some((index, 2))
        } else {
            None
        }
    })
}

fn bounded_string<'a>(value: &'a Value, key: &str, max: usize) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|value| {
        !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
    })
}

fn valid_bounded_text(value: &str, max: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty()) && value.len() <= max && !value.chars().any(char::is_control)
}

pub(crate) fn valid_conversation_id(value: &str) -> bool {
    value.strip_prefix("nyxa-").is_some_and(|suffix| {
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn public_error(code: &str) -> NyxIdChatError {
    let message = match code {
        "insufficient_credits" => "NyxID 余额不足，这次对话没有执行。",
        "turn_timeout" => "这次处理超时了，没有继续执行。",
        "capacity_exceeded" | "assistant_unavailable" => "NyxID 助手暂时不可用，请稍后再试。",
        "cancelled" => "这次处理已经停止。",
        _ => "这次处理没有完成，请稍后再试。",
    };
    NyxIdChatError {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(cursor: u64, name: &str, extra: Value) -> Value {
        let mut value = serde_json::json!({"cursor": cursor, "event": name});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        value
    }

    fn history_message(
        id: &str,
        seq: i64,
        turn_id: &str,
        role: NyxIdChatMessageRole,
    ) -> NyxIdChatHistoryMessage {
        NyxIdChatHistoryMessage {
            id: id.to_owned(),
            seq,
            turn_id: turn_id.to_owned(),
            role,
            text: id.to_owned(),
            status: NyxIdChatMessageStatus::Completed,
            error_code: None,
        }
    }

    #[test]
    fn recovery_projects_only_the_receipts_exact_turn() {
        let history = NyxIdChatHistory {
            conversation: NyxIdChatHistoryConversation {
                id: "nyxa-0123456789abcdef0123456789abcdef".to_owned(),
                active_turn: true,
            },
            messages: vec![
                history_message(
                    "prior-result",
                    1,
                    "turn-prior",
                    NyxIdChatMessageRole::Assistant,
                ),
                history_message("target-user", 2, "turn-target", NyxIdChatMessageRole::User),
                history_message(
                    "later-result",
                    3,
                    "turn-later",
                    NyxIdChatMessageRole::Assistant,
                ),
            ],
        };

        let active = history
            .clone()
            .into_exact_admission_turn("turn-target", true)
            .unwrap();
        assert!(active.conversation.active_turn);
        assert_eq!(active.messages.len(), 1);
        assert_eq!(active.messages[0].id, "target-user");

        assert!(
            history
                .into_exact_admission_turn("turn-target", false)
                .is_err(),
            "a prior or later assistant result must not terminate the receipt's turn"
        );
    }

    #[test]
    fn terminal_recovery_requires_and_returns_the_exact_turns_assistant_result() {
        let history = NyxIdChatHistory {
            conversation: NyxIdChatHistoryConversation {
                id: "nyxa-0123456789abcdef0123456789abcdef".to_owned(),
                active_turn: true,
            },
            messages: vec![
                history_message("target-user", 1, "turn-target", NyxIdChatMessageRole::User),
                history_message(
                    "target-result",
                    2,
                    "turn-target",
                    NyxIdChatMessageRole::Assistant,
                ),
                history_message("later-user", 3, "turn-later", NyxIdChatMessageRole::User),
            ],
        };

        let terminal = history
            .into_exact_admission_turn("turn-target", false)
            .unwrap();
        assert!(!terminal.conversation.active_turn);
        assert_eq!(
            terminal
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["target-user", "target-result"]
        );
    }

    #[test]
    fn projects_only_public_turn_state_and_text() {
        let mut projection = TurnProjection::default();
        let request = uuid::Uuid::new_v4().to_string();
        let conversation = "nyxa-0123456789abcdef0123456789abcdef";

        let started = projection
            .apply(
                &request,
                event(
                    1,
                    "turn.status",
                    serde_json::json!({
                        "conversation_id": conversation,
                        "turn_id": "turn-1",
                        "status": "running"
                    }),
                ),
            )
            .unwrap();
        assert!(matches!(started, Some(NyxIdChatEvent::Started { .. })));

        let delta = projection
            .apply(
                &request,
                event(
                    2,
                    "block.delta",
                    serde_json::json!({
                        "block_id": "block-1",
                        "text": "已经帮你处理好了"
                    }),
                ),
            )
            .unwrap();
        assert_eq!(
            delta,
            Some(NyxIdChatEvent::Delta {
                request_id: request.clone(),
                text: "已经帮你处理好了".to_owned(),
            })
        );

        let snapshot = projection
            .apply(
                &request,
                event(
                    3,
                    "block.completed",
                    serde_json::json!({
                        "block_id": "block-1",
                        "block": {
                            "type": "text",
                            "block_id": "block-1",
                            "text": "完整回复：已经帮你处理好了"
                        }
                    }),
                ),
            )
            .unwrap();
        assert_eq!(
            snapshot,
            Some(NyxIdChatEvent::Snapshot {
                request_id: request.clone(),
                text: "完整回复：已经帮你处理好了".to_owned(),
            })
        );

        let completed = projection
            .apply(
                &request,
                event(
                    4,
                    "turn.completed",
                    serde_json::json!({
                        "turn_id": "turn-1",
                        "status": "completed",
                        "error": null
                    }),
                ),
            )
            .unwrap();
        assert!(matches!(
            completed,
            Some(NyxIdChatEvent::Completed {
                status: NyxIdChatStatus::Completed,
                error: None,
                ..
            })
        ));
    }

    #[test]
    fn decoder_preserves_split_utf8_and_crlf_records() {
        let raw = "data: {\"cursor\":1,\"event\":\"block.delta\",\"text\":\"你好\"}\r\n\r\n";
        let bytes = raw.as_bytes();
        let split = raw.find('好').unwrap() + 1;
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(&bytes[..split]).unwrap().is_empty());
        let values = decoder.push(&bytes[split..]).unwrap();
        assert_eq!(values[0]["text"], "你好");
    }

    #[test]
    fn rejects_conversation_identity_changes() {
        let mut projection = TurnProjection::default();
        projection
            .apply(
                "00000000-0000-4000-8000-000000000000",
                event(
                    1,
                    "turn.status",
                    serde_json::json!({
                        "conversation_id": "nyxa-0123456789abcdef0123456789abcdef",
                        "turn_id": "turn-1",
                        "status": "running"
                    }),
                ),
            )
            .unwrap();
        let error = projection
            .apply(
                "00000000-0000-4000-8000-000000000000",
                event(
                    2,
                    "turn.status",
                    serde_json::json!({
                        "conversation_id": "nyxa-ffffffffffffffffffffffffffffffff",
                        "turn_id": "turn-1",
                        "status": "waiting"
                    }),
                ),
            )
            .unwrap_err();
        assert!(error.contains("身份"));
    }

    #[test]
    fn rejects_a_different_conversation_when_continuing_a_thread() {
        let expected = "nyxa-0123456789abcdef0123456789abcdef";
        let mut projection = TurnProjection::new(Some(expected));

        let error = projection
            .apply(
                "00000000-0000-4000-8000-000000000000",
                event(
                    1,
                    "turn.status",
                    serde_json::json!({
                        "conversation_id": "nyxa-ffffffffffffffffffffffffffffffff",
                        "turn_id": "turn-1",
                        "status": "running"
                    }),
                ),
            )
            .unwrap_err();

        assert!(error.contains("身份"));
    }

    #[test]
    fn accepts_the_expected_conversation_when_continuing_a_thread() {
        let expected = "nyxa-0123456789abcdef0123456789abcdef";
        let mut projection = TurnProjection::new(Some(expected));

        let started = projection
            .apply(
                "00000000-0000-4000-8000-000000000000",
                event(
                    1,
                    "turn.status",
                    serde_json::json!({
                        "conversation_id": expected,
                        "turn_id": "turn-1",
                        "status": "running"
                    }),
                ),
            )
            .unwrap();

        assert!(matches!(
            started,
            Some(NyxIdChatEvent::Started {
                conversation_id,
                ..
            }) if conversation_id == expected
        ));
    }

    #[test]
    fn ignores_replayed_or_out_of_order_events() {
        let mut projection = TurnProjection::default();
        let request = "00000000-0000-4000-8000-000000000000";
        projection
            .apply(
                request,
                event(
                    1,
                    "turn.status",
                    serde_json::json!({
                        "conversation_id": "nyxa-0123456789abcdef0123456789abcdef",
                        "turn_id": "turn-1",
                        "status": "running"
                    }),
                ),
            )
            .unwrap();
        assert!(
            projection
                .apply(
                    request,
                    event(
                        2,
                        "block.delta",
                        serde_json::json!({
                            "block_id": "block-1",
                            "text": "kept"
                        })
                    ),
                )
                .unwrap()
                .is_some()
        );
        assert!(
            projection
                .apply(
                    request,
                    event(
                        2,
                        "block.delta",
                        serde_json::json!({
                            "block_id": "block-1",
                            "text": "duplicate"
                        })
                    ),
                )
                .unwrap()
                .is_none()
        );
        assert!(
            projection
                .apply(
                    request,
                    event(
                        1,
                        "turn.completed",
                        serde_json::json!({
                            "turn_id": "turn-1",
                            "status": "completed",
                            "error": null
                        })
                    ),
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(projection.reply_bytes, "kept".len());
        assert!(projection.terminal.is_none());

        let completed = projection
            .apply(
                request,
                event(
                    3,
                    "turn.completed",
                    serde_json::json!({
                        "turn_id": "turn-1",
                        "status": "completed",
                        "error": null
                    }),
                ),
            )
            .unwrap();
        assert!(matches!(
            completed,
            Some(NyxIdChatEvent::Completed {
                status: NyxIdChatStatus::Completed,
                ..
            })
        ));
    }

    #[test]
    fn history_projects_only_the_public_reconciliation_contract() {
        let conversation_id = "nyxa-0123456789abcdef0123456789abcdef";
        let wire: NyxIdChatHistoryWire = serde_json::from_value(serde_json::json!({
            "conversation": {
                "id": conversation_id,
                "title": "private title",
                "model": "private model",
                "agent": { "id": "private-agent" },
                "active_turn": {
                    "turn_id": "turn-private",
                    "activities": [{ "label": "private activity" }],
                    "attachments": [{ "id": "private-attachment" }]
                }
            },
            "messages": [
                {
                    "id": "message-1",
                    "seq": 1,
                    "turn_id": "turn-1",
                    "role": "user",
                    "text": "请帮我处理",
                    "status": "completed",
                    "error_code": null,
                    "activities": [{ "label": "private activity" }],
                    "attachments": [{ "id": "private-attachment" }],
                    "credential": "must-not-escape"
                },
                {
                    "id": "message-2",
                    "seq": 2,
                    "turn_id": "turn-1",
                    "role": "assistant",
                    "text": "已经处理好了",
                    "status": "failed",
                    "error_code": "assistant_failed"
                }
            ],
            "acknowledgements": [{ "summary": "private acknowledgement" }],
            "approvals": [{ "summary": "private approval" }],
            "waiting": [{ "title": "private waiting item" }]
        }))
        .unwrap();

        let public = wire.into_public(conversation_id).unwrap();
        assert_eq!(
            serde_json::to_value(public).unwrap(),
            serde_json::json!({
                "conversation": {
                    "id": conversation_id,
                    "activeTurn": true
                },
                "messages": [
                    {
                        "id": "message-1",
                        "seq": 1,
                        "turnId": "turn-1",
                        "role": "user",
                        "text": "请帮我处理",
                        "status": "completed",
                        "errorCode": null
                    },
                    {
                        "id": "message-2",
                        "seq": 2,
                        "turnId": "turn-1",
                        "role": "assistant",
                        "text": "已经处理好了",
                        "status": "failed",
                        "errorCode": "assistant_failed"
                    }
                ]
            })
        );
    }

    #[test]
    fn command_errors_serialize_only_the_public_disposition_and_message() {
        let error =
            NyxIdChatCommandError::admission_unknown("无法确认 NyxID 是否已经接收这条消息。");

        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({
                "kind": "admission_unknown",
                "message": "无法确认 NyxID 是否已经接收这条消息。"
            })
        );
    }

    #[test]
    fn history_rejects_mismatched_or_invalid_public_fields() {
        let expected = "nyxa-0123456789abcdef0123456789abcdef";
        let other = "nyxa-ffffffffffffffffffffffffffffffff";
        let mismatched: NyxIdChatHistoryWire = serde_json::from_value(serde_json::json!({
            "conversation": { "id": other, "active_turn": null },
            "messages": []
        }))
        .unwrap();
        assert!(mismatched.into_public(expected).is_err());

        let invalid_message: NyxIdChatHistoryWire = serde_json::from_value(serde_json::json!({
            "conversation": { "id": expected, "active_turn": null },
            "messages": [{
                "id": "message-1",
                "seq": 0,
                "turn_id": "turn-1",
                "role": "event",
                "text": "",
                "status": "completed",
                "error_code": null
            }]
        }))
        .unwrap();
        assert!(invalid_message.into_public(expected).is_err());

        let unknown_role = serde_json::from_value::<NyxIdChatHistoryWire>(serde_json::json!({
            "conversation": { "id": expected, "active_turn": null },
            "messages": [{
                "id": "message-1",
                "seq": 1,
                "turn_id": "turn-1",
                "role": "tool",
                "text": "not public",
                "status": "completed",
                "error_code": null
            }]
        }));
        assert!(unknown_role.is_err());
    }
}
