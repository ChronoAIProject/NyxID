//! Durable provider responses. Reads never recreate or resubmit model/tool work.
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel, NyxbotEvent},
        nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse},
    },
};
use bson::{Document, doc};
use chrono::Utc;
use mongodb::{Database, IndexModel, options::IndexOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

pub const RELAY_READINESS: &str = "nyxbot:relay-response-recovery-v1";
pub const RETENTION_HOURS: i64 = 24;
pub const MEDIA_UNAVAILABLE: &str = "Attachments are unavailable on this native gateway connection. Please send the information as text or upload it in NyxID.";
const MAX_TEXT_CHARS: usize = 32_000;
// Relay turns retain the engine's complete answer, including multipart text.
// Snapshot + terminal + item frames repeat it; keep the encrypted document
// below MongoDB's 16 MiB document limit. Native output keeps its existing cap.
const MAX_PAYLOAD_BYTES: usize = 15 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub struct Payload {
    pub snapshot: Value,
    pub frames: Vec<Value>,
}
fn invalid() -> AppError {
    AppError::ValidationError("Invalid recovery command".into())
}
fn unavailable() -> AppError {
    AppError::Internal("Response recovery unavailable".into())
}

/// Only protocol command identity is echoed. In particular, opaque event
/// capabilities, credentials and arbitrary client metadata cannot enter replay.
pub fn metadata(value: &Value) -> AppResult<BTreeMap<String, String>> {
    let map: BTreeMap<String, String> =
        serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    if map.len() != 4
        || map.iter().any(|(k, v)| {
            !matches!(
                k.as_str(),
                "cmaeg.channel" | "cmaeg.event" | "cmaeg.partition" | "cmaeg.actor"
            ) || v.is_empty()
                || v.len() > 512
        })
    {
        return Err(invalid());
    }
    Ok(map)
}
impl Payload {
    pub fn new(id: &str, identity: &str, metadata: BTreeMap<String, String>) -> Self {
        let snapshot = json!({"id":id,"object":"response","status":"queued", "output":[],
            "conversation":{"id":identity}, "metadata":metadata});
        let mut result = Self {
            snapshot,
            frames: vec![],
        };
        result.lifecycle("created");
        result
    }
    fn lifecycle(&mut self, name: &str) {
        self.frames.push(
            json!({"type":format!("response.{name}"),"sequence_number":self.frames.len(),
            "response":self.snapshot}),
        );
    }
    pub fn progress(&mut self) -> bool {
        if self.snapshot["status"] != "queued" {
            return false;
        }
        self.snapshot["status"] = json!("in_progress");
        self.lifecycle("in_progress");
        true
    }
    pub fn finish(&mut self, text: Option<&str>, failed: bool) {
        let text = text.map(|text| text.chars().take(MAX_TEXT_CHARS).collect::<String>());
        self.finish_text(text.as_deref(), failed);
    }
    fn finish_text(&mut self, text: Option<&str>, failed: bool) {
        self.progress();
        if let Some(text) = text.filter(|s| !s.trim().is_empty()) {
            let item = json!({"id":format!("msg_{}",self.snapshot["id"].as_str().unwrap_or_default()),
                "type":"message","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":text}]});
            self.snapshot["output"] = json!([item]);
            self.frames.push(
                json!({"type":"response.output_item.done", "sequence_number":self.frames.len(),
                "output_index":0,"item":item}),
            );
        }
        self.snapshot["status"] = json!(if failed { "failed" } else { "completed" });
        if failed {
            self.snapshot["error"] = json!({"code":"provider_turn_failed","message":"NyxBot could not finish this turn."});
        }
        self.lifecycle(if failed { "failed" } else { "completed" });
    }
    pub async fn seal(&self, keys: &EncryptionKeys) -> AppResult<Vec<u8>> {
        let bytes = zeroize::Zeroizing::new(serde_json::to_vec(self).map_err(|_| unavailable())?);
        if bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(unavailable());
        }
        keys.encrypt(&bytes).await
    }
}
pub async fn payload(keys: &EncryptionKeys, row: &GatewayResponse) -> AppResult<Payload> {
    let bytes = zeroize::Zeroizing::new(keys.decrypt(&row.payload_ciphertext).await?);
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(unavailable());
    }
    serde_json::from_slice(&bytes).map_err(|_| unavailable())
}
pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    for (keys, opts) in [
        (
            doc! {"response_id":1},
            IndexOptions::builder().unique(true).build(),
        ),
        (
            doc! {"expires_at":1},
            IndexOptions::builder().expire_after(Duration::ZERO).build(),
        ),
        (doc! {"event_key":1}, IndexOptions::default()),
        (
            doc! {"user_id":1,"conversation_id":1},
            IndexOptions::default(),
        ),
        (doc! {"terminal":1,"checked_at":1}, IndexOptions::default()),
    ] {
        db.collection::<Document>(RESPONSES)
            .create_index(IndexModel::builder().keys(keys).options(opts).build())
            .await?;
    }
    Ok(())
}
pub async fn load(
    db: &Database,
    channel: &NyxbotChannel,
    id: &str,
) -> AppResult<Option<GatewayResponse>> {
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {
            "_id":&channel.user_id,"is_active":true,"platform_personal_deleting":{"$ne":true},
        })
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let response = db
        .collection::<GatewayResponse>(RESPONSES)
        .find_one(doc! {"response_id":id,"channel_id":&channel.id,
        "user_id":&channel.user_id,"binding_id":&channel.binding_id,
        "expires_at":{"$gt":bson::DateTime::now()}})
        .await?;
    if channel.org_specialist.is_some()
        && let Some(response) = &response
        && let Some(event) = db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":&response.event_key,"channel_id":&channel.id})
            .await?
        && let Some(d) = event.delivery.as_ref()
    {
        let e = d
            .origin
            .org_execution
            .as_ref()
            .ok_or_else(super::org_channel_service::denied)?;
        super::org_channel_service::validate(db, &d.origin, &e.actor_id, &e.agent_id).await?;
    }
    Ok(response)
}
pub async fn finish(
    db: &Database,
    keys: &EncryptionKeys,
    row: &GatewayResponse,
    text: Option<&str>,
    failed: bool,
) -> AppResult<()> {
    if row.terminal {
        return Ok(());
    }
    let mut body = payload(keys, row).await?;
    let event = db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&row.event_key})
        .await?;
    let text = text.map(|text| {
        if event.as_ref().is_some_and(|event| {
            event.relay_message_id.is_none() && event.source_media_declared.unwrap_or(0) > 0
        }) && !text.contains(MEDIA_UNAVAILABLE)
        {
            let prefix: String = text.chars().take(MAX_TEXT_CHARS - 512).collect();
            format!("{prefix}\n\n{MEDIA_UNAVAILABLE}")
        } else {
            text.to_owned()
        }
    });
    if event
        .as_ref()
        .is_some_and(|event| event.relay_message_id.is_some())
    {
        body.finish_text(text.as_deref(), failed);
    } else {
        body.finish(text.as_deref(), failed);
    }
    let sealed = body.seal(keys).await?;
    // No upsert: a concurrent purge always wins. All frames and the snapshot
    // become visible atomically, before any network byte is exposed.
    let changed = db.collection::<GatewayResponse>(RESPONSES).update_one(doc! {"_id":&row.id,"terminal":false,"revision":row.revision,
        "expires_at":{"$gt":bson::DateTime::now()}}, doc! {"$set":{"terminal":true,
        "payload_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:sealed}},"$inc":{"revision":1_i64}}).await?;
    if changed.modified_count == 1 {
        super::audit_service::log_async(
            db.clone(),
            Some(row.user_id.clone()),
            "nyxbot_channel_answer_delivery".into(),
            Some(
                json!({"event_id":row.event_key,"conversation_id":row.conversation_id,
                "turn_id":event.as_ref().and_then(|e| e.turn_id.as_deref()),
                "transport":"gateway","outcome":"recovery_ready",
                "reason":"gateway_delivery_receipt_not_available","response_failed":failed}),
            ),
            None,
            None,
            None,
            None,
        );

        db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME).update_one(
            doc! {"_id":&row.channel_id,"binding_id":&row.binding_id,"transport":"gateway"},
            doc! {"$set":{"gateway_native_delivery":if failed {"response_failed"} else {"response_ready"}}}).await?;
    }
    Ok(())
}

/// Called by snapshots/replay and the existing delivery sweep. Transcript is
/// authoritative across process restarts; no in-memory receiver is required.
pub async fn refresh<F, Fut>(
    db: &Database,
    keys: &EncryptionKeys,
    row: &GatewayResponse,
    relay_images: F,
) -> AppResult<()>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = String>,
{
    if row.terminal {
        return Ok(());
    }
    db.collection::<GatewayResponse>(RESPONSES)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"checked_at":bson::DateTime::now()}},
        )
        .await?;
    let Some(event) = db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&row.event_key,"user_id":&row.user_id})
        .await?
    else {
        return finish(db, keys, row, None, true).await;
    };
    if let (Some(conversation), Some(turn)) = (&event.conversation_id, &event.turn_id) {
        use crate::models::assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES};
        if let Some(message) = db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {"user_id":super::org_channel_service::event_actor(&event),
            "conversation_id":conversation,"turn_id":turn,"role":"assistant", "status":{"$in":["completed","failed","cancelled","interrupted"]}}).await? {
            let guest_refused=event.delivery.as_ref().and_then(|d|d.origin.org_execution.as_ref()).is_some_and(|e|e.guest) && message.status!="completed";
            let start_refused = message.error_code.as_deref() == Some("assistant_start_unavailable") || guest_refused;
            let mut text = if guest_refused {super::org_channel_service::UNAVAILABLE.into()} else if start_refused { super::assistant_nyxagent::TurnError::new("assistant_start_unavailable").message } else { message.text };
            let images=super::assistant_nyxagent::turn_tool_attachments(db,super::org_channel_service::event_actor(&event),conversation,turn).await?;
            if event.relay_message_id.is_none() && images.iter().any(super::assistant_nyxagent::chat_tool_image) {
                text = text.chars().take(MAX_TEXT_CHARS - 512).collect();
                text.push_str("\n\nImages are unavailable on this native gateway connection. View them in NyxID.");
            }
            if event.relay_message_id.is_some() && images.iter().any(super::assistant_nyxagent::chat_tool_image) {
                // The HTTP/delivery layer supplies the existing relay image
                // ledger. Persist its result with the terminal frames so GET
                // replay cannot change the answer or repeat uncertain sends.
                text = relay_images(text).await;
            }
            finish(db,keys,row,Some(&text),message.status != "completed" && !start_refused).await?;
        } else {
            let mut body=payload(keys,row).await?;
            if body.progress() {
                let sealed=body.seal(keys).await?;
                db.collection::<GatewayResponse>(RESPONSES).update_one(doc! {"_id":&row.id,"revision":row.revision,"terminal":false},
                    doc! {"$set":{"payload_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:sealed}},"$inc":{"revision":1_i64}}).await?;
            }
        }
    } else if event.status != "queued"
        && Utc::now() - event.created_at > chrono::Duration::minutes(2)
    {
        // A crash before turn admission must not leave a permanently queued
        // response. Fence any slow admission before publishing a terminal error.
        if db
            .collection::<NyxbotEvent>(EVENTS)
            .update_one(
                doc! {"_id":&event.id,"turn_id":bson::Bson::Null,
                "status":{"$in":["running","resuming"]}},
                doc! {"$set":{"status":"refused"}},
            )
            .await?
            .matched_count
            == 1
            || event.status == "refused"
        {
            finish(db, keys, row, None, true).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_frames_are_contiguous_and_bind_exact_command_identity() {
        for text in [None, Some("Refused"), Some("Completed answer")] {
            let metadata=metadata(&json!({"cmaeg.channel":"abc","cmaeg.event":"discord:100:500","cmaeg.partition":"def","cmaeg.actor":"7"})).unwrap();
            let mut p = Payload::new("resp_1", "nyxbot_stable", metadata.clone());
            assert_eq!(p.frames[0]["response"]["status"], "queued");
            p.finish(text, false);
            for (seq, frame) in p.frames.iter().enumerate() {
                assert_eq!(frame["sequence_number"], seq);
                if let Some(response) = frame.get("response") {
                    assert_eq!(response["conversation"]["id"], "nyxbot_stable");
                    assert_eq!(response["metadata"], json!(metadata));
                    assert_eq!(response["id"], "resp_1");
                }
            }
            assert_eq!(p.frames.last().unwrap()["response"], p.snapshot);
            assert_eq!(p.snapshot["status"], "completed");
        }
    }
    #[test]
    fn command_metadata_never_echoes_event_capabilities_or_credentials() {
        for extra in ["cmaeg.event_ref", "agent_key", "token"] {
            let mut value = json!({"cmaeg.channel":"a","cmaeg.event":"b","cmaeg.partition":"c","cmaeg.actor":"d"});
            value[extra] = json!("secret");
            assert!(metadata(&value).is_err());
        }
    }
    #[test]
    fn terminal_frame_and_output_are_bounded() {
        let mut p = Payload::new("resp_1", "nyxbot_1", BTreeMap::new());
        p.finish(Some(&"😀".repeat(1_000_000)), true);
        assert!(serde_json::to_vec(&p).unwrap().len() < MAX_PAYLOAD_BYTES);
        assert_eq!(p.snapshot["status"], "failed");
    }
    #[test]
    fn relay_recovery_preserves_a_long_multipart_answer() {
        let text = "完整回复".repeat(20_000);
        let mut payload = Payload::new("resp_long", "nyxbot_1", BTreeMap::new());
        payload.finish_text(Some(&text), false);
        assert_eq!(payload.snapshot["output"][0]["content"][0]["text"], text);
        assert!(serde_json::to_vec(&payload).unwrap().len() < MAX_PAYLOAD_BYTES);
    }
}
