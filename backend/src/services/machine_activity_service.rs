//! Machine receipts are metadata only. Optional output uses encrypted attachments,
//! a human-owned thread policy, and the existing fenced retention/purge paths.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_attachment::{AssistantAttachment, COLLECTION_NAME as ATTACHMENTS},
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        machine_receipt::MachineReceipt,
    },
    services::assistant_acknowledgement_service::ChatAuthority,
};
use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
};
use nyxid_machine::Operation;
use serde_json::Value;
use uuid::Uuid;

// A scoped future, never a global mutable "current call". Universal/native tools
// share the outer MCP activity; concurrent calls cannot steal the correlation.
tokio::task_local! { pub static ACTIVITY_ID: Option<String>; }
pub fn activity_id() -> Option<String> {
    ACTIVITY_ID.try_with(Clone::clone).ok().flatten()
}

pub fn action(operation: Operation, args: &Value) -> &'static str {
    match operation {
        Operation::Exec => "command.exec",
        Operation::Job => "command.status",
        Operation::JobCancel => "command.cancel",
        Operation::ReadFile => "file.read",
        Operation::WriteFile => "file.write",
        Operation::EditFile => "file.edit",
        Operation::ListFiles => "file.list",
        Operation::SaveAttachment => "file.save_attachment",
        Operation::ShareFile => "file.share",
        Operation::Computer => "computer.action",
        Operation::FillLogin => "browser.fill_login",
        Operation::Browser => match args["action"].as_str() {
            Some("navigate") => "browser.navigate",
            Some("click") => "browser.click",
            Some("type") => "browser.type",
            Some("select") => "browser.select",
            Some("press") => "browser.press_key",
            Some("scroll") => "browser.scroll",
            Some("snapshot") => "browser.snapshot",
            Some("screenshot") => "browser.screenshot",
            Some("evaluate") => "browser.evaluate",
            Some("console") => "browser.console",
            Some("network") => "browser.network",
            Some("back") => "browser.back",
            Some("forward") => "browser.forward",
            Some("find") => "browser.find",
            Some("tabs") => "browser.tabs",
            Some("tabs_new") => "browser.tabs_new",
            Some("tabs_switch") => "browser.tabs_switch",
            Some("tabs_close") => "browser.tabs_close",
            Some("wait") => "browser.wait",
            _ => "browser.action",
        },
        _ => "machine.action",
    }
}

/// File sizes are not read lengths. Retain only bytes actually transferred;
/// older edit replies that provide only a digest have no invented byte count.
pub fn transferred_bytes(operation: Operation, args: &Value, result: &Value) -> Option<u64> {
    match operation {
        Operation::ReadFile => {
            let end = result["offset"].as_u64()?;
            let start = args["offset"]
                .as_u64()
                .unwrap_or(0)
                .min(result["size"].as_u64()?);
            end.checked_sub(start)
        }
        Operation::WriteFile
        | Operation::EditFile
        | Operation::SaveAttachment
        | Operation::ShareFile => result["bytes"].as_u64().or_else(|| result["size"].as_u64()),
        _ => None,
    }
}

pub fn outcome(result: &Value, operation: Operation) -> &'static str {
    if result.get("error").is_some()
        || result["isError"] == true
        || result["timed_out"] == true
        || result["status"] == "refused"
        || result["exit_code"].as_i64().is_some_and(|v| v != 0)
    {
        "error"
    } else if operation == Operation::JobCancel || result["status"] == "cancelled" {
        "cancelled"
    } else if result["status"] == "running" {
        "running"
    } else {
        "completed"
    }
}

pub fn settle_activity(
    activity: &mut crate::models::assistant_conversation::TurnActivity,
    error: Option<&str>,
) {
    if let Some(receipt) = activity.machine.as_mut()
        && receipt.status == "running"
        && (matches!(activity.status.as_str(), "running" | "error") || error == Some("cancelled"))
    {
        receipt.status = match error {
            Some("cancelled") => "cancelled",
            Some(_) => "error",
            None if activity.status == "error" => "error",
            None => "unknown",
        }
        .into();
    }
}

pub fn receipt(
    chat: &ChatAuthority,
    node: &str,
    operation: Operation,
    args: &Value,
) -> MachineReceipt {
    MachineReceipt {
        operation_id: Uuid::new_v4().to_string(),
        node_id: node.into(),
        agent_id: chat.agent_id.clone(),
        context_mode: None,
        action: action(operation, args).into(),
        status: "running".into(),
        job_id: args["job_id"]
            .as_str()
            .filter(|v| Uuid::parse_str(v).is_ok())
            .map(str::to_owned),
        exit_code: None,
        bytes: None,
        duration_ms: None,
        error_code: None,
        screenshot_id: None,
        preview_id: None,
        preview_enabled: false,
    }
}

/// Input is already redacted by the node's live secret registry. Apply a second
/// bounded scrub for common credential assignments, then strip terminal controls
/// before truncation. Never scan commands/paths or load credentials to make a card.
pub fn excerpt(result: &Value) -> String {
    use std::sync::LazyLock;
    static SECRETS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
        r#"(?im)(?:authorization|cookie|set-cookie|[a-z0-9_]*(?:token|password|secret|api[_-]?key)[a-z0-9_]*)\s*[:=]\s*[^\r\n]+|\bBearer\s+[^\s]+|\b(?:nyx_(?:nauth|nreg|key)|sk)[_-][A-Za-z0-9_-]+"#
    ).expect("static scrub pattern")
    });
    fn clean(value: &str, cap: usize, lines: &mut usize) -> String {
        let mut plain = String::new();
        // Bound work even for a misbehaving node. Node responses are themselves bounded.
        let mut chars = value.chars().take(64 * 1024).peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                match chars.next() {
                    Some('[') => {
                        for c in chars.by_ref() {
                            if ('@'..='~').contains(&c) {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        let mut escape = false;
                        for c in chars.by_ref() {
                            if c == '\u{7}' || (escape && c == '\\') {
                                break;
                            }
                            escape = c == '\u{1b}';
                        }
                    }
                    _ => {}
                }
            } else if c == '\n'
                || c == '\t'
                || (!c.is_control()
                    && !matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
            {
                plain.push(c);
            }
        }
        let scrubbed = SECRETS.replace_all(&plain, "[redacted]");
        let mut out = String::new();
        for c in scrubbed.chars() {
            if out.len() + c.len_utf8() > cap || *lines >= 40 {
                break;
            }
            out.push(c);
            if c == '\n' {
                *lines += 1;
            }
        }
        out
    }
    let mut lines = 0;
    let stdout = clean(
        result["stdout"].as_str().unwrap_or_default(),
        2048,
        &mut lines,
    );
    let stderr = clean(
        result["stderr"].as_str().unwrap_or_default(),
        1024,
        &mut lines,
    );
    if stdout.is_empty() && stderr.is_empty() {
        String::new()
    } else {
        format!("stdout (bounded):\n{stdout}\nstderr (bounded):\n{stderr}")
    }
}

pub async fn record(
    db: &Database,
    chat: &ChatAuthority,
    receipt: &MachineReceipt,
) -> AppResult<()> {
    let Some(activity) = activity_id() else {
        return Ok(());
    };
    db.collection::<bson::Document>(CONVERSATIONS).update_one(
        doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,"active_turn.turn_id":&chat.turn_id,
            "active_turn.stop_requested":false,"active_turn.activities.id":activity},
        doc! {"$set":{"active_turn.activities.$.machine":bson::to_bson(receipt).map_err(|_|AppError::Internal("Machine receipt encoding failed".into()))?}},
    ).await?;
    Ok(())
}

pub async fn preview_enabled(db: &Database, chat: &ChatAuthority) -> AppResult<bool> {
    let row = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {
            "_id":&chat.conversation_id,"user_id":&chat.user_id,"active_turn.turn_id":&chat.turn_id,
            "active_turn.stop_requested":false,
        })
        .await?;
    // Group tasks remain metadata-only; a private opt-in cannot publish output
    // to other participants. No agent or machine-admin setting enables this.
    Ok(row.is_some_and(|r| r.machine_previews && r.group_id.is_none() && !r.guest_turn))
}

pub async fn preview_policy(
    db: &Database,
    actor: &str,
    id: &str,
    enabled: Option<bool>,
) -> AppResult<bool> {
    let row = super::assistant_nyxagent::get(db, actor, id).await?;
    if row.group_id.is_some() || row.guest_turn {
        return Err(AppError::Forbidden(
            "Machine excerpts are available only in private owner threads".into(),
        ));
    }
    if let Some(enabled) = enabled {
        let updated = db
            .collection::<bson::Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id":id,"user_id":actor,"group_id":bson::Bson::Null,"guest_turn":{"$ne":true}},
                doc! {"$set":{"machine_previews":enabled}},
            )
            .await?;
        if updated.matched_count != 1 {
            return Err(AppError::NotFound("Conversation not found".into()));
        }
        Ok(enabled)
    } else {
        Ok(row.machine_previews)
    }
}

pub async fn store_preview(
    state: &AppState,
    chat: &ChatAuthority,
    receipt: &mut MachineReceipt,
    result: &Value,
) -> AppResult<()> {
    let Some(activity) = activity_id() else {
        return Ok(());
    };
    if !receipt.preview_enabled
        || !matches!(receipt.action.as_str(), "command.exec" | "command.status")
    {
        return Ok(());
    }
    let text = zeroize::Zeroizing::new(excerpt(result));
    if text.is_empty() {
        return Ok(());
    }
    let id = Uuid::new_v4().to_string();
    let encrypted = state.encryption_keys.encrypt(text.as_bytes()).await?;
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    // Capture and receipt reference commit together. Stop, settling, disabling,
    // deletion and eviction all prevent new retention, including late replies.
    let changed = state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,"group_id":bson::Bson::Null,
            "machine_previews":true,"guest_turn":{"$ne":true},"active_turn.turn_id":&chat.turn_id,
            "active_turn.stop_requested":false,"active_turn.activities.id":&activity},
            doc! {"$set":{"active_turn.activities.$.machine.preview_id":&id}},
        )
        .session(&mut session)
        .await?;
    if changed.matched_count == 1 {
        state
            .db
            .collection::<AssistantAttachment>(ATTACHMENTS)
            .insert_one(AssistantAttachment {
                id: id.clone(),
                origin: "machine_preview".into(),
                user_id: chat.user_id.clone(),
                conversation_id: chat.conversation_id.clone(),
                turn_id: chat.turn_id.clone().unwrap_or_default(),
                content_type: "text/plain".into(),
                size: text.len() as i64,
                data_encrypted: encrypted,
                created_at: Utc::now(),
            })
            .session(&mut session)
            .await?;
        session.commit_transaction().await?;
        receipt.preview_id = Some(id);
    } else {
        session.abort_transaction().await?;
    }
    Ok(())
}

/// Job state is authoritative; a successful exec response is not job completion.
/// Bounded batch lookup, never an N+1 query per activity.
pub async fn refresh_jobs(
    db: &Database,
    user: &str,
    receipts: Vec<&mut MachineReceipt>,
) -> AppResult<()> {
    refresh_jobs_for_owners(db, &[user.to_owned()], receipts).await
}

/// Only call after authorizing group transcript access. These are already
/// published metadata receipts, never pointers to private output.
pub async fn refresh_group_jobs(
    db: &Database,
    messages: &mut [crate::models::assistant_group::GroupMessage],
) -> AppResult<()> {
    let owners: Vec<_> = messages
        .iter()
        .map(|m| m.author_user_id.as_ref().unwrap_or(&m.user_id).clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let receipts = messages
        .iter_mut()
        .flat_map(|m| m.activities.iter_mut())
        .filter_map(|a| a.machine.as_deref_mut())
        .collect();
    refresh_jobs_for_owners(db, &owners, receipts).await
}

async fn refresh_jobs_for_owners(
    db: &Database,
    owners: &[String],
    receipts: Vec<&mut MachineReceipt>,
) -> AppResult<()> {
    use futures::TryStreamExt;
    let ids: Vec<_> = receipts
        .iter()
        .filter(|r| r.status == "running")
        .filter_map(|r| r.job_id.clone())
        .take(2000)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let rows: Vec<bson::Document> = db
        .collection(crate::models::machine_job::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":&ids},"user_id":{"$in":owners}})
        .projection(doc! {"_id":1,"state":1,"exit_code":1,"receipt_cancelled":1})
        .limit(2000)
        .max_time(std::time::Duration::from_secs(3))
        .await?
        .try_collect()
        .await?;
    for receipt in receipts {
        if receipt.status == "running" && receipt.job_id.as_ref().is_some_and(|id| ids.contains(id))
        {
            let Some(job) = rows
                .iter()
                .find(|r| r.get_str("_id").ok() == receipt.job_id.as_deref())
            else {
                // Expired job metadata cannot prove that work is still running.
                receipt.status = "unknown".into();
                continue;
            };
            let state = job.get_str("state").unwrap_or("running");
            receipt.exit_code = job.get_i64("exit_code").ok();
            receipt.status = match state {
                _ if job.get_bool("receipt_cancelled").unwrap_or(false) => "cancelled",
                "cancelled" => "cancelled",
                "finished" if receipt.exit_code.is_some_and(|c| c != 0) => "error",
                "finished" => "finished",
                _ => "running",
            }
            .into();
        }
    }
    Ok(())
}

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ActivityQuery {
    pub agent_id: Option<String>,
    #[serde(default)]
    pub unattributed: bool,
    pub before: Option<String>,
    pub limit: Option<u32>,
}
#[derive(serde::Serialize)]
pub struct ActivityEntry {
    pub id: String,
    pub agent_id: Option<String>,
    pub actor_id: Option<String>,
    pub action: String,
    pub outcome: String,
    pub operation_id: Option<String>,
    pub activity_id: Option<String>,
    pub job_id: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
    pub created_at: chrono::DateTime<Utc>,
}
#[derive(serde::Serialize)]
pub struct ActivityPage {
    pub entries: Vec<ActivityEntry>,
    pub next_cursor: Option<String>,
    pub machine_name: Option<String>,
    pub agents: Vec<ActivityAgent>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ActivityAgent {
    #[serde(rename(deserialize = "_id"))]
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub kind: crate::models::assistant_agent::AgentKind,
}

/// The viewer's personal agents and this machine's organization specialists.
/// Called after the machine activity ACL; never expose another member's private agents.
pub async fn activity_agents(
    db: &Database,
    viewer: &str,
    owner: &str,
) -> AppResult<Vec<ActivityAgent>> {
    use futures::TryStreamExt;
    let mut scopes = vec![doc! {"user_id":viewer}];
    if owner != viewer
        && super::org_agent_service::can_use(
            &super::org_agent_service::access(db, viewer, owner).await?,
        )
    {
        scopes.push(doc! {"user_id":owner,"kind":"specialist"});
    }
    Ok(db
        .collection(crate::models::assistant_agent::COLLECTION_NAME)
        .find(doc! {"$or":scopes})
        .projection(doc! {"_id":1,"name":1,"display_name":1,"kind":1})
        .sort(doc! {"name":1,"_id":1})
        .limit(500)
        .max_time(std::time::Duration::from_secs(3))
        .await?
        .try_collect()
        .await?)
}

/// Current names for one already-authorized transcript page. Apply the node
/// read ACL (personal owner, org admin/member) with one membership snapshot and
/// batch queries, never one lookup per receipt. Do not persist display names.
pub async fn machine_names(
    db: &Database,
    viewer: &str,
    ids: Vec<String>,
) -> AppResult<std::collections::HashMap<String, String>> {
    use futures::TryStreamExt;
    let ids: std::collections::HashSet<_> = ids.into_iter().take(8000).collect();
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let orgs: Vec<_> = super::org_service::list_memberships_for_member(db, viewer, false)
        .await?
        .into_iter()
        .filter(|m| m.role.can_proxy())
        .map(|m| m.org_user_id)
        .collect();
    let mut owners = vec![viewer.to_owned()];
    if !orgs.is_empty() {
        let rows: Vec<bson::Document> = db
            .collection(crate::models::user::COLLECTION_NAME)
            .find(doc! {"_id":{"$in":orgs},"user_type":"org","is_active":true})
            .projection(doc! {"_id":1})
            .max_time(std::time::Duration::from_secs(3))
            .await?
            .try_collect()
            .await?;
        owners.extend(
            rows.iter()
                .filter_map(|r| r.get_str("_id").ok().map(str::to_owned)),
        );
    }
    let rows: Vec<bson::Document> = db.collection(crate::models::node::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":ids.into_iter().collect::<Vec<_>>()},"user_id":{"$in":owners},"is_active":true})
        .projection(doc! {"_id":1,"name":1}).limit(8000)
        .max_time(std::time::Duration::from_secs(3)).await?.try_collect().await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some((
                r.get_str("_id").ok()?.to_owned(),
                r.get_str("name").ok()?.to_owned(),
            ))
        })
        .collect())
}

fn uuid(value: Option<&str>) -> Option<String> {
    value
        .filter(|s| Uuid::parse_str(s).is_ok())
        .map(str::to_owned)
}

pub async fn list(db: &Database, node: &str, query: ActivityQuery) -> AppResult<ActivityPage> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use futures::TryStreamExt;
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let mut filter = doc! {"event_type":"machine_operation","event_data.node_id":node};
    if query.unattributed {
        if query.agent_id.is_some() {
            return Err(AppError::ValidationError(
                "Choose an agent or unattributed activity".into(),
            ));
        }
        filter.insert("event_data.agent_id", bson::Bson::Null);
    }
    if let Some(agent) = query.agent_id {
        let id = uuid(Some(&agent))
            .ok_or_else(|| AppError::ValidationError("Invalid agent ID".into()))?;
        filter.insert("event_data.agent_id", id);
    }
    if let Some(cursor) = query.before {
        if cursor.len() > 200 {
            return Err(AppError::ValidationError("Invalid activity cursor".into()));
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(cursor)
            .ok()
            .and_then(|b| serde_json::from_slice::<(i64, String)>(&b).ok())
            .filter(|(_, id)| Uuid::parse_str(id).is_ok())
            .ok_or_else(|| AppError::ValidationError("Invalid activity cursor".into()))?;
        let date = bson::DateTime::from_millis(decoded.0);
        filter.insert(
            "$or",
            vec![
                doc! {"created_at":{"$lt":date}},
                doc! {"created_at":date,"_id":{"$lt":decoded.1}},
            ],
        );
    }
    let mut rows:Vec<bson::Document>=db.collection(crate::models::audit_log::COLLECTION_NAME).find(filter)
        .projection(doc!{"_id":1,"user_id":1,"created_at":1,"event_data.agent_id":1,"event_data.operation":1,
            "event_data.action":1,"event_data.outcome":1,"event_data.operation_id":1,"event_data.activity_id":1,
            "event_data.job_id":1,"event_data.exit_code":1,"event_data.duration_ms":1})
        .sort(doc!{"created_at":-1,"_id":-1}).limit(i64::from(limit)+1)
        .max_time(std::time::Duration::from_secs(3)).await?.try_collect().await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if more {
        rows.last().and_then(|row| {
            Some(
                URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&(
                        row.get_datetime("created_at").ok()?.timestamp_millis(),
                        row.get_str("_id").ok()?,
                    ))
                    .ok()?,
                ),
            )
        })
    } else {
        None
    };
    // At most 100 rows (clamped page size); grow naturally.
    let mut entries = Vec::new();
    for row in rows {
        let Ok(data) = row.get_document("event_data") else {
            continue;
        };
        let op = data
            .get_str("operation")
            .ok()
            .and_then(|s| serde_json::from_value::<Operation>(Value::String(s.into())).ok());
        let candidate = data.get_str("action").ok();
        let action_name = candidate
            .filter(|s| valid_action(s))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                op.map(|o| action(o, &Value::Null))
                    .unwrap_or("machine.action")
                    .into()
            });
        let outcome = data
            .get_str("outcome")
            .ok()
            .filter(|s| {
                matches!(
                    *s,
                    "completed"
                        | "failed"
                        | "refused"
                        | "running"
                        | "finished"
                        | "error"
                        | "cancelled"
                )
            })
            .unwrap_or("unknown")
            .into();
        entries.push(ActivityEntry {
            id: row.get_str("_id").unwrap_or_default().into(),
            agent_id: uuid(data.get_str("agent_id").ok()),
            actor_id: uuid(row.get_str("user_id").ok()),
            action: action_name,
            outcome,
            operation_id: uuid(data.get_str("operation_id").ok()),
            activity_id: uuid(data.get_str("activity_id").ok()),
            job_id: uuid(data.get_str("job_id").ok()),
            exit_code: data.get_i64("exit_code").ok(),
            duration_ms: data
                .get_i64("duration_ms")
                .ok()
                .and_then(|n| u64::try_from(n).ok()),
            created_at: row
                .get_datetime("created_at")
                .map(|d| d.to_chrono())
                .unwrap_or_default(),
        });
    }
    let ids: Vec<_> = entries.iter().filter_map(|r| r.job_id.as_ref()).collect();
    if !ids.is_empty() {
        let jobs: Vec<bson::Document> = db
            .collection(crate::models::machine_job::COLLECTION_NAME)
            .find(doc! {"_id":{"$in":ids},"node_id":node})
            .projection(doc! {"_id":1,"state":1,"exit_code":1,"receipt_cancelled":1})
            .limit(100)
            .max_time(std::time::Duration::from_secs(3))
            .await?
            .try_collect()
            .await?;
        for entry in &mut entries {
            if entry.outcome == "running" && entry.job_id.is_some() {
                let Some(job) = jobs
                    .iter()
                    .find(|j| j.get_str("_id").ok() == entry.job_id.as_deref())
                else {
                    entry.outcome = "unknown".into();
                    continue;
                };
                entry.outcome = match job.get_str("state").ok() {
                    _ if job.get_bool("receipt_cancelled").unwrap_or(false) => "cancelled",
                    Some("finished") if job.get_i64("exit_code").ok().is_some_and(|c| c != 0) => {
                        "error"
                    }
                    Some("finished") => "finished",
                    Some("cancelled") => "cancelled",
                    _ => "running",
                }
                .into();
                entry.exit_code = job.get_i64("exit_code").ok();
            }
        }
    }
    Ok(ActivityPage {
        entries,
        next_cursor,
        machine_name: None,
        agents: Vec::new(),
    })
}

fn valid_action(value: &str) -> bool {
    matches!(
        value,
        "command.exec"
            | "command.status"
            | "command.cancel"
            | "file.read"
            | "file.write"
            | "file.edit"
            | "file.list"
            | "file.save_attachment"
            | "file.share"
            | "computer.action"
            | "browser.fill_login"
            | "browser.action"
            | "machine.action"
            | "browser.navigate"
            | "browser.click"
            | "browser.type"
            | "browser.select"
            | "browser.press_key"
            | "browser.scroll"
            | "browser.snapshot"
            | "browser.screenshot"
            | "browser.evaluate"
            | "browser.console"
            | "browser.network"
            | "browser.back"
            | "browser.forward"
            | "browser.find"
            | "browser.tabs"
            | "browser.tabs_new"
            | "browser.tabs_switch"
            | "browser.tabs_close"
            | "browser.wait"
    )
}

/// Group receipts contain metadata from this exact group turn only. Attachment
/// IDs and private-thread preview consent never cross the publication boundary.
pub async fn publish_group(
    db: &Database,
    thread: &AssistantConversation,
    message: &crate::models::assistant_group::GroupMessage,
) -> AppResult<()> {
    let Some(turn) = thread.active_turn.as_ref() else {
        return Ok(());
    };
    if thread.group_id.as_deref() != Some(message.group_id.as_str())
        || thread.agent_id != message.agent_id
        || thread.group_request_id != message.request_id
    {
        return Ok(());
    }
    let row=db.collection::<crate::models::assistant_message::AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc!{"conversation_id":&thread.id,"user_id":&thread.user_id,"turn_id":&turn.turn_id,"role":"assistant"}).await?;
    let mut activities = row
        .map(|m| m.activities)
        .unwrap_or_else(|| turn.activities.clone());
    activities.retain(|a| a.machine.is_some());
    activities.truncate(super::assistant_nyxagent::MAX_TURN_ACTIVITIES as usize);
    for activity in &mut activities {
        if let Some(receipt) = activity.machine.as_mut() {
            receipt.screenshot_id = None;
            receipt.preview_id = None;
            receipt.preview_enabled = false;
        }
    }
    db.collection::<bson::Document>(crate::models::assistant_group::MESSAGES_COLLECTION_NAME)
        .update_one(doc!{"_id":&message.id,"group_id":&message.group_id,"user_id":&message.user_id,"agent_id":&thread.agent_id},
        doc!{"$set":{"activities":bson::to_bson(&activities).map_err(|_|AppError::Internal("Machine receipt encoding failed".into()))?}}).await?;
    Ok(())
}

#[cfg(test)]
#[path = "machine_activity_tests.rs"]
mod tests;
