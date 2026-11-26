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
        let bytes = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|_| anyhow::anyhow!("Invalid conversation cursor"))?;
        let cursor: Self = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid conversation cursor"))?;
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
        let (page, last) = if choose_guest {
            (&mut guest, &mut guest_last)
        } else {
            (&mut owner, &mut owner_last)
        };
        let (row, value) = page.entries.pop_front().expect("selected conversation");
        *last = Some(row);
        rows.push(value);
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
    emit_discovery(
        discover(&mut api, agent.as_deref(), cursor.as_deref(), limit).await?,
        auth.output,
    )
}

async fn discover(
    api: &mut ApiClient,
    agent: Option<&str>,
    cursor: Option<&str>,
    limit: u32,
) -> Result<Result<Value, CommandFailure>> {
    let Some(agent) = agent else {
        if cursor.is_some_and(|cursor| cursor.starts_with(CURSOR_PREFIX)) {
            bail!("A guest conversation cursor requires the original --agent");
        }
        return Ok(read_discovery(api, &owner_path(None, cursor, limit)).await);
    };
    let initial = cursor.is_none();
    let cursor = match cursor {
        Some(cursor) => match cursor.strip_prefix(CURSOR_PREFIX) {
            Some(token) => Cursor::decode(token, agent)?,
            None => {
                return Ok(
                    read_discovery(api, &owner_path(Some(agent), Some(cursor), limit)).await,
                );
            }
        },
        None => Cursor {
            agent: agent.to_owned(),
            owner: Position::Start,
            guest: Position::Start,
        },
    };
    let owner = page(api, &cursor.owner, |cursor| {
        owner_path(Some(agent), cursor, limit)
    })
    .await;
    if let Err(failure) = owner.as_ref()
        && failure.discovery_refusal != Some(Refusal::SpecialistUse)
    {
        return Ok(owner.map(|_| Value::Null));
    }
    let guest = page(api, &cursor.guest, |cursor| {
        guest_path(agent, cursor, limit)
    })
    .await;
    let guest = match guest {
        Ok(value) => value,
        Err(failure)
            if owner.as_ref().is_ok_and(Option::is_some)
                && failure.discovery_refusal == Some(Refusal::EndpointOrGuestMissing) =>
        {
            return Ok(owner.map(|value| value.expect("available owner page")));
        }
        Err(failure)
            if owner.is_err()
                && failure.discovery_refusal == Some(Refusal::EndpointOrGuestMissing) =>
        {
            return Ok(owner.map(|_| Value::Null));
        }
        Err(failure) => return Ok(Err(failure)),
    };
    if initial
        && owner.is_ok()
        && guest.as_ref().is_some_and(|page| {
            page["next_cursor"].is_null()
                && page["conversations"].as_array().is_some_and(Vec::is_empty)
        })
    {
        return Ok(owner.map(|value| value.expect("initial owner page")));
    }
    let empty = || json!({"conversations": [], "next_cursor": null});
    let result = merge(
        owner.ok().flatten().unwrap_or_else(empty),
        guest.unwrap_or_else(empty),
        cursor,
        limit,
    )?;
    Ok(Ok(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    type Reply = (String, &'static str, Option<Value>);

    async fn fixture(replies: Vec<Reply>) -> (ApiClient, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            for (path, status, value) in replies {
                let (socket, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(socket);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let expected = format!("/api/v1{path}");
                assert_eq!(
                    line.split_whitespace().take(2).collect::<Vec<_>>(),
                    ["GET", expected.as_str()]
                );
                loop {
                    line.clear();
                    if reader.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
                let body = value.as_ref().map(Value::to_string).unwrap_or_default();
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
        });
        (
            ApiClient::new(&format!("http://{address}"), "test-fixture".into())
                .unwrap()
                .without_token_refresh(),
            task,
        )
    }

    fn index(rows: &[(&str, u8)], next: Option<&str>) -> Value {
        let rows: Vec<_> = rows.iter().map(|(id, minute)| {
            json!({"id": id, "last_message_at": format!("2026-01-01T00:{minute:02}:00Z"), "read_only": id.starts_with(HANDLE_PREFIX)})
        }).collect();
        json!({"conversations": rows, "next_cursor": next})
    }

    #[tokio::test]
    async fn scoped_guest_discovery_pages_both_sources_within_the_total_limit() {
        let owner_a = format!("nyxa-{}", "a".repeat(32));
        let owner_b = format!("nyxa-{}", "b".repeat(32));
        let guest_a = format!("org-guest-{}", "a".repeat(32));
        let guest_b = format!("org-guest-{}", "b".repeat(32));
        let partial_guest = format!("1767225780000:{guest_a}");
        let (mut api, server) = fixture(vec![
            (
                owner_path(Some("agent"), None, 2),
                "200 OK",
                Some(index(&[(&owner_a, 4), (&owner_b, 1)], None)),
            ),
            (
                guest_path("agent", None, 2),
                "200 OK",
                Some(index(&[(&guest_a, 3), (&guest_b, 2)], None)),
            ),
            (
                owner_path(Some("agent"), Some(&format!("1767225840000:{owner_a}")), 2),
                "200 OK",
                Some(index(&[(&owner_b, 1)], None)),
            ),
            (
                guest_path("agent", Some(&partial_guest), 2),
                "200 OK",
                Some(index(&[(&guest_b, 2)], None)),
            ),
        ])
        .await;
        let first = discover(&mut api, Some("agent"), None, 2)
            .await
            .unwrap()
            .ok()
            .unwrap();
        assert_eq!(first["conversations"][0]["id"], owner_a);
        assert_eq!(first["conversations"][1]["id"], guest_a);
        let cursor = first["next_cursor"].as_str().unwrap();
        assert!(
            discover(&mut api, Some("other"), Some(cursor), 2)
                .await
                .is_err()
        );
        let second = discover(&mut api, Some("agent"), Some(cursor), 2)
            .await
            .unwrap()
            .ok()
            .unwrap();
        assert_eq!(second["conversations"][0]["id"], guest_b);
        assert_eq!(second["conversations"][1]["id"], owner_b);
        assert_eq!(second["next_cursor"], Value::Null);
        assert!(history_path(&guest_b, 20).contains("/guest-conversations/"));
        assert!(!history_path(&owner_b, 20).contains("/guest-conversations/"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn scoped_guest_discovery_preserves_old_servers_and_authority_failures() {
        let owner = json!({"conversations": [], "next_cursor": null, "extension": "preserve"});
        let guest = index(&[("org-guest-a", 1)], None);
        let denial = json!({"error":"not_found","error_code":1003,"message":"Agent not found"});
        let owner_path = owner_path(Some("agent"), None, 2);
        let guest_path = guest_path("agent", None, 2);
        for (first, second, expected) in [
            (
                ("200 OK", Some(owner.clone())),
                ("404 Not Found", None),
                Some(owner.clone()),
            ),
            (
                ("404 Not Found", Some(denial.clone())),
                ("200 OK", Some(guest.clone())),
                Some(guest),
            ),
            (
                ("200 OK", Some(owner.clone())),
                ("503 Service Unavailable", None),
                None,
            ),
            (("200 OK", Some(owner)), ("401 Unauthorized", None), None),
        ] {
            let (mut api, server) = fixture(vec![
                (owner_path.clone(), first.0, first.1),
                (guest_path.clone(), second.0, second.1),
            ])
            .await;
            let result = discover(&mut api, Some("agent"), None, 2).await.unwrap();
            if let Some(expected) = expected {
                assert_eq!(result.ok().unwrap(), expected);
            } else {
                assert_eq!(
                    result.err().unwrap().http_status,
                    second.0[..3].parse().ok()
                );
            }
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn resumes_the_ordinary_cursor_when_guest_history_ends_or_disappears() {
        let owner_id = format!("nyxa-{}", "c".repeat(32));
        let resume_owner = format!("1767225780000:{owner_id}");
        let resume_guest = format!("1767225840000:org-guest-{}", "d".repeat(32));
        let combined = Cursor {
            agent: "agent".into(),
            owner: Position::After(resume_owner.clone()),
            guest: Position::After(resume_guest.clone()),
        }
        .next()
        .unwrap()
        .unwrap();
        let owner = index(&[(&owner_id, 1)], Some("ordinary-next-cursor"));
        let (mut api, server) = fixture(vec![
            (
                owner_path(Some("agent"), Some(&resume_owner), 1),
                "200 OK",
                Some(owner.clone()),
            ),
            (
                guest_path("agent", Some(&resume_guest), 1),
                "404 Not Found",
                None,
            ),
            (
                owner_path(Some("agent"), Some("ordinary-next-cursor"), 1),
                "400 Bad Request",
                Some(json!({"error":"bad_request","error_code":1000,"message":"Invalid cursor"})),
            ),
        ])
        .await;
        let recovered = discover(&mut api, Some("agent"), Some(&combined), 1)
            .await
            .unwrap()
            .ok()
            .unwrap();
        assert_eq!(recovered, owner);
        let result = discover(&mut api, Some("agent"), Some("ordinary-next-cursor"), 1)
            .await
            .unwrap();
        assert_eq!(result.err().unwrap().http_status, Some(400));
        assert!(
            discover(&mut api, Some("agent"), Some("org-guest-v1.!"), 1)
                .await
                .is_err()
        );
        server.await.unwrap();
    }
}
