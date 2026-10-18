//! Turn-scoped resource handles and shared image materialization.
use super::{
    api_key_mutation_service as transactions,
    assistant_acknowledgement_service::ChatAuthority,
    mcp_service as mcp,
    resource_adapter::{self, ResourceCandidate, ResourceRead},
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::COLLECTION_NAME as CONVERSATIONS,
        assistant_resource::{AssistantResource, COLLECTION_NAME},
    },
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

pub const TOOL_NAME: &str = "nyx__resource_get";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    Attach,
    Inspect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetRequest {
    #[serde(rename = "ref")]
    pub resource_ref: String,
    pub output: Output,
}

pub fn definition() -> mcp::McpToolDefinition {
    mcp::McpToolDefinition {
        name: TOOL_NAME.into(),
        description: "Retrieve an image using a resource ref returned by a source tool. output=attach displays it in this chat and schedules supported channel delivery; output=inspect lets you view it without channel delivery. Both intents can be requested on the same ref. Source permissions are checked again. No URL or setup skill is needed. If stale, reread the source and use its new ref.".into(),
        input_schema: json!({"type":"object","properties":{"ref":{"type":"string"},"output":{"type":"string","enum":["attach","inspect"]}},"required":["ref","output"],"additionalProperties":false}),
    }
}

pub fn missing() -> AppError {
    AppError::NotFound("Resource not found or expired; reread the source.".into())
}
pub fn stale() -> AppError {
    AppError::Conflict("resource_stale: source changed; reread it and use its new ref.".into())
}

/// Independent of the live-turn rollout flag. Used again by the final atomic commit.
pub(crate) fn live_filter(chat: &ChatAuthority) -> AppResult<bson::Document> {
    if chat.turn_stopped || !chat.turn_live {
        return Err(AppError::AssistantTurnRequired);
    }
    let turn = chat
        .turn_id
        .as_deref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let now = Utc::now();
    let mut filter = doc! {
        "_id": &chat.conversation_id, "user_id": &chat.user_id,
        "agent_id": &chat.agent_id, "credential_api_key_id": &chat.api_key_id,
        "guest_turn": if chat.guest { bson::Bson::Boolean(true) } else { bson::Bson::Document(doc!{"$ne":true}) },
        "active_turn.turn_id": turn, "active_turn.stop_requested": false,
        "$or": [
            {"active_turn.lease_expires_at":{"$gt":bson::DateTime::from_chrono(now)}},
            {"active_turn.lease_expires_at":bson::Bson::Null,
             "active_turn.started_at":{"$gt":bson::DateTime::from_chrono(now-Duration::seconds(super::assistant_nyxagent::ACTIVE_TURN_TTL_SECS))}},
        ],
    };
    if let Some(origin) = &chat.platform_origin {
        filter.insert(
            "active_turn.platform_origin",
            bson::to_bson(origin).map_err(|_| AppError::AssistantTurnRequired)?,
        );
    }
    Ok(filter)
}

pub async fn require_live(db: &Database, chat: &ChatAuthority) -> AppResult<()> {
    if db
        .collection::<bson::Document>(CONVERSATIONS)
        .find_one(live_filter(chat)?)
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(())
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    let collection = db.collection::<AssistantResource>(COLLECTION_NAME);
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    collection
        .create_index(
            IndexModel::builder()
                .keys(
                    doc! {"user_id":1,"conversation_id":1,"turn_id":1,"api_key_id":1,"identity":1},
                )
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    Ok(())
}

/// Issuance failure is optional to the source read. The transaction serializes with
/// Stop/settlement and other issuers by updating the same active-turn reference list.
pub async fn annotations(
    db: &Database,
    chat: &ChatAuthority,
    read: &ResourceRead,
) -> AppResult<String> {
    let rows = issue_many(db, chat, read, &read.candidates).await?;
    let entries: Vec<_> = rows
        .iter()
        .map(|row| {
            let label = read
                .candidates
                .iter()
                .find(|candidate| {
                    candidate.selector == row.selector && candidate.revision == row.revision
                })
                .map(|candidate| candidate.label.chars().take(60).collect::<String>())
                .unwrap_or_else(|| "Image".into());
            json!({"ref":row.id,"kind":"image","label":label})
        })
        .collect();
    if entries.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "NyxID image resources (use nyx__resource_get with ref and output=attach to display, or inspect to view): {}\n\n",
        json!(entries)
    ))
}

async fn issue_many(
    db: &Database,
    chat: &ChatAuthority,
    read: &ResourceRead,
    candidates: &[ResourceCandidate],
) -> AppResult<Vec<AssistantResource>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let now = Utc::now();
    let pending: Vec<_> = candidates
        .iter()
        .take(resource_adapter::MAX_RESOURCES)
        .map(|candidate| {
            Ok(AssistantResource {
                id: Uuid::new_v4().to_string(),
                user_id: chat.user_id.clone(),
                conversation_id: chat.conversation_id.clone(),
                turn_id: chat
                    .turn_id
                    .clone()
                    .ok_or(AppError::AssistantTurnRequired)?,
                api_key_id: chat.api_key_id.clone(),
                agent_id: chat.agent_id.clone(),
                guest: chat.guest,
                identity: mcp::canonical_sha256(json!([
                    &read.source,
                    &candidate.selector,
                    &candidate.revision
                ])),
                source: read.source.clone(),
                arguments: read.arguments.clone(),
                selector: candidate.selector.clone(),
                revision: candidate.revision.clone(),
                attachment_id: Uuid::new_v4().to_string(),
                completed: false,
                created_at: now,
                expires_at: now + Duration::hours(1),
            })
        })
        .collect::<AppResult<_>>()?;
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let chat = chat.clone();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation: AppResult<_> = async {
                let parents = db.collection::<bson::Document>(CONVERSATIONS);
                let parent = parents
                    .find_one(live_filter(&chat)?)
                    .projection(doc! {"active_turn.resource_refs": 1})
                    .session(&mut *session)
                    .await?
                    .ok_or(AppError::AssistantTurnRequired)?;
                let used = parent
                    .get_document("active_turn")
                    .ok()
                    .and_then(|turn| turn.get_array("resource_refs").ok())
                    .map_or(0, Vec::len);
                let mut remaining = resource_adapter::MAX_RESOURCES.saturating_sub(used);
                let records = db.collection::<AssistantResource>(COLLECTION_NAME);
                let mut cursor = records
                    .find(doc! {
                        "user_id": &chat.user_id,
                        "conversation_id": &chat.conversation_id,
                        "turn_id": chat.turn_id.as_deref().ok_or(AppError::AssistantTurnRequired)?,
                        "api_key_id": &chat.api_key_id,
                        "expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())},
                    })
                    .limit(resource_adapter::MAX_RESOURCES as i64)
                    .session(&mut *session)
                    .await?;
                let existing: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
                let mut out = Vec::new();
                let mut created = Vec::new();
                for proposed in &pending {
                    if let Some(row) = existing
                        .iter()
                        .chain(created.iter())
                        .find(|row| row.identity == proposed.identity)
                    {
                        out.push(row.clone());
                    } else if remaining > 0 {
                        remaining -= 1;
                        created.push(proposed.clone());
                        out.push(proposed.clone());
                    }
                }
                if !created.is_empty() {
                    let ids: Vec<_> = created.iter().map(|row| row.id.clone()).collect();
                    if parents
                        .update_one(
                            live_filter(&chat)?,
                            doc! {"$push": {"active_turn.resource_refs": {"$each": ids}}},
                        )
                        .session(&mut *session)
                        .await?
                        .matched_count
                        != 1
                    {
                        return Err(AppError::AssistantTurnRequired);
                    }
                    records.insert_many(&created).session(&mut *session).await?;
                }
                Ok(out)
            }
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

#[cfg(test)]
async fn issue(
    db: &Database,
    chat: &ChatAuthority,
    read: &ResourceRead,
    candidate: &ResourceCandidate,
) -> AppResult<Option<AssistantResource>> {
    Ok(issue_many(db, chat, read, std::slice::from_ref(candidate))
        .await?
        .into_iter()
        .next())
}

pub async fn load(db: &Database, chat: &ChatAuthority, id: &str) -> AppResult<AssistantResource> {
    if Uuid::parse_str(id).is_err() {
        return Err(missing());
    }
    require_live(db, chat).await?;
    db.collection::<AssistantResource>(COLLECTION_NAME)
        .find_one(doc! {
            "_id":id,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
            "turn_id":chat.turn_id.as_deref().ok_or(AppError::AssistantTurnRequired)?,
            "api_key_id":&chat.api_key_id,"agent_id":&chat.agent_id,"guest":chat.guest,
            "expires_at":{"$gt":bson::DateTime::from_chrono(Utc::now())},
        })
        .await?
        .ok_or_else(missing)
}

pub fn matches_endpoint(
    row: &AssistantResource,
    service: &mcp::McpToolService,
    endpoint: &mcp::McpToolEndpoint,
) -> bool {
    row.source.service_id == service.service_id
        && row.source.endpoint_id == endpoint.endpoint_id
        && Some(row.source.generation) == mcp::producer_operation_generation(service, endpoint)
        && row.source.endpoint_digest == mcp::endpoint_contract_digest(endpoint)
}

pub fn select<'a>(
    row: &AssistantResource,
    read: &'a ResourceRead,
) -> AppResult<&'a ResourceCandidate> {
    if row.source != read.source || row.arguments != read.arguments {
        return Err(stale());
    }
    let mut candidates = read
        .candidates
        .iter()
        .filter(|c| c.selector == row.selector && c.revision == row.revision);
    let candidate = candidates.next().ok_or_else(stale)?;
    if candidates.next().is_some() {
        return Err(stale());
    }
    Ok(candidate)
}

/// Fresh credential-free, public-address-pinned client. The model supplies only a
/// handle; the adapter obtains the temporary URL from a newly authorized source read.
pub async fn fetch(candidate: &ResourceCandidate) -> AppResult<mcp::ToolMedia> {
    #[cfg(test)]
    let trusted_base = resource_adapter::test_media_origin(candidate.url());
    #[cfg(not(test))]
    let trusted_base: Option<String> = None;
    fetch_url(candidate.url(), trusted_base.as_deref()).await
}

async fn fetch_url(url: &str, trusted_base: Option<&str>) -> AppResult<mcp::ToolMedia> {
    let client = super::channel_media_service::media_client(url, None, trusted_base).await?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| super::channel_media_service::fetch_failed())?;
    let fetched =
        super::channel_media_service::bounded_response(response, mcp::MAX_TOOL_IMAGE_BYTES as u64)
            .await?;
    let media =
        mcp::tool_media(200, fetched.mime_type.as_deref(), &fetched.bytes).ok_or_else(|| {
            AppError::BadRequest("Resource is not a supported PNG, JPEG, GIF or WebP image.".into())
        })?;
    let extracted =
        super::attachment_extraction::extract(media.bytes.clone(), String::new()).await?;
    if extracted.content_type != media.content_type {
        return Err(AppError::BadRequest(
            "Resource image content type does not match.".into(),
        ));
    }
    Ok(media)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::assistant_conversation::TurnAttachment;
    use crate::models::assistant_resource::ResourceSource;
    use crate::services::assistant_nyxagent as engine;

    fn read(revision: &str) -> ResourceRead {
        let id = "fe63677d-5d00-4da4-8a43-908c0175b440";
        let arguments = json!({"block_id":id});
        let text = json!({"object":"list","results":[{"object":"block","id":id,"type":"image","last_edited_time":revision,"image":{"type":"file","file":{"url":"https://example.org/photo.png?private=secret"}}}]}).to_string();
        ResourceRead {
            source: ResourceSource {
                adapter: "notion.v1".into(),
                service_id: Uuid::new_v4().to_string(),
                endpoint_id: Uuid::new_v4().to_string(),
                generation: 1,
                endpoint_digest: "digest".into(),
                authority_digest: "authority".into(),
                tool_name: "notion__notion_list_block_children".into(),
            },
            candidates: resource_adapter::candidates(&arguments, &text),
            arguments,
        }
    }
    async fn setup(prefix: &str) -> (crate::AppState, ChatAuthority) {
        let f = crate::services::assistant_authority_tests::orchestrator_fixture(prefix).await;
        (f.state, f.chat)
    }
    fn media() -> mcp::ToolMedia {
        // The commit helper consumes an already validated image. Decoder coverage below
        // tests the untrusted URL path separately.
        mcp::ToolMedia {
            content_type: "image/png".into(),
            bytes: vec![1, 2, 3, 4],
        }
    }
    #[tokio::test]
    async fn resource_handles_are_scoped_bounded_deduplicated_and_url_free() {
        let (state, chat) = setup("resource_handles").await;
        let read = read("2026-10-17T00:00:00Z");
        let (a, b) = tokio::join!(
            issue(&state.db, &chat, &read, &read.candidates[0]),
            issue(&state.db, &chat, &read, &read.candidates[0])
        );
        let a = a.unwrap().unwrap();
        let b = b.unwrap().unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.attachment_id, b.attachment_id);
        let document = state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&a.id})
            .await
            .unwrap()
            .unwrap();
        let serialized = format!("{document:?}");
        assert!(!serialized.contains("private=secret"));
        assert!(!serialized.contains("https://"));
        let mut other = chat.clone();
        other.user_id = "other".into();
        assert!(load(&state.db, &other, &a.id).await.is_err());
        other = chat.clone();
        other.api_key_id = "rotated".into();
        assert!(load(&state.db, &other, &a.id).await.is_err());
        for i in 1..resource_adapter::MAX_RESOURCES {
            let mut extra = read.candidates[0].revision.clone();
            extra.push_str(&i.to_string());
            let mut next = self::read("2026-10-17T00:00:00Z");
            next.source = read.source.clone();
            next.candidates[0].revision = extra;
            assert!(
                issue(&state.db, &chat, &next, &next.candidates[0])
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        let mut next = self::read("2026-10-17T01:00:00Z");
        next.source = read.source.clone();
        assert!(
            issue(&state.db, &chat, &next, &next.candidates[0])
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            state
                .db
                .collection::<AssistantResource>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            resource_adapter::MAX_RESOURCES as u64
        );
        state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn resource_commit_is_atomic_idempotent_and_upgrades_both_intents() {
        let (state, chat) = setup("resource_commit").await;
        let read = read("2026-10-17T00:00:00Z");
        let row = issue(&state.db, &chat, &read, &read.candidates[0])
            .await
            .unwrap()
            .unwrap();
        let media = media();
        let (a, b) = tokio::join!(
            engine::attach_resource_image(
                &state.db,
                &state.encryption_keys,
                &chat,
                &row,
                Some(&media),
                Output::Attach
            ),
            engine::attach_resource_image(
                &state.db,
                &state.encryption_keys,
                &chat,
                &row,
                Some(&media),
                Output::Inspect
            )
        );
        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(a.attachment.id, b.attachment.id);
        let current = engine::get(&state.db, &chat.user_id, &chat.conversation_id)
            .await
            .unwrap();
        let attachments = &current.active_turn.unwrap().attachments;
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].chat_delivery, Some(true));
        assert_eq!(attachments[0].image_inspection, Some(true));
        let stored = state
            .db
            .collection::<crate::models::assistant_attachment::AssistantAttachment>(
                crate::models::assistant_attachment::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&row.attachment_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            state
                .encryption_keys
                .decrypt(&stored.data_encrypted)
                .await
                .unwrap(),
            media.bytes
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert!(load(&state.db, &chat, &row.id).await.unwrap().completed);
        // Repeated attach preserves requested inspection and the attachment UUID.
        let repeated = engine::attach_resource_image(
            &state.db,
            &state.encryption_keys,
            &chat,
            &row,
            None,
            Output::Attach,
        )
        .await
        .unwrap();
        assert_eq!(repeated.attachment.image_inspection, Some(true));
        assert_eq!(
            Uuid::parse_str(&row.attachment_id)
                .unwrap()
                .get_version_num(),
            4
        );
        state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn resource_download_cannot_commit_after_stop_expiry_rotation_or_new_turn() {
        let (state, chat) = setup("resource_fences").await;
        let read = read("2026-10-17T00:00:00Z");
        let row = issue(&state.db, &chat, &read, &read.candidates[0])
            .await
            .unwrap()
            .unwrap();
        let parents = state.db.collection::<bson::Document>(CONVERSATIONS);
        let original = parents
            .find_one(doc! {"_id":&chat.conversation_id})
            .await
            .unwrap()
            .unwrap();
        for change in [
            doc! {"active_turn.stop_requested":true},
            doc! {"active_turn.lease_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))},
            doc! {"credential_api_key_id":"rotated"},
            doc! {"active_turn.turn_id":"next-turn"},
            doc! {"guest_turn":true},
        ] {
            parents
                .update_one(doc! {"_id":&chat.conversation_id}, doc! {"$set":change})
                .await
                .unwrap();
            assert!(
                engine::attach_resource_image(
                    &state.db,
                    &state.encryption_keys,
                    &chat,
                    &row,
                    Some(&media()),
                    Output::Attach
                )
                .await
                .is_err()
            );
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(
                        crate::models::assistant_attachment::COLLECTION_NAME
                    )
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            assert!(
                !state
                    .db
                    .collection::<AssistantResource>(COLLECTION_NAME)
                    .find_one(doc! {"_id":&row.id})
                    .await
                    .unwrap()
                    .unwrap()
                    .completed
            );
            parents
                .replace_one(doc! {"_id":&chat.conversation_id}, original.clone())
                .await
                .unwrap();
        }
        state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&chat.api_key_id},
                doc! {"$set":{"is_active":false}},
            )
            .await
            .unwrap();
        assert!(
            engine::attach_resource_image(
                &state.db,
                &state.encryption_keys,
                &chat,
                &row,
                Some(&media()),
                Output::Attach
            )
            .await
            .is_err()
        );
        state.db.collection::<AssistantResource>(COLLECTION_NAME).update_one(doc!{"_id":&row.id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
        assert!(load(&state.db, &chat, &row.id).await.is_err());
        state.db.drop().await.unwrap();
    }
    #[test]
    fn resource_selection_fails_on_revisions_authority_or_contract_drift() {
        let mut read = read("2026-10-17T00:00:00Z");
        let candidate = &read.candidates[0];
        let row = AssistantResource {
            id: Uuid::new_v4().to_string(),
            user_id: "owner".into(),
            conversation_id: "thread".into(),
            turn_id: "turn".into(),
            api_key_id: "key".into(),
            agent_id: "agent".into(),
            guest: false,
            identity: "identity".into(),
            source: read.source.clone(),
            arguments: read.arguments.clone(),
            selector: candidate.selector.clone(),
            revision: candidate.revision.clone(),
            attachment_id: Uuid::new_v4().to_string(),
            completed: false,
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(1),
        };
        assert!(select(&row, &read).is_ok());
        read.candidates[0].revision = "2026-10-17T01:00:00Z".into();
        assert!(select(&row, &read).is_err());
        read.candidates[0].revision = row.revision.clone();
        read.source.authority_digest = "changed".into();
        assert!(select(&row, &read).is_err());
        assert!(
            serde_json::from_value::<GetRequest>(
                json!({"ref":row.id,"output":"attach","url":"https://example.org"})
            )
            .is_err()
        );
        let legacy: TurnAttachment = serde_json::from_value(
            json!({"id":"id","content_type":"image/png","size":1,"label":"tool"}),
        )
        .unwrap();
        assert_eq!(legacy.image_inspection, None);
    }
    #[tokio::test]
    async fn resource_downloader_bounds_validates_and_never_follows_redirects() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        Mock::given(path("/image"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .set_body_bytes(png.into_inner()),
            )
            .mount(&server)
            .await;
        Mock::given(path("/malformed"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .set_body_bytes(b"\x89PNG\r\n\x1a\ninvalid".as_slice()),
            )
            .mount(&server)
            .await;
        Mock::given(path("/large"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .insert_header(
                        "content-length",
                        (mcp::MAX_TOOL_IMAGE_BYTES + 1).to_string(),
                    )
                    .set_body_bytes(vec![0u8; mcp::MAX_TOOL_IMAGE_BYTES + 1]),
            )
            .mount(&server)
            .await;
        Mock::given(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/image", server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(path("/pdf"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/pdf")
                    .set_body_string("%PDF-1.7"),
            )
            .mount(&server)
            .await;
        assert_eq!(
            fetch_url(&format!("{}/image", server.uri()), Some(&server.uri()))
                .await
                .unwrap()
                .content_type,
            "image/png"
        );
        assert!(
            fetch_url(
                &format!("{}/image?signature={}", server.uri(), "a".repeat(3000)),
                Some(&server.uri())
            )
            .await
            .is_ok()
        );
        assert!(
            fetch_url(
                &format!("{}/image?signature={}", server.uri(), "a".repeat(8192)),
                Some(&server.uri())
            )
            .await
            .is_err()
        );
        for p in ["malformed", "large", "redirect", "pdf"] {
            assert!(
                fetch_url(&format!("{}/{p}", server.uri()), Some(&server.uri()))
                    .await
                    .is_err(),
                "{p}"
            );
        }
        for url in [
            "https://127.0.0.1/image",
            "http://example.org/image",
            "https://[::1]/image",
        ] {
            assert!(fetch_url(url, None).await.is_err());
        }
    }
}
