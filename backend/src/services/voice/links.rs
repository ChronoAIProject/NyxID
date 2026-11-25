//! URLs belong in the visible private thread, never in a voice model's context.
use std::sync::LazyLock;

use mongodb::{
    Database,
    bson::{self, doc},
};
use regex::Regex;
use serde_json::Value;

use crate::{
    errors::{AppError, AppResult},
    models::assistant_message::AssistantMessage,
};

pub(crate) const PLACEHOLDER: &str = "[link omitted]";
pub(crate) const INSTRUCTIONS: &str = "Never read URLs, tokens or identifiers aloud. For links_in_chat, briefly say the link is in the chat. A link publication failure means it was not sent; do not claim otherwise.";
static URLS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:[a-z][a-z0-9+.-]*://|www\.)[^\s<>\"`\\]+"#).expect("voice URL pattern")
});
static TOKENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"nyx_(?:bcl|clk)_[A-Za-z0-9_-]+").expect("connect token pattern"));
static URL_TOKENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"nyx_[A-Za-z0-9]+_[A-Za-z0-9_-]+").expect("URL token pattern"));
// These frontend routes carry one-time credentials even when a model mangles
// the token. Ordinary page links in task replies must remain usable.
const SECRET_TASK_REPLY_ROUTES: &[&str] = &[
    "/connect",
    "/cli/pair",
    "/login/device",
    "/login/agent-key",
    "/login/code",
];

pub(crate) fn redact_tokens(text: &str) -> String {
    // Redact a credential-bearing URL as a whole before replacing bare tokens.
    // Inserting a spaced placeholder inside its path would leave URL fragments
    // when a later URL-aware task/context sanitizer runs.
    let without_token_urls = map_urls(text, |raw| {
        if TOKENS.is_match(raw) {
            PLACEHOLDER.to_owned()
        } else {
            raw.to_owned()
        }
    });
    TOKENS
        .replace_all(&without_token_urls, PLACEHOLDER)
        .into_owned()
}
pub(crate) fn voice_row(message: &AssistantMessage) -> bool {
    message.via.as_deref() == Some("voice") || message.voice.is_some() || publication_row(message)
}

pub(crate) fn message_text<'a>(
    message: &AssistantMessage,
    text: &'a str,
) -> std::borrow::Cow<'a, str> {
    if voice_row(message) {
        std::borrow::Cow::Owned(redact_tokens(text))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// A loaded thread's scope needs no history or authorization reads.
pub(crate) fn thread_text(hidden_voice: bool, text: String) -> String {
    if hidden_voice {
        redact_tokens(&text)
    } else {
        text
    }
}

/// Callers without a loaded conversation need only its hidden-voice flag.
/// An unavailable flag must not expose credentials or fail the caller's turn.
pub(crate) async fn hidden_voice_thread(db: &Database, user: &str, conversation: &str) -> bool {
    db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": conversation, "user_id": user})
        .projection(doc! {"_id": 0, "voice_parent_conversation_id": 1})
        .await
        .map_or(true, |row| {
            row.is_some_and(|row| row.get_str("voice_parent_conversation_id").is_ok())
        })
}
pub(crate) fn publication_row(message: &AssistantMessage) -> bool {
    message
        .turn_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.voice_link_publication.is_some())
}
pub(crate) async fn published_count(
    db: &Database,
    user: &str,
    parent: &str,
    hidden: &str,
    turn: &str,
) -> AppResult<u64> {
    use futures::TryStreamExt;
    let mut rows = db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .find(doc! {
            "user_id":user,"conversation_id":parent,
            "turn_metadata.voice_link_publication.conversation_id":hidden,
            "turn_metadata.voice_link_publication.turn_id":turn,
        })
        .projection(doc! {"turn_metadata.voice_link_publication.links_in_chat":1})
        .await?;
    let mut count = 0_u64;
    while let Some(row) = rows.try_next().await? {
        count = count.saturating_add(
            row.get_document("turn_metadata")
                .ok()
                .and_then(|m| m.get_document("voice_link_publication").ok())
                .and_then(|m| m.get_i64("links_in_chat").ok())
                .unwrap_or(0)
                .max(0) as u64,
        );
    }
    Ok(count)
}

pub(crate) fn contains_url(text: &str) -> bool {
    let lowercase = text.to_ascii_lowercase();
    lowercase.contains("://") || lowercase.contains("www.")
}
fn closing_pair(ch: char) -> Option<char> {
    match ch {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        '）' => Some('（'),
        '」' => Some('「'),
        '』' => Some('『'),
        '】' => Some('【'),
        '》' => Some('《'),
        '〉' => Some('〈'),
        '”' => Some('“'),
        '’' => Some('‘'),
        _ => None,
    }
}
fn boundary(raw: &str) -> usize {
    let mut end = raw.len();
    while let Some(ch) = raw[..end].chars().next_back() {
        if let Some(opener) = closing_pair(ch) {
            let part = &raw[..end];
            if part.matches(ch).count() > part.matches(opener).count() {
                end -= ch.len_utf8();
                continue;
            }
            break;
        }
        if matches!(
            ch,
            '.' | ','
                | ';'
                | '!'
                | '?'
                | ':'
                | '*'
                | '\''
                | '"'
                | '。'
                | '，'
                | '、'
                | '！'
                | '？'
                | '；'
                | '：'
                | '…'
                | '・'
        ) || ('\u{ff01}'..='\u{ff0f}').contains(&ch)
            || ('\u{ff1a}'..='\u{ff20}').contains(&ch)
            || ('\u{ff3b}'..='\u{ff40}').contains(&ch)
            || ('\u{ff5b}'..='\u{ff65}').contains(&ch)
            || ('\u{3000}'..='\u{303f}').contains(&ch)
        {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }
    end
}

// Preserve complete URL extents, including Unicode paths/query values. Known
// one-time connect paths have an ASCII token alphabet; adjacent prose is text.
fn spans(text: &str) -> Vec<(usize, usize)> {
    static STARTS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(?:[a-z][a-z0-9+.-]*://|www\.)").unwrap());
    static CONNECT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^(?:https?://|www\.)[^\s/]+/connect/(?:bot/)?nyx_(?:bcl|clk)_[A-Za-z0-9_-]+",
        )
        .unwrap()
    });
    let starts: Vec<_> = STARTS.find_iter(text).collect();
    let mut spans = Vec::new();
    let mut cursor = 0;
    for (index, start) in starts.iter().enumerate() {
        if start.start() < cursor {
            continue;
        }
        let rest = &text[start.start()..];
        let extent = URLS.find(rest).map_or(start.len(), |found| found.end());
        let extent = starts
            .iter()
            .skip(index + 1)
            .find(|next| {
                let before = text[..next.start()].chars().next_back().unwrap_or(' ');
                matches!(
                    before,
                    '。' | '，'
                        | '、'
                        | '！'
                        | '？'
                        | '；'
                        | '：'
                        | '…'
                        | '・'
                        | ','
                        | ';'
                        | '!'
                        | '?'
                        | ')'
                        | ']'
                        | '}'
                ) || (!before.is_ascii()
                    && text[start.end()..next.start()]
                        .contains(['。', '，', '、', '！', '？', '；', '：', '…', '・']))
                    || closing_pair(before).is_some()
                    || ('\u{ff01}'..='\u{ff0f}').contains(&before)
                    || ('\u{ff1a}'..='\u{ff20}').contains(&before)
                    || ('\u{ff3b}'..='\u{ff40}').contains(&before)
                    || ('\u{ff5b}'..='\u{ff65}').contains(&before)
                    || ('\u{3000}'..='\u{303f}').contains(&before)
            })
            .map_or(extent, |next| extent.min(next.start() - start.start()));
        let raw = &rest[..extent];
        let end = CONNECT
            .find(raw)
            .map_or_else(|| boundary(raw), |found| found.end());
        cursor = start.start() + end;
        spans.push((start.start(), cursor));
    }
    spans
}
pub(super) fn map_urls(text: &str, mut replace: impl FnMut(&str) -> String) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    for (start, end) in spans(text) {
        output.push_str(&text[cursor..start]);
        output.push_str(&replace(&text[start..end]));
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}
pub(crate) fn redact(text: &str) -> (String, usize) {
    let mut count = 0;
    let without_urls = map_urls(text, |_| {
        count += 1;
        PLACEHOLDER.to_owned()
    });
    (redact_tokens(&without_urls), count)
}

pub(crate) fn frontend_origins(config: &crate::config::AppConfig) -> Vec<String> {
    config
        .accepted_frontend_urls()
        .filter_map(|value| {
            url::Url::parse(value)
                .ok()
                .map(|url| url.origin().ascii_serialization())
        })
        .collect()
}

/// Keep ordinary page links; only the exact publication row owns secret URLs.
pub(crate) fn redact_task_reply(text: &str, origins: &[String]) -> String {
    redact_tokens(&map_urls(text, |raw| {
        let secret_url = URL_TOKENS.is_match(raw)
            || url::Url::parse(raw).ok().is_some_and(|url| {
                origins.contains(&url.origin().ascii_serialization())
                    && SECRET_TASK_REPLY_ROUTES.iter().any(|route| {
                        url.path() == *route
                            || url
                                .path()
                                .strip_prefix(*route)
                                .is_some_and(|rest| rest.starts_with('/'))
                    })
            });
        if secret_url {
            PLACEHOLDER.to_owned()
        } else {
            raw.to_owned()
        }
    }))
}
#[cfg(test)]
fn redact_value(value: &mut Value) {
    match value {
        Value::String(text) => *text = redact(text).0,
        Value::Array(values) => values.iter_mut().for_each(redact_value),
        Value::Object(values) => {
            let original = std::mem::take(values);
            for (key, mut value) in original {
                redact_value(&mut value);
                values.insert(redact(&key).0, value);
            }
        }
        _ => {}
    }
}
pub(crate) fn collect(value: &Value, origins: &[String], links: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            for (start, end) in spans(text) {
                let raw = ascii_url(&text[start..end]);
                if let Ok(url) = url::Url::parse(raw)
                    && matches!(url.scheme(), "https" | "http")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && origins.contains(&url.origin().ascii_serialization())
                    && !links.iter().any(|link| {
                        link == raw || (link.starts_with(raw) && boundary(link) == raw.len())
                    })
                {
                    links.push(raw.to_owned());
                }
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect(value, origins, links)),
        Value::Object(values) => {
            // Prefer explicit URL values over a prose rendering of the same
            // destination. Still inspect the rest for additional destinations.
            if let Some(url) = values.get("url") {
                if let Some(raw) = url.as_str().filter(|raw| ascii_url(raw) == *raw) {
                    if let Ok(url) = url::Url::parse(raw)
                        && matches!(url.scheme(), "https" | "http")
                        && url.username().is_empty()
                        && url.password().is_none()
                        && origins.contains(&url.origin().ascii_serialization())
                        && !links.iter().any(|link| link == raw)
                    {
                        links.push(raw.to_owned());
                    }
                } else {
                    collect(url, origins, links);
                }
            }
            for (key, value) in values {
                if key == "url" {
                    continue;
                }
                collect(&Value::String(key.clone()), origins, links);
                collect(value, origins, links);
            }
        }
        _ => {}
    }
}

// RFC 3986 unreserved + reserved + percent-encoding alphabet. NyxID page
// destinations are ASCII; unencoded CJK characters belong to surrounding prose.
fn ascii_url(raw: &str) -> &str {
    let end = raw
        .bytes()
        .position(|byte| {
            !(byte.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&byte))
        })
        .unwrap_or(raw.len());
    &raw[..end]
}
pub(super) fn private_url(raw: &str, origins: &[String]) -> bool {
    TOKENS.is_match(raw)
        || url::Url::parse(raw)
            .ok()
            .is_some_and(|url| origins.contains(&url.origin().ascii_serialization()))
}
fn redact_private_value(value: &mut Value, origins: &[String]) -> u64 {
    match value {
        Value::String(text) => {
            let mut omitted = 0;
            let redacted = map_urls(text, |raw| {
                if private_url(raw, origins) {
                    omitted += 1;
                    PLACEHOLDER.to_owned()
                } else {
                    raw.to_owned()
                }
            });
            omitted += TOKENS.find_iter(&redacted).count() as u64;
            *text = redact_tokens(&redacted);
            omitted
        }
        Value::Array(values) => values
            .iter_mut()
            .map(|value| redact_private_value(value, origins))
            .sum(),
        Value::Object(values) => {
            let original = std::mem::take(values);
            let mut omitted = 0;
            for (key, mut value) in original {
                omitted += redact_private_value(&mut value, origins);
                let mut key = Value::String(key);
                omitted += redact_private_value(&mut key, origins);
                values.insert(key.as_str().unwrap_or_default().to_owned(), value);
            }
            omitted
        }
        _ => 0,
    }
}

pub(crate) struct Publication {
    pub user_id: String,
    pub parent_id: String,
    pub conversation_id: String,
    pub turn_id: String,
}

fn publication_id(call: &Publication, tool_call: &str, links: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(
        serde_json::to_vec(links).expect("URL list serialization"),
    ));
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!(
            "nyxid:voice-links:{}:{}:{tool_call}:{digest}",
            call.conversation_id, call.turn_id
        )
        .as_bytes(),
    )
    .to_string()
}

/// The unique message _id and conversation sequence are committed together.
/// This deliberately never uses turn completion or channel delivery machinery:
/// a NyxBot home thread may have reply_channel set by its last private-chat sender.
async fn publish(
    db: &Database,
    call: &Publication,
    tool_call: &str,
    links: &[String],
) -> AppResult<()> {
    use crate::models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as THREADS},
        assistant_message::COLLECTION_NAME as MESSAGES,
    };
    let id = publication_id(call, tool_call, links);
    let text = links
        .iter()
        .map(|url| format!("<{url}>"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let links = links.to_vec();
    let db = db.clone();
    let call = Publication {
        user_id: call.user_id.clone(),
        parent_id: call.parent_id.clone(),
        conversation_id: call.conversation_id.clone(),
        turn_id: call.turn_id.clone(),
    };
    let mut tx = db.client().start_session().await?;
    tx.start_transaction().and_run2(async move |tx| {
        let result: AppResult<()> = Box::pin(async {
            let messages = db.collection::<AssistantMessage>(MESSAGES);
            if messages.find_one(doc!{"_id": &id, "user_id": &call.user_id, "conversation_id": &call.parent_id}).session(&mut *tx).await?.is_some() { return Ok(()); }
            if db.collection::<bson::Document>(THREADS).update_one(doc! {
                "_id": &call.conversation_id, "user_id": &call.user_id,
                "voice_parent_conversation_id": &call.parent_id,
                "active_turn.turn_id": &call.turn_id, "active_turn.stop_requested": {"$ne":true},
                "$or":[{"active_turn.lease_expires_at":{"$gt":bson::DateTime::now()}},
                    {"active_turn.lease_expires_at":null,"active_turn.started_at":{"$gt":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(crate::services::assistant_nyxagent::ACTIVE_TURN_TTL_SECS))}}]
            }, doc!{"$set":{"updated_at":bson::DateTime::now()}}).session(&mut *tx).await?.matched_count != 1 { return Err(AppError::AssistantTurnRequired); }
            let thread = db.collection::<AssistantConversation>(THREADS).find_one_and_update(
                doc!{"_id": &call.parent_id, "user_id": &call.user_id},
                doc!{"$inc":{"message_count":1},"$set":{"updated_at":bson::DateTime::now()}})
                .return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await?
                .ok_or_else(|| AppError::NotFound("Voice conversation unavailable".into()))?;
            messages.insert_one(AssistantMessage {
                supplemental_result_claims: Vec::new(),
                automation_work: None, execution_task: None, delegated_images: Default::default(),
                media_queue_notice_id: None, media_event_ids: Vec::new(), media_pending: None, media_bound: None,
                platform_delivery_recovery_queued: false, platform_reply: None, platform_origin: None,
                author: None, steering: None,
                // No speech segment: this row is a presentation boundary.
                voice: None, execution_pending: true, id: id.clone(), conversation_id: call.parent_id.clone(),
                user_id: call.user_id.clone(), seq: thread.message_count, turn_id: call.turn_id.clone(),
                role: "assistant".into(), text: text.clone(), status: "completed".into(), error_code: None,
                upstream_error_code: None, error_detail: None, turn_metadata: Some(crate::models::assistant_message::TurnMetadata {
                    voice_link_publication: Some(crate::models::assistant_message::VoiceLinkPublication {
                        conversation_id: call.conversation_id.clone(), turn_id: call.turn_id.clone(), links_in_chat: links.len() as u64,
                    }), ..Default::default()
                }), specialist_id: None,
                created_at: chrono::Utc::now(), activities: Vec::new(), activity_total: 0, attachments: Vec::new(),
                origin: None, via: Some("voice".into()), retry_of: None, retry_claimed_by: None,
                activity_count: None, continuation_of: None, report_to: None,
            }).session(&mut *tx).await?;
            Ok(())
        }).await;
        crate::services::api_key_mutation_service::transaction_result(result)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

/// Fail closed on presentation: tool success is retained, delivery failure is
/// explicit, and raw URLs/tokens are removed even if the publisher is unavailable.
#[cfg(test)]
pub(crate) async fn present(
    state: &crate::AppState,
    call: &Publication,
    tool_call: &str,
    result: Value,
) -> Value {
    present_optional(state, Some(call), tool_call, result).await
}

pub(crate) async fn present_optional(
    state: &crate::AppState,
    call: Option<&Publication>,
    tool_call: &str,
    mut result: Value,
) -> Value {
    let origins = frontend_origins(&state.config);
    let mut links = Vec::new();
    collect(&result, &origins, &mut links);
    let published = links.is_empty()
        || match call {
            Some(call) if !tool_call.is_empty() => {
                publish(&state.db, call, tool_call, &links).await.is_ok()
            }
            _ => false,
        };
    let omitted = redact_private_value(&mut result, &origins);
    if omitted > 0 && links.is_empty() {
        if !result.is_object() {
            result = serde_json::json!({"result":result});
        }
        result["links_not_sent_to_chat"] = omitted.into();
        result["link_notice"] = "Private links or tokens were omitted and were NOT sent to the chat. Never repeat them aloud or claim they were sent.".into();
        if let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) {
            content.push(serde_json::json!({"type":"text","text":"Private links or tokens were omitted and were NOT sent to the chat. Never repeat them aloud or claim they were sent."}));
        }
    }
    if !links.is_empty() {
        if !result.is_object() {
            result = serde_json::json!({"result":result});
        }
        result["links_in_chat"] = if published {
            links.len().into()
        } else {
            0.into()
        };
        if !published {
            result["link_publication"] = "failed".into();
            result["link_notice"] = "The link could not be sent to the chat. Do not claim it was sent, and do not repeat the service action.".into();
        }
    }
    if !links.is_empty() {
        let facts = serde_json::json!({
            "links_in_chat": if published { links.len() } else { 0 },
            "link_publication": if published { "sent" } else { "failed" },
            "notice": if published { "The exact links were sent to the chat. Say briefly that the link is in the chat; never read URLs or tokens aloud." } else { "The link was NOT sent. Do not claim it was sent and do not repeat the service action." },
        });
        if let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) {
            content.push(serde_json::json!({"type":"text","text":facts.to_string()}));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_defense_redacts_complete_urls_and_is_stable_before_task_reply_sanitization() {
        let origins = ["https://nyxid.ai".into()];
        for url in [
            "https://nyxid.ai/connect/nyx_clk_example",
            "https://nyxid.ai/connect/bot/nyx_bcl_example",
            "https://external.test/path?token=nyx_clk_example",
        ] {
            let text = format!("链接：{url}。 nyx_bcl_bare https://nyxid.ai/docs/cli");
            let expected = "链接：[link omitted]。 [link omitted] https://nyxid.ai/docs/cli";
            let redacted = redact_tokens(&text);
            assert_eq!(redacted, expected);
            assert_eq!(redact_tokens(&redacted), expected);
            assert_eq!(redact_task_reply(&redacted, &origins), expected);
        }
    }

    #[test]
    fn cjk_urls_and_bare_tokens_never_enter_model_context() {
        let (text, count) = redact(
            "链接：https://nyxid.ai/connect/bot/nyx_bcl_example。还有www.example.com，nyx_clk_example",
        );
        assert_eq!(count, 2);
        assert!(!contains_url(&text));
        assert!(!text.contains("nyx_bcl_") && !text.contains("nyx_clk_"));
        assert_eq!(text, "链接：[link omitted]。还有[link omitted]");
    }

    #[test]
    fn copied_task_defense_preserves_external_findings_without_any_connect_url_fragment() {
        let text = redact_task_reply(
            "https://nyxid.ai/connect/bot/nyx_bxl_mangled。 https://nyxid.ai/connect/nyz_clk_truncated。 https://github.com/owner/repo/pull/42 nyx_clk_example nyx_bcl_example https://nyxid.ai/settings。 NYX_CLIENT_ID nyx_config nyx_cert nyx_b nyx_c",
            &["https://nyxid.ai".into()],
        );
        assert_eq!(
            text,
            "[link omitted]。 [link omitted]。 https://github.com/owner/repo/pull/42 [link omitted] [link omitted] https://nyxid.ai/settings。 NYX_CLIENT_ID nyx_config nyx_cert nyx_b nyx_c"
        );
    }

    #[test]
    fn copied_task_reply_redacts_only_secret_routes_and_token_urls() {
        let frontend = "https://nyxid.ai";
        let origins = [frontend.into(), "https://alias.nyxid.ai".into()];
        let settings = crate::services::assistant_links::AssistantPage::Settings { tab: "privacy" }
            .url(frontend);
        for (url, secret) in [
            (format!("{frontend}/docs/cli"), false),
            (settings, false),
            (format!("{frontend}/assistant/approvals"), false),
            (format!("{frontend}/blog/voice"), false),
            (format!("{frontend}/connectivity"), false),
            (format!("{frontend}/cli/pairing"), false),
            (format!("{frontend}/login/device-help"), false),
            (format!("{frontend}/login/agent-keys"), false),
            (format!("{frontend}/login/codes"), false),
            (
                "https://external.test/login/device?user_code=ABC".into(),
                false,
            ),
            (format!("{frontend}/connect/bot/nyx_bxl_abc"), true),
            (format!("{frontend}/connect/nyx_clk_x"), true),
            (format!("{frontend}/connect/nyz_clx_typo"), true),
            (format!("{frontend}/cli/pair?code=ABC"), true),
            (format!("{frontend}/login/device?user_code=ABC"), true),
            (format!("{frontend}/login/agent-key?code=ABC"), true),
            (format!("{frontend}/login/code?code=ABC"), true),
            ("https://alias.nyxid.ai/connect/bot/typo".into(), true),
            (
                "https://external.test/path?token=nyx_other_secret".into(),
                true,
            ),
            (format!("{frontend}/docs/nyx_bxl_abc"), true),
        ] {
            let text = format!("链接：{url}。 NYX_CLIENT_ID nyx_config nyx_cert");
            let expected = if secret { PLACEHOLDER } else { &url };
            assert_eq!(
                redact_task_reply(&text, &origins),
                format!("链接：{expected}。 NYX_CLIENT_ID nyx_config nyx_cert"),
                "{url}"
            );
        }
    }
    #[test]
    fn frontend_publication_matches_shared_client_connect_link_vectors_without_fragments() {
        let fixtures: Value =
            serde_json::from_str(include_str!("../../../../shared/autolink-punctuation.json"))
                .unwrap();
        for fixture in fixtures.as_array().unwrap() {
            let text = fixture["text"].as_str().unwrap();
            let mut links = Vec::new();
            collect(
                &Value::String(text.into()),
                &["https://nyxid.ai".into()],
                &mut links,
            );
            let expected: Vec<_> = fixture["hrefs"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .filter(|url| url.starts_with("https://nyxid.ai/"))
                .collect();
            assert_eq!(links, expected, "{text}");
            let context = redact(text).0;
            assert!(
                !contains_url(&context)
                    && !context.contains("nyx_bcl_")
                    && !context.contains("nyx_clk_"),
                "{context}"
            );
        }
        let context = redact(
            "https://zh.wikipedia.org/wiki/中文（消歧义） https://example.com/s?q=你好，世界&x=1",
        )
        .0;
        assert!(!context.contains("消歧义") && !context.contains("世界"));
    }
    #[test]
    fn trailing_boundary_preserves_interior_unmatched_closers_without_model_fragments() {
        for url in [
            "https://en.wikipedia.org/wiki/Ender’s_Game",
            "https://example.com/a)b",
            "https://example.com/a”b",
            "https://example.com/（balanced）",
        ] {
            assert_eq!(map_urls(url, |raw| format!("<{raw}>")), format!("<{url}>"));
            assert_eq!(redact(url).0, PLACEHOLDER);
        }
    }

    #[test]
    fn only_exact_configured_origins_are_published() {
        let result = serde_json::json!({"content":[{"text":"链接：https://nyxid.ai/connect/bot/nyx_bcl_example。 https://nyxid.ai.evil.test/z https://other.test/z"}]});
        let mut links = Vec::new();
        collect(&result, &["https://nyxid.ai".into()], &mut links);
        assert_eq!(links, ["https://nyxid.ai/connect/bot/nyx_bcl_example"]);
        let mut sanitized = result;
        redact_value(&mut sanitized);
        assert!(!sanitized.to_string().contains("nyx_bcl_"));
        assert!(!sanitized.to_string().contains("https://"));
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use crate::models::assistant_voice_session::COLLECTION_NAME as CALLS;
    use crate::models::{
        assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS},
        assistant_conversation::{
            AssistantConversation, COLLECTION_NAME as THREADS, ChannelOrigin,
        },
        assistant_message::COLLECTION_NAME as MESSAGES,
    };
    use futures::TryStreamExt;

    async fn present(
        state: &crate::AppState,
        call: &crate::models::assistant_voice_session::VoiceSession,
        tool: &str,
        result: Value,
    ) -> Value {
        let hidden = format!("hidden-{}", call.id);
        state.db.collection::<bson::Document>(THREADS).update_one(doc!{"_id":&hidden},doc!{"$setOnInsert":{
            "user_id":&call.user_id,"voice_parent_conversation_id":&call.conversation_id,
            "active_turn":{"turn_id":&call.id,"stop_requested":false,"started_at":bson::DateTime::now(),"lease_expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(5))}
        }}).upsert(true).await.unwrap();
        super::present(
            state,
            &Publication {
                user_id: call.user_id.clone(),
                parent_id: call.conversation_id.clone(),
                conversation_id: hidden,
                turn_id: call.id.clone(),
            },
            tool,
            result,
        )
        .await
    }

    fn publication_id(
        state: &crate::AppState,
        call: &crate::models::assistant_voice_session::VoiceSession,
        tool: &str,
    ) -> String {
        let mut links = Vec::new();
        collect(
            &tool_result(state),
            &frontend_origins(&state.config),
            &mut links,
        );
        super::publication_id(
            &Publication {
                user_id: call.user_id.clone(),
                parent_id: call.conversation_id.clone(),
                conversation_id: format!("hidden-{}", call.id),
                turn_id: call.id.clone(),
            },
            tool,
            &links,
        )
    }
    fn tool_result(state: &crate::AppState) -> Value {
        serde_json::json!({"content":[{"type":"text","text":format!(
            "链接：{}/connect/bot/nyx_bcl_example。 {}/connect/nyx_clk_example",
            state.config.frontend_url, state.config.frontend_url
        )}], "isError":false})
    }
    fn assert_private_model_output(output: &Value, count: usize) {
        let serialized = output.to_string();
        assert!(!serialized.contains("nyx_bcl_") && !serialized.contains("nyx_clk_"));
        assert!(!contains_url(&serialized));
        assert!(serialized.contains(PLACEHOLDER));
        assert_eq!(output["links_in_chat"], count);
    }

    async fn published_fixture(
        name: &str,
    ) -> (
        crate::AppState,
        AssistantConversation,
        crate::models::assistant_voice_session::VoiceSession,
        AssistantMessage,
    ) {
        use crate::services::{assistant_nyxagent as engine, assistant_voice as voice};
        let (state, thread, call) = super::super::tests::setup(name).await;
        let request = voice::enqueue(
            &state.db,
            &call.user_id,
            &thread.id,
            &call.id,
            "link-producer",
            "Create a link",
        )
        .await
        .unwrap();
        let request = voice::ensure_task_conversation(&state.db, &state.encryption_keys, &request)
            .await
            .unwrap();
        let start: engine::TurnRequest = serde_json::from_value(serde_json::json!({
            "conversation_id": request.task_conversation_id, "text": "Create a link",
        }))
        .unwrap();
        let hidden = Box::pin(engine::begin_turn_with_voice(
            &state.db,
            &call.user_id,
            &start,
            &state.encryption_keys,
            Some(&request.id),
        ))
        .await
        .unwrap();
        let publication = Publication {
            user_id: call.user_id.clone(),
            parent_id: thread.id.clone(),
            conversation_id: hidden.id.clone(),
            turn_id: hidden.active_turn.as_ref().unwrap().turn_id.clone(),
        };
        let output = super::present(&state, &publication, "published", tool_result(&state)).await;
        assert_eq!(output["links_in_chat"], 2);
        let mut links = Vec::new();
        collect(
            &tool_result(&state),
            &frontend_origins(&state.config),
            &mut links,
        );
        let id = super::publication_id(&publication, "published", &links);
        let row: AssistantMessage = state
            .db
            .collection(MESSAGES)
            .find_one(doc! {"_id": id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(uuid::Uuid::parse_str(&row.id).unwrap().get_version_num(), 5);
        assert_eq!(row.role, "assistant");
        assert_eq!(row.status, "completed");
        assert_eq!(row.via.as_deref(), Some("voice"));
        assert!(row.execution_pending && row.voice.is_none() && super::publication_row(&row));
        finish(&state, &hidden, "The links are in chat").await;
        (state, thread, call, row)
    }

    async fn finish(state: &crate::AppState, thread: &AssistantConversation, text: &str) {
        use crate::services::assistant_nyxagent as engine;
        let error = Box::pin(engine::finish_turn(
            &state.db,
            thread,
            &thread.credential_api_key_id,
            &uuid::Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: text.into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        ))
        .await
        .unwrap();
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn published_link_cannot_be_claimed_as_voice_input_at_either_claim_site() {
        use crate::models::assistant_voice::{REQUESTS, RequestState, VoiceRequest};
        use crate::services::{assistant_nyxagent as engine, assistant_voice as voice};
        for handler in [false, true] {
            let (state, thread, call, link) = published_fixture("voice_link_claim_shield").await;
            let request = voice::enqueue(
                &state.db,
                &call.user_id,
                &thread.id,
                &call.id,
                "crafted-link-input",
                "Ordinary input",
            )
            .await
            .unwrap();
            state.db.collection::<bson::Document>(REQUESTS).update_one(
                doc! {"_id": &request.id},
                doc! {"$set": {"message_id": &link.id, "message_seq": link.seq, "turn_id": &link.turn_id}},
            ).await.unwrap();
            let crafted: VoiceRequest = state
                .db
                .collection(REQUESTS)
                .find_one(doc! {"_id": &request.id})
                .await
                .unwrap()
                .unwrap();
            let crafted =
                voice::ensure_task_conversation(&state.db, &state.encryption_keys, &crafted)
                    .await
                    .unwrap();
            if handler {
                Box::pin(crate::handlers::assistant_voice::sweep(&state))
                    .await
                    .unwrap();
            } else {
                let start: engine::TurnRequest = serde_json::from_value(serde_json::json!({
                    "conversation_id": crafted.task_conversation_id, "text": link.text,
                }))
                .unwrap();
                let result = Box::pin(engine::begin_turn_with_voice(
                    &state.db,
                    &call.user_id,
                    &start,
                    &state.encryption_keys,
                    Some(&crafted.id),
                ))
                .await;
                assert!(
                    matches!(result, Err(crate::errors::AppError::Conflict(ref reason)) if reason == "Voice input unavailable")
                );
            }
            let saved = voice::get(&state.db, &call.user_id, &thread.id, &request.id)
                .await
                .unwrap();
            assert_eq!(
                saved.state,
                if handler {
                    RequestState::Cancelled
                } else {
                    RequestState::Queued
                }
            );
            let hidden: AssistantConversation = state
                .db
                .collection(THREADS)
                .find_one(doc! {"_id": crafted.task_conversation_id.as_deref().unwrap()})
                .await
                .unwrap()
                .unwrap();
            assert!(hidden.active_turn.is_none());
            assert_eq!(hidden.message_count, 0);
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(MESSAGES)
                    .count_documents(doc! {"conversation_id": &hidden.id})
                    .await
                    .unwrap(),
                0
            );
            let retained: AssistantMessage = state
                .db
                .collection(MESSAGES)
                .find_one(doc! {"_id": &link.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(retained.text, link.text);
            assert!(retained.execution_pending && super::publication_row(&retained));
            state.db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn published_link_shield_survives_lost_request_recovery() {
        use crate::models::assistant_voice::{REQUESTS, RequestState};
        use crate::services::assistant_voice as voice;
        let (state, thread, call, link) = published_fixture("voice_link_lost_shield").await;
        let request = voice::enqueue(
            &state.db,
            &call.user_id,
            &thread.id,
            &call.id,
            "crafted-lost",
            "Ordinary input",
        )
        .await
        .unwrap();
        let request = voice::ensure_task_conversation(&state.db, &state.encryption_keys, &request)
            .await
            .unwrap();
        state.db.collection::<bson::Document>(REQUESTS).update_one(
            doc! {"_id": &request.id},
            doc! {"$set": {"message_id": &link.id, "message_seq": link.seq, "turn_id": &link.turn_id,
                "state": "claimed", "created_at": bson::DateTime::from_millis(0)}},
        ).await.unwrap();
        voice::recover(&state.db).await.unwrap();
        let saved = voice::get(&state.db, &call.user_id, &thread.id, &request.id)
            .await
            .unwrap();
        assert_eq!(saved.state, RequestState::Cancelled);
        assert_eq!(saved.terminal_error_code.as_deref(), Some("turn_lost"));
        let retained: AssistantMessage = state
            .db
            .collection(MESSAGES)
            .find_one(doc! {"_id": &link.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.text, link.text);
        assert!(retained.execution_pending && super::publication_row(&retained));
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(MESSAGES)
                .count_documents(
                    doc! {"conversation_id": request.task_conversation_id, "role": "user"}
                )
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn published_link_history_is_completed_and_later_owner_and_voice_turns_settle() {
        use crate::services::{assistant_nyxagent as engine, assistant_voice as voice};
        let (state, thread, call, link) = published_fixture("voice_link_later_turns").await;
        finish(&state, &thread, "Initial response").await;
        let axum::Json(history) = crate::handlers::assistant_nyxagent::history(
            axum::extract::State(state.clone()),
            crate::handlers::assistant_team::owner_auth(&call.user_id).unwrap(),
            axum::extract::Path(thread.id.clone()),
            axum::extract::Query(Default::default()),
        )
        .await
        .unwrap();
        let dto = serde_json::to_value(history).unwrap();
        let row = dto["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == link.id)
            .unwrap();
        assert_eq!(row["role"], "assistant");
        assert_eq!(row["status"], "completed");
        assert_eq!(row["text"], link.text);
        assert_eq!(row["execution_pending"], true);
        assert!(row["voice"].is_null());
        let start: engine::TurnRequest = serde_json::from_value(serde_json::json!({
            "conversation_id": thread.id, "text": "Next owner question",
        }))
        .unwrap();
        let next = Box::pin(engine::begin_turn(
            &state.db,
            &call.user_id,
            &start,
            &state.encryption_keys,
        ))
        .await
        .unwrap();
        finish(&state, &next, "Next owner answer").await;
        let request = voice::enqueue(
            &state.db,
            &call.user_id,
            &thread.id,
            &call.id,
            "later-voice",
            "Next voice question",
        )
        .await
        .unwrap();
        let request = voice::ensure_task_conversation(&state.db, &state.encryption_keys, &request)
            .await
            .unwrap();
        let start: engine::TurnRequest = serde_json::from_value(serde_json::json!({
            "conversation_id": request.task_conversation_id, "text": "Next voice question",
        }))
        .unwrap();
        let hidden = Box::pin(engine::begin_turn_with_voice(
            &state.db,
            &call.user_id,
            &start,
            &state.encryption_keys,
            Some(&request.id),
        ))
        .await
        .unwrap();
        assert_eq!(
            hidden
                .active_turn
                .as_ref()
                .unwrap()
                .voice_request_id
                .as_deref(),
            Some(request.id.as_str())
        );
        finish(&state, &hidden, "Next voice answer").await;
        voice::publish_completed_results(&state).await.unwrap();
        let saved = voice::get(&state.db, &call.user_id, &thread.id, &request.id)
            .await
            .unwrap();
        assert_eq!(
            saved.state,
            crate::models::assistant_voice::RequestState::Completed
        );
        let (_, rows) = engine::history_page(&state.db, &call.user_id, &thread.id, 50, None)
            .await
            .unwrap();
        assert!(
            rows.iter()
                .any(|row| row.text == "Next owner answer" && row.status == "completed")
        );
        assert!(
            rows.iter()
                .any(|row| row.text == "Next voice answer" && row.status == "completed")
        );
        assert!(
            rows.iter()
                .any(|row| row.id == link.id && row.execution_pending)
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn published_link_is_removed_by_conversation_deletion() {
        let (state, thread, call, link) = published_fixture("voice_link_delete").await;
        finish(&state, &thread, "Initial response").await;
        super::super::session::close(&state.db, &call, "user_ended", true)
            .await
            .unwrap();
        crate::services::assistant_nyxagent::delete(&state.db, &call.user_id, &thread.id)
            .await
            .unwrap();
        assert!(
            state
                .db
                .collection::<bson::Document>(MESSAGES)
                .find_one(doc! {"_id": link.id})
                .await
                .unwrap()
                .is_none()
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn published_link_is_removed_by_owner_erasure() {
        let (state, thread, call, link) = published_fixture("voice_link_owner_purge").await;
        finish(&state, &thread, "Initial response").await;
        super::super::session::close(&state.db, &call, "user_ended", true)
            .await
            .unwrap();
        // Owner erasure waits for the voice runtime's independent billing and
        // session retention lifecycle. Drain that real prerequisite first.
        let closed = super::super::session::get(&state.db, &call.user_id, &thread.id, &call.id)
            .await
            .unwrap();
        super::super::session::write(
            &state.db,
            &closed,
            doc! {
                "purge_requested": true, "lease_until": bson::DateTime::from_millis(0),
            },
        )
        .await
        .unwrap();
        super::super::runtime::recover(&state).await.unwrap();
        assert!(
            state
                .db
                .collection::<bson::Document>(CALLS)
                .find_one(doc! {"_id": &call.id})
                .await
                .unwrap()
                .is_none()
        );
        let admin = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &admin,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        crate::test_utils::accept_and_process_admin_erasure(&state.db, &admin, &call.user_id)
            .await
            .unwrap();
        assert!(
            state
                .db
                .collection::<bson::Document>(MESSAGES)
                .find_one(doc! {"_id": link.id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .db
                .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! {"_id": &call.user_id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .db
                .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! {"_id": admin})
                .await
                .unwrap()
                .is_some()
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn unpublished_private_tokens_have_explicit_omission_facts_without_delivery_claims() {
        let (state, _, call) = super::super::tests::setup("voice_private_token_facts").await;
        let output = present(
            &state,
            &call,
            "unpublished-token",
            serde_json::json!({"content":[{"type":"text","text":"nyx_clk_private nyx_bcl_private"}]}),
        ).await;
        assert_eq!(output["links_not_sent_to_chat"], 2);
        assert!(output.get("links_in_chat").is_none());
        assert!(output.to_string().contains("NOT sent"));
        assert!(
            !output.to_string().contains("nyx_clk_") && !output.to_string().contains("nyx_bcl_")
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(MESSAGES)
                .count_documents(doc! {"turn_metadata.voice_link_publication": {"$exists": true}})
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn direct_tool_links_publish_once_across_concurrent_replay_and_lease_recovery() {
        let (state, thread, call) = super::super::tests::setup("voice_private_tool_links").await;
        let original = tool_result(&state);
        let (first, second) = tokio::join!(
            present(&state, &call, "tool-call-one", original.clone()),
            present(&state, &call, "tool-call-one", original.clone()),
        );
        assert_private_model_output(&first, 2);
        assert_private_model_output(&second, 2);
        // A new worker generation must reuse the same publication identity.
        let mut recovered = call.clone();
        recovered.generation += 1;
        recovered.lease_owner = "recovered-worker".into();
        state.db.collection::<bson::Document>(CALLS).update_one(doc!{"_id":&call.id},
            doc!{"$set":{"generation":recovered.generation,"lease_owner":&recovered.lease_owner}}).await.unwrap();
        assert_private_model_output(
            &present(&state, &recovered, "tool-call-one", original).await,
            2,
        );
        let rows: Vec<AssistantMessage> = state
            .db
            .collection(MESSAGES)
            .find(doc! {"_id":publication_id(&state, &call, "tool-call-one")})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            row.text,
            format!(
                "<{}/connect/bot/nyx_bcl_example>\n\n<{}/connect/nyx_clk_example>",
                state.config.frontend_url, state.config.frontend_url
            )
        );
        assert_eq!(row.via.as_deref(), Some("voice"));
        assert_eq!(row.role, "assistant");
        assert!(
            row.voice.is_none(),
            "Link publication is a #2415 boundary, never a spoken segment"
        );
        assert!(row.execution_pending && row.error_detail.is_none());
        assert!(super::publication_row(row));
        let updated: AssistantConversation = state
            .db
            .collection(THREADS)
            .find_one(doc! {"_id":&thread.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            updated.message_count,
            thread.message_count + 1,
            "The unique publication and sequence increment commit together"
        );
        assert_ne!(
            publication_id(&state, &call, "tool-call-one"),
            publication_id(&state, &call, "tool-call-two")
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reused_rpc_id_publishes_each_exact_link_list_once_with_truthful_facts() {
        let (state, thread, call) = super::super::tests::setup("voice_link_identity_content").await;
        let messages = state.db.collection::<AssistantMessage>(MESSAGES);
        for suffix in ["first", "second", "first", "second"] {
            let url = format!("{}/connect/bot/nyx_bcl_{suffix}", state.config.frontend_url);
            let result = present(
                &state,
                &call,
                "same-rpc-id",
                serde_json::json!({
                    "url": url, "content": [{"type":"text", "text": &url}], "isError": false
                }),
            )
            .await;
            assert_private_model_output(&result, 1);
            assert_eq!(result["isError"], false);
            assert_eq!(
                messages
                    .count_documents(doc! {"conversation_id": &thread.id,
                    "text": format!("<{url}>")})
                    .await
                    .unwrap(),
                1
            );
        }
        assert_eq!(
            messages
                .count_documents(doc! {"conversation_id": &thread.id,
                "turn_metadata.voice_link_publication": {"$exists": true}})
                .await
                .unwrap(),
            2
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn frontend_publication_uses_structured_urls_and_never_includes_cjk_prose() {
        let (state, _, call) = super::super::tests::setup("voice_link_url_extent").await;
        let url = format!("{}/login/device?user_code=ABCD", state.config.frontend_url);
        let structured = format!(
            "{}/assistant?panel=settings&panelTab=privacy",
            state.config.frontend_url
        );
        for (id, raw, expected) in [
            (
                "prose",
                serde_json::json!({"text": format!("{url}，请输入")}),
                url.clone(),
            ),
            (
                "structured",
                serde_json::json!({"url": &structured, "text": format!("打开{structured}，请输入")}),
                structured,
            ),
            (
                "structured-punctuation",
                serde_json::json!({"url": format!("{url}!"), "text": format!("打开{url}!")}),
                format!("{url}!"),
            ),
        ] {
            let output = present(&state, &call, id, raw).await;
            assert_private_model_output(&output, 1);
            let row = state.db.collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"conversation_id": &call.conversation_id, "text": format!("<{expected}>")})
                .await.unwrap().expect("exact URL row");
            assert_eq!(row.text, format!("<{expected}>"));
        }
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn publication_marker_excludes_recap_and_checkpoint_with_token_defense_for_legacy_rows() {
        use crate::services::assistant_nyxagent as engine;
        let (state, thread, call) =
            super::super::tests::setup("voice_link_history_exclusion").await;
        let output = present(&state, &call, "history", tool_result(&state)).await;
        assert_eq!(output["links_in_chat"], 2);
        let row = state
            .db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {"_id":publication_id(&state, &call,"history")})
            .await
            .unwrap()
            .unwrap();
        assert!(engine::bounded_recap(std::slice::from_ref(&row)).is_empty());
        assert!(row.supplemental_result_claims.is_empty());
        let mut tx = state.db.client().start_session().await.unwrap();
        tx.start_transaction().await.unwrap();
        engine::write_checkpoint_in_session(
            &state.db,
            &mut tx,
            &thread,
            thread.active_turn.as_ref().unwrap(),
            &row,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        tx.commit_transaction().await.unwrap();
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::assistant_checkpoint::COLLECTION_NAME)
                .count_documents(doc! {"_id":&thread.id})
                .await
                .unwrap(),
            0
        );
        let mut legacy = row.clone();
        legacy.turn_metadata = None;
        legacy.execution_pending = false;
        legacy.text = "Legacy nyx_bcl_example and nyx_clk_example".into();
        let recap = engine::bounded_recap(&[legacy]);
        assert!(
            recap.contains(PLACEHOLDER)
                && !recap.contains("nyx_bcl_")
                && !recap.contains("nyx_clk_")
        );
        let checkpoint = crate::models::assistant_checkpoint::AssistantCheckpoint {
            source_turn_ids: None,
            goal_turn_id: None,
            conversation_id: thread.id.clone(),
            user_id: call.user_id.clone(),
            turn_id: "old".into(),
            status: "completed".into(),
            error_code: None,
            goal: Some("nyx_clk_example".into()),
            output: Some("nyx_bcl_example".into()),
            completed_actions: vec![],
            failed_actions: vec![],
            last_successful_turn_id: None,
            last_successful_at: None,
            updated_at: chrono::Utc::now(),
        };
        let context = engine::checkpoint_line(&checkpoint, true);
        assert!(!context.contains("nyx_bcl_") && !context.contains("nyx_clk_"));
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn api_origin_urls_are_preserved_at_mcp_without_a_visible_publication() {
        let (state, thread, call) = super::super::tests::setup("voice_api_links").await;
        assert_ne!(state.config.base_url, state.config.frontend_url);
        let messages = state.db.collection::<bson::Document>(MESSAGES);
        let before = messages
            .count_documents(doc! {"conversation_id":&thread.id})
            .await
            .unwrap();
        let output = present(
            &state,
            &call,
            "api-resource",
            serde_json::json!({
                "resource_uri": format!("{}/api/v1/proxy/s/example", state.config.base_url)
            }),
        )
        .await;
        assert!(contains_url(&output.to_string()));
        let mut handles = super::super::link_handles::LinkHandles::default();
        let context = handles.present(&state, output.clone());
        assert!(!contains_url(&context.to_string()));
        assert_eq!(context["links_not_sent_to_chat"], 1);
        assert!(output.get("links_in_chat").is_none());
        assert_eq!(
            messages
                .count_documents(doc! {"conversation_id":&thread.id})
                .await
                .unwrap(),
            before
        );
        assert!(
            messages
                .find_one(doc! {"_id":publication_id(&state, &call, "api-resource")})
                .await
                .unwrap()
                .is_none()
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn markdown_emphasis_and_quotes_publish_exact_urls_preserving_token_characters() {
        let (state, _, call) = super::super::tests::setup("voice_markdown_links").await;
        let first = format!("{}/connect/bot/nyx_bcl_x_-", state.config.frontend_url);
        let second = format!("{}/connect/nyx_clk_x", state.config.frontend_url);
        let output = present(
            &state,
            &call,
            "markdown",
            serde_json::json!({
                "text": format!("**{first}** '{second}'")
            }),
        )
        .await;
        assert_private_model_output(&output, 2);
        let row = state
            .db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {"conversation_id": &call.conversation_id, "turn_metadata.voice_link_publication": {"$exists": true}})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.text, format!("<{first}>\n\n<{second}>"));
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn home_thread_reply_channel_never_receives_voice_tool_links_or_an_outbox() {
        use tracing::instrument::WithSubscriber;
        #[derive(Clone)]
        struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let commands = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = commands.clone();
        let handler = mongodb::event::EventHandler::callback(move |event| {
            if let mongodb::event::command::CommandEvent::Started(event) = event {
                for operation in ["find", "insert", "update", "findAndModify", "aggregate"] {
                    if let Ok(collection) = event.command.get_str(operation) {
                        // Namespace metadata only: never retain command bodies.
                        observed.lock().unwrap().push(collection.to_owned());
                    }
                }
            }
        });
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "voice_links_home_channel",
            handler,
        )
        .await;
        let (state, thread, call) = super::super::tests::setup_in_db(db).await;
        let agent =
            crate::services::assistant_team_service::agent_for_conversation(&state.db, &thread)
                .await
                .unwrap();
        assert_eq!(
            agent.kind,
            crate::models::assistant_agent::AgentKind::Nyxbot
        );
        state
            .db
            .collection::<AssistantAgent>(AGENTS)
            .update_one(
                doc! {"_id":&agent.id},
                doc! {"$set":{"home_conversation_id":&thread.id}},
            )
            .await
            .unwrap();
        let reply_channel = ChannelOrigin {
            org_execution: None,
            native_event_id: None,
            thread: None,
            nyxbot_channel_id: uuid::Uuid::new_v4().to_string(),
            partition: "owner-private-chat".into(),
            platform: "telegram".into(),
        };
        state
            .db
            .collection::<AssistantConversation>(THREADS)
            .update_one(
                doc! {"_id":&thread.id},
                doc! {"$set":{"reply_channel":bson::to_bson(&reply_channel).unwrap()}},
            )
            .await
            .unwrap();
        let capture = Capture(Default::default());
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish();
        commands.lock().unwrap().clear();
        let output = present(&state, &call, "private-channel-tool", tool_result(&state))
            .with_subscriber(subscriber)
            .await;
        assert_private_model_output(&output, 2);
        let accessed = commands.lock().unwrap().clone();
        assert!(!accessed.is_empty());
        assert!(
            accessed
                .iter()
                .all(|collection| [MESSAGES, THREADS, CALLS].contains(&collection.as_str())),
            "Publication must never consult a channel route, delivery or credential"
        );
        for collection in [
            "channel_text_deliveries",
            "channel_reply_deliveries",
            "channel_messages",
            "nyxbot_gateway_responses",
            "nyxbot_events",
        ] {
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "No channel send/outbox: {collection}"
            );
        }
        // Raw links have exactly one intended home: the app-visible message.
        for collection in state.db.list_collection_names().await.unwrap() {
            if collection == MESSAGES {
                continue;
            }
            let rows: Vec<bson::Document> = state
                .db
                .collection(&collection)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            for row in rows {
                let text = row.to_string();
                assert!(
                    !text.contains("nyx_bcl_example") && !text.contains("nyx_clk_example"),
                    "No raw token in {collection}, including audits, error_detail, context or summaries"
                );
            }
        }
        let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(!logs.contains("nyx_bcl_") && !logs.contains("nyx_clk_"));
        let context = super::super::runtime::initial_context(&state.db, &call)
            .await
            .unwrap();
        assert!(!context.live.contains("nyx_bcl_") && !context.live.contains("nyx_clk_"));
        assert!(
            !context.delegation.contains("nyx_bcl_") && !context.delegation.contains("nyx_clk_")
        );
        assert!(!context.live.contains("<https://"));
        assert!(!context.live.contains(PLACEHOLDER));
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn publication_failure_keeps_verified_result_but_never_claims_link_delivery() {
        let (state, thread, call) =
            super::super::tests::setup("voice_link_publication_failure").await;
        state
            .db
            .collection::<AssistantConversation>(THREADS)
            .delete_one(doc! {"_id":&thread.id})
            .await
            .unwrap();
        let output = present(&state, &call, "failed-publication", tool_result(&state)).await;
        assert_private_model_output(&output, 0);
        assert_eq!(output["link_publication"], "failed");
        assert_eq!(
            output["isError"], false,
            "Publication failure must not replay a successful action"
        );
        assert!(
            output["link_notice"]
                .as_str()
                .unwrap()
                .contains("Do not claim it was sent")
        );
        assert_eq!(
            state
                .db
                .collection::<AssistantMessage>(MESSAGES)
                .count_documents(doc! {"_id":publication_id(&state, &call,"failed-publication")})
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }
}
