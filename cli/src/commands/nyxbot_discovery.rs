use super::{ApiClient, ApiError, AuthArgs, CommandFailure, ROOT, emit_discovery, read_discovery};
use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const CURSOR_PREFIX: &str = "org-guest-v1.";
const HANDLE_PREFIX: &str = "org-guest-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    EndpointOrGuestMissing,
    SpecialistUse,
}

pub(super) fn refusal(error: &ApiError) -> Option<Refusal> {
    let envelope = error.response();
    match (
        error.status().as_u16(),
        envelope.map(|value| value.message.as_str()),
    ) {
        (404, Some("Agent not found"))
        | (
            403,
            Some(
                "Active organization membership with service access is required to use this agent",
            ),
        ) => Some(Refusal::SpecialistUse),
        (404, None | Some("Guest conversation not found" | "Assistant route not found.")) => {
            Some(Refusal::EndpointOrGuestMissing)
        }
        _ => None,
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Position {
    Start,
    After(String),
    End,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    agent: String,
    owner: Position,
    guest: Position,
}

impl Cursor {
    fn decode(token: &str, agent: &str) -> Result<Self> {
        if token.len() > 4096 {
            bail!("Invalid conversation cursor");
        }
        let bytes = URL_SAFE_NO_PAD.decode(token)?;
        let cursor: Self = serde_json::from_slice(&bytes)?;
        if cursor.agent != agent {
            bail!("The conversation cursor belongs to another agent");
        }
        Ok(cursor)
    }

    fn next(self) -> Result<Option<String>> {
        if matches!((&self.owner, &self.guest), (Position::End, Position::End)) {
            return Ok(None);
        }
        Ok(Some(format!(
            "{CURSOR_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&self)?)
        )))
    }
}

fn owner_path(agent: Option<&str>, cursor: Option<&str>, limit: u32) -> String {
    let mut path = format!("{ROOT}/conversations?limit={limit}");
    if let Some(agent) = agent {
        path.push_str(&format!("&agent_id={}", urlencoding::encode(agent)));
    }
    append_cursor(&mut path, cursor);
    path
}

fn guest_path(agent: &str, cursor: Option<&str>, limit: u32) -> String {
    let mut path = format!(
        "{ROOT}/agents/{}/guest-conversations?limit={limit}",
        urlencoding::encode(agent)
    );
    append_cursor(&mut path, cursor);
    path
}

fn append_cursor(path: &mut String, cursor: Option<&str>) {
    if let Some(cursor) = cursor {
        path.push_str(&format!("&cursor={}", urlencoding::encode(cursor)));
    }
}

pub(super) fn history_path(conversation: &str, limit: u32) -> String {
    let resource = if conversation.starts_with(HANDLE_PREFIX) {
        "guest-conversations"
    } else {
        "conversations"
    };
    format!(
        "{ROOT}/{resource}/{}?limit={limit}",
        urlencoding::encode(conversation)
    )
}

#[derive(Deserialize)]
struct Row {
    id: String,
    last_message_at: DateTime<Utc>,
}

struct Page {
    entries: std::collections::VecDeque<(Row, Value)>,
    next_cursor: Option<String>,
}

impl Page {
    fn parse(value: Value) -> Result<Self> {
        #[derive(Deserialize)]
        struct Index {
            conversations: Vec<Value>,
            next_cursor: Option<String>,
        }
        let index: Index = serde_json::from_value(value)?;
        let entries = index
            .conversations
            .into_iter()
            .map(|value| Ok((serde_json::from_value(value.clone())?, value)))
            .collect::<Result<_>>()?;
        Ok(Self {
            entries,
            next_cursor: index.next_cursor,
        })
    }

    fn after(&self, start: Position, last: Option<Row>) -> Position {
        if self.entries.is_empty() {
            return self
                .next_cursor
                .clone()
                .map_or(Position::End, Position::After);
        }
        last.map_or(start, |row| {
            Position::After(format!(
                "{}:{}",
                row.last_message_at.timestamp_millis(),
                row.id
            ))
        })
    }
}

fn merge(owner: Value, guest: Value, mut cursor: Cursor, limit: u32) -> Result<Value> {
    let mut owner = Page::parse(owner)?;
    let mut guest = Page::parse(guest)?;
    let mut rows = Vec::new();
    let mut owner_last = None;
    let mut guest_last = None;
    while rows.len() < limit as usize {
        let choose_guest = match (owner.entries.front(), guest.entries.front()) {
            (None, None) => break,
            (None, Some(_)) => true,
            (Some(_), None) => false,
            (Some((owner, _)), Some((guest, _))) => {
                // Both servers break equal timestamps using the underlying conversation UUID.
                (
                    guest.last_message_at,
                    guest.id.trim_start_matches(HANDLE_PREFIX),
                ) > (owner.last_message_at, owner.id.trim_start_matches("nyxa-"))
            }
        };
        let (row, value) = if choose_guest {
            let (row, value) = guest.entries.pop_front().expect("selected guest row");
            (&mut guest_last, (row, value))
        } else {
            let (row, value) = owner.entries.pop_front().expect("selected owner row");
            (&mut owner_last, (row, value))
        };
        *row = Some(value.0);
        rows.push(value.1);
    }
    cursor.owner = owner.after(cursor.owner, owner_last);
    cursor.guest = guest.after(cursor.guest, guest_last);
    Ok(json!({"conversations": rows, "next_cursor": cursor.next()?}))
}

async fn page(
    api: &mut ApiClient,
    position: &Position,
    path: impl FnOnce(Option<&str>) -> String,
) -> Result<Option<Value>, CommandFailure> {
    let cursor = match position {
        Position::Start => None,
        Position::After(cursor) => Some(cursor.as_str()),
        Position::End => return Ok(None),
    };
    read_discovery(api, &path(cursor)).await.map(Some)
}

pub(super) async fn list(
    auth: AuthArgs,
    agent: Option<String>,
    cursor: Option<String>,
    limit: u32,
) -> Result<()> {
    let mut api = ApiClient::from_auth(&auth)?;
    let Some(agent) = agent.as_deref() else {
        if cursor
            .as_deref()
            .is_some_and(|cursor| cursor.starts_with(CURSOR_PREFIX))
        {
            bail!("A guest conversation cursor requires the original --agent");
        }
        return emit_discovery(
            read_discovery(&mut api, &owner_path(None, cursor.as_deref(), limit)).await,
            auth.output,
        );
    };
    let initial = cursor.is_none();
    let cursor = match cursor.as_deref() {
        Some(cursor) => match cursor.strip_prefix(CURSOR_PREFIX) {
            Some(token) => Cursor::decode(token, agent)?,
            None => {
                return emit_discovery(
                    read_discovery(&mut api, &owner_path(Some(agent), Some(cursor), limit)).await,
                    auth.output,
                );
            }
        },
        None => Cursor {
            agent: agent.to_owned(),
            owner: Position::Start,
            guest: Position::Start,
        },
    };
    let owner = page(&mut api, &cursor.owner, |cursor| {
        owner_path(Some(agent), cursor, limit)
    })
    .await;
    if let Err(failure) = owner.as_ref()
        && failure.discovery_refusal != Some(Refusal::SpecialistUse)
    {
        return emit_discovery(owner.map(|_| Value::Null), auth.output);
    }
    let guest = page(&mut api, &cursor.guest, |cursor| {
        guest_path(agent, cursor, limit)
    })
    .await;
    let guest = match guest {
        Ok(value) => value,
        Err(failure)
            if initial
                && owner.is_ok()
                && failure.discovery_refusal == Some(Refusal::EndpointOrGuestMissing) =>
        {
            return emit_discovery(
                owner.map(|value| value.expect("initial owner page")),
                auth.output,
            );
        }
        Err(failure)
            if owner.is_err()
                && failure.discovery_refusal == Some(Refusal::EndpointOrGuestMissing) =>
        {
            return emit_discovery(owner.map(|_| Value::Null), auth.output);
        }
        Err(failure) => return emit_discovery(Err(failure), auth.output),
    };
    if initial
        && owner.is_ok()
        && guest.as_ref().is_some_and(|page| {
            page["next_cursor"].is_null()
                && page["conversations"].as_array().is_some_and(Vec::is_empty)
        })
    {
        return emit_discovery(
            owner.map(|value| value.expect("initial owner page")),
            auth.output,
        );
    }
    let empty = || json!({"conversations": [], "next_cursor": null});
    let result = merge(
        owner.ok().flatten().unwrap_or_else(empty),
        guest.unwrap_or_else(empty),
        cursor,
        limit,
    )?;
    emit_discovery(Ok(result), auth.output)
}
