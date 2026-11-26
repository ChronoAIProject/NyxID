//! Bot-owned personal data. Every storage operation is scoped to the verified guest.
use std::time::Duration;

use bson::{Document, Regex, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{ClientSession, Database, IndexModel};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use uuid::{Uuid, Version};

use super::policy::GuestDataPermit;
use crate::{
    errors::{AppError, AppResult},
    models::channel_guest_personal_record::{COLLECTION_NAME, ChannelGuestPersonalRecord},
    services::mcp_service::McpToolDefinition,
};

pub(crate) const SOURCE_ID: &str = "personal_records";
pub(crate) const ACTIONS: [&str; 4] = ["create", "read", "update", "delete"];
pub(crate) const FIELDS: [&str; 2] = ["title", "body"];
const CREATE: &str = "nyxid__personal_records_create";
const LIST: &str = "nyxid__personal_records_list";
const READ: &str = "nyxid__personal_records_read";
const UPDATE: &str = "nyxid__personal_records_update";
const DELETE: &str = "nyxid__personal_records_delete";
const TITLE_LIMIT: usize = 120;
const BODY_LIMIT: usize = 2000;
const PAGE_SIZE: usize = 10;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn is_tool(name: &str) -> bool {
    matches!(name, CREATE | LIST | READ | UPDATE | DELETE)
}

pub(crate) fn tool_definitions() -> Vec<McpToolDefinition> {
    let id = json!({"type":"string","format":"uuid"});
    let title = json!({"type":"string","maxLength":TITLE_LIMIT});
    let body = json!({"type":"string","maxLength":BODY_LIMIT});
    [
        (CREATE, "Create a personal record in this bot for the current guest. Supply at least one field.",
            json!({"title":title,"body":body}), json!([])),
        (LIST, "List the current guest's personal records. Use next_cursor to continue the same search. Use read to fetch a record's details.",
            json!({"title_prefix":{"type":"string","minLength":1,"maxLength":TITLE_LIMIT},"cursor":id}), json!([])),
        (READ, "Read one of the current guest's personal records by its record_id.",
            json!({"record_id":id}), json!(["record_id"])),
        (UPDATE, "Update the supplied fields on one of the current guest's personal records. Use an empty string to clear a field.",
            json!({"record_id":id,"title":title,"body":body}), json!(["record_id"])),
        (DELETE, "Permanently delete one of the current guest's personal records.",
            json!({"record_id":id}), json!(["record_id"])),
    ]
    .into_iter()
    .map(|(name, description, properties, required)| McpToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema: json!({
            "type":"object","properties":properties,"required":required,"additionalProperties":false
        }),
    })
    .collect()
}

fn optional_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordFields {
    #[serde(default, deserialize_with = "optional_text")]
    title: Option<String>,
    #[serde(default, deserialize_with = "optional_text")]
    body: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordId {
    record_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateRecord {
    record_id: String,
    #[serde(default, deserialize_with = "optional_text")]
    title: Option<String>,
    #[serde(default, deserialize_with = "optional_text")]
    body: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListRecords {
    #[serde(default, deserialize_with = "optional_text")]
    title_prefix: Option<String>,
    #[serde(default, deserialize_with = "optional_text")]
    cursor: Option<String>,
}

pub(crate) enum PersonalRecordRequest {
    Create(RecordFields),
    List(ListRecords),
    Read(RecordId),
    Update(UpdateRecord),
    Delete(RecordId),
}

impl PersonalRecordRequest {
    pub(crate) fn action(&self) -> &'static str {
        match self {
            Self::Create(_) => "create",
            Self::List(_) | Self::Read(_) => "read",
            Self::Update(_) => "update",
            Self::Delete(_) => "delete",
        }
    }

    pub(crate) fn record_id(&self) -> Option<&str> {
        match self {
            Self::Read(id) | Self::Delete(id) => Some(&id.record_id),
            Self::Update(update) => Some(&update.record_id),
            Self::Create(_) | Self::List(_) => None,
        }
    }

    fn field_values(&self) -> [Option<&str>; 2] {
        match self {
            Self::Create(fields) => [fields.title.as_deref(), fields.body.as_deref()],
            Self::Update(update) => [update.title.as_deref(), update.body.as_deref()],
            Self::List(list) => [list.title_prefix.as_deref(), None],
            Self::Read(_) | Self::Delete(_) => [None, None],
        }
    }

    pub(crate) fn fields(&self) -> impl Iterator<Item = &'static str> {
        FIELDS
            .into_iter()
            .zip(self.field_values())
            .filter_map(|(field, value)| value.map(|_| field))
    }
}

fn decode<T: serde::de::DeserializeOwned>(args: &Value) -> AppResult<T> {
    if !args.is_object() {
        return Err(AppError::BadRequest(
            "Personal record arguments must be an object".into(),
        ));
    }
    serde_json::from_value(args.clone())
        .map_err(|_| AppError::BadRequest("Invalid personal record arguments".into()))
}

fn valid_id(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|parsed| {
        parsed.get_version() == Some(Version::Random) && parsed.to_string() == id
    })
}

fn valid_text(text: &str, limit: usize, multiline: bool) -> bool {
    text.chars().count() <= limit
        && text
            .chars()
            .all(|ch| !ch.is_control() || (multiline && matches!(ch, '\t' | '\r' | '\n')))
}

pub(crate) fn parse(name: &str, arguments: &Value) -> AppResult<PersonalRecordRequest> {
    use PersonalRecordRequest::{Create, Delete, List, Read, Update};
    let request = match name {
        CREATE => Create(decode(arguments)?),
        LIST => List(decode(arguments)?),
        READ => Read(decode(arguments)?),
        UPDATE => Update(decode(arguments)?),
        DELETE => Delete(decode(arguments)?),
        _ => return Err(AppError::NotFound("Unknown personal record tool".into())),
    };
    if request.record_id().is_some_and(|id| !valid_id(id)) {
        return Err(AppError::BadRequest("Invalid personal record ID".into()));
    }
    if let List(list) = &request
        && (list.cursor.as_deref().is_some_and(|id| !valid_id(id))
            || list.title_prefix.as_deref().is_some_and(str::is_empty))
    {
        return Err(AppError::BadRequest(
            "Invalid personal record search".into(),
        ));
    }
    let [title, body] = request.field_values();
    if title.is_some_and(|s| !valid_text(s, TITLE_LIMIT, false))
        || body.is_some_and(|s| !valid_text(s, BODY_LIMIT, true))
        || (matches!(request, Create(_) | Update(_)) && title.is_none() && body.is_none())
    {
        return Err(AppError::BadRequest(
            "Invalid personal record fields".into(),
        ));
    }
    Ok(request)
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Document>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"bot_id":1,"bot_ownership_version":1,"meta_grant_id":1,"owner_id":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"bot_id":1,"meta_grant_id":1,"_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

#[derive(Deserialize)]
struct BotOwnership {
    #[serde(default)]
    ownership_version: i64,
    #[serde(default)]
    meta_binding: Option<MetaBinding>,
}

#[derive(Deserialize)]
struct MetaBinding {
    generation: i64,
}

fn owned(
    bot_id: &str,
    bot_ownership_version: i64,
    owner_id: &str,
    meta_grant_id: Option<&str>,
    record_id: Option<&str>,
) -> Document {
    let mut filter = doc! {
        "bot_id":bot_id,
        "bot_ownership_version":bot_ownership_version,
        "owner_id":owner_id,
        "meta_grant_id":meta_grant_id,
    };
    if let Some(id) = record_id {
        filter.insert("_id", id);
    }
    filter
}

#[cfg(test)]
async fn owns_record(
    db: &Database,
    bot_id: &str,
    owner_id: &str,
    record_id: &str,
) -> AppResult<bool> {
    if !valid_id(record_id) {
        return Ok(false);
    }
    let Some(bot) = db
        .collection::<BotOwnership>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":bot_id,"is_active":true})
        .projection(doc! {"ownership_version":1})
        .await?
    else {
        return Ok(false);
    };
    Ok(db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(owned(
            bot_id,
            bot.ownership_version,
            owner_id,
            None,
            Some(record_id),
        ))
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}

pub(crate) async fn execute(
    db: &Database,
    session: &mut ClientSession,
    permit: &GuestDataPermit,
    meta_grant_id: Option<&str>,
    request: PersonalRecordRequest,
) -> AppResult<Value> {
    let denied = || AppError::Forbidden("Personal record access is unavailable".into());
    let owner = permit.owner_id().ok_or_else(denied)?;
    let allowed_fields = if request.action() == "read" {
        permit.read_fields()
    } else {
        permit.write_fields()
    };
    if permit.source_id() != SOURCE_ID
        || permit.action() != request.action()
        || request
            .fields()
            .any(|field| !allowed_fields.iter().any(|allowed| allowed == field))
    {
        return Err(denied());
    }
    // Serialize with retirement and ownership transfer. The receiving owner
    // uses a fresh dataset even if the same actor and binding can be reused.
    let mut bot_filter = doc! {"_id":permit.bot_id(),"is_active":true};
    match meta_grant_id {
        Some(grant) if !grant.is_empty() => {
            bot_filter.extend(doc! {
                "platform":{"$in":["messenger","instagram"]},
                "meta_binding.grant_id":grant,
                "meta_binding.erasure_requested":{"$ne":true},
                "meta_binding.provider_revoked":{"$ne":true},
                "meta_binding.operator_suspended":{"$ne":true},
                "meta_binding.grant_reconnect_required":{"$ne":true},
            });
        }
        Some(_) => return Err(denied()),
        None => {
            bot_filter.insert("platform", doc! {"$nin":["messenger","instagram"]});
        }
    }
    let bot = db
        .collection::<BotOwnership>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one_and_update(
            bot_filter,
            doc! {"$set":{
                "personal_records_fence":Uuid::new_v4().to_string(),
                "channel_guest_data_present":true,
            }},
        )
        .projection(doc! {"ownership_version":1,"meta_binding.generation":1})
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    if let Some(grant_id) = meta_grant_id {
        let generation = bot.meta_binding.ok_or_else(denied)?.generation;
        crate::services::channel_meta_service::scopes::fence_reference(
            db,
            session,
            &crate::models::channel_meta::GrantRef {
                bot_id: permit.bot_id().into(),
                grant_id: grant_id.into(),
                generation,
            },
            true,
        )
        .await?;
    }
    execute_owned(
        db,
        session,
        permit,
        bot.ownership_version,
        owner,
        meta_grant_id,
        request,
    )
    .await
}

pub(crate) async fn purge_bot(db: &Database, bot_id: &str) -> AppResult<()> {
    db.collection::<Document>(COLLECTION_NAME)
        .delete_many(doc! {"bot_id":bot_id})
        .await?;
    Ok(())
}

pub(crate) async fn purge_before_ownership(
    db: &Database,
    bot_id: &str,
    version: i64,
) -> AppResult<bool> {
    const PAGE: i64 = 100;
    let rows = db.collection::<Document>(COLLECTION_NAME);
    let filter = doc! {"bot_id":bot_id,"bot_ownership_version":{"$lt":version}};
    let page = rows
        .find(filter.clone())
        .projection(doc! {"_id":1})
        .limit(PAGE)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    if !page.is_empty() {
        let ids: Vec<_> = page.iter().filter_map(|row| row.get("_id")).collect();
        let mut target = filter;
        target.insert("_id", doc! {"$in":ids});
        rows.delete_many(target).await?;
    }
    Ok(page.len() < PAGE as usize)
}

fn projection(allowed_fields: &[String], summary: bool) -> Document {
    let mut fields = doc! {"_id":1};
    for &field in &FIELDS {
        if !(summary && field == "body") && allowed_fields.iter().any(|allowed| allowed == field) {
            fields.insert(field, 1);
        }
    }
    fields
}

fn entry(row: &Document) -> AppResult<Value> {
    let id = row
        .get_str("_id")
        .map_err(|_| AppError::Internal("Malformed personal record".into()))?;
    let mut result = serde_json::Map::new();
    result.insert("record_id".into(), json!(id));
    for &field in &FIELDS {
        if let Ok(text) = row.get_str(field) {
            result.insert(field.into(), json!(text));
        }
    }
    Ok(Value::Object(result))
}

async fn execute_owned(
    db: &Database,
    session: &mut ClientSession,
    permit: &GuestDataPermit,
    bot_ownership_version: i64,
    owner_id: &str,
    meta_grant_id: Option<&str>,
    request: PersonalRecordRequest,
) -> AppResult<Value> {
    let bot_id = permit.bot_id();
    let read_fields = permit.read_fields();
    let rows = db.collection::<Document>(COLLECTION_NAME);
    let mut filter = owned(
        bot_id,
        bot_ownership_version,
        owner_id,
        meta_grant_id,
        request.record_id(),
    );
    let missing = || AppError::NotFound("Personal record not found".into());
    match request {
        PersonalRecordRequest::Create(fields) => {
            let now = Utc::now();
            let record = ChannelGuestPersonalRecord {
                id: Uuid::new_v4().to_string(),
                bot_id: bot_id.into(),
                bot_ownership_version,
                meta_grant_id: meta_grant_id.map(str::to_owned),
                owner_id: owner_id.into(),
                title: fields.title,
                body: fields.body,
                created_at: now,
                updated_at: now,
            };
            db.collection::<ChannelGuestPersonalRecord>(COLLECTION_NAME)
                .insert_one(&record)
                .session(session)
                .await?;
            Ok(json!({"record_id":record.id}))
        }
        PersonalRecordRequest::List(list) => {
            if let Some(prefix) = list.title_prefix {
                filter.insert(
                    "title",
                    Regex {
                        pattern: format!("^{}", regex::escape(&prefix)),
                        options: String::new(),
                    },
                );
            }
            if let Some(cursor) = list.cursor {
                filter.insert("_id", doc! {"$gt":cursor});
            }
            let mut cursor = rows
                .find(filter)
                .projection(projection(read_fields, true))
                .sort(doc! {"_id":1})
                .limit((PAGE_SIZE + 1) as i64)
                .max_time(SEARCH_TIMEOUT)
                .session(&mut *session)
                .await?;
            let mut records = cursor.stream(session).try_collect::<Vec<_>>().await?;
            let next_cursor = if records.len() > PAGE_SIZE {
                records.pop();
                records.last().and_then(|last| last.get_str("_id").ok())
            } else {
                None
            };
            let entries = records.iter().map(entry).collect::<AppResult<Vec<_>>>()?;
            Ok(json!({"records":entries,"next_cursor":next_cursor}))
        }
        PersonalRecordRequest::Read(_) => {
            let row = rows
                .find_one(filter)
                .projection(projection(read_fields, false))
                .session(session)
                .await?
                .ok_or_else(missing)?;
            entry(&row)
        }
        PersonalRecordRequest::Update(update) => {
            let mut patch = doc! {"updated_at":bson::DateTime::now()};
            if let Some(title) = update.title {
                patch.insert("title", title);
            }
            if let Some(body) = update.body {
                patch.insert("body", body);
            }
            let result = rows
                .update_one(filter, doc! {"$set":patch})
                .session(session)
                .await?;
            if result.matched_count == 0 {
                return Err(missing());
            }
            Ok(json!({"record_id":update.record_id}))
        }
        PersonalRecordRequest::Delete(id) => {
            let result = rows.delete_one(filter).session(session).await?;
            if result.deleted_count == 0 {
                return Err(missing());
            }
            Ok(json!({"record_id":id.record_id,"deleted":true}))
        }
    }
}

#[cfg(test)]
mod tests;
