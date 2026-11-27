//! Exact connected ledger-write decisions in the originating personal owner DM.
//! Prepared effects are ephemeral; stored bindings contain no arguments or capabilities.
use bson::{Document, doc};
use chrono::Utc;
use mongodb::{ClientSession, Database};
use serde_json::Value;

use super::{assistant_acknowledgement_service::ChatAuthority, assistant_nyxagent as engine};
use crate::{
    errors::{AppError, AppResult},
    models::{
        approval_request::{ApprovalRequest, SupervisorSource, SupervisorWrite},
        assistant_conversation::{AssistantConversation, ChannelOrigin, TurnOrigin},
        service_approval_config::ApprovalMode,
    },
};

#[derive(Clone)]
pub(crate) struct Parent {
    conversation: String,
    key: String,
    turn: String,
}
pub(crate) fn parent(chat: &ChatAuthority) -> Option<Parent> {
    if !chat.is_orchestrator() || !chat.turn_live || chat.turn_stopped || chat.guest {
        return None;
    }
    Some(Parent {
        conversation: chat.conversation_id.clone(),
        key: chat.api_key_id.clone(),
        turn: chat.turn_id.clone()?,
    })
}

fn refused() -> AppError {
    AppError::Forbidden(
        "This exact ledger-write review is unavailable; no new write is authorized".into(),
    )
}
fn fingerprint(domain: &[u8], value: &[u8]) -> AppResult<String> {
    super::audit_service::keyed_fingerprint(
        super::audit_service::audit_chain_hmac_key(),
        domain,
        value,
    )
    .ok_or_else(refused)
}

/// Called only by the engine's delegation admission transaction. Neither model
/// text nor a mutable report destination can construct this source.
pub(crate) async fn assign(
    db: &Database,
    tx: &mut ClientSession,
    child: &mut AssistantConversation,
    parent: Option<&Parent>,
) -> AppResult<()> {
    let Some(causal) = parent else { return Ok(()) };
    let Some(turn) = child.active_turn.as_ref() else {
        return Ok(());
    };
    if turn.origin != TurnOrigin::Orchestrator || !personal(child) || !child.is_subagent() {
        return Ok(());
    }
    let Some(target) = turn.report_to.as_deref() else {
        return Ok(());
    };
    if target != causal.conversation {
        return Ok(());
    }
    let Some(parent) = db.collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":target,"user_id":&child.user_id,"credential_api_key_id":&causal.key,"active_turn.turn_id":&causal.turn}).session(&mut *tx).await?
    else { return Ok(()) };
    let Some(assigning) = engine::live_turn(&parent, Utc::now()).filter(|t| !t.stop_requested)
    else {
        return Ok(());
    };
    if !personal(&parent)
        || parent.is_subagent()
        || !matches!(assigning.origin, TurnOrigin::Channel | TurnOrigin::Event)
        || assigning.trigger_run_id.is_some()
        || assigning.automation_work.is_some()
        || assigning.voice_request_id.is_some()
        || assigning.invocation_id.is_some()
    {
        return Ok(());
    }
    // A yes-code continuation can be an Event. Accept only one positively
    // authenticated original source, including its carried channel context.
    let contexts: Vec<_> = assigning
        .channel_context
        .iter()
        .chain(assigning.events.iter().flat_map(|e| &e.channel_contexts))
        .collect();
    let Some(context) = contexts.first().filter(|_| contexts.len() == 1) else {
        return Ok(());
    };
    if context.guest
        || !context.original_message
        || context.conversation_kind != "private"
        || context.sender_kind != crate::models::channel_thread::ThreadSenderKind::Human
        || context.org_origin.is_some()
        || context.channel_guest.is_some()
        || context.followed
    {
        return Ok(());
    }
    let source = match super::execution_permission_channel::admit_supervisor(
        db,
        tx,
        &parent,
        &context.source_event_id,
    )
    .await
    {
        Ok(source) => source,
        Err(AppError::Forbidden(_) | AppError::NotFound(_) | AppError::Conflict(_)) => {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let binding = SupervisorSource {
        channel: source,
        assigning_turn: assigning.turn_id.clone(),
        reset_at: parent.context_reset_at,
    };
    if db.collection::<Document>("assistant_conversations").update_one(
        doc! {"_id":&parent.id,"credential_api_key_id":&causal.key,"active_turn.turn_id":&assigning.turn_id,"active_turn.stop_requested":false},
        doc! {"$inc":{"supervisor_assignment_fence":1_i64}},
    ).session(&mut *tx).await?.matched_count != 1 { return Err(refused()); }
    child.active_turn.as_mut().unwrap().supervisor_source = Some(Box::new(binding));
    Ok(())
}

fn personal(row: &AssistantConversation) -> bool {
    !row.guest_turn
        && !row.shared_agent_thread
        && row.agent_owner_id.is_none()
        && row.group_id.is_none()
        && row.collaboration.is_none()
        && row.platform_thread.is_none()
        && !row.automation_thread
        && !row.invocation_controlled
        && row.voice_parent_conversation_id.is_none()
}

pub(crate) fn turn_guidance(row: &AssistantConversation) -> &'static str {
    if personal(row)
        && row.is_subagent()
        && row
            .active_turn
            .as_ref()
            .is_some_and(|turn| turn.supervisor_source.is_some())
    {
        "\n\nIf you make a connected write, independently read the affected resource before reporting its final state. A write acknowledgement is not readback evidence. After uncertain dispatch, observe without automatically repeating the write."
    } else {
        ""
    }
}

/// Complete, bounded display from a closed schema. Nested values, URLs,
/// credential names/shapes and control text are never sent to the owner chat.
fn safe_text(text: &str, limit: usize) -> bool {
    let lower = text.to_lowercase();
    !text.is_empty()
        && text.chars().count() <= limit
        && !text.chars().any(|c| {
            c.is_control() || matches!(c, '<' | '>' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '~' | '|' | '#' | '@' | '&' | '\\'
                | '\u{00ad}' | '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
        })
        && !super::assistant_team_service::looks_secret(text)
        && !super::assistant_skill_authoring::credential_shaped(text)
        && ![
            "bearer ",
            "cookie",
            "password",
            "secret",
            "token",
            "credential",
            "authorization",
            "api_key",
            "apikey",
            "://",
            "www.",
            "密码",
            "密钥",
            "令牌",
        ]
        .iter()
        .any(|word| lower.contains(word))
        && !text
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .any(|word| word.len() > 24)
}

// The supported JSON shape has exactly two map levels. Capture raw scalar
// tokens only inside `fields`, checking decimal value before building a review.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RowUpdate {
    fields: ScalarFields,
}

struct ScalarFields(serde_json::Map<String, Value>);
impl<'de> serde::Deserialize<'de> for ScalarFields {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = ScalarFields;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a unique map of scalar ledger fields")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<ScalarFields, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.len() >= 8 || values.contains_key(&key) {
                        return Err(serde::de::Error::custom("too many or duplicate fields"));
                    }
                    let raw = map.next_value::<Box<serde_json::value::RawValue>>()?;
                    let token = raw.get().trim();
                    if matches!(token.as_bytes().first(), Some(b'{' | b'[')) {
                        return Err(serde::de::Error::custom("ledger fields must be scalar"));
                    }
                    // This parser receives a scalar token, so nested maps never
                    // enter a fresh deserializer or recursively call this visitor.
                    let value: Value =
                        serde_json::from_str(token).map_err(serde::de::Error::custom)?;
                    if let Value::Number(number) = &value
                        && canonical_decimal(token) != canonical_decimal(&number.to_string())
                    {
                        return Err(serde::de::Error::custom(
                            "number cannot be reviewed exactly",
                        ));
                    }
                    values.insert(key, value);
                }
                Ok(ScalarFields(values))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

pub(crate) struct Effect {
    body_digest: String,
    fields_digest: String,
    review: String,
    path: String,
}

/// Apply the same caller admission and default layers as HTTP egress. Only
/// unambiguous JSON media headers may accompany this bounded review adapter.
pub(crate) fn http_headers_allow_review(
    headers: &http::HeaderMap,
    defaults: &[&[crate::models::default_request_header::ResolvedDefaultRequestHeader]],
) -> bool {
    let forwarded = super::proxy_service::collect_forward_headers_with_defaults(headers, defaults);
    if forwarded
        .iter()
        .any(|(name, _)| !matches!(name.as_str(), "content-type" | "accept"))
        || ["content-type", "accept"]
            .iter()
            .any(|name| headers.get_all(*name).iter().count() > 1)
    {
        return false;
    }
    let effective = crate::models::default_request_header::merge_into_header_list(
        crate::models::default_request_header::SecretHeaders::from(forwarded),
        defaults,
    );
    let media = |name: &str| {
        effective
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    };
    let json = media("content-type")
        .and_then(|value| value.parse::<mime::Mime>().ok())
        .is_some_and(|kind| {
            kind.type_() == mime::APPLICATION
                && kind.subtype() == mime::JSON
                && kind.params().count() <= 1
                && kind.params().all(|(name, value)| {
                    name == mime::CHARSET && value.as_str().eq_ignore_ascii_case("utf-8")
                })
        });
    json && media("accept").is_none_or(|value| value.eq_ignore_ascii_case("application/json"))
}

fn canonical_decimal(text: &str) -> Option<String> {
    let text = text.trim();
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((n, e)) => (n, e.parse::<i32>().ok()?),
        None => (text, 0_i32),
    };
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some("0".into());
    }
    let significant = digits.trim_end_matches('0');
    let exponent = exponent
        .checked_sub(fraction.len() as i32)?
        .checked_add((digits.len() - significant.len()) as i32)?;
    Some(format!(
        "{}{significant}e{exponent}",
        if negative { "-" } else { "" }
    ))
}
/// Both proxy and catalog MCP supply final prepared bytes here, never raw tool arguments.
pub(crate) fn effect(
    slug: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
    headers: bool,
) -> Option<Effect> {
    if !matches!(
        slug,
        "api-feishu" | "api-feishu-bot" | "api-lark" | "api-lark-bot"
    ) || method != "PUT"
        || query.is_some()
        || headers
        || body.len() > 4096
    {
        return None;
    }
    let path = format!("/{}", path.trim_start_matches('/'));
    let canonical = if matches!(slug, "api-lark-bot" | "api-feishu-bot") {
        path.strip_prefix("/open-apis")?
    } else {
        path.as_str()
    };
    let mut effect = effect_fields(canonical, body)?;
    effect.path = path;
    Some(effect)
}

fn effect_fields(path: &str, body: &[u8]) -> Option<Effect> {
    // Derived struct deserializers also accept positional sequences. The
    // provider contract requires an object with the named `fields` member.
    if body
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        != Some(b'{')
    {
        return None;
    }
    let parts: Vec<_> = path.split('/').collect();
    let [
        "",
        "bitable",
        "v1",
        "apps",
        base,
        "tables",
        table,
        "records",
        record,
    ] = parts.as_slice()
    else {
        return None;
    };
    let ids = [*base, *table, *record];
    if ids.iter().any(|id| {
        id.len() > 64
            || id.is_empty()
            || !id.chars().all(|c| c.is_ascii_alphanumeric())
            || super::assistant_skill_authoring::credential_shaped(id)
            || super::assistant_team_service::looks_secret(id)
    }) {
        return None;
    }
    let update = serde_json::from_slice::<RowUpdate>(body).ok()?;
    let fields = &update.fields.0;
    if fields.is_empty() || fields.len() > 8 {
        return None;
    }
    let mut review = format!(
        "Base: {}\nTable: {}\nRecord: {}\nProposed field values (quoted data):",
        ids[0], ids[1], ids[2]
    );
    for (name, value) in fields {
        if !safe_text(name, 48)
            || !(value.is_boolean()
                || value.is_number()
                || value.is_null()
                || value.as_str().is_some_and(|s| safe_text(s, 96)))
        {
            return None;
        }
        review.push_str(&format!(
            "\n{} = {}",
            serde_json::to_string(name).ok()?,
            value
        ));
    }
    if review.chars().count() > 1200 {
        return None;
    }
    Some(Effect {
        body_digest: fingerprint(b"supervisor-body-v1", body).ok()?,
        fields_digest: fingerprint(
            b"supervisor-fields-v1",
            &serde_json::to_vec(&(
                path,
                fields
                    .iter()
                    .map(|(name, value)| {
                        (
                            name,
                            match value {
                                Value::Number(number) => serde_json::json!([
                                    "number",
                                    canonical_decimal(&number.to_string())
                                ]),
                                value => serde_json::json!(["value", value]),
                            },
                        )
                    })
                    .collect::<std::collections::BTreeMap<_, _>>(),
            ))
            .ok()?,
        )
        .ok()?,
        review,
        path: path.into(),
    })
}

/// Kept only in the original connected call; never serialized or Debug printed.
pub(crate) struct PreparedReview {
    pub binding: Box<SupervisorWrite>,
    text: String,
    code: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare(
    db: &Database,
    chat: Option<&ChatAuthority>,
    connection: Option<&str>,
    owner: &str,
    mode: &ApprovalMode,
    effect: Option<Effect>,
    base: &str,
    catalog: &str,
) -> AppResult<Option<PreparedReview>> {
    let Some((chat, connection, effect)) = chat
        .zip(connection)
        .zip(effect)
        .map(|((c, s), e)| (c, s, e))
    else {
        return Ok(None);
    };
    if *mode != ApprovalMode::PerRequest
        || chat.is_orchestrator()
        || chat.guest
        || !chat.turn_live
        || chat.turn_stopped
        || chat.org_agent_access.is_some()
        || chat.platform_thread
        || chat.invocation_controlled
        || chat.execution_voice_task
        || chat.execution_group_id.is_some()
        || chat.collaboration_thread
        || owner != chat.user_id
    {
        return Ok(None);
    }
    let row = db.collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":&chat.conversation_id,"user_id":owner,"credential_api_key_id":&chat.api_key_id,
            "active_turn.turn_id":&chat.turn_id,"active_turn.stop_requested":false}).await?;
    let Some(source) = row
        .filter(personal)
        .and_then(|r| r.active_turn?.supervisor_source)
    else {
        return Ok(None);
    };
    let code = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
    let url = format!(
        "{}/{}",
        base.trim_end_matches('/'),
        effect.path.trim_start_matches('/')
    );
    let binding = Box::new(SupervisorWrite {
        version: 1,
        source: *source,
        catalog_service: catalog.into(),
        specialist_agent: chat.agent_id.clone(),
        specialist_conversation: chat.conversation_id.clone(),
        specialist_turn: chat.turn_id.clone().ok_or_else(refused)?,
        specialist_key: chat.api_key_id.clone(),
        connection: connection.into(),
        authority: authority(db, owner, connection, &chat.api_key_id, catalog).await?,
        url_digest: fingerprint(b"supervisor-url-v1", url.as_bytes())?,
        body_digest: effect.body_digest,
        effect_digest: Some(effect.fields_digest),
        code_digest: fingerprint(b"supervisor-code-v1", code.as_bytes())?,
        review_confirmed: false,
        dispatching: false,
        dm_decision_receipt: None,
    });
    fence(db, &binding).await?;
    let service = db
        .collection::<Document>("user_services")
        .find_one(doc! {"_id":connection,"user_id":owner})
        .projection(doc! {"endpoint_id":1})
        .await?
        .ok_or_else(refused)?;
    let endpoint = service.get_str("endpoint_id").map_err(|_| refused())?;
    let label = db
        .collection::<Document>("user_endpoints")
        .find_one(doc! {"_id":endpoint,"user_id":owner})
        .projection(doc! {"label":1})
        .await?
        .and_then(|r| r.get_str("label").ok().map(str::to_owned))
        .filter(|s| safe_text(s, 64))
        .unwrap_or_else(|| "Selected ledger connection".into());
    let label = serde_json::to_string(&label).map_err(|_| refused())?;
    let text = format!(
        "Ledger-write approval — one record, one waiting request.\nConnection: {label} ({connection})\n{}\nCurrent values are not verified by this review. The specialist must read the record independently after writing.\nThis approval grants no future writes.",
        effect.review
    );
    Ok(Some(PreparedReview {
        binding,
        text,
        code,
    }))
}

async fn authority(
    db: &Database,
    owner: &str,
    connection: &str,
    key: &str,
    catalog: &str,
) -> AppResult<String> {
    db.collection::<Document>("user_services")
        .find_one(doc! {
            "_id":connection,"user_id":owner,"is_active":true,"catalog_service_id":catalog
        })
        .await?
        .ok_or_else(refused)?;
    let snapshot =
        super::execution_permission_dispatch::authority::Snapshot::load(db, connection, key, None)
            .await?;
    fingerprint(b"supervisor-authority-v1", snapshot.digest().as_bytes())
}

async fn fence(db: &Database, binding: &SupervisorWrite) -> AppResult<()> {
    super::transaction_service::run_db_only(db, &(db.clone(), binding.clone()), |tx, (db, b)| {
        Box::pin(fence_in_session(db, tx, b))
    })
    .await
}

async fn fence_in_session(
    db: &Database,
    tx: &mut ClientSession,
    b: &SupervisorWrite,
) -> AppResult<()> {
    if b.version != 1 {
        return Err(refused());
    }
    let snapshot = super::execution_permission_dispatch::authority::Snapshot::load(
        db,
        &b.connection,
        &b.specialist_key,
        None,
    )
    .await?;
    if fingerprint(b"supervisor-authority-v1", snapshot.digest().as_bytes())? != b.authority {
        return Err(refused());
    }
    snapshot.fence(db, tx).await?;
    let source = &b.source.channel;
    super::execution_permission_channel::validate_supervisor(db, tx, source, true).await?;
    let rows = db.collection::<AssistantConversation>("assistant_conversations");
    let child = rows.find_one_and_update(doc! {
        "_id":&b.specialist_conversation,"user_id":&source.actor_user_id,"agent_id":&b.specialist_agent,
        "credential_api_key_id":&b.specialist_key,"active_turn.turn_id":&b.specialist_turn,
        "active_turn.stop_requested":false,"active_turn.supervisor_source":bson::to_bson(&b.source).map_err(|_|refused())?
    },doc! {"$inc":{"supervisor_write_fence":1_i64}}).session(&mut *tx).await?.ok_or_else(refused)?;
    if !personal(&child) || engine::live_turn(&child, Utc::now()).is_none() {
        return Err(refused());
    }
    let parent = rows.find_one_and_update(doc! {
        "_id":&source.conversation_id,"user_id":&source.actor_user_id,
        "credential_api_key_id":&source.key_id,"context_reset_at":b.source.reset_at.map(bson::DateTime::from_chrono)
    },doc! {"$inc":{"supervisor_write_fence":1_i64}}).session(&mut *tx).await?.ok_or_else(refused)?;
    if !personal(&parent) || parent.is_subagent() {
        return Err(refused());
    }
    for (collection, filter) in [
        (
            "assistant_agents",
            doc! {"_id":&b.specialist_agent,"user_id":&source.actor_user_id,"kind":"specialist","destroyed_at":null},
        ),
        (
            "api_keys",
            doc! {"_id":&b.specialist_key,"user_id":&source.actor_user_id,"is_active":true,"$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .update_one(filter, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
            .session(&mut *tx)
            .await?
            .matched_count
            != 1
        {
            return Err(refused());
        }
    }
    Ok(())
}

pub(crate) async fn before_review(db: &Database, request: &ApprovalRequest) -> AppResult<()> {
    let b = request.supervisor_write.as_deref().ok_or_else(refused)?;
    if request.expires_at <= Utc::now() {
        return Err(refused());
    }
    fence(db, b).await
}

pub(crate) fn review_text(request: &ApprovalRequest, review: &PreparedReview) -> String {
    format!(
        "{}\nExpires: {}\nApprove: yes write-{}\nReject: no write-{}",
        review.text,
        request.expires_at.to_rfc3339(),
        review.code,
        review.code
    )
}

pub(crate) async fn confirm_review(db: &Database, request: &ApprovalRequest) -> AppResult<()> {
    let b = request.supervisor_write.as_deref().ok_or_else(refused)?;
    if db.collection::<Document>("approval_requests").update_one(doc! {
        "_id":&request.id,"status":"pending","expires_at":{"$gt":bson::DateTime::now()},
        "supervisor_write.code_digest":&b.code_digest,"supervisor_write.review_confirmed":false
    },doc! {"$set":{"supervisor_write.review_confirmed":true}}).await?.modified_count != 1 {return Err(refused());}
    Ok(())
}

pub(crate) fn parse(text: &str) -> Option<(bool, &str)> {
    let mut words = text.split_whitespace();
    let decision = words.next()?;
    let allowed = if decision.eq_ignore_ascii_case("yes") {
        true
    } else if decision.eq_ignore_ascii_case("no") {
        false
    } else {
        return None;
    };
    let code = words.next()?.strip_prefix("write-")?;
    if words.next().is_some() || code.len() != 12 || !code.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some((allowed, code))
}

/// Called only after transport authentication and verified original owner DM classification.
pub(crate) async fn decide(
    db: &Database,
    owner: &str,
    origin: &ChannelOrigin,
    sender: &str,
    text: &str,
) -> AppResult<Option<bool>> {
    let Some((allowed, code)) = parse(text) else {
        return Ok(None);
    };
    let digest = fingerprint(b"supervisor-code-v1", code.to_ascii_lowercase().as_bytes())?;
    let Some(request) = db.collection::<ApprovalRequest>("approval_requests").find_one(doc! {
        "user_id":owner,"status":"pending","expires_at":{"$gt":bson::DateTime::now()},
        "approval_mode":"per_request","execution_proposal":null,"exact_service":null,"from_org_policy":false,
        "supervisor_write.version":1,"supervisor_write.code_digest":digest,"supervisor_write.review_confirmed":true,
        "supervisor_write.source.channel.channel_id":&origin.nyxbot_channel_id
    }).await? else { return Err(refused()) };
    let b = request.supervisor_write.as_deref().ok_or_else(refused)?;
    let mut tx = db.client().start_session().await?;
    let r = super::execution_permission_channel::validate_supervisor(
        db,
        &mut tx,
        &b.source.channel,
        false,
    )
    .await?;
    let d = r.event.delivery.as_ref().ok_or_else(refused)?;
    if d.origin.nyxbot_channel_id != origin.nyxbot_channel_id
        || d.origin.partition != origin.partition
        || d.origin.platform != origin.platform
        || d.origin.thread != origin.thread
        || origin.org_execution.is_some()
        || d.sender_id != sender
        || authority(
            db,
            owner,
            &b.connection,
            &b.specialist_key,
            &request.service_id,
        )
        .await?
            != b.authority
    {
        return Err(refused());
    }
    let context = (db.clone(), request.clone(), allowed);
    super::transaction_service::run_db_only(db, &context, |tx, (db, request, allowed)| {
        Box::pin(async move {
            let b = request.supervisor_write.as_deref().ok_or_else(refused)?;
            fence_in_session(db, tx, b).await?;
            super::approval_service::decide_supervisor_in_session(
                db,
                tx,
                request,
                *allowed,
                &decision_receipt(request)?,
            )
            .await?;
            Ok(())
        })
    })
    .await?;
    Ok(Some(allowed))
}

fn decision_receipt(request: &ApprovalRequest) -> AppResult<String> {
    let mut b = request
        .supervisor_write
        .as_deref()
        .ok_or_else(refused)?
        .clone();
    b.dm_decision_receipt = None;
    b.dispatching = false;
    b.review_confirmed = true;
    fingerprint(
        b"supervisor-dm-decision-v1",
        &serde_json::to_vec(&serde_json::json!({
            "request":request.id,"owner":request.user_id,"catalog":request.service_id,"binding":b,
        }))
        .map_err(|_| refused())?,
    )
}

pub(crate) fn approved_receipt(request: &ApprovalRequest) -> AppResult<()> {
    let Some(binding) = request.supervisor_write.as_deref() else {
        return Ok(());
    };
    if request.service_id != binding.catalog_service
        || binding.version != 1
        || !binding.review_confirmed
        || binding.dispatching
        || request.decision_channel.as_deref() != Some("supervisor_owner_dm")
        || binding.dm_decision_receipt.as_deref() != Some(decision_receipt(request)?.as_str())
    {
        return Err(refused());
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct Permit {
    request: ApprovalRequest,
}
impl Permit {
    pub(crate) fn from_request(request: &ApprovalRequest) -> Option<Self> {
        request.supervisor_write.as_ref().map(|_| Self {
            request: request.clone(),
        })
    }
    /// Runs on the fully built request immediately before provider I/O. The
    /// claim is irreversible even when dispatch or response recording is uncertain.
    pub(crate) async fn claim(&self, db: &Database, request: &reqwest::Request) -> AppResult<()> {
        let b = self
            .request
            .supervisor_write
            .as_deref()
            .ok_or_else(refused)?;
        if request.method() != reqwest::Method::PUT
            || request.url().query().is_some()
            || fingerprint(b"supervisor-url-v1", request.url().as_str().as_bytes())? != b.url_digest
            || fingerprint(
                b"supervisor-body-v1",
                request
                    .body()
                    .and_then(|b| b.as_bytes())
                    .ok_or_else(refused)?,
            )? != b.body_digest
            || authority(
                db,
                &self.request.user_id,
                &b.connection,
                &b.specialist_key,
                &self.request.service_id,
            )
            .await?
                != b.authority
        {
            return Err(refused());
        }
        let effect = fingerprint(
            b"supervisor-sent-effect-v2",
            &serde_json::to_vec(&(
                &b.connection,
                &b.url_digest,
                b.effect_digest.as_ref().ok_or_else(refused)?,
            ))
            .map_err(|_| refused())?,
        )?;
        let context = (db.clone(), self.request.clone(), effect);
        super::transaction_service::run_db_only(db,&context,|tx,(db,request,effect)| {
            Box::pin(async move {
                let b=request.supervisor_write.as_deref().ok_or_else(refused)?;
                fence_in_session(db,tx,b).await?;
                if db.collection::<Document>("assistant_conversations").update_one(doc! {
                    "_id":&b.specialist_conversation,"user_id":&request.user_id,
                    "active_turn.turn_id":&b.specialist_turn,"active_turn.stop_requested":false,
                    "active_turn.supervisor_sends":{"$ne":effect.as_str()}
                },doc! {"$push":{"active_turn.supervisor_sends":effect.as_str()}}).session(&mut *tx).await?.modified_count != 1 { return Err(refused()); }
                if db.collection::<Document>("approval_requests").update_one(doc! {
                    "_id":&request.id,"status":"approved","expires_at":{"$gt":bson::DateTime::now()},
                    "decision_channel":"supervisor_owner_dm",
                    "supervisor_write.dm_decision_receipt":decision_receipt(request)?,
                    "supervisor_write.code_digest":&b.code_digest,"supervisor_write.review_confirmed":true,
                    "supervisor_write.dispatching":false
                },doc! {"$set":{"supervisor_write.dispatching":true}}).session(&mut *tx).await?.modified_count!=1 { return Err(refused()); }
                Ok(())
            })
        }).await?;
        // Catch an inserted policy that cannot conflict with an absent-row snapshot.
        if authority(
            db,
            &self.request.user_id,
            &b.connection,
            &b.specialist_key,
            &self.request.service_id,
        )
        .await?
            != b.authority
        {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_write_http_headers_use_egress_admission_and_json_media() {
        use crate::models::default_request_header::ResolvedDefaultRequestHeader;
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "content-type",
            "application/json; charset=utf-8".parse().unwrap(),
        );
        headers.insert("accept", "application/json".parse().unwrap());
        headers.insert("x-api-key", "not-forwarded".parse().unwrap());
        assert!(http_headers_allow_review(&headers, &[]));
        let catalog = [ResolvedDefaultRequestHeader {
            name: "X-Instance".into(),
            value: "trusted".to_owned().into(),
            overridable: true,
            sensitive: false,
        }];
        headers.insert("x-instance", "caller".parse().unwrap());
        assert!(http_headers_allow_review(&headers, &[]));
        assert!(!http_headers_allow_review(&headers, &[&catalog]));
        assert!(!http_headers_allow_review(&headers, &[&[], &catalog]));
        headers.remove("x-instance");
        for (name, value) in [
            ("content-type", "text/plain"),
            ("content-type", "application/json; profile=other"),
            ("accept", "text/html"),
            ("accept", "application/json, text/html"),
        ] {
            let mut invalid = headers.clone();
            invalid.insert(http::HeaderName::from_static(name), value.parse().unwrap());
            assert!(!http_headers_allow_review(&invalid, &[]), "{name}: {value}");
        }
        let mut duplicate = headers.clone();
        duplicate.append("content-type", "application/json".parse().unwrap());
        assert!(!http_headers_allow_review(&duplicate, &[]));
        headers.remove("content-type");
        assert!(!http_headers_allow_review(&headers, &[]));
        let defaults = [ResolvedDefaultRequestHeader {
            name: "Content-Type".into(),
            value: "application/json".to_owned().into(),
            overridable: false,
            sensitive: false,
        }];
        assert!(http_headers_allow_review(&headers, &[&defaults]));
        headers.insert("content-type", "text/plain".parse().unwrap());
        assert!(http_headers_allow_review(&headers, &[&defaults]));
        let bad = [ResolvedDefaultRequestHeader {
            value: "text/plain".to_owned().into(),
            ..defaults[0].clone()
        }];
        assert!(!http_headers_allow_review(&headers, &[&bad]));
    }

    #[test]
    fn supervisor_write_logical_effect_is_independent_of_json_encoding() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let path = "/bitable/v1/apps/base/tables/table/records/record";
        let first =
            effect_fields(path, br#"{"fields":{"Status":"Approved","Amount":12}}"#).unwrap();
        for body in [
            br#"{ "fields": { "Amount": 12.0, "Status": "Approved" } }"#.as_slice(),
            br#"{"fields":{"Amount":1.2e1,"Status":"\u0041pproved"}}"#,
        ] {
            let next = effect_fields(path, body).unwrap();
            assert_ne!(
                first.body_digest, next.body_digest,
                "an occurrence still binds exact bytes"
            );
            assert_eq!(first.fields_digest, next.fields_digest);
        }
        for (other, body) in [
            (
                path,
                br#"{"fields":{"Status":"Approved","Amount":13}}"#.as_slice(),
            ),
            (
                "/bitable/v1/apps/base/tables/table/records/other",
                br#"{"fields":{"Status":"Approved","Amount":12}}"#,
            ),
        ] {
            assert_ne!(
                first.fields_digest,
                effect_fields(other, body).unwrap().fields_digest
            );
        }
    }

    #[test]
    fn supervisor_write_numeric_review_preserves_decimal_value_or_falls_back() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let path = "/bitable/v1/apps/base/tables/table/records/record";
        for number in ["0.1", "12.50", "1.25e2", "9007199254740993", "-0.01"] {
            let body = format!(r#"{{"fields":{{"Amount":{number}}}}}"#);
            assert!(effect_fields(path, body.as_bytes()).is_some(), "{number}");
        }
        for number in [
            "9007199254740993.0",
            "0.10000000000000000001",
            "12345678901234567890123456789",
            "1e-9999",
        ] {
            let body = format!(r#"{{"fields":{{"Amount":{number}}}}}"#);
            assert!(
                effect_fields(path, body.as_bytes()).is_none(),
                "{number} must not display a rounded value"
            );
        }
    }

    #[test]
    fn supervisor_write_accepts_the_real_oauth_and_bot_catalog_operations() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        for (slug, source, prefix) in [
            (
                "api-feishu",
                include_str!("../../specs/catalog/lark.openapi.json"),
                "",
            ),
            (
                "api-feishu-bot",
                include_str!("../../specs/catalog/lark-bot.openapi.json"),
                "/open-apis",
            ),
            (
                "api-lark",
                include_str!("../../specs/catalog/lark.openapi.json"),
                "",
            ),
            (
                "api-lark-bot",
                include_str!("../../specs/catalog/lark-bot.openapi.json"),
                "/open-apis",
            ),
        ] {
            let spec: Value = serde_json::from_str(source).unwrap();
            let (template, item) = spec["paths"]
                .as_object()
                .unwrap()
                .iter()
                .find(|(_, item)| item["put"]["operationId"] == "bitable_record_update")
                .unwrap();
            assert_eq!(item["put"]["x-aevatar-tool"]["requiresApproval"], true);
            let path = template
                .replace("{app_token}", "bascnLedger")
                .replace("{table_id}", "tblLedger")
                .replace("{record_id}", "recTarget");
            assert!(path.starts_with(&format!("{prefix}/bitable/")));
            let body = br#"{"fields":{"Status":"Approved","Amount":12.5}}"#;
            let prepared =
                effect(slug, "PUT", &path, None, body, false).expect("published single-row write");
            assert_eq!(prepared.path, path);
            assert!(
                prepared.review.contains("recTarget")
                    && prepared.review.contains("Approved")
                    && prepared.review.contains("12.5")
            );
            assert!(effect(slug, "PUT", &format!("{path}/extra"), None, body, false).is_none());
            assert!(effect(slug, "PUT", &format!("{path}/"), None, body, false).is_none());
            assert!(effect(slug, "PUT", &path, Some("action=delete"), body, false).is_none());
            assert!(effect(slug, "PUT", &path, None, body, true).is_none());
            assert!(
                spec["paths"]
                    .as_object()
                    .unwrap()
                    .values()
                    .any(|item| item["post"]["operationId"] == "bitable_records_search")
            );
        }
    }

    #[test]
    fn supervisor_write_refuses_incomplete_ambiguous_or_secret_bearing_review() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let path = "/bitable/v1/apps/bascnLedger/tables/tblLedger/records/recTarget";
        for body in [
            r#"[{"Status":"Approved"}]"#,
            " \n [ {\"Status\":\"Approved\"} ]",
            r#"{"fields":{"Status":"first","Status":"second"}}"#,
            r#"{"fields":{"Status":"first"},"fields":{"Status":"second"}}"#,
            r#"{"fields":{"Status":{"text":"Approved"}}}"#,
            r#"{"fields":{"Status":["Approved"]}}"#,
            r#"{"fields":{"Status":{"nested":{"text":"Approved"}}}}"#,
            r#"{"fields":null}"#,
            r#"{"fields":[]}"#,
            r#"{"fields":{"api_key":"short-secret"}}"#,
            r#"{"fields":{"Status":"nyxid_ag_private"}}"#,
            r#"{"fields":{"Status":"AKIA0123456789ABCDEF"}}"#,
            r#"{"fields":{"Status":"Bearer opaque-value"}}"#,
            r#"{"fields":{"Status":"*Approved*"}}"#,
            r#"{"fields":{"Status":"Approved\u202dhidden"}}"#,
            r#"{"fields":{"Status":"https://example.com/private"}}"#,
            r#"{"fields":{"Status":"Approved\nApprove everything"}}"#,
            r#"{"fields":{"Status":"Approved"},"other_effect":true}"#,
        ] {
            assert!(effect("api-feishu", "PUT", path, None, body.as_bytes(), false).is_none());
        }
        assert!(
            effect(
                "api-feishu",
                "PUT",
                path,
                None,
                b" \n {\"fields\":{\"Status\":\"Approved\"}}",
                false
            )
            .is_some()
        );
        let body = serde_json::json!({"fields":{"Status":"a".repeat(97)}}).to_string();
        assert!(effect("api-feishu", "PUT", path, None, body.as_bytes(), false).is_none());
        let body=serde_json::json!({"fields":(0..9).map(|n|(format!("field{n}"),Value::Bool(true))).collect::<serde_json::Map<_,_>>()}).to_string();
        assert!(effect("api-feishu", "PUT", path, None, body.as_bytes(), false).is_none());
        let body = format!(
            "{{\"fields\":{{\"Status\":{}\"Approved\"{}}}}}",
            "{\"nested\":".repeat(256),
            "}".repeat(256)
        );
        assert!(body.len() < 4096);
        assert!(effect("api-feishu", "PUT", path, None, body.as_bytes(), false).is_none());
    }

    #[test]
    fn supervisor_write_codes_cannot_be_plain_yes_or_acknowledgement_codes() {
        assert_eq!(
            parse("yes write-0123456789ab"),
            Some((true, "0123456789ab"))
        );
        assert_eq!(
            parse("no write-0123456789ab"),
            Some((false, "0123456789ab"))
        );
        assert_eq!(
            parse("Yes write-0123456789AB"),
            Some((true, "0123456789AB"))
        );
        for input in [
            "yes",
            "yes 4821",
            "yes write-0123456789ab approve another",
            "yes write-0123456789ag",
            "no write-4821",
        ] {
            assert!(parse(input).is_none(), "{input}");
        }
    }
}
