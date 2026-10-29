//! Feature flags (platform-admin managed).
//!
//! Code-declared flags and runtime-created flags share rollout resolution:
//! code default -> global -> org -> legacy role -> org user -> platform user.
//! An explicit disable wins ties between matching org memberships.
//! Feature flag operators manage their own runtime definitions; feature flag
//! admins manage every flag. Creator identity is separate from editable metadata.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::feature_flag_definition::{
    COLLECTION_NAME as DEFINITIONS, FeatureFlagDefinition,
};
use crate::models::feature_flag_metadata::{
    COLLECTION_NAME as METADATA_COLLECTION, FeatureFlagMetadata,
};
use crate::models::feature_flag_override::{COLLECTION_NAME, FeatureFlagOverride, FlagTargetKind};
use crate::models::org_membership::OrgRole;
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::services::org_service;

// ─────────────────────────────────────────────────────────────────────────────
// Registry (source of truth — declared in code)
// ─────────────────────────────────────────────────────────────────────────────

/// A feature flag definition. `default_enabled` is the value used when no
/// override applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureFlagDef {
    pub key: &'static str,
    pub description: &'static str,
    pub default_enabled: bool,
}

/// The known feature flags. Keys are stable identifiers referenced by both the
/// backend and `frontend/src/lib/feature-flags.ts`.
///
const AI_ASSISTANT_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: "experimental:ai-assistant",
    description: "AI Assistant chat surface (mock-data preview).",
    default_enabled: false,
};

/// Staged rollout for usage billing: only flagged owners are charged and see
/// the billing surface, even when `BILLING_ENABLED` and Lago are configured.
/// Off by default so charging can be piloted on a staff-selected org cohort.
pub const BILLING_FLAG_KEY: &str = "experimental:billing";

#[cfg(not(test))]
const BILLING_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: BILLING_FLAG_KEY,
    description: "Usage billing and wallet charging (staged rollout).",
    default_enabled: false,
};

/// Test builds default the billing flag ON: the billing suites assume
/// charging is active, and the disabled path is exercised explicitly through
/// override rows.
#[cfg(test)]
const BILLING_FLAG_TEST: FeatureFlagDef = FeatureFlagDef {
    key: BILLING_FLAG_KEY,
    description: "Usage billing and wallet charging (staged rollout).",
    default_enabled: true,
};

/// Personal rollout for new hosted Machine admission and runtime-rate writes.
/// Existing management, runtime accounting and cleanup remain available.
pub const REMOTE_MACHINES_FLAG_KEY: &str = "experimental:remote-machines";

const REMOTE_MACHINES_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: REMOTE_MACHINES_FLAG_KEY,
    description: "Enable hosted Remote Machine creation, resume/rekey and administrative runtime-rate writes for the acting person; existing management and accounting continue when off.",
    default_enabled: false,
};

/// Operator gate for the Aevatar chat wire-log diagnostic. Off by default and
/// toggled at runtime (platform-global, org cohort, or per user) so enabling a
/// browser-side capture of raw assistant payloads never needs a redeploy.
pub const AEVATAR_CHAT_WIRE_LOG_FLAG_KEY: &str = "experimental:aevatar-chat-wire-log";

const AEVATAR_CHAT_WIRE_LOG_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: AEVATAR_CHAT_WIRE_LOG_FLAG_KEY,
    description: "Exposes the Aevatar chat wire-log diagnostic (per-browser capture).",
    default_enabled: false,
};

/// Operator-selected engine for the human assistant surface. The backend
/// direct routes enforce this independently of the frontend selection.
pub const DIRECT_CHAT_ENGINE_FLAG_KEY: &str = "experimental:direct-chat-engine";

const DIRECT_CHAT_ENGINE_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: DIRECT_CHAT_ENGINE_FLAG_KEY,
    description: "Selects direct Chrono-LLM instead of Aevatar for assistant chat.",
    default_enabled: false,
};

pub const VOICE_GROK_FLAG_KEY: &str = "assistant:voice-grok";
const VOICE_GROK_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: VOICE_GROK_FLAG_KEY,
    description: "Grok voice private beta (push-to-talk/headphones only).",
    default_enabled: false,
};
pub const VOICE_GROK_PLATFORM_FLAG_KEY: &str = "assistant:voice-grok-platform";
const VOICE_GROK_PLATFORM_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: VOICE_GROK_PLATFORM_FLAG_KEY,
    description: "Paid platform Grok voice; enable only after provider invoice validation.",
    default_enabled: false,
};

pub const VOICE_OPENAI_PLATFORM_FLAG_KEY: &str = "assistant:voice-openai-platform";
const VOICE_OPENAI_PLATFORM_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: VOICE_OPENAI_PLATFORM_FLAG_KEY,
    description: "Allows platform GPT-Live only after forced-close and invoice reconciliation validation.",
    default_enabled: false,
};

pub const ASSISTANT_VOICE_FLAG_KEY: &str = "assistant:voice";
const ASSISTANT_VOICE_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: ASSISTANT_VOICE_FLAG_KEY,
    description: "Enables private assistant voice foundations; provider adapters require a later rollout.",
    default_enabled: false,
};

pub const DISCORD_GATEWAY_FLAG_KEY: &str = "channel:discord-gateway";
const DISCORD_GATEWAY_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: DISCORD_GATEWAY_FLAG_KEY,
    description: "Receive ordinary Discord messages through leased Gateway sessions. Interactions remain independent.",
    default_enabled: false,
};

pub const NYXBOT_THREAD_FOLLOW_FLAG_KEY: &str = "nyxbot:thread-follow";
pub const NYXBOT_ADAPTERS_FLAG_KEY: &str = "nyxbot:service-adapters";
const NYXBOT_ADAPTERS_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: NYXBOT_ADAPTERS_FLAG_KEY,
    description: "Phone/SMS/Custom LLM bindings. Enable only after every API replica enforces channel_adapter_origin at tool dispatch.",
    default_enabled: false,
};
const NYXBOT_THREAD_FOLLOW_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: NYXBOT_THREAD_FOLLOW_FLAG_KEY,
    description: "Staged direct-relay channel thread follow. Gateway follow remains unavailable until a compatible gateway is negotiated.",
    default_enabled: false,
};

pub const SKILL_PIN_DISABLE_FLAG_KEY: &str = "assistant:skill-pin-disable";
const SKILL_PIN_DISABLE_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: SKILL_PIN_DISABLE_FLAG_KEY,
    description: "Enables persisted disabled skill pins after all replicas understand the field.",
    default_enabled: false,
};

pub const ASSISTANT_MODEL_CONTROLS_FLAG_KEY: &str = "assistant:model-controls";
const ASSISTANT_MODEL_CONTROLS_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: ASSISTANT_MODEL_CONTROLS_FLAG_KEY,
    description: "Shows advanced model and reasoning controls for internal assistant users. Other users see Thinking and Permissions.",
    default_enabled: false,
};

pub const ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY: &str = "assistant:workspace-preview";
const ASSISTANT_WORKSPACE_PREVIEW_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY,
    description: "Shows the Activity and Artifacts workspace previews for an explicitly selected rollout audience.",
    default_enabled: false,
};

pub const NYXAGENT_ENGINE_FLAG_KEY: &str = "assistant:nyxagent-engine";
const NYXAGENT_ENGINE_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: NYXAGENT_ENGINE_FLAG_KEY,
    description: "Routes assistant chat through NyxAgent (catalog slug llm-nyx) instead of Aevatar.",
    default_enabled: true,
};

pub const SHARED_AGENTS_FLAG_KEY: &str = "assistant:shared-agents";
const SHARED_AGENTS_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: SHARED_AGENTS_FLAG_KEY,
    description: "Allows personal specialist sharing after all replicas enforce shared authority and consent.",
    default_enabled: false,
};

const ORG_AGENTS_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: "assistant:org-agents",
    description: "Allows organization specialist creation after all replicas enforce live member authority.",
    default_enabled: false,
};

pub const GUEST_OPERATION_SCOPES_FLAG_KEY: &str = "assistant:guest-operation-scopes";
const GUEST_OPERATION_SCOPES_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: GUEST_OPERATION_SCOPES_FLAG_KEY,
    description: "Configure guest-only operation limits after all replicas enforce them. Saved limits are always enforced.",
    default_enabled: false,
};

pub const AGENT_OPERATION_SCOPES_FLAG_KEY: &str = "assistant:operation-scopes";
const AGENT_OPERATION_SCOPES_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: AGENT_OPERATION_SCOPES_FLAG_KEY,
    description: "Allows configuring specialist operation scopes after every replica supports enforcement. Existing scopes are always enforced.",
    default_enabled: false,
};

pub const ASSISTANT_CHANNEL_MEDIA_FLAG_KEY: &str = "assistant:channel-media";
const ASSISTANT_CHANNEL_MEDIA_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
    description: "Admit owner Lark/Feishu media after every replica supports channel uploads.",
    default_enabled: false,
};
pub const ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY: &str = "assistant:attachment-writes";
const ASSISTANT_ATTACHMENT_WRITES_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY,
    description: "Enable managed attachment writes after every replica supports their enforcement.",
    default_enabled: false,
};

pub const ASSISTANT_INVOCATIONS_FLAG_KEY: &str = "assistant:owner-invocations";
const ASSISTANT_INVOCATIONS_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: ASSISTANT_INVOCATIONS_FLAG_KEY,
    description: "Admit owner CLI/API invocations only after every replica supports their lineage. After first admission, use forward fixes; disabling admission and draining work do not permit older binaries to return.",
    default_enabled: false,
};

pub const ASSISTANT_LIVE_TURN_GATE_FLAG_KEY: &str = "assistant:live-turn-gate";
const ASSISTANT_LIVE_TURN_GATE_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: ASSISTANT_LIVE_TURN_GATE_FLAG_KEY,
    description: "Require assistant conversation keys to have a live turn before execution.",
    default_enabled: false,
};

pub const MACHINE_CAPABILITIES_FLAG_KEY: &str = "assistant:machine-capabilities";
const MACHINE_CAPABILITIES_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: MACHINE_CAPABILITIES_FLAG_KEY,
    description: "Enable the capability editor and deny-by-default new assignments after upgrading all replicas; stored restrictions are always enforced.",
    default_enabled: false,
};
pub const MACHINE_CONTEXTS_FLAG_KEY: &str = "assistant:machine-contexts";
const MACHINE_CONTEXTS_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: MACHINE_CONTEXTS_FLAG_KEY,
    description: "Opt in to separate workspaces and browsers on supported Linux nodes with owner approval. Full isolation requires a separate machine container or VM.",
    default_enabled: false,
};

pub const AGENT_LEARNING_FLAG_KEY: &str = "assistant:agent-learning";
const AGENT_LEARNING_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: AGENT_LEARNING_FLAG_KEY,
    description: "Allows opt-in automatic learning proposals for assistant agents.",
    default_enabled: false,
};

pub const MOBILE_MFA_FLAG_KEY: &str = "auth:mobile-mfa";
const MOBILE_MFA_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: MOBILE_MFA_FLAG_KEY,
    description: "Require MFA for mobile social sign-ins. While off, anyone holding the account's social first factor can sign in through mobile without an MFA code.",
    default_enabled: false,
};

/// Resolve once for the signing-in person, including live org overrides.
pub async fn mobile_mfa_required(db: &mongodb::Database, person: &str) -> AppResult<bool> {
    Ok(resolve_personal_features(db, person)
        .await?
        .iter()
        .any(|key| key == MOBILE_MFA_FLAG_KEY))
}

pub const INVITATION_CODE_FLAG_KEY: &str = "auth:invitation-code";
const INVITATION_CODE_FLAG: FeatureFlagDef = FeatureFlagDef {
    key: INVITATION_CODE_FLAG_KEY,
    description: "Require an invitation code for new account registration (global only).",
    default_enabled: true,
};

/// NyxBot reaches personal and org chat-app bots through the Agent Event
/// Gateway (Telegram always does). Resolve each flag for the linking person,
/// including their live org membership overrides. Turning it on moves verified
/// bots when the gateway accepts the platform; refusals stay on NyxID's relay
/// and retry daily. Turning it off returns gateway bots to the relay. Both
/// directions wait for active answers and retain the route and settings.
/// Keyed by canonical platform.
pub const NYXBOT_GATEWAY_FLAGS: &[(&str, &str)] = &[
    ("lark", "nyxbot:gateway-lark"),
    ("feishu", "nyxbot:gateway-feishu"),
    ("discord", "nyxbot:gateway-discord"),
    ("slack", "nyxbot:gateway-slack"),
    ("whatsapp", "nyxbot:gateway-whatsapp"),
    ("x", "nyxbot:gateway-x"),
    ("aurinko", "nyxbot:gateway-aurinko"),
];

const fn nyxbot_gateway_flag(key: &'static str, description: &'static str) -> FeatureFlagDef {
    FeatureFlagDef {
        key,
        description,
        default_enabled: false,
    }
}

const NYXBOT_GATEWAY_FLAG_DEFS: [FeatureFlagDef; 7] = [
    nyxbot_gateway_flag(
        "nyxbot:gateway-lark",
        "NyxBot reaches personal and linked org Lark bots through the Agent Event Gateway once the gateway takes Lark (text only there); flag off returns them to NyxID's relay.",
    ),
    nyxbot_gateway_flag(
        "nyxbot:gateway-feishu",
        "NyxBot reaches personal and linked org Feishu bots through the Agent Event Gateway once the gateway takes Feishu (text only there); flag off returns them to NyxID's relay.",
    ),
    nyxbot_gateway_flag(
        "nyxbot:gateway-discord",
        "NyxBot reaches personal and linked org Discord bots through the Agent Event Gateway once the gateway verifies Discord; flag off returns them to NyxID's relay.",
    ),
    nyxbot_gateway_flag(
        "nyxbot:gateway-slack",
        "NyxBot reaches personal and linked org Slack bots through the Agent Event Gateway once the gateway verifies Slack; flag off returns them to NyxID's relay.",
    ),
    nyxbot_gateway_flag(
        "nyxbot:gateway-whatsapp",
        "NyxBot reaches personal and linked org WhatsApp bots through the Agent Event Gateway once the gateway verifies WhatsApp; flag off returns them to NyxID's relay.",
    ),
    nyxbot_gateway_flag(
        "nyxbot:gateway-x",
        "NyxBot reaches personal and linked org X bots through the Agent Event Gateway once the gateway supports X; flag off returns them to NyxID's relay.",
    ),
    nyxbot_gateway_flag(
        "nyxbot:gateway-aurinko",
        "NyxBot reaches personal and linked org email (Aurinko) bots through the Agent Event Gateway once the gateway supports them; flag off returns them to NyxID's relay.",
    ),
];

#[cfg(not(test))]
pub const FEATURE_FLAGS: &[FeatureFlagDef] = &[
    INVITATION_CODE_FLAG,
    MOBILE_MFA_FLAG,
    NYXAGENT_ENGINE_FLAG,
    ASSISTANT_MODEL_CONTROLS_FLAG,
    ASSISTANT_WORKSPACE_PREVIEW_FLAG,
    SKILL_PIN_DISABLE_FLAG,
    NYXBOT_THREAD_FOLLOW_FLAG,
    DISCORD_GATEWAY_FLAG,
    NYXBOT_ADAPTERS_FLAG,
    AGENT_OPERATION_SCOPES_FLAG,
    GUEST_OPERATION_SCOPES_FLAG,
    ASSISTANT_CHANNEL_MEDIA_FLAG,
    ASSISTANT_ATTACHMENT_WRITES_FLAG,
    ASSISTANT_LIVE_TURN_GATE_FLAG,
    ASSISTANT_INVOCATIONS_FLAG,
    MACHINE_CAPABILITIES_FLAG,
    MACHINE_CONTEXTS_FLAG,
    AGENT_LEARNING_FLAG,
    ORG_AGENTS_FLAG,
    SHARED_AGENTS_FLAG,
    ASSISTANT_VOICE_FLAG,
    VOICE_OPENAI_PLATFORM_FLAG,
    VOICE_GROK_FLAG,
    VOICE_GROK_PLATFORM_FLAG,
    AI_ASSISTANT_FLAG,
    BILLING_FLAG,
    REMOTE_MACHINES_FLAG,
    AEVATAR_CHAT_WIRE_LOG_FLAG,
    DIRECT_CHAT_ENGINE_FLAG,
    NYXBOT_GATEWAY_FLAG_DEFS[0],
    NYXBOT_GATEWAY_FLAG_DEFS[1],
    NYXBOT_GATEWAY_FLAG_DEFS[2],
    NYXBOT_GATEWAY_FLAG_DEFS[3],
    NYXBOT_GATEWAY_FLAG_DEFS[4],
    NYXBOT_GATEWAY_FLAG_DEFS[5],
    NYXBOT_GATEWAY_FLAG_DEFS[6],
];

/// Test builds carry a placeholder flag so the resolution / override pipeline
/// can exercise multiple definitions alongside the production registry entry.
#[cfg(test)]
pub const FEATURE_FLAGS: &[FeatureFlagDef] = &[
    INVITATION_CODE_FLAG,
    MOBILE_MFA_FLAG,
    NYXAGENT_ENGINE_FLAG,
    ASSISTANT_MODEL_CONTROLS_FLAG,
    ASSISTANT_WORKSPACE_PREVIEW_FLAG,
    SKILL_PIN_DISABLE_FLAG,
    NYXBOT_THREAD_FOLLOW_FLAG,
    DISCORD_GATEWAY_FLAG,
    NYXBOT_ADAPTERS_FLAG,
    AGENT_OPERATION_SCOPES_FLAG,
    GUEST_OPERATION_SCOPES_FLAG,
    ASSISTANT_CHANNEL_MEDIA_FLAG,
    ASSISTANT_ATTACHMENT_WRITES_FLAG,
    ASSISTANT_LIVE_TURN_GATE_FLAG,
    ASSISTANT_INVOCATIONS_FLAG,
    MACHINE_CAPABILITIES_FLAG,
    MACHINE_CONTEXTS_FLAG,
    AGENT_LEARNING_FLAG,
    ORG_AGENTS_FLAG,
    SHARED_AGENTS_FLAG,
    ASSISTANT_VOICE_FLAG,
    VOICE_OPENAI_PLATFORM_FLAG,
    VOICE_GROK_FLAG,
    VOICE_GROK_PLATFORM_FLAG,
    AI_ASSISTANT_FLAG,
    BILLING_FLAG_TEST,
    REMOTE_MACHINES_FLAG,
    AEVATAR_CHAT_WIRE_LOG_FLAG,
    DIRECT_CHAT_ENGINE_FLAG,
    NYXBOT_GATEWAY_FLAG_DEFS[0],
    NYXBOT_GATEWAY_FLAG_DEFS[1],
    NYXBOT_GATEWAY_FLAG_DEFS[2],
    NYXBOT_GATEWAY_FLAG_DEFS[3],
    NYXBOT_GATEWAY_FLAG_DEFS[4],
    NYXBOT_GATEWAY_FLAG_DEFS[5],
    NYXBOT_GATEWAY_FLAG_DEFS[6],
    FeatureFlagDef {
        key: "example_ui",
        description: "Test-only placeholder flag.",
        default_enabled: false,
    },
];

/// Look up a flag definition by key. `None` for unknown keys.
pub fn find_flag(key: &str) -> Option<&'static FeatureFlagDef> {
    FEATURE_FLAGS.iter().find(|f| f.key == key)
}

/// A resolved definition. Built-in definitions have no runtime creator.
#[derive(Clone, Debug)]
pub struct FlagDefinition {
    pub id: Option<String>,
    pub key: String,
    pub description: String,
    pub default_enabled: bool,
    pub created_by: Option<String>,
}

impl From<&FeatureFlagDef> for FlagDefinition {
    fn from(def: &FeatureFlagDef) -> Self {
        Self {
            id: None,
            key: def.key.to_owned(),
            description: def.description.to_owned(),
            default_enabled: def.default_enabled,
            created_by: None,
        }
    }
}

impl From<FeatureFlagDefinition> for FlagDefinition {
    fn from(def: FeatureFlagDefinition) -> Self {
        Self {
            id: Some(def.id),
            key: def.flag_key,
            description: def.description,
            default_enabled: def.default_enabled,
            created_by: Some(def.created_by),
        }
    }
}

pub async fn list_definitions(db: &mongodb::Database) -> AppResult<Vec<FlagDefinition>> {
    let rows: Vec<FeatureFlagDefinition> = db
        .collection::<FeatureFlagDefinition>(DEFINITIONS)
        .find(doc! {})
        .await?
        .try_collect()
        .await?;
    let mut definitions: Vec<FlagDefinition> = FEATURE_FLAGS.iter().map(Into::into).collect();
    definitions.extend(
        rows.into_iter()
            .filter(|row| find_flag(&row.flag_key).is_none())
            .map(Into::into),
    );
    definitions.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(definitions)
}

pub async fn get_definition(db: &mongodb::Database, key: &str) -> AppResult<FlagDefinition> {
    if let Some(def) = find_flag(key) {
        return Ok(def.into());
    }
    db.collection::<FeatureFlagDefinition>(DEFINITIONS)
        .find_one(doc! { "flag_key": key })
        .await?
        .map(Into::into)
        .ok_or_else(|| AppError::BadRequest(format!("unknown feature flag '{key}'")))
}

pub fn validate_flag_key(key: &str) -> AppResult<()> {
    if key.is_empty()
        || key.len() > 128
        || !key.as_bytes()[0].is_ascii_lowercase() && !key.as_bytes()[0].is_ascii_digit()
        || !key
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b":_-".contains(&c))
    {
        return Err(AppError::BadRequest("flag key must be 1-128 lowercase letters, digits, colons, underscores or hyphens, starting with a letter or digit".into()));
    }
    Ok(())
}

pub async fn create_definition(
    db: &mongodb::Database,
    key: &str,
    description: &str,
    default_enabled: bool,
    actor: &str,
) -> AppResult<FlagDefinition> {
    validate_flag_key(key)?;
    if find_flag(key).is_some() {
        return Err(AppError::Conflict("Feature flag already exists".into()));
    }
    let description =
        normalize_metadata_field(Some(description), MAX_FLAG_DESCRIPTION_LEN, "description")?
            .ok_or_else(|| AppError::BadRequest("description is required".into()))?;
    let row = FeatureFlagDefinition {
        id: Uuid::new_v4().to_string(),
        flag_key: key.to_owned(),
        description,
        default_enabled,
        created_by: actor.to_owned(),
        created_at: Utc::now(),
    };
    if let Err(error) = db
        .collection::<FeatureFlagDefinition>(DEFINITIONS)
        .insert_one(&row)
        .await
    {
        if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
        {
            return Err(AppError::Conflict("Feature flag already exists".into()));
        }
        return Err(error.into());
    }
    Ok(row.into())
}

// ─────────────────────────────────────────────────────────────────────────────
// Target
// ─────────────────────────────────────────────────────────────────────────────

/// Ergonomic view of an override's scope, used by the service and handlers.
/// Maps to the flat `(target_kind, target_key)` columns on the stored row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlagTarget {
    /// Platform-wide (everyone, org or not). A baseline below org scopes.
    Global,
    /// Every member of the org.
    Org,
    /// Members holding this role.
    Role(OrgRole),
    /// A single user. Their `member_user_id` (org context) or user id (personal).
    User(String),
}

impl FlagTarget {
    pub fn kind(&self) -> FlagTargetKind {
        match self {
            Self::Global => FlagTargetKind::Global,
            Self::Org => FlagTargetKind::Org,
            Self::Role(_) => FlagTargetKind::Role,
            Self::User(_) => FlagTargetKind::User,
        }
    }

    /// Storage key: `None` for global/org scope, role string for role scope,
    /// user id for user scope.
    pub fn key(&self) -> Option<String> {
        match self {
            Self::Global | Self::Org => None,
            Self::Role(role) => Some(role.as_str().to_string()),
            Self::User(user_id) => Some(user_id.clone()),
        }
    }

    /// Parse a wire target (`kind` + optional `value`) into a `FlagTarget`,
    /// validating that role targets name a real role and that a value is
    /// present/absent as the kind requires.
    pub fn from_parts(kind: FlagTargetKind, value: Option<&str>) -> AppResult<Self> {
        match kind {
            FlagTargetKind::Global => Ok(Self::Global),
            FlagTargetKind::Org => Ok(Self::Org),
            FlagTargetKind::Role => {
                let raw = value.ok_or_else(|| {
                    AppError::BadRequest("role target requires a target_value".to_string())
                })?;
                let role = match raw {
                    "admin" => OrgRole::Admin,
                    "member" => OrgRole::Member,
                    "viewer" => OrgRole::Viewer,
                    other => {
                        return Err(AppError::BadRequest(format!(
                            "invalid org role '{other}'; expected admin, member, or viewer"
                        )));
                    }
                };
                Ok(Self::Role(role))
            }
            FlagTargetKind::User => {
                let user_id = value.ok_or_else(|| {
                    AppError::BadRequest("user target requires a target_value".to_string())
                })?;
                if user_id.trim().is_empty() {
                    return Err(AppError::BadRequest(
                        "user target_value must not be empty".to_string(),
                    ));
                }
                Ok(Self::User(user_id.to_string()))
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Pick the enabled value of the override matching `(flag_key, kind, key)`.
fn pick_override(
    overrides: &[FeatureFlagOverride],
    flag_key: &str,
    kind: FlagTargetKind,
    key: Option<&str>,
    definition_id: Option<&str>,
) -> Option<bool> {
    overrides
        .iter()
        .find(|o| {
            o.flag_key == flag_key
                && o.definition_id.as_deref() == definition_id
                && o.target_kind == kind
                && o.target_key.as_deref() == key
        })
        .map(|o| o.enabled)
}

/// Compute the enabled-flag keys for a member in an org context.
///
/// Pure and DB-free so precedence is unit-testable. Per flag, most-specific
/// wins: `default → global → org → legacy role → legacy org user → platform user`.
/// Both user scopes match `member_user_id`; legacy role rows match `role`.
pub fn resolve_from_overrides(
    platform: &[FeatureFlagOverride],
    org: &[FeatureFlagOverride],
    member_user_id: &str,
    role: OrgRole,
) -> Vec<String> {
    resolve_definitions_from_overrides(
        FEATURE_FLAGS
            .iter()
            .map(|d| (d.key, d.default_enabled, None)),
        platform,
        org,
        member_user_id,
        role,
    )
}

fn resolve_definitions_from_overrides<'a>(
    definitions: impl IntoIterator<Item = (&'a str, bool, Option<&'a str>)>,
    platform: &[FeatureFlagOverride],
    org: &[FeatureFlagOverride],
    member_user_id: &str,
    role: OrgRole,
) -> Vec<String> {
    let mut enabled_keys = Vec::new();
    for (key, default_enabled, definition_id) in definitions {
        let mut enabled = default_enabled;
        if let Some(v) = pick_override(platform, key, FlagTargetKind::Global, None, definition_id) {
            enabled = v;
        }
        if let Some(v) = pick_override(org, key, FlagTargetKind::Org, None, definition_id) {
            enabled = v;
        }
        if let Some(v) = pick_override(
            org,
            key,
            FlagTargetKind::Role,
            Some(role.as_str()),
            definition_id,
        ) {
            enabled = v;
        }
        if let Some(v) = pick_override(
            org,
            key,
            FlagTargetKind::User,
            Some(member_user_id),
            definition_id,
        ) {
            enabled = v;
        }
        if let Some(v) = pick_override(
            platform,
            key,
            FlagTargetKind::User,
            Some(member_user_id),
            definition_id,
        ) {
            enabled = v;
        }
        if enabled {
            enabled_keys.push(key.to_string());
        }
    }
    enabled_keys
}

/// Pick the enabled value of an org-scoped override matching
/// `(org_user_id, flag_key, kind, key)` from a mixed multi-org row set.
fn pick_org_override(
    overrides: &[FeatureFlagOverride],
    org_user_id: &str,
    flag_key: &str,
    kind: FlagTargetKind,
    key: Option<&str>,
    definition_id: Option<&str>,
) -> Option<bool> {
    overrides
        .iter()
        .find(|o| {
            o.org_user_id.as_deref() == Some(org_user_id)
                && o.definition_id.as_deref() == definition_id
                && o.flag_key == flag_key
                && o.target_kind == kind
                && o.target_key.as_deref() == key
        })
        .map(|o| o.enabled)
}

/// Resolve the most-specific matching org-scoped override for a member across
/// all active memberships. The returned value is `None` when no org row
/// applies. If multiple memberships have a row at the same specificity, an
/// explicit disable wins that tie; this keeps a disabled org from being
/// bypassed by another membership while preserving the normal specificity
/// ordering (`user` > `role` > `org`).
fn resolve_org_membership_override(
    org_rows: &[FeatureFlagOverride],
    flag_key: &str,
    memberships: &[(String, OrgRole)],
    user_id: &str,
    definition_id: Option<&str>,
) -> Option<bool> {
    for kind in [
        FlagTargetKind::User,
        FlagTargetKind::Role,
        FlagTargetKind::Org,
    ] {
        let mut found = false;
        let mut enabled = false;
        let mut disabled = false;
        for (org_id, role) in memberships {
            let key = match kind {
                FlagTargetKind::User => Some(user_id),
                FlagTargetKind::Role => Some(role.as_str()),
                FlagTargetKind::Org => None,
                FlagTargetKind::Global => None,
            };
            if let Some(value) =
                pick_org_override(org_rows, org_id, flag_key, kind, key, definition_id)
            {
                found = true;
                enabled |= value;
                if !value {
                    // A disable wins conflicts at the same specificity.
                    disabled = true;
                }
            }
        }
        if found {
            return Some(enabled && !disabled);
        }
    }
    None
}

/// Compute the enabled-flag keys for a user in the **personal** (non-org)
/// context — the resolution behind `/users/me` and every non-org surface
/// (sidebar, `/assistant`, …).
///
/// Per-flag precedence:
/// `default → global → org → role → org-scoped user → personal user`.
/// `memberships` are the user's **active** org memberships as
/// `(org_user_id, role)`; `org_rows` are override rows across those orgs.
///
/// Pure and DB-free so the precedence matrix is unit-testable.
#[cfg(test)]
pub fn resolve_personal_from_overrides(
    platform: &[FeatureFlagOverride],
    org_rows: &[FeatureFlagOverride],
    memberships: &[(String, OrgRole)],
    user_id: &str,
) -> Vec<String> {
    resolve_personal_definitions(
        FEATURE_FLAGS
            .iter()
            .map(|d| (d.key, d.default_enabled, None)),
        platform,
        org_rows,
        memberships,
        user_id,
    )
}

pub(crate) fn resolve_personal_definitions<'a>(
    definitions: impl IntoIterator<Item = (&'a str, bool, Option<&'a str>)>,
    platform: &[FeatureFlagOverride],
    org_rows: &[FeatureFlagOverride],
    memberships: &[(String, OrgRole)],
    user_id: &str,
) -> Vec<String> {
    let mut enabled_keys = Vec::new();
    for (key, default_enabled, definition_id) in definitions {
        let mut enabled = pick_override(platform, key, FlagTargetKind::Global, None, definition_id)
            .unwrap_or(default_enabled);
        if let Some(org_value) =
            resolve_org_membership_override(org_rows, key, memberships, user_id, definition_id)
        {
            enabled = org_value;
        }
        if let Some(personal_value) = pick_override(
            platform,
            key,
            FlagTargetKind::User,
            Some(user_id),
            definition_id,
        ) {
            enabled = personal_value;
        }
        if enabled {
            enabled_keys.push(key.to_string());
        }
    }
    enabled_keys
}

/// Resolve enabled-flag keys for a member of an org (org context). Applies the
/// platform-global baseline, the org's own overrides, and platform user overrides.
pub async fn resolve_enabled_features(
    db: &mongodb::Database,
    org_user_id: &str,
    member_user_id: &str,
    role: OrgRole,
) -> AppResult<Vec<String>> {
    let org = list_overrides(db, org_user_id).await?;
    let global = list_platform_overrides(db).await?;
    let definitions = list_definitions(db).await?;
    Ok(resolve_definitions_from_overrides(
        definitions
            .iter()
            .map(|d| (d.key.as_str(), d.default_enabled, d.id.as_deref())),
        &global,
        &org,
        member_user_id,
        role,
    ))
}

/// Resolve enabled-flag keys for a user in the personal (non-org) context.
/// Delivered on `GET /users/me`. Org-aware: evaluates matching rows from every
/// org the user is an active member of, applying the same specificity
/// precedence as org-context resolution. An org-level disable therefore
/// overrides a global enable for that member unless a more-specific user
/// override applies.
pub async fn resolve_personal_features(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<String>> {
    let memberships = org_service::list_memberships_for_member(db, user_id, false).await?;

    // One query for the platform rows (`org_user_id: null`) plus every
    // membership org's rows — no per-org fan-out on the hot /users/me path.
    let mut scope_keys: Vec<bson::Bson> = vec![bson::Bson::Null];
    scope_keys.extend(
        memberships
            .iter()
            .map(|m| bson::Bson::String(m.org_user_id.clone())),
    );
    let rows: Vec<FeatureFlagOverride> = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find(doc! { "org_user_id": { "$in": scope_keys } })
        .await?
        .try_collect()
        .await?;
    let (org_rows, platform): (Vec<_>, Vec<_>) =
        rows.into_iter().partition(|row| row.org_user_id.is_some());
    let membership_roles: Vec<(String, OrgRole)> = memberships
        .into_iter()
        .map(|m| (m.org_user_id, m.role))
        .collect();

    let definitions = list_definitions(db).await?;
    Ok(resolve_personal_definitions(
        definitions
            .iter()
            .map(|d| (d.key.as_str(), d.default_enabled, d.id.as_deref())),
        &platform,
        &org_rows,
        &membership_roles,
        user_id,
    ))
}

/// Signup has no user or organization yet, so only the global override applies.
pub async fn invitation_code_required(db: &mongodb::Database) -> AppResult<bool> {
    let override_row = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find_one(doc! {
            "org_user_id": bson::Bson::Null,
            "flag_key": INVITATION_CODE_FLAG_KEY,
            "definition_id": null,
            "target_kind": FlagTargetKind::Global.as_str(),
            "target_key": bson::Bson::Null,
        })
        .await?;
    Ok(override_row
        .map(|row| row.enabled)
        .unwrap_or(INVITATION_CODE_FLAG.default_enabled))
}

/// Whether the billing rollout flag is enabled for a billing owner.
///
/// Personal wallets use the person's active org memberships; org wallets use
/// that org's overrides. Both apply `default -> global -> org -> user`, including
/// the acting person's platform user override as the final value.
pub async fn billing_rollout_enabled(
    db: &mongodb::Database,
    billing_owner_id: &str,
    actor_user_id: &str,
) -> AppResult<bool> {
    let enabled = if billing_owner_id == actor_user_id {
        resolve_personal_features(db, billing_owner_id).await?
    } else {
        let org = list_overrides(db, billing_owner_id).await?;
        let global = list_platform_overrides(db).await?;
        resolve_from_overrides(&global, &org, actor_user_id, OrgRole::Member)
    };
    Ok(enabled.iter().any(|key| key == BILLING_FLAG_KEY))
}

/// Whether a grant recipient is covered by the billing rollout.
///
/// Person recipients use the same personal feature resolution as their
/// billing page. Organization recipients use the org-wide member baseline;
/// legacy member-specific overrides may still make an individual member's
/// result more specific until startup migration removes those rows.
pub async fn billing_recipient_rollout_enabled(
    db: &mongodb::Database,
    recipient: &User,
) -> AppResult<bool> {
    let enabled = if recipient.user_type.is_org() {
        resolve_enabled_features(db, &recipient.id, "", OrgRole::Member).await?
    } else {
        resolve_personal_features(db, &recipient.id).await?
    };
    Ok(enabled.iter().any(|key| key == BILLING_FLAG_KEY))
}

/// Whether the Aevatar chat wire-log diagnostic is enabled for the acting user.
///
/// Assistant chat is a **personal** surface, so this resolves through the same
/// specificity chain as `/users/me`: default, global, matching org scopes,
/// and finally a personal per-user override.
///
/// Callers gate a diagnostic that exposes raw upstream payloads to the
/// browser, so a resolution error must be treated as disabled — never as
/// enabled.
pub async fn aevatar_chat_wire_log_enabled(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<bool> {
    Ok(resolve_personal_features(db, user_id)
        .await?
        .iter()
        .any(|key| key == AEVATAR_CHAT_WIRE_LOG_FLAG_KEY))
}

/// Whether a personal-surface flag is on for a person: default, global,
/// matching org scopes, then their own override (as on `/users/me`).
pub async fn personal_flag_enabled(
    db: &mongodb::Database,
    user_id: &str,
    flag_key: &str,
) -> AppResult<bool> {
    let enabled = resolve_personal_features(db, user_id)
        .await?
        .iter()
        .any(|key| key == flag_key);
    if enabled && super::security_readiness_service::CAPABILITIES.contains(&flag_key) {
        warn_if_flag_in_effect_unready(db, user_id, flag_key).await;
    }
    Ok(enabled)
}

const UNREADY_FLAG_EVENT: &str = "security_readiness_flag_in_effect_unready";
const UNREADY_FLAG_INTERVAL: Duration = Duration::from_secs(60);
static UNREADY_FLAG_THROTTLE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

#[cfg(test)]
pub(crate) fn reset_unready_flag_throttle() {
    UNREADY_FLAG_THROTTLE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

/// Readiness gates admin activation; existing flag reads remain available.
/// Throttle locally before diagnostic I/O, then share a one-minute emission
/// window per flag across replicas so enabled unready flags remain visible.
async fn warn_if_flag_in_effect_unready(db: &mongodb::Database, user_id: &str, flag_key: &str) {
    let throttle_key = flag_key.to_owned();
    // Isolated test databases model independent deployments in one process.
    #[cfg(test)]
    let throttle_key = format!("{}:{throttle_key}", db.name());
    {
        let mut attempts = UNREADY_FLAG_THROTTLE
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        if attempts
            .get(&throttle_key)
            .is_some_and(|last| now.duration_since(*last) < UNREADY_FLAG_INTERVAL)
        {
            return;
        }
        attempts.insert(throttle_key, now);
    }
    match super::coordination_service::RateWindowStore::admit(
        db,
        UNREADY_FLAG_EVENT,
        flag_key,
        1,
        UNREADY_FLAG_INTERVAL,
    )
    .await
    {
        Ok(admission) if admission.allowed => {}
        Ok(_) => return,
        Err(error) => {
            tracing::debug!(flag_key, %error, "Flag readiness diagnostic rate window unavailable");
            return;
        }
    }
    let report = match super::security_readiness_service::report(db, flag_key).await {
        Ok(report) if !report.ready => report,
        Ok(_) => return,
        Err(error) => {
            tracing::debug!(flag_key, %error, "Flag readiness diagnostic unavailable");
            return;
        }
    };
    tracing::warn!(flag_key, user_id, blockers = ?report.blockers, "Readiness-gated flag {flag_key} is enabled but the fleet roster is not ready (blockers: {:?})", report.blockers);
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        &super::audit_service::AuditActor {
            user_id: user_id.to_owned(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        UNREADY_FLAG_EVENT,
        Some(serde_json::json!({
            "flag_key": flag_key,
            "user_id": user_id,
            "blockers": report.blockers,
        })),
    )
    .await;
}

/// Who a flag can be on for, so background work can skip a flag nobody has
/// turned on and narrow one piloted on a few people: `None` when it may be on
/// for anyone (on by default, or enabled globally or for an org), else exactly
/// the people with a personal enabling override (empty: nobody).
pub async fn flag_enabled_people(
    db: &mongodb::Database,
    flag_key: &str,
) -> AppResult<Option<Vec<String>>> {
    let definition = get_definition(db, flag_key).await?;
    if definition.default_enabled {
        return Ok(None);
    }
    let rows: Vec<FeatureFlagOverride> = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find(doc! { "flag_key": flag_key, "definition_id": definition.id, "enabled": true })
        .await?
        .try_collect()
        .await?;
    let mut people = Vec::new();
    for row in rows {
        match (row.org_user_id.as_deref(), row.target_kind, row.target_key) {
            (None, FlagTargetKind::User, Some(user_id)) => people.push(user_id),
            _ => return Ok(None),
        }
    }
    Ok(Some(people))
}

// ─────────────────────────────────────────────────────────────────────────────
// Management (platform admin)
// ─────────────────────────────────────────────────────────────────────────────

/// All override rows for an org.
pub async fn list_overrides(
    db: &mongodb::Database,
    org_user_id: &str,
) -> AppResult<Vec<FeatureFlagOverride>> {
    let rows: Vec<FeatureFlagOverride> = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find(doc! { "org_user_id": org_user_id })
        .await?
        .try_collect()
        .await?;
    Ok(rows)
}

/// All platform-level override rows: global rollout + personal (org-less) user
/// overrides. These have `org_user_id` unset (`null`), so an org-scoped
/// `list_overrides` never returns them and vice versa.
pub async fn list_platform_overrides(
    db: &mongodb::Database,
) -> AppResult<Vec<FeatureFlagOverride>> {
    let rows: Vec<FeatureFlagOverride> = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find(doc! { "org_user_id": bson::Bson::Null })
        .await?
        .try_collect()
        .await?;
    Ok(rows)
}

/// Safe user fields used to enrich platform-admin feature-flag responses.
#[derive(Clone, Debug)]
pub struct PlatformOverrideUserDisplay {
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct PlatformOverrideUserProjection {
    #[serde(rename = "_id")]
    id: String,
    email: String,
    #[serde(default)]
    display_name: Option<String>,
}

/// Resolve every user referenced by platform overrides in one projected query.
/// Deleted users are absent and become null display fields in the response.
pub async fn fetch_platform_override_users(
    db: &mongodb::Database,
    overrides: &[FeatureFlagOverride],
) -> AppResult<HashMap<String, PlatformOverrideUserDisplay>> {
    let user_ids: HashSet<&str> = overrides
        .iter()
        .filter(|row| row.target_kind == FlagTargetKind::User)
        .filter_map(|row| row.target_key.as_deref())
        .collect();
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let cursor = db
        .collection::<PlatformOverrideUserProjection>(USERS)
        .find(doc! { "_id": { "$in": user_ids.into_iter().collect::<Vec<_>>() } })
        .projection(doc! { "_id": 1, "email": 1, "display_name": 1 })
        .await?;
    let users: Vec<PlatformOverrideUserProjection> = cursor.try_collect().await?;
    Ok(users
        .into_iter()
        .map(|user| {
            (
                user.id,
                PlatformOverrideUserDisplay {
                    email: user.email,
                    display_name: user.display_name,
                },
            )
        })
        .collect())
}

pub struct RolloutTargetDisplay {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub slug: Option<String>,
}

pub async fn search_rollout_targets(
    db: &mongodb::Database,
    kind: &str,
    search: Option<&str>,
    limit: u32,
) -> AppResult<Vec<RolloutTargetDisplay>> {
    let mut filter = match kind {
        "person" => doc! { "user_type": { "$ne": "org" } },
        "org" => doc! { "user_type": "org" },
        _ => return Err(AppError::BadRequest("kind must be person or org".into())),
    };
    filter.insert("is_active", true);
    if let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) {
        if search.len() > 128 {
            return Err(AppError::BadRequest("search is too long".into()));
        }
        let pattern = regex::escape(search);
        filter.insert(
            "$or",
            vec![
                doc! { "email": { "$regex": &pattern, "$options": "i" } },
                doc! { "display_name": { "$regex": &pattern, "$options": "i" } },
                doc! { "slug": { "$regex": &pattern, "$options": "i" } },
            ],
        );
    }
    let rows: Vec<bson::Document> = db
        .collection::<bson::Document>(USERS)
        .find(filter)
        .projection(doc! { "_id": 1, "email": 1, "display_name": 1, "slug": 1 })
        .sort(doc! { "_id": 1 })
        .limit(i64::from(limit.clamp(1, 20)))
        .await?
        .try_collect()
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(RolloutTargetDisplay {
                id: row
                    .get_str("_id")
                    .map_err(|_| AppError::Internal("Invalid rollout target id".into()))?
                    .to_owned(),
                email: row.get_str("email").unwrap_or_default().to_owned(),
                display_name: row.get_str("display_name").ok().map(str::to_owned),
                slug: row.get_str("slug").ok().map(str::to_owned),
            })
        })
        .collect()
}

/// The org-row upsert behind [`set_platform_org_override`]. Gates live on the
/// caller.
async fn upsert_override_row(
    db: &mongodb::Database,
    org_user_id: &str,
    flag_key: &str,
    target: &FlagTarget,
    enabled: bool,
    actor_id: &str,
) -> AppResult<FeatureFlagOverride> {
    let definition = get_definition(db, flag_key).await?;
    let now = bson::DateTime::from_chrono(Utc::now());
    let target_key = match target.key() {
        Some(k) => bson::Bson::String(k),
        None => bson::Bson::Null,
    };

    let row = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {
                "org_user_id": org_user_id,
                "flag_key": flag_key,
                "definition_id": definition.id.clone(),
                "target_kind": target.kind().as_str(),
                "target_key": target_key.clone(),
            },
            doc! {
                "$set": {
                    "enabled": enabled,
                    "updated_at": now,
                    "updated_by": actor_id,
                },
                "$setOnInsert": {
                    "_id": Uuid::new_v4().to_string(),
                    "org_user_id": org_user_id,
                    "flag_key": flag_key,
                    "definition_id": definition.id.clone(),
                    "target_kind": target.kind().as_str(),
                    "target_key": target_key,
                    "created_at": now,
                },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| {
            AppError::Internal("feature flag override upsert did not return the row".to_string())
        })?;

    Ok(row)
}

/// Upsert a platform-level override (staff-controlled). Only `Global` and
/// personal `User` targets are valid — org/role scopes require an org, and are
/// rejected here.
pub async fn set_platform_override(
    db: &mongodb::Database,
    flag_key: &str,
    target: &FlagTarget,
    enabled: bool,
    actor_id: &str,
) -> AppResult<FeatureFlagOverride> {
    let definition = get_definition(db, flag_key).await?;
    if flag_key == INVITATION_CODE_FLAG_KEY && !matches!(target, FlagTarget::Global) {
        return Err(AppError::BadRequest(
            "invitation code flag supports only a global override".to_string(),
        ));
    }
    if matches!(target, FlagTarget::Org | FlagTarget::Role(_)) {
        return Err(AppError::BadRequest(
            "org and role targets require an org; use the org feature-flag API".to_string(),
        ));
    }
    if let FlagTarget::User(user_id) = target {
        let exists = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": user_id })
            .await?
            .is_some();
        if !exists {
            return Err(AppError::NotFound("User not found".to_string()));
        }
    }

    if enabled {
        super::security_readiness_service::require(db, flag_key).await?;
    }
    let now = bson::DateTime::from_chrono(Utc::now());
    let target_key = match target.key() {
        Some(k) => bson::Bson::String(k),
        None => bson::Bson::Null,
    };

    let row = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {
                "org_user_id": bson::Bson::Null,
                "flag_key": flag_key,
                "definition_id": definition.id.clone(),
                "target_kind": target.kind().as_str(),
                "target_key": target_key.clone(),
            },
            doc! {
                "$set": {
                    "enabled": enabled,
                    "updated_at": now,
                    "updated_by": actor_id,
                },
                "$setOnInsert": {
                    "_id": Uuid::new_v4().to_string(),
                    "flag_key": flag_key,
                    "definition_id": definition.id.clone(),
                    "target_kind": target.kind().as_str(),
                    "target_key": target_key,
                    "created_at": now,
                },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| {
            AppError::Internal(
                "platform feature flag override upsert did not return the row".to_string(),
            )
        })?;
    Ok(row)
}

/// Clear a platform-level override. Returns the removed row when present.
pub async fn clear_platform_override(
    db: &mongodb::Database,
    flag_key: &str,
    target: &FlagTarget,
) -> AppResult<Option<FeatureFlagOverride>> {
    let definition = get_definition(db, flag_key).await?;
    let target_key = match target.key() {
        Some(k) => bson::Bson::String(k),
        None => bson::Bson::Null,
    };
    let row = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find_one_and_delete(doc! {
            "org_user_id": bson::Bson::Null,
            "flag_key": flag_key,
            "definition_id": definition.id.clone(),
            "target_kind": target.kind().as_str(),
            "target_key": target_key,
        })
        .await?;
    Ok(row)
}

/// Upsert an org-wide override on behalf of platform staff — rollout targeting
/// for one org cohort. The target must be an existing org account (404
/// otherwise).
pub async fn set_platform_org_override(
    db: &mongodb::Database,
    org_user_id: &str,
    flag_key: &str,
    enabled: bool,
    actor_id: &str,
) -> AppResult<FeatureFlagOverride> {
    get_definition(db, flag_key).await?;
    if flag_key == INVITATION_CODE_FLAG_KEY {
        return Err(AppError::BadRequest(
            "invitation code flag supports only a global override".to_string(),
        ));
    }
    ensure_org_exists(db, org_user_id).await?;
    if enabled {
        super::security_readiness_service::require(db, flag_key).await?;
    }
    upsert_override_row(
        db,
        org_user_id,
        flag_key,
        &FlagTarget::Org,
        enabled,
        actor_id,
    )
    .await
}

/// Clear an org-wide override on behalf of platform staff. Returns the removed
/// row when present (idempotent otherwise).
pub async fn clear_platform_org_override(
    db: &mongodb::Database,
    org_user_id: &str,
    flag_key: &str,
) -> AppResult<Option<FeatureFlagOverride>> {
    let definition = get_definition(db, flag_key).await?;
    let row = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find_one_and_delete(doc! {
            "org_user_id": org_user_id,
            "flag_key": flag_key,
                "definition_id": definition.id.clone(),
            "target_kind": FlagTargetKind::Org.as_str(),
            "target_key": bson::Bson::Null,
        })
        .await?;
    Ok(row)
}

async fn ensure_org_exists(db: &mongodb::Database, org_user_id: &str) -> AppResult<()> {
    let is_org = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": org_user_id })
        .await?
        .map(|user| user.user_type.is_org())
        .unwrap_or(false);
    if !is_org {
        return Err(AppError::NotFound("Organization not found".to_string()));
    }
    Ok(())
}

/// Every org-wide (`target_kind = "org"`) override row across all orgs. Powers
/// the platform-admin list view only — resolution reads a single org's rows
/// via `list_overrides`, so this must never feed the resolver.
pub async fn list_all_org_scope_overrides(
    db: &mongodb::Database,
) -> AppResult<Vec<FeatureFlagOverride>> {
    let rows: Vec<FeatureFlagOverride> = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find(doc! {
            "target_kind": FlagTargetKind::Org.as_str(),
            "org_user_id": { "$ne": bson::Bson::Null },
        })
        .await?
        .try_collect()
        .await?;
    Ok(rows)
}

/// Safe org fields used to enrich platform-admin feature-flag responses.
#[derive(Clone, Debug)]
pub struct PlatformOverrideOrgDisplay {
    pub display_name: Option<String>,
    pub slug: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct PlatformOverrideOrgProjection {
    #[serde(rename = "_id")]
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    slug: Option<String>,
}

/// Resolve every org referenced by org-scope overrides in one projected query.
/// Deleted orgs are absent and become null display fields in the response.
pub async fn fetch_override_org_display(
    db: &mongodb::Database,
    overrides: &[FeatureFlagOverride],
) -> AppResult<HashMap<String, PlatformOverrideOrgDisplay>> {
    let org_ids: HashSet<&str> = overrides
        .iter()
        .filter_map(|row| row.org_user_id.as_deref())
        .collect();
    if org_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let cursor = db
        .collection::<PlatformOverrideOrgProjection>(USERS)
        .find(doc! { "_id": { "$in": org_ids.into_iter().collect::<Vec<_>>() } })
        .projection(doc! { "_id": 1, "display_name": 1, "slug": 1 })
        .await?;
    let orgs: Vec<PlatformOverrideOrgProjection> = cursor.try_collect().await?;
    Ok(orgs
        .into_iter()
        .map(|org| {
            (
                org.id,
                PlatformOverrideOrgDisplay {
                    display_name: org.display_name,
                    slug: org.slug,
                },
            )
        })
        .collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// Metadata (admin-authored documentation)
// ─────────────────────────────────────────────────────────────────────────────

/// Upper bound on an admin-written flag description.
pub const MAX_FLAG_DESCRIPTION_LEN: usize = 512;
/// Upper bound on an admin-written flag owner (a person, team, or contact).
pub const MAX_FLAG_OWNER_LEN: usize = 128;

/// Trim a free-text metadata field, treating blank input as "cleared", and
/// enforce its length budget.
fn normalize_metadata_field(
    value: Option<&str>,
    max_len: usize,
    field: &str,
) -> AppResult<Option<String>> {
    let Some(trimmed) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > max_len {
        return Err(AppError::BadRequest(format!(
            "{field} must be at most {max_len} characters"
        )));
    }
    Ok(Some(trimmed.to_string()))
}

/// Every metadata row, keyed by flag key. Rows for flags that have since left
/// the registry are dropped so a stale row can never resurrect a retired flag.
pub async fn list_metadata(
    db: &mongodb::Database,
) -> AppResult<HashMap<String, FeatureFlagMetadata>> {
    let rows: Vec<FeatureFlagMetadata> = db
        .collection::<FeatureFlagMetadata>(METADATA_COLLECTION)
        .find(doc! {})
        .await?
        .try_collect()
        .await?;
    let keys: HashMap<String, Option<String>> = list_definitions(db)
        .await?
        .into_iter()
        .map(|def| (def.key, def.id))
        .collect();
    Ok(rows
        .into_iter()
        .filter(|row| {
            keys.get(&row.flag_key)
                .is_some_and(|id| id == &row.definition_id)
                && (row.description.is_some() || row.owner.is_some())
        })
        .map(|row| (row.flag_key.clone(), row))
        .collect())
}

/// Replace the admin-authored description and owner for one flag.
///
/// Full-replace semantics: whatever is passed becomes the stored state, and a
/// blank or absent value clears that field. Clearing both fields deletes the
/// row entirely (the flag falls back to its code-declared description with no
/// owner) and returns `None`.
pub async fn set_metadata(
    db: &mongodb::Database,
    flag_key: &str,
    description: Option<&str>,
    owner: Option<&str>,
    actor_id: &str,
) -> AppResult<Option<FeatureFlagMetadata>> {
    let definition = get_definition(db, flag_key).await?;
    let description =
        normalize_metadata_field(description, MAX_FLAG_DESCRIPTION_LEN, "description")?;
    let owner = normalize_metadata_field(owner, MAX_FLAG_OWNER_LEN, "owner")?;

    let collection = db.collection::<FeatureFlagMetadata>(METADATA_COLLECTION);
    if description.is_none() && owner.is_none() {
        collection
            .find_one_and_delete(
                doc! { "flag_key": flag_key, "definition_id": definition.id.clone() },
            )
            .await?;
        return Ok(None);
    }

    let now = bson::DateTime::from_chrono(Utc::now());
    let row = collection
        .find_one_and_update(
            doc! { "flag_key": flag_key, "definition_id": definition.id.clone() },
            doc! {
                "$set": {
                    "description": description.clone().map(bson::Bson::String).unwrap_or(bson::Bson::Null),
                    "owner": owner.clone().map(bson::Bson::String).unwrap_or(bson::Bson::Null),
                    "updated_at": now,
                    "updated_by": actor_id,
                },
                "$setOnInsert": {
                    "_id": Uuid::new_v4().to_string(),
                    "flag_key": flag_key,
                    "definition_id": definition.id.clone(),
                    "created_at": now,
                },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| {
            AppError::Internal("feature flag metadata upsert did not return the row".to_string())
        })?;
    Ok(Some(row))
}

/// Sparse metadata update. Keep empty rows internally so a concurrent disjoint
/// write cannot be deleted by cleanup after the update.
pub async fn patch_metadata(
    db: &mongodb::Database,
    flag_key: &str,
    description: Option<Option<&str>>,
    owner: Option<Option<&str>>,
    actor_id: &str,
) -> AppResult<Option<FeatureFlagMetadata>> {
    let definition = get_definition(db, flag_key).await?;
    let collection = db.collection::<FeatureFlagMetadata>(METADATA_COLLECTION);
    let mut set = doc! {};
    for (field, value, limit) in [
        ("description", description, MAX_FLAG_DESCRIPTION_LEN),
        ("owner", owner, MAX_FLAG_OWNER_LEN),
    ] {
        if let Some(value) = value {
            let value = normalize_metadata_field(value, limit, field)?;
            set.insert(
                field,
                value.map(bson::Bson::String).unwrap_or(bson::Bson::Null),
            );
        }
    }
    let row = if set.is_empty() {
        collection
            .find_one(doc! { "flag_key": flag_key, "definition_id": definition.id.clone() })
            .await?
    } else {
        let now = bson::DateTime::from_chrono(Utc::now());
        let mut insert = doc! {
            "_id": Uuid::new_v4().to_string(), "flag_key": flag_key,
            "definition_id": definition.id.clone(), "created_at": now
        };
        for field in ["description", "owner"] {
            if !set.contains_key(field) {
                insert.insert(field, bson::Bson::Null);
            }
        }
        set.insert("updated_at", now);
        set.insert("updated_by", actor_id);
        collection
            .find_one_and_update(
                doc! { "flag_key": flag_key, "definition_id": definition.id.clone() },
                doc! { "$set": set, "$setOnInsert": insert },
            )
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await?
    };
    Ok(row.filter(|row| row.description.is_some() || row.owner.is_some()))
}

/// Cascade helper: drop every override for an org (used when the org is deleted).
pub async fn delete_all_for_org(db: &mongodb::Database, org_user_id: &str) -> AppResult<()> {
    db.collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .delete_many(doc! { "org_user_id": org_user_id })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserType;
    use crate::test_utils::{connect_test_database, test_user};

    fn override_row(
        org: Option<&str>,
        flag: &str,
        kind: FlagTargetKind,
        key: Option<&str>,
        enabled: bool,
    ) -> FeatureFlagOverride {
        FeatureFlagOverride {
            definition_id: None,
            id: Uuid::new_v4().to_string(),
            org_user_id: org.map(str::to_string),
            flag_key: flag.to_string(),
            target_kind: kind,
            target_key: key.map(str::to_string),
            enabled,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: "actor".to_string(),
        }
    }

    #[tokio::test]
    async fn security_readiness_gates_admin_enable_for_global_user_and_org_overrides() {
        let db = connect_test_database("feature_flag_activation")
            .await
            .expect("configured Mongo required");
        let user = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        for (id, kind) in [(&user, UserType::Person), (&org, UserType::Org)] {
            db.collection::<User>(USERS)
                .insert_one(test_user(id, kind))
                .await
                .unwrap();
        }
        for flag in FEATURE_FLAGS.iter().filter(|flag| {
            super::super::security_readiness_service::CAPABILITIES.contains(&flag.key)
        }) {
            for target in [FlagTarget::Global, FlagTarget::User(user.clone())] {
                assert!(matches!(
                    set_platform_override(&db, flag.key, &target, true, &user).await,
                    Err(AppError::SecurityReadinessUnavailable)
                ));
                assert!(
                    !set_platform_override(&db, flag.key, &target, false, &user)
                        .await
                        .unwrap()
                        .enabled
                );
            }
            assert!(matches!(
                set_platform_org_override(&db, &org, flag.key, true, &user).await,
                Err(AppError::SecurityReadinessUnavailable)
            ));
            assert!(
                !set_platform_org_override(&db, &org, flag.key, false, &user)
                    .await
                    .unwrap()
                    .enabled
            );
        }
        // Ordinary flags retain enable/disable behavior without readiness.
        for enabled in [true, false] {
            for target in [FlagTarget::Global, FlagTarget::User(user.clone())] {
                assert_eq!(
                    set_platform_override(&db, "example_ui", &target, enabled, &user)
                        .await
                        .unwrap()
                        .enabled,
                    enabled
                );
            }
            assert_eq!(
                set_platform_org_override(&db, &org, "example_ui", enabled, &user)
                    .await
                    .unwrap()
                    .enabled,
                enabled
            );
        }
        super::super::security_readiness_service::ready_test_replica(&db).await;
        for flag in FEATURE_FLAGS.iter().filter(|flag| {
            super::super::security_readiness_service::CAPABILITIES.contains(&flag.key)
        }) {
            for target in [FlagTarget::Global, FlagTarget::User(user.clone())] {
                assert!(
                    set_platform_override(&db, flag.key, &target, true, &user)
                        .await
                        .unwrap()
                        .enabled
                );
            }
            assert!(
                set_platform_org_override(&db, &org, flag.key, true, &user)
                    .await
                    .unwrap()
                    .enabled
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn personal_flag_enabled_when_unready_throttles_per_flag_and_ignores_report_errors() {
        use super::super::security_readiness_service as readiness;
        use crate::models::audit_log::AuditLog;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tracing::instrument::WithSubscriber;
        use tracing_subscriber::prelude::*;

        struct CountDiagnostics {
            warnings: Arc<AtomicUsize>,
            errors: Arc<AtomicUsize>,
        }
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CountDiagnostics {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                if *event.metadata().level() == tracing::Level::WARN
                    && event.metadata().fields().field("blockers").is_some()
                {
                    self.warnings.fetch_add(1, Ordering::SeqCst);
                }
                if *event.metadata().level() == tracing::Level::DEBUG
                    && event.metadata().fields().field("flag_key").is_some()
                    && event.metadata().fields().field("error").is_some()
                {
                    self.errors.fetch_add(1, Ordering::SeqCst);
                }
            }
        }

        let db = connect_test_database("personal_flag_unready")
            .await
            .expect("configured Mongo required");
        reset_unready_flag_throttle();
        let flag = ASSISTANT_LIVE_TURN_GATE_FLAG_KEY;
        let user = Uuid::new_v4().to_string();
        let other_user = Uuid::new_v4().to_string();
        // Simulate an override enabled before PR #1976, with no local observation.
        db.collection::<FeatureFlagOverride>(COLLECTION_NAME)
            .insert_one(override_row(None, flag, FlagTargetKind::Global, None, true))
            .await
            .unwrap();
        // Stay within a single shared rate window even when this suite starts
        // near a minute boundary.
        crate::test_utils::ensure_rate_window_headroom(
            std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(5),
        )
        .await;
        readiness::ready_test_replica(&db).await;
        let replicas =
            db.collection::<bson::Document>(crate::models::security_replica::COLLECTION_NAME);
        replicas.delete_many(doc! {}).await.unwrap();
        assert!(!readiness::report(&db, flag).await.unwrap().ready);
        let warnings = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(AtomicUsize::new(0));
        let subscriber =
            tracing::Dispatch::new(tracing_subscriber::registry().with(CountDiagnostics {
                warnings: warnings.clone(),
                errors: errors.clone(),
            }));
        let reads =
            futures::future::join_all((0..8).map(|_| personal_flag_enabled(&db, &user, flag)))
                .with_subscriber(subscriber.clone())
                .await;
        assert!(reads.into_iter().all(|result| matches!(result, Ok(true))));
        assert!(
            personal_flag_enabled(&db, &other_user, flag)
                .with_subscriber(subscriber.clone())
                .await
                .unwrap()
        );
        // Model a second replica by clearing only the local throttle. The
        // shared flag-keyed window must still suppress a different user.
        reset_unready_flag_throttle();
        assert!(
            personal_flag_enabled(&db, &other_user, flag)
                .with_subscriber(subscriber.clone())
                .await
                .unwrap()
        );
        assert_eq!(warnings.load(Ordering::SeqCst), 1);
        assert_eq!(errors.load(Ordering::SeqCst), 0);
        let audits = db.collection::<AuditLog>(crate::models::audit_log::COLLECTION_NAME);
        let events: Vec<_> = audits
            .find(doc! {"event_type": UNREADY_FLAG_EVENT})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(events.len(), 1, "all users share one flag-keyed window");
        assert_eq!(events[0].user_id.as_deref(), Some(user.as_str()));
        assert_eq!(events[0].event_data.as_ref().unwrap()["flag_key"], flag);
        assert_eq!(events[0].event_data.as_ref().unwrap()["user_id"], user);
        // A malformed fresh observation makes report() fail independently of
        // flag resolution. Diagnostic failures must still preserve the flag.
        replicas
            .insert_one(doc! {"_id": "malformed", "observed_at": bson::DateTime::now()})
            .await
            .unwrap();
        assert!(readiness::report(&db, flag).await.is_err());
        let windows = db
            .collection::<bson::Document>(crate::models::coordination::RATE_WINDOW_COLLECTION_NAME);
        windows
            .update_one(
                doc! {"namespace": UNREADY_FLAG_EVENT},
                doc! {"$set": {"count": "malformed"}},
            )
            .await
            .unwrap();
        // The local throttle must skip both the malformed window and report.
        assert!(
            personal_flag_enabled(&db, &user, flag)
                .with_subscriber(subscriber.clone())
                .await
                .unwrap()
        );
        assert_eq!(errors.load(Ordering::SeqCst), 0);
        reset_unready_flag_throttle();
        assert!(
            personal_flag_enabled(&db, &user, flag)
                .with_subscriber(subscriber.clone())
                .await
                .unwrap()
        );
        assert_eq!(errors.load(Ordering::SeqCst), 1);
        reset_unready_flag_throttle();
        windows
            .delete_many(doc! {"namespace": UNREADY_FLAG_EVENT})
            .await
            .unwrap();
        assert!(
            personal_flag_enabled(&db, &user, flag)
                .with_subscriber(subscriber)
                .await
                .unwrap()
        );
        assert_eq!(errors.load(Ordering::SeqCst), 2);
        assert_eq!(warnings.load(Ordering::SeqCst), 1);
        assert_eq!(
            audits
                .count_documents(doc! {"event_type": UNREADY_FLAG_EVENT})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[test]
    fn registry_keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for def in FEATURE_FLAGS {
            assert!(seen.insert(def.key), "duplicate flag key {}", def.key);
        }
    }

    #[test]
    fn remote_machines_defaults_off_and_personal_overrides_stay_actor_scoped() {
        let flag = find_flag("experimental:remote-machines")
            .expect("Remote Machines flag is registered in test builds");
        assert!(!flag.default_enabled);
        assert!(
            !resolve_personal_from_overrides(&[], &[], &[], "owner")
                .iter()
                .any(|key| key == flag.key)
        );
        let personal = [override_row(
            None,
            flag.key,
            FlagTargetKind::User,
            Some("owner"),
            true,
        )];
        for (actor, expected) in [("owner", true), ("other-owner", false)] {
            assert_eq!(
                resolve_personal_from_overrides(&personal, &[], &[], actor)
                    .iter()
                    .any(|key| key == flag.key),
                expected,
                "personal flag must resolve for the authenticated actor"
            );
        }
        let global = override_row(None, flag.key, FlagTargetKind::Global, None, true);
        let personal_off = override_row(None, flag.key, FlagTargetKind::User, Some("owner"), false);
        assert!(
            !resolve_personal_from_overrides(&[global, personal_off], &[], &[], "owner")
                .iter()
                .any(|key| key == flag.key),
            "the existing personal override must retain precedence over global rollout"
        );
    }

    #[tokio::test]
    async fn invitation_code_gate_defaults_on_and_uses_only_global_override() {
        let Some(db) = connect_test_database("invitation_code_flag").await else {
            eprintln!("skipping invitation code flag test: no local MongoDB available");
            return;
        };

        assert!(invitation_code_required(&db).await.expect("default"));
        let user_target = FlagTarget::User(Uuid::new_v4().to_string());
        assert!(matches!(
            set_platform_override(&db, INVITATION_CODE_FLAG_KEY, &user_target, false, "actor")
                .await,
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            set_platform_org_override(
                &db,
                &Uuid::new_v4().to_string(),
                INVITATION_CODE_FLAG_KEY,
                false,
                "actor",
            )
            .await,
            Err(AppError::BadRequest(_))
        ));

        set_platform_override(
            &db,
            INVITATION_CODE_FLAG_KEY,
            &FlagTarget::Global,
            false,
            "actor",
        )
        .await
        .expect("disable");
        assert!(!invitation_code_required(&db).await.expect("disabled"));
        set_platform_override(
            &db,
            INVITATION_CODE_FLAG_KEY,
            &FlagTarget::Global,
            true,
            "actor",
        )
        .await
        .expect("re-enable");
        assert!(invitation_code_required(&db).await.expect("enabled"));
    }

    #[test]
    fn default_applies_when_no_override() {
        // example_ui defaults to off.
        let enabled = resolve_from_overrides(&[], &[], "user-1", OrgRole::Member);
        assert!(!enabled.contains(&"example_ui".to_string()));
    }

    #[test]
    fn org_override_beats_default() {
        let org = vec![override_row(
            Some("org"),
            "example_ui",
            FlagTargetKind::Org,
            None,
            true,
        )];
        let enabled = resolve_from_overrides(&[], &org, "user-1", OrgRole::Member);
        assert!(enabled.contains(&"example_ui".to_string()));
    }

    #[test]
    fn global_beats_default_and_org_beats_global() {
        let global = vec![override_row(
            None,
            "example_ui",
            FlagTargetKind::Global,
            None,
            true,
        )];
        // Global (on) beats default (off) when there's no org override.
        assert!(
            resolve_from_overrides(&global, &[], "user-1", OrgRole::Member)
                .contains(&"example_ui".to_string())
        );
        // Org (off) wins over global (on) — most specific.
        let org = vec![override_row(
            Some("org"),
            "example_ui",
            FlagTargetKind::Org,
            None,
            false,
        )];
        assert!(
            !resolve_from_overrides(&global, &org, "user-1", OrgRole::Member)
                .contains(&"example_ui".to_string())
        );
    }

    #[test]
    fn role_override_beats_org() {
        let org = vec![
            override_row(Some("org"), "example_ui", FlagTargetKind::Org, None, true),
            override_row(
                Some("org"),
                "example_ui",
                FlagTargetKind::Role,
                Some("viewer"),
                false,
            ),
        ];
        // Viewer: role override (off) wins over org (on).
        let viewer = resolve_from_overrides(&[], &org, "user-1", OrgRole::Viewer);
        assert!(!viewer.contains(&"example_ui".to_string()));
        // Member: no role override, falls back to org (on).
        let member = resolve_from_overrides(&[], &org, "user-1", OrgRole::Member);
        assert!(member.contains(&"example_ui".to_string()));
    }

    #[test]
    fn user_override_beats_role_and_org() {
        let org = vec![
            override_row(Some("org"), "example_ui", FlagTargetKind::Org, None, false),
            override_row(
                Some("org"),
                "example_ui",
                FlagTargetKind::Role,
                Some("member"),
                false,
            ),
            override_row(
                Some("org"),
                "example_ui",
                FlagTargetKind::User,
                Some("user-1"),
                true,
            ),
        ];
        let target = resolve_from_overrides(&[], &org, "user-1", OrgRole::Member);
        assert!(target.contains(&"example_ui".to_string()));
        // A different user in the same role only sees the role/org result (off).
        let other = resolve_from_overrides(&[], &org, "user-2", OrgRole::Member);
        assert!(!other.contains(&"example_ui".to_string()));
    }

    // ── personal (non-org) resolution ─────────────────────────────────────
    fn personal_enabled(
        platform: &[FeatureFlagOverride],
        org_rows: &[FeatureFlagOverride],
        memberships: &[(String, OrgRole)],
        user_id: &str,
    ) -> bool {
        resolve_personal_from_overrides(platform, org_rows, memberships, user_id)
            .contains(&"example_ui".to_string())
    }

    fn member(org: &str, role: OrgRole) -> (String, OrgRole) {
        (org.to_string(), role)
    }

    #[test]
    fn precedence_matrix_matches_in_personal_and_org_contexts() {
        // Exercise absent, disabled, and enabled overrides over both defaults.
        for flag in ["example_ui", BILLING_FLAG_KEY, REMOTE_MACHINES_FLAG_KEY] {
            for global in [None, Some(false), Some(true)] {
                for org in [None, Some(false), Some(true)] {
                    for user in [None, Some(false), Some(true)] {
                        let mut platform = Vec::new();
                        if let Some(value) = global {
                            platform.push(override_row(
                                None,
                                flag,
                                FlagTargetKind::Global,
                                None,
                                value,
                            ));
                        }
                        if let Some(value) = user {
                            platform.push(override_row(
                                None,
                                flag,
                                FlagTargetKind::User,
                                Some("user-1"),
                                value,
                            ));
                        }
                        let org_rows: Vec<_> = org
                            .map(|value| {
                                override_row(Some("org"), flag, FlagTargetKind::Org, None, value)
                            })
                            .into_iter()
                            .collect();
                        let expected = user
                            .or(org)
                            .or(global)
                            .unwrap_or(find_flag(flag).unwrap().default_enabled);
                        let personal = resolve_personal_from_overrides(
                            &platform,
                            &org_rows,
                            &[member("org", OrgRole::Member)],
                            "user-1",
                        );
                        let explicit_org =
                            resolve_from_overrides(&platform, &org_rows, "user-1", OrgRole::Member);
                        for resolved in [personal, explicit_org] {
                            assert_eq!(
                                resolved.iter().any(|key| key == flag),
                                expected,
                                "{flag}: global={global:?}, org={org:?}, user={user:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn personal_default_off() {
        assert!(!personal_enabled(&[], &[], &[], "user-1"));
    }

    #[test]
    fn personal_global_then_user_precedence() {
        let platform = vec![
            override_row(None, "example_ui", FlagTargetKind::Global, None, true),
            override_row(
                None,
                "example_ui",
                FlagTargetKind::User,
                Some("user-1"),
                false,
            ),
        ];
        // user-1: personal override (off) wins over global (on).
        assert!(!personal_enabled(&platform, &[], &[], "user-1"));
        // user-2: no personal override, sees global (on).
        assert!(personal_enabled(&platform, &[], &[], "user-2"));
    }

    #[test]
    fn org_grant_lights_personal_surface() {
        // The prod scenario: org-wide enable must reach the member's personal
        // resolution (sidebar + /assistant read /users/me).
        let org_rows = vec![override_row(
            Some("chronoai"),
            "example_ui",
            FlagTargetKind::Org,
            None,
            true,
        )];
        assert!(personal_enabled(
            &[],
            &org_rows,
            &[member("chronoai", OrgRole::Member)],
            "user-1"
        ));
        // Same rows, but the user is not a member: no grant.
        assert!(!personal_enabled(&[], &org_rows, &[], "user-1"));
    }

    #[test]
    fn org_disable_overrides_global_and_same_scope_grants() {
        let global_on = vec![override_row(
            None,
            "example_ui",
            FlagTargetKind::Global,
            None,
            true,
        )];
        let org_off = vec![override_row(
            Some("org-a"),
            "example_ui",
            FlagTargetKind::Org,
            None,
            false,
        )];
        // An org disable is more specific than the platform-global enable,
        // including on personal surfaces such as /users/me and the sidebar.
        assert!(!personal_enabled(
            &global_on,
            &org_off,
            &[member("org-a", OrgRole::Member)],
            "user-1"
        ));
        // When multiple memberships have the same specificity, a disable
        // wins the tie so another org cannot bypass the kill switch.
        let mixed = vec![
            override_row(
                Some("org-a"),
                "example_ui",
                FlagTargetKind::Org,
                None,
                false,
            ),
            override_row(Some("org-b"), "example_ui", FlagTargetKind::Org, None, true),
        ];
        assert!(!personal_enabled(
            &[],
            &mixed,
            &[
                member("org-a", OrgRole::Member),
                member("org-b", OrgRole::Member)
            ],
            "user-1"
        ));
        assert!(!personal_enabled(
            &global_on,
            &mixed.iter().rev().cloned().collect::<Vec<_>>(),
            &[
                member("org-b", OrgRole::Member),
                member("org-a", OrgRole::Member)
            ],
            "user-1"
        ));
        let user_on = [override_row(
            None,
            "example_ui",
            FlagTargetKind::User,
            Some("user-1"),
            true,
        )];
        assert!(personal_enabled(
            &user_on,
            &mixed,
            &[
                member("org-a", OrgRole::Member),
                member("org-b", OrgRole::Member)
            ],
            "user-1"
        ));
        // With only the disabling org, the flag remains off.
        assert!(!personal_enabled(
            &[],
            &org_off,
            &[member("org-a", OrgRole::Member)],
            "user-1"
        ));
    }

    #[test]
    fn personal_resolution_uses_each_specificity_level_in_order() {
        let global_on = vec![override_row(
            None,
            "example_ui",
            FlagTargetKind::Global,
            None,
            true,
        )];
        let org_rows = vec![
            override_row(
                Some("org-a"),
                "example_ui",
                FlagTargetKind::Org,
                None,
                false,
            ),
            override_row(
                Some("org-a"),
                "example_ui",
                FlagTargetKind::Role,
                Some("member"),
                true,
            ),
            override_row(
                Some("org-a"),
                "example_ui",
                FlagTargetKind::User,
                Some("user-1"),
                false,
            ),
        ];
        let memberships = [member("org-a", OrgRole::Member)];

        // Org-scoped user (off) beats role (on), org (off), and global (on).
        assert!(!personal_enabled(
            &global_on,
            &org_rows,
            &memberships,
            "user-1"
        ));
        // Without the org-scoped user row, role (on) beats org (off).
        let without_org_user: Vec<_> = org_rows
            .iter()
            .filter(|row| row.target_kind != FlagTargetKind::User)
            .cloned()
            .collect();
        assert!(personal_enabled(
            &global_on,
            &without_org_user,
            &memberships,
            "user-1"
        ));
        // Without role or org rows, the global value beats the code default.
        assert!(personal_enabled(&global_on, &[], &memberships, "user-1"));
        // A platform user override is the final, most-specific value.
        let personal_off = vec![
            global_on[0].clone(),
            override_row(
                None,
                "example_ui",
                FlagTargetKind::User,
                Some("user-1"),
                false,
            ),
        ];
        assert!(!personal_enabled(
            &personal_off,
            &without_org_user,
            &memberships,
            "user-1"
        ));
        // Platform user overrides also win over legacy org user rows in org context.
        let personal_on = [override_row(
            None,
            "example_ui",
            FlagTargetKind::User,
            Some("user-1"),
            true,
        )];
        assert!(
            resolve_from_overrides(&personal_on, &org_rows, "user-1", OrgRole::Member)
                .contains(&"example_ui".to_string())
        );
    }

    #[test]
    fn org_grant_respects_role_and_member_specificity() {
        // Org-wide on, but this member's role is explicitly excluded.
        let rows = vec![
            override_row(Some("org-a"), "example_ui", FlagTargetKind::Org, None, true),
            override_row(
                Some("org-a"),
                "example_ui",
                FlagTargetKind::Role,
                Some("viewer"),
                false,
            ),
            override_row(
                Some("org-a"),
                "example_ui",
                FlagTargetKind::User,
                Some("user-2"),
                true,
            ),
        ];
        // Viewer role: role override withholds the org grant.
        assert!(!personal_enabled(
            &[],
            &rows,
            &[member("org-a", OrgRole::Viewer)],
            "user-1"
        ));
        // But a per-member enable inside the org beats the role exclusion.
        assert!(personal_enabled(
            &[],
            &rows,
            &[member("org-a", OrgRole::Viewer)],
            "user-2"
        ));
        // Member role: unaffected by the viewer exclusion.
        assert!(personal_enabled(
            &[],
            &rows,
            &[member("org-a", OrgRole::Member)],
            "user-1"
        ));
    }

    #[test]
    fn personal_user_override_is_final_over_org_grants() {
        let platform = vec![override_row(
            None,
            "example_ui",
            FlagTargetKind::User,
            Some("user-1"),
            false,
        )];
        let org_rows = vec![override_row(
            Some("org-a"),
            "example_ui",
            FlagTargetKind::Org,
            None,
            true,
        )];
        // Personal deny beats an org grant.
        assert!(!personal_enabled(
            &platform,
            &org_rows,
            &[member("org-a", OrgRole::Member)],
            "user-1"
        ));
    }

    #[test]
    fn from_parts_validates() {
        assert_eq!(
            FlagTarget::from_parts(FlagTargetKind::Org, None).unwrap(),
            FlagTarget::Org
        );
        assert_eq!(
            FlagTarget::from_parts(FlagTargetKind::Role, Some("admin")).unwrap(),
            FlagTarget::Role(OrgRole::Admin)
        );
        assert!(FlagTarget::from_parts(FlagTargetKind::Role, Some("owner")).is_err());
        assert!(FlagTarget::from_parts(FlagTargetKind::Role, None).is_err());
        assert!(FlagTarget::from_parts(FlagTargetKind::User, None).is_err());
        assert!(FlagTarget::from_parts(FlagTargetKind::User, Some("  ")).is_err());
    }

    #[test]
    fn runtime_flag_keys_are_bounded_and_path_safe() {
        for key in ["team:new-feature", "my_flag", "1-test", "x"] {
            validate_flag_key(key).unwrap();
        }
        for key in ["", ":oops", "two words", "a/b", "CAPS", "x?y", "x%2Fy", "é"] {
            assert!(validate_flag_key(key).is_err(), "{key}");
        }
        assert!(validate_flag_key(&"a".repeat(129)).is_err());
    }

    #[test]
    fn feature_flag_settings_never_cross_definition_identities() {
        let user = "member";
        let memberships = vec![("org".to_owned(), OrgRole::Member)];
        for (current, stale) in [
            (None, Some("runtime-id")),
            (Some("runtime-id"), None),
            (Some("runtime-id"), Some("older-runtime-id")),
        ] {
            for (org, kind, key) in [
                (None, FlagTargetKind::Global, None),
                (Some("org"), FlagTargetKind::Org, None),
                (Some("org"), FlagTargetKind::Role, Some("member")),
                (Some("org"), FlagTargetKind::User, Some(user)),
                (None, FlagTargetKind::User, Some(user)),
            ] {
                let mut row = override_row(org, "example_ui", kind, key, true);
                row.definition_id = stale.map(str::to_owned);
                let (platform, org_rows) = if org.is_some() {
                    (vec![], vec![row])
                } else {
                    (vec![row], vec![])
                };
                assert!(
                    resolve_definitions_from_overrides(
                        [("example_ui", false, current)],
                        &platform,
                        &org_rows,
                        user,
                        OrgRole::Member,
                    )
                    .is_empty(),
                    "stale {stale:?} must not enable {current:?} in {kind:?}"
                );
                assert!(
                    resolve_personal_definitions(
                        [("example_ui", false, current)],
                        &platform,
                        &org_rows,
                        &memberships,
                        user,
                    )
                    .is_empty()
                );

                let mut platform = platform;
                let mut org_rows = org_rows;
                let matching = platform
                    .first_mut()
                    .or_else(|| org_rows.first_mut())
                    .unwrap();
                matching.definition_id = current.map(str::to_owned);
                assert_eq!(
                    resolve_definitions_from_overrides(
                        [("example_ui", false, current)],
                        &platform,
                        &org_rows,
                        user,
                        OrgRole::Member,
                    ),
                    vec!["example_ui"]
                );
                assert_eq!(
                    resolve_personal_definitions(
                        [("example_ui", false, current)],
                        &platform,
                        &org_rows,
                        &memberships,
                        user,
                    ),
                    vec!["example_ui"]
                );
            }
        }
    }

    #[tokio::test]
    async fn feature_flag_runtime_and_builtin_settings_stay_separate_after_key_reuse() {
        let db = connect_test_database("flag_definition_identity")
            .await
            .expect("test database");
        let definitions = db.collection::<FeatureFlagDefinition>(DEFINITIONS);
        let overrides = db.collection::<FeatureFlagOverride>(COLLECTION_NAME);
        let metadata = db.collection::<FeatureFlagMetadata>(METADATA_COLLECTION);
        let actor = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&actor, UserType::Person))
            .await
            .unwrap();

        // Simulate a deployment introducing a built-in key previously created at runtime.
        let runtime_id = Uuid::new_v4().to_string();
        definitions
            .insert_one(FeatureFlagDefinition {
                id: runtime_id.clone(),
                flag_key: "example_ui".into(),
                description: "Runtime description".into(),
                default_enabled: true,
                created_by: actor.clone(),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        let mut stale = override_row(None, "example_ui", FlagTargetKind::Global, None, true);
        stale.definition_id = Some(runtime_id.clone());
        overrides.insert_one(stale).await.unwrap();
        metadata
            .insert_one(FeatureFlagMetadata {
                id: Uuid::new_v4().to_string(),
                flag_key: "example_ui".into(),
                definition_id: Some(runtime_id.clone()),
                description: Some("Runtime metadata".into()),
                owner: Some(actor.clone()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                updated_by: actor.clone(),
            })
            .await
            .unwrap();
        assert!(
            get_definition(&db, "example_ui")
                .await
                .unwrap()
                .id
                .is_none()
        );
        assert!(
            !resolve_personal_features(&db, &actor)
                .await
                .unwrap()
                .contains(&"example_ui".into())
        );
        assert!(!list_metadata(&db).await.unwrap().contains_key("example_ui"));
        let row = set_platform_override(&db, "example_ui", &FlagTarget::Global, true, &actor)
            .await
            .unwrap();
        assert!(row.definition_id.is_none());
        let row = patch_metadata(
            &db,
            "example_ui",
            Some(Some("Built-in metadata")),
            None,
            &actor,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(row.definition_id.is_none());
        assert_eq!(
            list_metadata(&db).await.unwrap()["example_ui"]
                .description
                .as_deref(),
            Some("Built-in metadata")
        );
        clear_platform_override(&db, "example_ui", &FlagTarget::Global)
            .await
            .unwrap();
        set_metadata(&db, "example_ui", None, None, &actor)
            .await
            .unwrap();
        assert_eq!(
            overrides
                .count_documents(doc! { "flag_key": "example_ui" })
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            metadata
                .count_documents(doc! { "flag_key": "example_ui" })
                .await
                .unwrap(),
            1
        );

        // A new runtime flag must ignore settings left behind by a retired built-in.
        let key = "retired:reused";
        overrides
            .insert_one(override_row(None, key, FlagTargetKind::Global, None, true))
            .await
            .unwrap();
        metadata
            .insert_one(FeatureFlagMetadata {
                id: Uuid::new_v4().to_string(),
                flag_key: key.into(),
                definition_id: None,
                description: Some("Retired metadata".into()),
                owner: Some("Former owner".into()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                updated_by: actor.clone(),
            })
            .await
            .unwrap();
        let fresh = create_definition(&db, key, "Fresh description", false, &actor)
            .await
            .unwrap();
        assert!(
            !resolve_personal_features(&db, &actor)
                .await
                .unwrap()
                .contains(&key.into())
        );
        assert!(!list_metadata(&db).await.unwrap().contains_key(key));
        let row = set_platform_override(&db, key, &FlagTarget::Global, true, &actor)
            .await
            .unwrap();
        assert_eq!(row.definition_id, fresh.id);
        assert!(
            resolve_personal_features(&db, &actor)
                .await
                .unwrap()
                .contains(&key.into())
        );
        let row = set_metadata(&db, key, Some("Fresh metadata"), None, &actor)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.definition_id, fresh.id);
        clear_platform_override(&db, key, &FlagTarget::Global)
            .await
            .unwrap();
        set_metadata(&db, key, None, None, &actor).await.unwrap();
        assert_eq!(
            overrides
                .count_documents(doc! { "flag_key": key })
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            metadata
                .count_documents(doc! { "flag_key": key })
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn runtime_flags_resolve_defaults_and_scoped_overrides() {
        let db = connect_test_database("runtime_flag_resolution")
            .await
            .expect("MongoDB required");
        let user_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_many([
                crate::test_utils::test_user(&user_id, crate::models::user::UserType::Person),
                crate::test_utils::test_user(&org_id, crate::models::user::UserType::Org),
            ])
            .await
            .unwrap();
        create_definition(&db, "runtime:test", "Runtime default", true, &user_id)
            .await
            .unwrap();
        assert!(
            resolve_personal_features(&db, &user_id)
                .await
                .unwrap()
                .contains(&"runtime:test".into())
        );
        set_platform_override(&db, "runtime:test", &FlagTarget::Global, false, &user_id)
            .await
            .unwrap();
        assert!(
            !resolve_personal_features(&db, &user_id)
                .await
                .unwrap()
                .contains(&"runtime:test".into())
        );
        set_platform_org_override(&db, &org_id, "runtime:test", true, &user_id)
            .await
            .unwrap();
        assert!(
            resolve_enabled_features(&db, &org_id, &user_id, OrgRole::Member)
                .await
                .unwrap()
                .contains(&"runtime:test".into())
        );
        set_platform_override(
            &db,
            "runtime:test",
            &FlagTarget::User(user_id.clone()),
            false,
            &user_id,
        )
        .await
        .unwrap();
        assert!(
            !resolve_enabled_features(&db, &org_id, &user_id, OrgRole::Member)
                .await
                .unwrap()
                .contains(&"runtime:test".into())
        );
        patch_metadata(
            &db,
            "runtime:test",
            Some(Some("Edited")),
            Some(Some("Team")),
            &user_id,
        )
        .await
        .unwrap();
        assert_eq!(
            list_metadata(&db).await.unwrap()["runtime:test"]
                .description
                .as_deref(),
            Some("Edited")
        );
        set_metadata(&db, "runtime:test", None, None, &user_id)
            .await
            .unwrap();
        assert_eq!(
            get_definition(&db, "runtime:test")
                .await
                .unwrap()
                .created_by
                .as_deref(),
            Some(user_id.as_str())
        );
    }

    #[test]
    fn find_flag_resolves_registry_keys() {
        assert!(find_flag("does-not-exist").is_none());
        assert!(find_flag("experimental:ai-assistant").is_some());
        assert!(find_flag("example_ui").is_some());
    }

    /// Registry keys are a wire contract with `frontend/src/lib/feature-flags.ts`
    /// (`FEATURE_FLAG`). A key that drifts on one side silently disables the
    /// surface it gates instead of failing, so the literals are pinned on both
    /// sides and a rename must be a deliberate two-sided edit.
    #[test]
    fn shipped_registry_key_literals_are_pinned() {
        let shipped: Vec<&str> = FEATURE_FLAGS
            .iter()
            .map(|def| def.key)
            // Test-only placeholder; it has no frontend counterpart.
            .filter(|key| *key != "example_ui")
            .collect();
        assert_eq!(
            shipped,
            vec![
                "auth:invitation-code",
                "auth:mobile-mfa",
                "assistant:nyxagent-engine",
                "assistant:model-controls",
                "assistant:workspace-preview",
                "assistant:skill-pin-disable",
                "nyxbot:thread-follow",
                "channel:discord-gateway",
                "nyxbot:service-adapters",
                "assistant:operation-scopes",
                "assistant:guest-operation-scopes",
                "assistant:channel-media",
                "assistant:attachment-writes",
                "assistant:live-turn-gate",
                "assistant:owner-invocations",
                "assistant:machine-capabilities",
                "assistant:machine-contexts",
                "assistant:agent-learning",
                "assistant:org-agents",
                "assistant:shared-agents",
                "assistant:voice",
                "assistant:voice-openai-platform",
                "assistant:voice-grok",
                "assistant:voice-grok-platform",
                "experimental:ai-assistant",
                "experimental:billing",
                "experimental:remote-machines",
                "experimental:aevatar-chat-wire-log",
                "experimental:direct-chat-engine",
                "nyxbot:gateway-lark",
                "nyxbot:gateway-feishu",
                "nyxbot:gateway-discord",
                "nyxbot:gateway-slack",
                "nyxbot:gateway-whatsapp",
                "nyxbot:gateway-x",
                "nyxbot:gateway-aurinko",
            ]
        );
        assert!(
            !find_flag(ASSISTANT_MODEL_CONTROLS_FLAG_KEY)
                .expect("model controls flag is registered")
                .default_enabled,
            "advanced model controls must default to off"
        );
        assert_eq!(
            AEVATAR_CHAT_WIRE_LOG_FLAG_KEY,
            "experimental:aevatar-chat-wire-log"
        );
        assert!(
            !find_flag(ASSISTANT_VOICE_FLAG_KEY)
                .expect("voice flag is registered")
                .default_enabled,
            "voice must default to off"
        );
        assert!(
            !find_flag(AEVATAR_CHAT_WIRE_LOG_FLAG_KEY)
                .expect("wire-log flag is registered")
                .default_enabled,
            "the wire-log diagnostic must default to off"
        );
    }

    #[tokio::test]
    async fn platform_org_override_crud() {
        let Some(db) = connect_test_database("feature_flag_crud").await else {
            eprintln!("skipping feature flag service test: no local MongoDB available");
            return;
        };
        let org_id = Uuid::new_v4().to_string();
        let actor = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .expect("insert org user");

        let stored = set_platform_org_override(&db, &org_id, "example_ui", true, &actor)
            .await
            .expect("set org override");
        assert_eq!(stored.org_user_id.as_deref(), Some(org_id.as_str()));
        assert_eq!(stored.flag_key, "example_ui");
        assert_eq!(stored.target_kind, FlagTargetKind::Org);
        assert!(stored.target_key.is_none());
        assert!(stored.enabled);
        assert_eq!(stored.updated_by, actor);

        // Resolution reflects the stored override (on).
        let resolved = resolve_enabled_features(&db, &org_id, &actor, OrgRole::Member)
            .await
            .expect("resolve");
        assert!(resolved.contains(&"example_ui".to_string()));

        // Idempotent upsert keeps the same row id, flips value.
        let updated = set_platform_org_override(&db, &org_id, "example_ui", false, &actor)
            .await
            .expect("update org override");
        assert_eq!(updated.id, stored.id);
        assert!(!updated.enabled);
        let resolved = resolve_enabled_features(&db, &org_id, &actor, OrgRole::Member)
            .await
            .expect("resolve after flip");
        assert!(!resolved.contains(&"example_ui".to_string()));

        // The row lands in the org's own override set and the cross-org admin
        // listing, with display enrichment.
        let org_rows = list_overrides(&db, &org_id).await.expect("org rows");
        assert_eq!(org_rows.len(), 1);
        let all = list_all_org_scope_overrides(&db)
            .await
            .expect("all org-scope rows");
        assert!(all.iter().any(|r| r.id == stored.id));
        let display = fetch_override_org_display(&db, &all)
            .await
            .expect("org display");
        assert_eq!(
            display.get(&org_id).and_then(|o| o.display_name.as_deref()),
            Some("Test Org")
        );

        // Clear removes it.
        let removed = clear_platform_org_override(&db, &org_id, "example_ui")
            .await
            .expect("clear");
        assert_eq!(removed.map(|r| r.id), Some(stored.id));
        let after = list_overrides(&db, &org_id)
            .await
            .expect("list after clear");
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn set_platform_org_override_rejects_unknown_flag_key() {
        let Some(db) = connect_test_database("feature_flag_unknown").await else {
            eprintln!("skipping feature flag service test: no local MongoDB available");
            return;
        };
        let err =
            set_platform_org_override(&db, &Uuid::new_v4().to_string(), "nope", true, "actor")
                .await
                .expect_err("unknown flag rejected");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn workspace_preview_defaults_off_and_supports_targeted_user_rollout() {
        let db = crate::test_utils::connect_transaction_test_database("workspace_preview_audience")
            .await;
        let actor = Uuid::new_v4().to_string();
        let selected_id = Uuid::new_v4().to_string();
        let ordinary_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&selected_id, UserType::Person),
                test_user(&ordinary_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .unwrap();
        for id in [&selected_id, &ordinary_id] {
            assert!(
                !resolve_personal_features(&db, id)
                    .await
                    .unwrap()
                    .iter()
                    .any(|key| key == ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY)
            );
        }
        set_platform_override(
            &db,
            ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY,
            &FlagTarget::User(selected_id.clone()),
            true,
            &actor,
        )
        .await
        .unwrap();
        for (id, expected) in [(&selected_id, true), (&ordinary_id, false)] {
            assert_eq!(
                resolve_personal_features(&db, id)
                    .await
                    .unwrap()
                    .iter()
                    .any(|key| key == ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY),
                expected
            );
            assert_eq!(
                resolve_enabled_features(&db, &org_id, id, OrgRole::Member)
                    .await
                    .unwrap()
                    .iter()
                    .any(|key| key == ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY),
                expected
            );
        }
        set_platform_override(
            &db,
            ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY,
            &FlagTarget::User(selected_id.clone()),
            false,
            &actor,
        )
        .await
        .unwrap();
        assert!(
            !resolve_personal_features(&db, &selected_id)
                .await
                .unwrap()
                .iter()
                .any(|key| key == ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY)
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_override_resolves_for_non_org_user() {
        let Some(db) = connect_test_database("feature_flag_platform").await else {
            eprintln!("skipping feature flag service test: no local MongoDB available");
            return;
        };
        let user = Uuid::new_v4().to_string();
        let actor = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user, UserType::Person))
            .await
            .expect("insert target user");

        // A user with zero orgs starts with the code default (off).
        assert!(
            !resolve_personal_features(&db, &user)
                .await
                .expect("resolve personal")
                .contains(&"example_ui".to_string())
        );

        // Global rollout enables it for everyone, org or not.
        let g = set_platform_override(&db, "example_ui", &FlagTarget::Global, true, &actor)
            .await
            .expect("set global");
        assert!(g.org_user_id.is_none());
        assert!(
            resolve_personal_features(&db, &user)
                .await
                .expect("resolve personal")
                .contains(&"example_ui".to_string())
        );

        // A personal per-user override (off) wins over global (on).
        set_platform_override(
            &db,
            "example_ui",
            &FlagTarget::User(user.clone()),
            false,
            &actor,
        )
        .await
        .expect("set personal user");
        assert!(
            !resolve_personal_features(&db, &user)
                .await
                .expect("resolve personal")
                .contains(&"example_ui".to_string())
        );

        // org/role targets are rejected at the platform layer.
        assert!(matches!(
            set_platform_override(&db, "example_ui", &FlagTarget::Org, true, &actor).await,
            Err(AppError::BadRequest(_))
        ));

        // Clearing the global override removes it.
        assert!(
            clear_platform_override(&db, "example_ui", &FlagTarget::Global)
                .await
                .expect("clear global")
                .is_some()
        );
    }

    #[tokio::test]
    async fn set_platform_org_override_requires_existing_org() {
        let Some(db) = connect_test_database("feature_flag_org_validation").await else {
            eprintln!("skipping feature flag service test: no local MongoDB available");
            return;
        };
        let actor = Uuid::new_v4().to_string();
        let person = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&person, UserType::Person))
            .await
            .expect("insert person user");

        // Unknown id and a person account both 404 as "not an org".
        assert!(matches!(
            set_platform_org_override(&db, &Uuid::new_v4().to_string(), "example_ui", true, &actor)
                .await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            set_platform_org_override(&db, &person, "example_ui", true, &actor).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn admin_org_override_reaches_member_personal_resolution() {
        use crate::models::org_membership::{
            COLLECTION_NAME as MEMBERSHIPS, MemberScopeSource, OrgMembership,
        };

        let Some(db) = connect_test_database("feature_flag_admin_org").await else {
            eprintln!("skipping feature flag service test: no local MongoDB available");
            return;
        };
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let outsider_id = Uuid::new_v4().to_string();
        let actor = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .expect("insert org user");
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(OrgMembership {
                id: Uuid::new_v4().to_string(),
                org_user_id: org_id.clone(),
                member_user_id: member_id.clone(),
                role: OrgRole::Member,
                scope_source: MemberScopeSource::Inherit,
                allowed_service_ids: None,
                created_at: Utc::now(),
                revoked_at: None,
            })
            .await
            .expect("insert membership");

        // The prod scenario: platform admin enables the flag org-wide without
        // being an org member; the member's personal resolution picks it up.
        let row = set_platform_org_override(&db, &org_id, "example_ui", true, &actor)
            .await
            .expect("admin org enable");
        assert_eq!(row.org_user_id.as_deref(), Some(org_id.as_str()));
        assert_eq!(row.target_kind, FlagTargetKind::Org);
        assert!(
            resolve_personal_features(&db, &member_id)
                .await
                .expect("resolve member")
                .contains(&"example_ui".to_string())
        );
        // A non-member sees nothing from the org grant.
        assert!(
            !resolve_personal_features(&db, &outsider_id)
                .await
                .expect("resolve outsider")
                .contains(&"example_ui".to_string())
        );

        // Clear removes the grant; resolution falls back to default (off).
        assert!(
            clear_platform_org_override(&db, &org_id, "example_ui")
                .await
                .expect("admin org clear")
                .is_some()
        );
        assert!(
            !resolve_personal_features(&db, &member_id)
                .await
                .expect("resolve after clear")
                .contains(&"example_ui".to_string())
        );
    }

    #[tokio::test]
    async fn mobile_mfa_required_resolves_live_org_override_for_person() {
        let db =
            crate::test_utils::connect_transaction_test_database("mobile_mfa_org_override").await;
        let org = Uuid::new_v4().to_string();
        let person = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        db.collection::<User>(USERS)
            .insert_one(test_user(&person, UserType::Person))
            .await
            .unwrap();
        db.collection::<crate::models::org_membership::OrgMembership>(
            crate::models::org_membership::COLLECTION_NAME,
        )
        .insert_one(crate::test_utils::test_membership(
            &org,
            &person,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
        assert!(!mobile_mfa_required(&db, &person).await.unwrap());
        set_platform_org_override(&db, &org, MOBILE_MFA_FLAG_KEY, true, &person)
            .await
            .unwrap();
        assert!(mobile_mfa_required(&db, &person).await.unwrap());
        db.collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
            .update_one(
                doc! {"org_user_id": &org, "member_user_id": &person},
                doc! {"$set": {"revoked_at": bson::DateTime::now()}},
            )
            .await
            .unwrap();
        assert!(!mobile_mfa_required(&db, &person).await.unwrap());
        db.drop().await.unwrap();
    }

    #[test]
    fn metadata_fields_normalize_and_bound() {
        assert_eq!(normalize_metadata_field(None, 10, "owner").unwrap(), None);
        assert_eq!(
            normalize_metadata_field(Some("   "), 10, "owner").unwrap(),
            None
        );
        assert_eq!(
            normalize_metadata_field(Some("  team  "), 10, "owner").unwrap(),
            Some("team".to_string())
        );
        // The budget counts characters, not bytes, so multi-byte owners get
        // the same allowance as ASCII ones.
        assert_eq!(
            normalize_metadata_field(Some("平台团队"), 4, "owner").unwrap(),
            Some("平台团队".to_string())
        );
        assert!(matches!(
            normalize_metadata_field(Some("平台团队五"), 4, "owner"),
            Err(AppError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn metadata_crud_round_trip() {
        let Some(db) = connect_test_database("feature_flag_metadata").await else {
            eprintln!("skipping feature flag metadata test: no local MongoDB available");
            return;
        };
        let actor = Uuid::new_v4().to_string();

        // No row until an admin writes one.
        assert!(list_metadata(&db).await.expect("empty list").is_empty());

        let stored = set_metadata(
            &db,
            "example_ui",
            Some("  Gates the redesigned panel.  "),
            Some("  Platform team  "),
            &actor,
        )
        .await
        .expect("set metadata")
        .expect("row present");
        assert_eq!(
            stored.description.as_deref(),
            Some("Gates the redesigned panel.")
        );
        assert_eq!(stored.owner.as_deref(), Some("Platform team"));
        assert_eq!(stored.updated_by, actor);

        // Full-replace: an omitted field is cleared, and the row keeps its id.
        let updated = set_metadata(&db, "example_ui", None, Some("Growth team"), &actor)
            .await
            .expect("update metadata")
            .expect("row present");
        assert_eq!(updated.id, stored.id);
        assert!(updated.description.is_none());
        assert_eq!(updated.owner.as_deref(), Some("Growth team"));

        let listed = list_metadata(&db).await.expect("list metadata");
        assert_eq!(
            listed
                .get("example_ui")
                .and_then(|row| row.owner.as_deref()),
            Some("Growth team")
        );

        // Clearing every field deletes the row rather than leaving a husk.
        assert!(
            set_metadata(&db, "example_ui", Some("  "), None, &actor)
                .await
                .expect("clear metadata")
                .is_none()
        );
        assert!(
            list_metadata(&db)
                .await
                .expect("list after clear")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn metadata_rejects_unknown_flag_and_overlong_fields() {
        let Some(db) = connect_test_database("feature_flag_metadata_validation").await else {
            eprintln!("skipping feature flag metadata validation test: no local MongoDB available");
            return;
        };
        let actor = Uuid::new_v4().to_string();

        assert!(matches!(
            set_metadata(&db, "nope", Some("x"), None, &actor).await,
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            set_metadata(
                &db,
                "example_ui",
                Some(&"x".repeat(MAX_FLAG_DESCRIPTION_LEN + 1)),
                None,
                &actor,
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            set_metadata(
                &db,
                "example_ui",
                None,
                Some(&"x".repeat(MAX_FLAG_OWNER_LEN + 1)),
                &actor,
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn metadata_for_retired_flags_is_not_listed() {
        let Some(db) = connect_test_database("feature_flag_metadata_retired").await else {
            eprintln!("skipping feature flag retired metadata test: no local MongoDB available");
            return;
        };
        // A row left behind by a flag that has since been deleted from the
        // registry must never surface as if the flag still existed.
        db.collection::<FeatureFlagMetadata>(METADATA_COLLECTION)
            .insert_one(FeatureFlagMetadata {
                definition_id: None,
                id: Uuid::new_v4().to_string(),
                flag_key: "retired_flag".to_string(),
                description: Some("Long gone.".to_string()),
                owner: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                updated_by: "actor".to_string(),
            })
            .await
            .expect("insert stale row");
        assert!(
            !list_metadata(&db)
                .await
                .expect("list metadata")
                .contains_key("retired_flag")
        );
    }

    #[tokio::test]
    async fn revoked_membership_does_not_grant() {
        use crate::models::org_membership::{
            COLLECTION_NAME as MEMBERSHIPS, MemberScopeSource, OrgMembership,
        };

        let Some(db) = connect_test_database("feature_flag_revoked_member").await else {
            eprintln!("skipping feature flag service test: no local MongoDB available");
            return;
        };
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let actor = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .expect("insert org user");
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(OrgMembership {
                id: Uuid::new_v4().to_string(),
                org_user_id: org_id.clone(),
                member_user_id: member_id.clone(),
                role: OrgRole::Member,
                scope_source: MemberScopeSource::Inherit,
                allowed_service_ids: None,
                created_at: Utc::now(),
                revoked_at: Some(Utc::now()),
            })
            .await
            .expect("insert revoked membership");

        set_platform_org_override(&db, &org_id, "example_ui", true, &actor)
            .await
            .expect("admin org enable");
        assert!(
            !resolve_personal_features(&db, &member_id)
                .await
                .expect("resolve revoked member")
                .contains(&"example_ui".to_string())
        );
    }

    #[tokio::test]
    async fn admin_form_metadata_patch_preserves_disjoint_writes_and_clears() {
        let db = connect_test_database("admin_form_metadata_patch")
            .await
            .expect("Mongo required");
        set_metadata(&db, "example_ui", Some("Old"), Some("Original owner"), "a")
            .await
            .unwrap();
        patch_metadata(&db, "example_ui", None, Some(Some("New owner")), "b")
            .await
            .unwrap();
        let saved = patch_metadata(&db, "example_ui", Some(Some("New description")), None, "a")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.owner.as_deref(), Some("New owner"));
        let saved = patch_metadata(&db, "example_ui", Some(None), None, "a")
            .await
            .unwrap()
            .unwrap();
        assert!(saved.description.is_none());
        assert_eq!(saved.owner.as_deref(), Some("New owner"));
        assert!(
            patch_metadata(&db, "example_ui", None, Some(Some("  ")), "a")
                .await
                .unwrap()
                .is_none()
        );
        assert!(!list_metadata(&db).await.unwrap().contains_key("example_ui"));
        let saved = patch_metadata(&db, "example_ui", None, Some(Some("Restored")), "b")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.owner.as_deref(), Some("Restored"));
        let unchanged = patch_metadata(&db, "example_ui", None, None, "a")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.updated_at, saved.updated_at);
        // PUT remains a full replacement.
        let replaced = set_metadata(&db, "example_ui", Some("PUT"), None, "a")
            .await
            .unwrap()
            .unwrap();
        assert!(replaced.owner.is_none());
    }

    #[test]
    fn channel_media_and_attachment_writes_are_registered_default_off() {
        for key in [
            ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
            ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY,
        ] {
            let flag = FEATURE_FLAGS.iter().find(|flag| flag.key == key).unwrap();
            assert!(!flag.default_enabled);
        }
    }
}
