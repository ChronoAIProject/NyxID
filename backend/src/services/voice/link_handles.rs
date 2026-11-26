//! Short-lived URL references for native tool chaining, never spoken URLs.
use serde_json::{Value, json};
use std::collections::HashMap;

pub(super) struct LinkHandles {
    namespace: String,
    urls: HashMap<String, String>,
}
impl Default for LinkHandles {
    fn default() -> Self {
        Self {
            namespace: {
                use rand::Rng;
                const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
                let mut rng = rand::thread_rng();
                (0..12)
                    .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
                    .collect()
            },
            urls: HashMap::new(),
        }
    }
}
impl LinkHandles {
    pub(super) fn present(&mut self, state: &crate::AppState, mut output: Value) -> Value {
        let origins: Vec<String> = state
            .config
            .accepted_frontend_urls()
            .filter_map(|value| {
                url::Url::parse(value)
                    .ok()
                    .map(|url| url.origin().ascii_serialization())
            })
            .collect();
        let visible_reply = output["type"] == "task_status";
        let mut omitted = 0;
        let mut handled = 0;
        self.visit_output(&mut output, &origins, &mut omitted, &mut handled);
        if omitted + handled > 0 {
            if !output.is_object() {
                output = json!({"result":output});
            }
            if visible_reply {
                output["links_visible_in_chat_reply"] = (omitted + handled).into();
            } else {
                output["links_not_sent_to_chat"] = (omitted + handled).into();
            }
            output["link_handles_usable_as_tool_arguments"] = true.into();
            output["link_notice"] = if visible_reply {
                "These URLs were omitted from speech context but are visible in the chat reply. Link handles are usable only as service tool argument values in this call. Never read handles, URLs or tokens aloud."
            } else { "Link handles may be used exactly as tool argument values in this call. These links were NOT sent to the chat. Never read handles, URLs or tokens aloud. An omitted link has no usable handle; look it up again if needed." }.into();
        }
        output
    }
    fn visit_output(
        &mut self,
        value: &mut Value,
        origins: &[String],
        omitted: &mut u64,
        handled: &mut u64,
    ) {
        match value {
            Value::String(text) => {
                *text = super::links::map_urls(text, |raw| {
                    if super::links::private_url(raw, origins) {
                        *omitted += 1;
                        return super::links::PLACEHOLDER.to_owned();
                    }
                    if let Some((handle, _)) = self.urls.iter().find(|(_, url)| url.as_str() == raw)
                    {
                        *handled += 1;
                        return handle.clone();
                    }
                    if self.urls.len() >= 64 {
                        *omitted += 1;
                        return super::links::PLACEHOLDER.to_owned();
                    }
                    let handle = format!("[link {}:{}]", self.namespace, self.urls.len() + 1);
                    self.urls.insert(handle.clone(), raw.to_owned());
                    *handled += 1;
                    handle
                });
                let clean = super::links::redact_tokens(text);
                if clean != *text {
                    *omitted += 1;
                }
                *text = clean;
            }
            Value::Array(values) => {
                for value in values {
                    self.visit_output(value, origins, omitted, handled);
                }
            }
            Value::Object(values) => {
                let original = std::mem::take(values);
                for (key, mut value) in original {
                    let mut key = Value::String(key);
                    self.visit_output(&mut key, origins, omitted, handled);
                    self.visit_output(&mut value, origins, omitted, handled);
                    values.insert(key.as_str().unwrap_or_default().to_owned(), value);
                }
            }
            _ => {}
        }
    }
    /// Only exact string values issued by this instance may resolve. Unknown
    /// handles are data: leave them literal and report that fact after dispatch.
    pub(super) fn restore(&self, value: &mut Value) -> Vec<String> {
        let mut unknown = Vec::new();
        match value {
            Value::String(text) if text.starts_with("[link ") && text.ends_with(']') => {
                if let Some(url) = self.urls.get(text) {
                    *text = url.clone();
                } else {
                    unknown.push(text.clone());
                }
            }
            Value::Array(values) => {
                for value in values {
                    unknown.extend(self.restore(value));
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    unknown.extend(self.restore(value));
                }
            }
            _ => {}
        }
        unknown.sort();
        unknown.dedup();
        unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn external_links_chain_by_exact_per_call_handles_and_never_claim_chat_delivery() {
        let (state, _, _) = super::super::tests::setup("voice_link_handles").await;
        let mut handles = LinkHandles::default();
        let url = "https://search.example/doc?q=hello";
        let output = handles.present(&state, json!({"url":url,"again":url}));
        let handle = output["url"].as_str().unwrap();
        assert_eq!(output["again"], handle);
        assert!(!output.to_string().contains(url));
        assert_eq!(output["links_not_sent_to_chat"], 2);
        assert!(output.get("links_in_chat").is_none());
        let mut arguments = json!({"url":handle,"nested":[handle],"prose":format!("see {handle}")});
        assert!(handles.restore(&mut arguments).is_empty());
        assert_eq!(arguments["url"], url);
        assert_eq!(arguments["nested"][0], url);
        assert_eq!(arguments["prose"], format!("see {handle}"));
        let mut foreign = json!({"url":handle});
        assert_eq!(LinkHandles::default().restore(&mut foreign), [handle]);
        assert_eq!(foreign["url"], handle);
        let mut replay = json!({"url":handle});
        assert!(handles.restore(&mut replay).is_empty());
        assert_eq!(replay["url"], url);
        assert_eq!(handles.namespace.len(), 12);
        assert!(
            handles
                .namespace
                .bytes()
                .all(|ch| b"abcdefghijklmnopqrstuvwxyz234567".contains(&ch))
        );
        for index in 0..70 {
            let _ = handles.present(
                &state,
                json!({"url":format!("https://search.example/{index}")}),
            );
        }
        assert_eq!(handles.urls.len(), 64);
        let omitted = handles.present(&state, json!({"url":"https://new.example/overflow"}));
        assert_eq!(omitted["url"], super::super::links::PLACEHOLDER);
        assert_eq!(omitted["links_not_sent_to_chat"], 1);
        let secret = format!("{}/connect/bot/nyx_bcl_example", state.config.frontend_url);
        let private = handles.present(
            &state,
            json!({secret.clone():secret,"bare":"nyx_clk_example"}),
        );
        assert!(
            !private.to_string().contains("nyx_bcl_") && !private.to_string().contains("nyx_clk_")
        );
        assert_eq!(
            handles.urls.len(),
            64,
            "private links never receive handles"
        );
        let status = handles.present(&state, json!({"type":"task_status","tasks":[{"result":"https://external.example/chat-result"}]}));
        assert_eq!(status["links_visible_in_chat_reply"], 1);
        assert!(status.get("links_not_sent_to_chat").is_none());
        assert!(!status["link_notice"].as_str().unwrap().contains("NOT sent"));
        state.db.drop().await.unwrap();
    }
}
