//! Immutable, encrypted, turn-bound snapshots. Reading a part never dispatches a service.
use super::{
    api_key_mutation_service as transactions, assistant_acknowledgement_service::ChatAuthority,
    assistant_resource_service as live, record_field_service,
};
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::COLLECTION_NAME as CONVERSATIONS,
        assistant_tool_result::{AssistantToolResult, COLLECTION_NAME},
    },
};
use bson::{Document, doc};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    options::{IndexOptions, ReadConcern, WriteConcern},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub const TOOL_NAME: &str = "nyxid__read_tool_result";
pub const BUDGET: usize = 9_000;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_TURN_BYTES: i64 = 32 * 1024 * 1024;
const MAX_RESULTS: i64 = 16;
const INSTRUCTIONS: &str = "Read next_part with nyxid__read_tool_result(ref, part) until complete. If not listed, use nyx__call_tool with tool_name nyxid__read_tool_result and arguments_json encoding ref and part. Concatenate content in part order, without separators, for the exact original text. Never repeat the source operation to retrieve parts. Older replicas may report unknown tool, not found, or owner_only for this reader; retry only the part read on an upgraded replica. References expire with this turn; after Retry request review instead of repeating a completed write.";

pub fn definition() -> super::mcp_service::McpToolDefinition {
    super::mcp_service::McpToolDefinition {
        name: TOOL_NAME.into(),
        description: "Read a bounded part of a completed service result using its ref and zero-based part. Same live turn and credential only; no service request, billing or action confirmation. Concatenate content exactly. Expired refs cannot be recovered by replaying a write.".into(),
        input_schema: json!({"type":"object","properties":{"ref":{"type":"string"},"part":{"type":"integer","minimum":0}},"required":["ref","part"],"additionalProperties":false}),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    #[serde(rename = "ref")]
    pub reference: String,
    pub part: usize,
}
pub fn missing() -> AppError {
    AppError::NotFound("Tool result unavailable or expired for this turn. No service request was sent. Do not repeat a completed write; request review.".into())
}
pub fn unavailable() -> Value {
    json!({"result_unavailable":true,"operation_completed":true,"message":"The service completed, but its large result could not be retained within the turn's limits. Do not repeat a write. Request review; no provider paging is implied."})
}
pub fn envelope(text: &str) -> Value {
    json!({"content":[{"type":"text","text":text}],"isError":false})
}
fn page(reference: &str, part: usize, parts: usize, size: usize, content: &str) -> Value {
    json!({"version":1,"ref":reference,"part":part,"parts":parts,"total_bytes":size,
        "encoding":"utf-8","content":content,"next_part":(part+1<parts).then_some(part+1),
        "complete":part+1==parts,"instructions":INSTRUCTIONS})
}
/// Reserve enough room for all metadata; measure the two JSON escaping layers.
/// Prefer a newline only when it keeps at least half the candidate bytes.
fn boundaries(text: &str) -> Vec<i64> {
    let mut out = vec![0];
    let mut start = 0;
    while start < text.len() {
        let mut end = start;
        let mut cost = 0;
        for c in text[start..].chars() {
            let next = match c {
                '"' | '\\' => 4,
                '\n' | '\r' | '\t' | '\u{0008}' | '\u{000c}' => 3,
                c if c <= '\u{001f}' => 7,
                c => c.len_utf8(),
            };
            if cost + next > BUDGET - 1600 {
                break;
            }
            cost += next;
            end += c.len_utf8();
        }
        if end < text.len()
            && let Some(n) = text[start..end].rfind('\n')
            && n + 1 >= (end - start) / 2
        {
            end = start + n + 1;
        }
        out.push(end as i64);
        start = end;
    }
    out
}
fn render(row: &AssistantToolResult, part: usize, text: &str) -> AppResult<Value> {
    let parts = row.boundaries.len().checked_sub(1).ok_or_else(missing)?;
    if part >= parts || text.len() != row.size as usize {
        return Err(missing());
    }
    let content = text
        .get(row.boundaries[part] as usize..row.boundaries[part + 1] as usize)
        .ok_or_else(missing)?;
    let value = page(&row.id, part, parts, text.len(), content);
    if envelope(&value.to_string()).to_string().len() > BUDGET {
        return Err(missing());
    }
    Ok(value)
}
fn context_path(row: &AssistantToolResult) -> [&str; 8] {
    [
        "data_encrypted",
        &row.actor_id,
        &row.conversation_id,
        &row.turn_id,
        &row.api_key_id,
        &row.agent_id,
        if row.guest { "guest" } else { "owner" },
        "v1",
    ]
}
async fn require_live(db: &Database, chat: &ChatAuthority) -> AppResult<()> {
    // Includes platform/channel/invocation authority; require_live independently
    // enforces the turn even while the historical live-turn flag is off.
    Box::pin(chat.ensure_live_turn(db, None, None, "assistant.tool_result")).await?;
    live::require_live(db, chat).await
}
pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Document>(COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id":1,"conversation_id":1,"turn_id":1})
                .build(),
            IndexModel::builder().keys(doc! {"actor_id":1}).build(),
        ])
        .await?;
    Ok(())
}
// The general state_version also changes on last_used_at (authentication).
// Reuse the existing authority-only fence so part reads survive normal use.
async fn key_authority_fence(db: &Database, chat: &ChatAuthority) -> AppResult<i64> {
    let row = db
        .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {
            "_id":&chat.api_key_id,"user_id":&chat.user_id,"is_active":true,
            "$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}],
        })
        .projection(doc! {"machine_external_fence":1})
        .await?
        .ok_or_else(missing)?;
    Ok(row.get_i64("machine_external_fence").unwrap_or(0))
}
pub async fn retain(
    db: &Database,
    keys: &EncryptionKeys,
    chat: &ChatAuthority,
    actor: &str,
    text: &str,
) -> AppResult<Value> {
    if text.len() > MAX_BYTES {
        return Err(missing());
    }
    require_live(db, chat).await?;
    let mut row = AssistantToolResult {
        id: Uuid::new_v4().to_string(),
        version: 1,
        user_id: chat.user_id.clone(),
        actor_id: actor.into(),
        conversation_id: chat.conversation_id.clone(),
        turn_id: chat.turn_id.clone().ok_or_else(missing)?,
        api_key_id: chat.api_key_id.clone(),
        key_authority_fence: key_authority_fence(db, chat).await?,
        agent_id: chat.agent_id.clone(),
        guest: chat.guest,
        size: text.len() as i64,
        boundaries: boundaries(text),
        data_encrypted: vec![],
        encryption_version: keys.contextual_record_writes().then_some(3),
        expires_at: Utc::now() + Duration::minutes(30),
    };
    let path = context_path(&row);
    row.data_encrypted = record_field_service::encrypt_context(
        keys,
        &RecordContext::new(COLLECTION_NAME, &row.id, &row.user_id, &path)?,
        text.as_bytes(),
    )
    .await?;
    let first = render(&row, 0, text)?;
    let mut session = db.client().start_session().await?;
    let expected_key_authority_fence = row.key_authority_fence;
    let db = db.clone();
    let chat = chat.clone();
    let live_db = db.clone();
    let live_chat = chat.clone();
    session.start_transaction().read_concern(ReadConcern::snapshot()).write_concern(WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                super::erasure_admission::fence_in_session(&db, &row.user_id, session).await?;
                if row.actor_id != row.user_id { super::erasure_admission::fence_in_session(&db, &row.actor_id, session).await?; }
                if !super::assistant_invocation_service::fence_in_session(&db, session, &chat.user_id, chat.invocation_id.as_deref()).await? { return Err(missing()); }
                // Serialize quota accounting with other issuers and Stop/settlement.
                // Count retained rows, not ignored additive ActiveTurn fields that an
                // older replica could drop when it replaces its turn snapshot.
                if db.collection::<Document>(CONVERSATIONS).update_one(live::live_filter(&chat)?,doc!{"$currentDate":{"updated_at":true}}).session(&mut *session).await?.matched_count != 1 { return Err(missing()); }
                let mut cursor = db.collection::<Document>(COLLECTION_NAME).find(doc!{"conversation_id":&row.conversation_id,"user_id":&row.user_id,"turn_id":&row.turn_id}).projection(doc!{"size":1}).limit(MAX_RESULTS+1).session(&mut *session).await?;
                let existing: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
                if existing.len() >= MAX_RESULTS as usize || existing.iter().map(|v|v.get_i64("size").unwrap_or(MAX_TURN_BYTES)).sum::<i64>() + row.size > MAX_TURN_BYTES { return Err(missing()); }
                db.collection::<AssistantToolResult>(COLLECTION_NAME).insert_one(&row).session(&mut *session).await?;
                Ok(())
            }.await;
            transactions::transaction_result(result)
        }).await.map_err(transactions::map_transaction_error)?;
    require_live(&live_db, &live_chat).await?;
    if key_authority_fence(&live_db, &live_chat).await? != expected_key_authority_fence {
        return Err(missing());
    }
    Ok(first)
}
pub async fn read(
    db: &Database,
    keys: &EncryptionKeys,
    chat: &ChatAuthority,
    actor: &str,
    request: &ReadRequest,
) -> AppResult<Value> {
    if Uuid::parse_str(&request.reference).is_err() {
        return Err(missing());
    }
    require_live(db, chat).await?;
    let row = db.collection::<AssistantToolResult>(COLLECTION_NAME).find_one(doc!{
        "_id":&request.reference,"version":1,"user_id":&chat.user_id,"actor_id":actor,
        "conversation_id":&chat.conversation_id,"turn_id":chat.turn_id.as_deref().ok_or_else(missing)?,
        "api_key_id":&chat.api_key_id,"agent_id":&chat.agent_id,"guest":chat.guest,
        "expires_at":{"$gt":bson::DateTime::now()},
    }).await?.ok_or_else(missing)?;
    if key_authority_fence(db, chat).await? != row.key_authority_fence {
        return Err(missing());
    }
    let path = context_path(&row);
    let bytes = record_field_service::decrypt_context(
        keys,
        &RecordContext::new(COLLECTION_NAME, &row.id, &row.user_id, &path)?,
        row.encryption_version,
        &row.data_encrypted,
    )
    .await?;
    let text = std::str::from_utf8(&bytes).map_err(|_| missing())?;
    let value = render(&row, request.part, text)?;
    require_live(db, chat).await?;
    if key_authority_fence(db, chat).await? != row.key_authority_fence {
        return Err(missing());
    }
    Ok(value)
}

pub(crate) mod storage;

#[cfg(test)]
mod tests;
