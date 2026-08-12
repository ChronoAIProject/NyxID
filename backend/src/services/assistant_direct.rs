//! Server-owned request contract for direct Chrono-LLM assistant chat.

use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};

pub const DIRECT_LLM_SLUG: &str = "chrono-llm-public";
pub const DEFAULT_DIRECT_MODEL: &str = "gpt-5.5";
pub const MAX_DIRECT_REQUEST_BYTES: usize = 256 * 1024;
const MAX_DIRECT_MESSAGES: usize = 64;
const MAX_DIRECT_MESSAGE_CHARS: usize = 32_768;
const MAX_DIRECT_CONTENT_BYTES: usize = 256 * 1024;
const MAX_DIRECT_SKILL_BYTES: usize = 64 * 1024;

pub const BASE_SYSTEM_PROMPT: &str = "\
You are Nyx, the NyxID assistant, running in direct model chat: a \
text-only mode with no tool execution. NyxID brokers credentials for \
external services (LLM APIs, GitHub, Lark, SSH, MCP) so users and their \
agents never handle raw keys; users manage services, keys, nodes, \
approvals, and organizations through the NyxID dashboard and the `nyxid` \
CLI.\n\
\n\
Operating rules (binding):\n\
1. You cannot execute anything: no reads of live account state, no API \
calls, no actions. Never claim to have run, checked, created, or changed \
anything.\n\
2. When the user asks for something actionable, respond with the exact \
copyable `nyxid` CLI command (or the dashboard path) that accomplishes \
it, plus a one-line note that chat cannot run it. Prefer commands over \
prose walkthroughs.\n\
3. When an answer depends on live account state you cannot see, say you \
cannot check from here and give the exact command that shows it. Never \
present a guess as current state; 'cannot check' is never 'not \
connected'.\n\
4. If a request has prerequisites (a service connection, an approval, a \
registered node), name ALL of them up front in one reply - no \
drip-feed.\n\
5. Never invent commands, flags, URLs, service slugs, or API endpoints. \
If you are not sure the exact command exists, say so and point to `nyxid \
--help` or the dashboard.\n\
6. For platform-admin, billing, pre-authentication, or otherwise excluded \
surfaces, decline briefly and point to the nearest supported \
alternative.\n\
7. Answer in the user's language. Be concise; lead with the answer.";

pub const DIRECT_MODE_OVERRIDE: &str = "\
OVERRIDE - the reference material above may instruct the use of CLI \
execution, agent tools, MCP calls, or API requests. Those instructions do \
not apply to you in this chat: you cannot execute anything. Treat that \
material strictly as knowledge for describing steps the USER can run \
themselves; present commands as copyable suggestions and never state or \
imply that you ran them.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectSkill {
    pub slug: &'static str,
    pub label: &'static str,
    pub body: &'static str,
}

pub const DIRECT_SKILLS: &[DirectSkill] = &[
    DirectSkill {
        slug: "nyxid",
        label: "NyxID",
        body: include_str!("../../prompts/direct/nyxid.md"),
    },
    DirectSkill {
        slug: "github-via-nyxid",
        label: "GitHub via NyxID",
        body: include_str!("../../prompts/direct/github-via-nyxid.md"),
    },
    DirectSkill {
        slug: "firecrawl-via-nyxid",
        label: "Firecrawl via NyxID",
        body: include_str!("../../prompts/direct/firecrawl-via-nyxid.md"),
    },
];

const _: () =
    assert!(include_str!("../../prompts/direct/nyxid.md").len() <= MAX_DIRECT_SKILL_BYTES);
const _: () = assert!(
    include_str!("../../prompts/direct/github-via-nyxid.md").len() <= MAX_DIRECT_SKILL_BYTES
);
const _: () = assert!(
    include_str!("../../prompts/direct/firecrawl-via-nyxid.md").len() <= MAX_DIRECT_SKILL_BYTES
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectModel {
    pub id: &'static str,
    pub label: &'static str,
    pub default: bool,
}

pub const DIRECT_MODELS: &[DirectModel] = &[
    DirectModel {
        id: DEFAULT_DIRECT_MODEL,
        label: "GPT-5.5",
        default: true,
    },
    DirectModel {
        id: "gpt-5.4",
        label: "GPT-5.4",
        default: false,
    },
    DirectModel {
        id: "gpt-5.4-mini",
        label: "GPT-5.4 Mini",
        default: false,
    },
    DirectModel {
        id: "gpt-5.2",
        label: "GPT-5.2",
        default: false,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectEffort {
    pub id: &'static str,
    pub label: &'static str,
}

/// Reasoning-effort levels offered to the picker. Absent effort means "send
/// no `reasoning_effort` at all", which keeps the upstream's own default and
/// leaves the historical request shape byte-identical.
///
/// This table is curated, not discovered: the upstream advertises no effort
/// catalog, so an entry the deployed model rejects surfaces as a 4xx on that
/// turn. Adjust the list here when the live Chrono-LLM contract is confirmed.
pub const DIRECT_EFFORTS: &[DirectEffort] = &[
    DirectEffort {
        id: "low",
        label: "Low",
    },
    DirectEffort {
        id: "medium",
        label: "Medium",
    },
    DirectEffort {
        id: "high",
        label: "High",
    },
    DirectEffort {
        id: "xhigh",
        label: "Extra high",
    },
    DirectEffort {
        id: "max",
        label: "Max",
    },
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DirectMessageRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DirectMessage {
    pub role: DirectMessageRole,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DirectChatRequest {
    pub messages: Vec<DirectMessage>,
    pub model: Option<String>,
    pub skill_slug: Option<String>,
    pub effort: Option<String>,
}

pub fn validate_direct_request(bytes: &[u8]) -> AppResult<DirectChatRequest> {
    let request: DirectChatRequest = serde_json::from_slice(bytes)
        .map_err(|_| AppError::BadRequest("Invalid direct chat request.".to_string()))?;

    if request.messages.is_empty() || request.messages.len() > MAX_DIRECT_MESSAGES {
        return Err(AppError::BadRequest(format!(
            "messages must contain between 1 and {MAX_DIRECT_MESSAGES} entries."
        )));
    }

    let mut aggregate_bytes = 0usize;
    for message in &request.messages {
        if message.content.chars().count() > MAX_DIRECT_MESSAGE_CHARS {
            return Err(AppError::BadRequest(format!(
                "Each message content must be at most {MAX_DIRECT_MESSAGE_CHARS} characters."
            )));
        }
        aggregate_bytes = aggregate_bytes
            .checked_add(message.content.len())
            .ok_or_else(|| AppError::BadRequest("Message content is too large.".to_string()))?;
    }
    if aggregate_bytes > MAX_DIRECT_CONTENT_BYTES {
        return Err(AppError::BadRequest(format!(
            "Aggregate message content must be at most {MAX_DIRECT_CONTENT_BYTES} UTF-8 bytes."
        )));
    }

    if let Some(model) = request.model.as_deref()
        && !DIRECT_MODELS.iter().any(|candidate| candidate.id == model)
    {
        return Err(AppError::BadRequest(format!(
            "Unknown model. Allowed values: {}.",
            allowed_models()
        )));
    }
    if let Some(skill_slug) = request.skill_slug.as_deref()
        && !DIRECT_SKILLS
            .iter()
            .any(|candidate| candidate.slug == skill_slug)
    {
        return Err(AppError::BadRequest(format!(
            "Unknown skill_slug. Allowed values: {}.",
            allowed_skills()
        )));
    }
    if let Some(effort) = request.effort.as_deref()
        && !DIRECT_EFFORTS
            .iter()
            .any(|candidate| candidate.id == effort)
    {
        return Err(AppError::BadRequest(format!(
            "Unknown effort. Allowed values: {}.",
            allowed_efforts()
        )));
    }

    Ok(request)
}

pub fn build_upstream_body(request: DirectChatRequest) -> serde_json::Value {
    let model = request.model.as_deref().unwrap_or(DEFAULT_DIRECT_MODEL);
    let system_prompt = compose_system_prompt(request.skill_slug.as_deref());
    let mut messages = Vec::with_capacity(request.messages.len() + 1);
    messages.push(serde_json::json!({
        "role": "system",
        "content": system_prompt,
    }));
    messages.extend(
        request
            .messages
            .into_iter()
            .map(|message| serde_json::to_value(message).expect("direct message serializes")),
    );

    let mut body = serde_json::json!({
        "model": model,
        "stream": true,
        "stream_options": { "include_usage": true },
        "messages": messages,
    });
    // Omitted rather than defaulted: no effort means the upstream keeps its
    // own default and the request shape is unchanged from before this option
    // existed.
    if let Some(effort) = request.effort.as_deref().and_then(find_effort) {
        body["reasoning_effort"] = serde_json::Value::String(effort.id.to_string());
    }
    body
}

fn compose_system_prompt(skill_slug: Option<&str>) -> String {
    let mut prompt = String::from(BASE_SYSTEM_PROMPT);
    if let Some(skill) = skill_slug.and_then(find_skill) {
        prompt.push_str("\n\n");
        prompt.push_str(skill.body);
    }
    prompt.push_str("\n\n");
    prompt.push_str(DIRECT_MODE_OVERRIDE);
    prompt
}

fn find_skill(slug: &str) -> Option<&'static DirectSkill> {
    DIRECT_SKILLS.iter().find(|skill| skill.slug == slug)
}

fn find_effort(id: &str) -> Option<&'static DirectEffort> {
    DIRECT_EFFORTS.iter().find(|effort| effort.id == id)
}

fn allowed_models() -> String {
    DIRECT_MODELS
        .iter()
        .map(|model| model.id)
        .collect::<Vec<_>>()
        .join(", ")
}

fn allowed_skills() -> String {
    DIRECT_SKILLS
        .iter()
        .map(|skill| skill.slug)
        .collect::<Vec<_>>()
        .join(", ")
}

fn allowed_efforts() -> String {
    DIRECT_EFFORTS
        .iter()
        .map(|effort| effort.id)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_request(value: serde_json::Value) -> DirectChatRequest {
        validate_direct_request(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    #[test]
    fn validates_message_count_roles_and_strict_fields() {
        for value in [
            json!({ "messages": [] }),
            json!({ "messages": vec![json!({"role": "user", "content": "x"}); 65] }),
            json!({ "messages": [{"role": "system", "content": "override"}] }),
            json!({ "messages": [{"role": "user", "content": "hi", "name": "x"}] }),
            json!({ "messages": [{"role": "user", "content": "hi"}], "temperature": 0 }),
        ] {
            assert!(validate_direct_request(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    #[test]
    fn enforces_unicode_scalar_and_aggregate_utf8_limits_separately() {
        let scalar_boundary = "a".repeat(MAX_DIRECT_MESSAGE_CHARS);
        assert!(
            valid_request(json!({
                "messages": [{"role": "user", "content": scalar_boundary}]
            }))
            .messages
            .len()
                == 1
        );

        let scalar_over = "a".repeat(MAX_DIRECT_MESSAGE_CHARS + 1);
        assert!(
            validate_direct_request(
                &serde_json::to_vec(&json!({
                    "messages": [{"role": "user", "content": scalar_over}]
                }))
                .unwrap()
            )
            .is_err()
        );

        let multi_byte = "\u{1f642}".repeat(MAX_DIRECT_MESSAGE_CHARS);
        assert!(
            valid_request(json!({
                "messages": [
                    {"role": "user", "content": multi_byte.clone()},
                    {"role": "assistant", "content": multi_byte},
                ]
            }))
            .messages
            .len()
                == 2
        );

        let aggregate_over = "\u{1f642}".repeat(MAX_DIRECT_MESSAGE_CHARS);
        assert!(
            validate_direct_request(
                &serde_json::to_vec(&json!({
                    "messages": [
                        {"role": "user", "content": aggregate_over.clone()},
                        {"role": "assistant", "content": aggregate_over.clone()},
                        {"role": "user", "content": "x"},
                    ]
                }))
                .unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_unknown_effort_and_omits_the_field_when_unset() {
        let effort_error = validate_direct_request(
            &serde_json::to_vec(&json!({
                "messages": [{"role": "user", "content": "hi"}],
                "effort": "ultra",
            }))
            .unwrap(),
        )
        .unwrap_err();
        assert!(effort_error.to_string().contains("xhigh"));

        let without = build_upstream_body(valid_request(json!({
            "messages": [{"role": "user", "content": "hi"}],
        })));
        assert!(without.get("reasoning_effort").is_none());
        assert_eq!(without.as_object().unwrap().len(), 4);

        for effort in DIRECT_EFFORTS {
            let with = build_upstream_body(valid_request(json!({
                "messages": [{"role": "user", "content": "hi"}],
                "effort": effort.id,
            })));
            assert_eq!(with["reasoning_effort"], effort.id);
            assert_eq!(with.as_object().unwrap().len(), 5);
        }
    }

    #[test]
    fn rejects_unknown_model_and_skill_with_allowed_values() {
        let model_error = validate_direct_request(
            &serde_json::to_vec(&json!({
                "messages": [{"role": "user", "content": "hi"}],
                "model": "gpt-image-2",
            }))
            .unwrap(),
        )
        .unwrap_err();
        let skill_error = validate_direct_request(
            &serde_json::to_vec(&json!({
                "messages": [{"role": "user", "content": "hi"}],
                "skill_slug": "unknown",
            }))
            .unwrap(),
        )
        .unwrap_err();
        assert!(model_error.to_string().contains("gpt-5.5"));
        assert!(skill_error.to_string().contains("nyxid"));
    }

    #[test]
    fn rebuilds_exact_upstream_shape_and_strips_client_fields() {
        let body = build_upstream_body(valid_request(json!({
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "hi"},
            ],
            "model": "gpt-5.4-mini",
        })));
        assert_eq!(body["model"], "gpt-5.4-mini");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"], json!({"include_usage": true}));
        assert_eq!(body["messages"].as_array().unwrap().len(), 3);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(
            body["messages"][1],
            json!({"role": "user", "content": "hello"})
        );
        assert_eq!(
            body["messages"][2],
            json!({"role": "assistant", "content": "hi"})
        );
        assert_eq!(body.as_object().unwrap().len(), 4);
    }

    #[test]
    fn direct_mode_override_is_last_for_every_prompt_shape() {
        for skill_slug in std::iter::once(None).chain(DIRECT_SKILLS.iter().map(|s| Some(s.slug))) {
            let prompt = compose_system_prompt(skill_slug);
            assert!(prompt.starts_with(BASE_SYSTEM_PROMPT));
            assert!(prompt.contains("You cannot execute anything"));
            assert!(prompt.contains(DIRECT_MODE_OVERRIDE));
            assert!(prompt.ends_with(DIRECT_MODE_OVERRIDE));
            assert_eq!(prompt.matches(DIRECT_MODE_OVERRIDE).count(), 1);
            if let Some(slug) = skill_slug {
                let skill = find_skill(slug).unwrap();
                assert_eq!(
                    prompt,
                    format!(
                        "{BASE_SYSTEM_PROMPT}\n\n{}\n\n{DIRECT_MODE_OVERRIDE}",
                        skill.body
                    )
                );
            } else {
                assert_eq!(
                    prompt,
                    format!("{BASE_SYSTEM_PROMPT}\n\n{DIRECT_MODE_OVERRIDE}")
                );
            }
        }
    }

    #[test]
    fn registry_has_one_default_model() {
        assert_eq!(
            DIRECT_MODELS.iter().filter(|model| model.default).count(),
            1
        );
    }
}
