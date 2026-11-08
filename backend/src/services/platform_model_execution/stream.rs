use super::adapters::{self, Native, Output, invalid_response};
use crate::errors::AppResult;
use serde_json::{Value, json};

/// Bounded cumulative usage; no addition of repeated provider usage snapshots.
/// Tool arguments are buffered until a complete terminal response so a caller
/// never receives an unusable partial function call or a native identifier.
pub(super) struct Decoder {
    native: Native,
    decoder: crate::services::pool_ai_service::EventDecoder,
    body: Value,
    usage: Option<Value>,
    pub terminal: bool,
    sequence: Option<(i64, Value)>,
    claude_final_usage: bool,
}
impl Decoder {
    pub fn new(native: Native) -> Self {
        Self {
            native,
            decoder: Default::default(),
            body: match native {
                Native::Claude => json!({"type":"message","content":[],"stop_reason":null}),
                Native::Gemini => {
                    json!({"candidates":[{"content":{"parts":[]},"finishReason":null}]})
                }
                Native::GptOss => {
                    json!({"choices":[{"message":{"content":"","tool_calls":[]},"finish_reason":null}]})
                }
                Native::Responses => Value::Null,
            },
            usage: None,
            terminal: false,
            sequence: None,
            claude_final_usage: false,
        }
    }
    pub fn partial_evidence(&self) -> Option<crate::services::billing::model_usage::Evidence> {
        self.usage
            .as_ref()
            .and_then(|u| adapters::evidence(self.native, u).ok())
    }
    pub fn push(&mut self, bytes: &[u8]) -> AppResult<Vec<String>> {
        let mut texts = vec![];
        for event in self.decoder.push(bytes)? {
            if event.data == "[DONE]" {
                if self.terminal
                    || self.native != Native::GptOss
                    || self.body["choices"][0]["finish_reason"].is_null()
                {
                    return Err(invalid_response());
                }
                self.terminal = true;
                continue;
            }
            let data: Value = serde_json::from_str(&event.data).map_err(|_| invalid_response())?;
            if let Some(n) = data.get("sequence_number") {
                let n = n
                    .as_i64()
                    .filter(|n| *n >= 0)
                    .ok_or_else(invalid_response)?;
                if let Some((last, previous)) = &self.sequence {
                    if *last == n && previous == &data {
                        continue;
                    }
                    if n <= *last {
                        return Err(invalid_response());
                    }
                }
                self.sequence = Some((n, data.clone()));
            }
            if self.terminal
                || data.get("error").is_some()
                || event.event_type.as_deref() == Some("error")
            {
                return Err(invalid_response());
            }
            match self.native {
                Native::Responses => match data["type"].as_str().or(event.event_type.as_deref()) {
                    Some("response.output_text.delta") => {
                        texts.push(data["delta"].as_str().ok_or_else(invalid_response)?.into())
                    }
                    Some("response.completed" | "response.incomplete") => {
                        self.body = data["response"].clone();
                        self.usage = self.body.get("usage").cloned();
                        self.terminal = true;
                    }
                    Some("response.failed" | "response.error") => return Err(invalid_response()),
                    // These events contain private response/item identifiers. No forwarding.
                    Some(
                        "response.created"
                        | "response.in_progress"
                        | "response.output_item.added"
                        | "response.output_item.done"
                        | "response.content_part.added"
                        | "response.content_part.done"
                        | "response.output_text.done"
                        | "response.function_call_arguments.delta"
                        | "response.function_call_arguments.done"
                        | "response.reasoning_summary_part.added"
                        | "response.reasoning_summary_part.done"
                        | "response.reasoning_summary_text.delta"
                        | "response.reasoning_summary_text.done",
                    ) => {}
                    _ => return Err(invalid_response()),
                },
                Native::Claude => match data["type"].as_str() {
                    Some("message_start") => {
                        if self.usage.is_some() {
                            return Err(invalid_response());
                        }
                        self.usage = data["message"].get("usage").cloned();
                    }
                    Some("content_block_start") => {
                        let index = data["index"].as_u64().ok_or_else(invalid_response)? as usize;
                        let parts = self.body["content"]
                            .as_array_mut()
                            .ok_or_else(invalid_response)?;
                        if index != parts.len() || index > 128 {
                            return Err(invalid_response());
                        }
                        let block = &data["content_block"];
                        match block["type"].as_str() {
                            Some("text") => { let t = block["text"].as_str().ok_or_else(invalid_response)?; texts.push(t.into()); parts.push(json!({"type":"text","text":t})); }
                            Some("tool_use") => parts.push(json!({"type":"tool_use","name":block["name"],"input":block["input"],"partial":""})),
                            _ => return Err(invalid_response()),
                        }
                    }
                    Some("content_block_delta") => {
                        let index = data["index"].as_u64().ok_or_else(invalid_response)? as usize;
                        let part = self.body["content"]
                            .as_array_mut()
                            .and_then(|p| p.get_mut(index))
                            .ok_or_else(invalid_response)?;
                        match data["delta"]["type"].as_str() {
                            Some("text_delta") if part["type"] == "text" => {
                                let t = data["delta"]["text"]
                                    .as_str()
                                    .ok_or_else(invalid_response)?;
                                let mut s = part["text"]
                                    .as_str()
                                    .ok_or_else(invalid_response)?
                                    .to_owned();
                                s.push_str(t);
                                part["text"] = json!(s);
                                texts.push(t.into());
                            }
                            Some("input_json_delta") if part["type"] == "tool_use" => {
                                let mut s = part["partial"]
                                    .as_str()
                                    .ok_or_else(invalid_response)?
                                    .to_owned();
                                s.push_str(
                                    data["delta"]["partial_json"]
                                        .as_str()
                                        .ok_or_else(invalid_response)?,
                                );
                                part["partial"] = json!(s);
                            }
                            _ => return Err(invalid_response()),
                        }
                    }
                    Some("content_block_stop") => {
                        let index = data["index"].as_u64().ok_or_else(invalid_response)? as usize;
                        let part = self.body["content"]
                            .as_array_mut()
                            .and_then(|p| p.get_mut(index))
                            .ok_or_else(invalid_response)?;
                        if part["type"] == "tool_use"
                            && part["partial"].as_str().is_some_and(|s| !s.is_empty())
                        {
                            part["input"] = serde_json::from_str(part["partial"].as_str().unwrap())
                                .map_err(|_| invalid_response())?;
                        }
                    }
                    Some("message_delta") => {
                        if self.claude_final_usage
                            || data["usage"]["output_tokens"]
                                .as_i64()
                                .is_none_or(|n| n < 0)
                        {
                            return Err(invalid_response());
                        }
                        self.claude_final_usage = true;
                        self.body["stop_reason"] = data["delta"]["stop_reason"].clone();
                        let usage = self
                            .usage
                            .as_mut()
                            .and_then(Value::as_object_mut)
                            .ok_or_else(invalid_response)?;
                        for (k, v) in data["usage"].as_object().ok_or_else(invalid_response)? {
                            usage.insert(k.clone(), v.clone());
                        }
                    }
                    Some("message_stop") => {
                        if !self.claude_final_usage {
                            return Err(invalid_response());
                        }
                        self.terminal = true;
                    }
                    Some("ping") => {}
                    _ => return Err(invalid_response()),
                },
                Native::Gemini => {
                    if let Some(u) = data.get("usageMetadata") {
                        self.usage = Some(u.clone());
                    }
                    if let Some(candidates) = data["candidates"].as_array() {
                        if candidates.len() != 1 {
                            return Err(invalid_response());
                        }
                        let c = &candidates[0];
                        if let Some(parts) = c["content"]["parts"].as_array() {
                            for part in parts {
                                if part.get("thoughtSignature").is_some() {
                                    return Err(invalid_response());
                                }
                                if let Some(t) = part["text"].as_str()
                                    && part["thought"] != true
                                {
                                    texts.push(t.into());
                                }
                                self.body["candidates"][0]["content"]["parts"]
                                    .as_array_mut()
                                    .unwrap()
                                    .push(part.clone());
                            }
                        }
                        if let Some(reason) = c.get("finishReason") {
                            self.body["candidates"][0]["finishReason"] = reason.clone();
                        }
                    }
                    // Gemini's final usage may follow finishReason. EOF is its terminal marker.
                }
                Native::GptOss => {
                    if let Some(u) = data.get("usage").filter(|u| !u.is_null()) {
                        self.usage = Some(u.clone());
                    }
                    let choices = data["choices"].as_array().ok_or_else(invalid_response)?;
                    if choices.len() > 1 {
                        return Err(invalid_response());
                    }
                    if let Some(c) = choices.first() {
                        if c["index"] != 0 {
                            return Err(invalid_response());
                        }
                        if let Some(t) = c["delta"]["content"].as_str() {
                            let mut s = self.body["choices"][0]["message"]["content"]
                                .as_str()
                                .unwrap()
                                .to_owned();
                            s.push_str(t);
                            self.body["choices"][0]["message"]["content"] = json!(s);
                            texts.push(t.into());
                        }
                        if let Some(calls) = c["delta"]["tool_calls"].as_array() {
                            for call in calls {
                                let index = call["index"]
                                    .as_u64()
                                    .filter(|n| *n < 128)
                                    .ok_or_else(invalid_response)?
                                    as usize;
                                let stored = self.body["choices"][0]["message"]["tool_calls"]
                                    .as_array_mut()
                                    .unwrap();
                                if index > stored.len() {
                                    return Err(invalid_response());
                                }
                                if index == stored.len() {
                                    stored.push(json!({"type":"function","function":{"name":"","arguments":""}}));
                                }
                                for key in ["name", "arguments"] {
                                    if let Some(t) = call["function"][key].as_str() {
                                        let mut s = stored[index]["function"][key]
                                            .as_str()
                                            .unwrap()
                                            .to_owned();
                                        s.push_str(t);
                                        stored[index]["function"][key] = json!(s);
                                    }
                                }
                            }
                        }
                        if let Some(reason) = c.get("finish_reason").filter(|r| !r.is_null()) {
                            self.body["choices"][0]["finish_reason"] = reason.clone();
                        }
                    }
                }
            }
        }
        Ok(texts)
    }
    pub fn finish(
        &mut self,
        clean_eof: bool,
        id: &str,
    ) -> AppResult<(
        AppResult<Output>,
        crate::services::billing::model_usage::Evidence,
    )> {
        if self.native == Native::Gemini && clean_eof {
            self.terminal = !self.body["candidates"][0]["finishReason"].is_null();
        }
        if !self.terminal {
            return Err(invalid_response());
        }
        let usage = self.usage.as_ref().ok_or_else(invalid_response)?;
        Ok((
            adapters::output(self.native, &self.body, id),
            adapters::evidence(self.native, usage)?,
        ))
    }
}
