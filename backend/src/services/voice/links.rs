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
    models::{assistant_message::AssistantMessage, assistant_voice_session::VoiceSession},
};

pub(super) const PLACEHOLDER: &str = "[link omitted]";
pub(super) const INSTRUCTIONS: &str = "Never read URLs, tokens or identifiers aloud. For links_in_chat, briefly say the link is in the chat. A link publication failure means it was not sent; do not claim otherwise.";
static URLS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:[a-z][a-z0-9+.-]*://|www\.)[^\s<>\"`\\。，、！？；：「」『』（）【】《》〈〉“”‘’…・\x{FF01}-\x{FF0F}\x{FF1A}-\x{FF20}\x{FF3B}-\x{FF40}\x{FF5B}-\x{FF65}\x{3000}-\x{303F}]+"#).expect("voice URL pattern")
});
static TOKENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"nyx_(?:bcl|clk)_[A-Za-z0-9_-]+").expect("connect token pattern"));

pub(super) fn contains_url(text: &str) -> bool {
    let lowercase = text.to_ascii_lowercase();
    lowercase.contains("://") || lowercase.contains("www.")
}
pub(super) fn redact(text: &str) -> (String, usize) {
    let count = URLS.find_iter(text).count();
    let without_urls = URLS.replace_all(text, PLACEHOLDER);
    (
        TOKENS.replace_all(&without_urls, PLACEHOLDER).into_owned(),
        count,
    )
}
pub(super) fn redact_value(value: &mut Value) {
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
fn collect(value: &Value, origins: &[String], links: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            for found in URLS.find_iter(text) {
                let raw = found
                    .as_str()
                    .trim_end_matches(['.', ',', ';', '!', '?', ')', ']', '}', '*', '\'']);
                if let Ok(url) = url::Url::parse(raw)
                    && matches!(url.scheme(), "https" | "http")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && origins.contains(&url.origin().ascii_serialization())
                    && !links.iter().any(|link| link == raw)
                {
                    links.push(raw.to_owned());
                }
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect(value, origins, links)),
        Value::Object(values) => values
            .values()
            .for_each(|value| collect(value, origins, links)),
        _ => {}
    }
}
fn publication_id(call: &VoiceSession, tool_call: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("nyxid:voice-links:{}:{tool_call}", call.id).as_bytes(),
    )
    .to_string()
}

/// The unique message _id and conversation sequence are committed together.
/// This deliberately never uses turn completion or channel delivery machinery:
/// a NyxBot home thread may have reply_channel set by its last private-chat sender.
async fn publish(
    db: &Database,
    call: &VoiceSession,
    tool_call: &str,
    links: &[String],
) -> AppResult<()> {
    use crate::models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as THREADS},
        assistant_message::COLLECTION_NAME as MESSAGES,
        assistant_voice_session::COLLECTION_NAME as CALLS,
    };
    let id = publication_id(call, tool_call);
    let text = links
        .iter()
        .map(|url| format!("<{url}>"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let db = db.clone();
    let call = call.clone();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction().and_run2(async move |tx| {
        let result: AppResult<()> = Box::pin(async {
            let messages = db.collection::<AssistantMessage>(MESSAGES);
            if messages.find_one(doc!{"_id": &id, "user_id": &call.user_id, "conversation_id": &call.conversation_id}).session(&mut *tx).await?.is_some() { return Ok(()); }
            if db.collection::<bson::Document>(CALLS).update_one(super::session::fence(&call), doc!{"$set":{"transcript_fence":uuid::Uuid::new_v4().to_string()}}).session(&mut *tx).await?.matched_count != 1 { return Err(AppError::VoiceProviderUnavailable); }
            let thread = db.collection::<AssistantConversation>(THREADS).find_one_and_update(
                doc!{"_id": &call.conversation_id, "user_id": &call.user_id},
                doc!{"$inc":{"message_count":1},"$set":{"updated_at":bson::DateTime::now()}})
                .return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await?
                .ok_or_else(|| AppError::NotFound("Voice conversation unavailable".into()))?;
            messages.insert_one(AssistantMessage {
                automation_work: None, execution_task: None, delegated_images: Default::default(),
                media_queue_notice_id: None, media_event_ids: Vec::new(), media_pending: None, media_bound: None,
                platform_delivery_recovery_queued: false, platform_reply: None, platform_origin: None,
                author: None, steering: None,
                // No speech segment: this row is a presentation boundary.
                voice: None, execution_pending: false, id: id.clone(), conversation_id: call.conversation_id.clone(),
                user_id: call.user_id.clone(), seq: thread.message_count, turn_id: call.id.clone(),
                role: "assistant".into(), text: text.clone(), status: "completed".into(), error_code: None,
                upstream_error_code: None, error_detail: None, turn_metadata: None, specialist_id: None,
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
pub(super) async fn present(
    state: &crate::AppState,
    call: &VoiceSession,
    tool_call: &str,
    mut result: Value,
) -> Value {
    let origins = state
        .config
        .accepted_frontend_urls()
        .filter_map(|value| {
            url::Url::parse(value)
                .ok()
                .map(|url| url.origin().ascii_serialization())
        })
        .collect::<Vec<_>>();
    let mut links = Vec::new();
    collect(&result, &origins, &mut links);
    let published = links.is_empty() || publish(&state.db, call, tool_call, &links).await.is_ok();
    redact_value(&mut result);
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
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cjk_urls_and_bare_tokens_never_enter_model_context() {
        let (text, count) = redact(
            "链接：https://nyxid.ai/connect/bot/nyx_bcl_example。还有www.example.com，nyx_clk_example",
        );
        assert_eq!(count, 2);
        assert!(!contains_url(&text));
        assert!(!text.contains("nyx_bcl_") && !text.contains("nyx_clk_"));
        assert!(text.contains('。') && text.contains('，'));
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
    use crate::models::{
        assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS},
        assistant_conversation::{
            AssistantConversation, COLLECTION_NAME as THREADS, ChannelOrigin,
        },
        assistant_message::COLLECTION_NAME as MESSAGES,
        assistant_voice_session::COLLECTION_NAME as CALLS,
    };
    use futures::TryStreamExt;

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
            .find(doc! {"_id":publication_id(&call, "tool-call-one")})
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
        assert!(!row.execution_pending && row.error_detail.is_none());
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
            publication_id(&call, "tool-call-one"),
            publication_id(&call, "tool-call-two")
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn api_origin_urls_are_redacted_without_a_visible_publication() {
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
        assert!(output.to_string().contains(PLACEHOLDER));
        assert!(!contains_url(&output.to_string()));
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
                .find_one(doc! {"_id":publication_id(&call, "api-resource")})
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
            .find_one(doc! {"_id":publication_id(&call,"markdown")})
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
        assert!(context.live.contains(PLACEHOLDER));
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
                .count_documents(doc! {"_id":publication_id(&call,"failed-publication")})
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }
}
