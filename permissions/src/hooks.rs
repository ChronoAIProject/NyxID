//! Trusted, asynchronous check hooks. An allow continues the normal policy
//! pipeline; it never replaces authorization. Hook inputs are immutable and
//! exclude transport credentials. User-supplied code is not sandboxed here.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Plan, Policy, Response};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookStage {
    BeforeExecute,
    AfterResponse,
}

impl std::fmt::Display for HookStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BeforeExecute => "before_execute",
            Self::AfterResponse => "after_response",
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookDefinition {
    pub name: String,
    pub handler: String,
    pub stage: HookStage,
    /// Empty means every operation allowed by the enclosing policy.
    #[serde(default)]
    pub operations: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    pub config: Value,
}

fn default_timeout() -> u64 {
    500
}

pub struct HookContext<'a> {
    pub policy: &'a Policy,
    pub plan: &'a Plan,
    /// Already sanitized by the provider adapter; absent before execution.
    pub response: Option<&'a Response>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HookDecision {
    Allow,
    Deny,
}

/// No free-form error text can expose resource content or secrets to callers.
#[derive(Debug)]
pub struct HookFailure;

#[async_trait]
pub trait PermissionHook: Send + Sync {
    fn validate(&self, stage: HookStage, config: &Value) -> Result<(), Error>;
    async fn check(
        &self,
        context: HookContext<'_>,
        config: &Value,
    ) -> Result<HookDecision, HookFailure>;
}

#[derive(Default)]
pub struct HookRegistry {
    handlers: HashMap<String, Arc<dyn PermissionHook>>,
}

impl HookRegistry {
    pub fn builtins() -> Self {
        Self {
            handlers: HashMap::from([
                (
                    "pause_switch".into(),
                    Arc::new(PauseSwitch) as Arc<dyn PermissionHook>,
                ),
                (
                    "response_markers".into(),
                    Arc::new(ResponseMarkers) as Arc<dyn PermissionHook>,
                ),
            ]),
        }
    }

    pub fn register(&mut self, name: &str, handler: Arc<dyn PermissionHook>) -> Result<(), Error> {
        if !valid_name(name) || self.handlers.contains_key(name) {
            return Err(Error::Policy("invalid or duplicate hook handler"));
        }
        self.handlers.insert(name.into(), handler);
        Ok(())
    }

    pub(crate) fn bind(self, policy: &Policy) -> Result<Vec<BoundHook>, Error> {
        if policy.hooks.len() > 8 {
            return Err(Error::Policy("maximum eight hooks per policy"));
        }
        let mut names = HashSet::new();
        policy
            .hooks
            .iter()
            .map(|definition| {
                if !valid_name(&definition.name)
                    || !names.insert(&definition.name)
                    || !(1..=2000).contains(&definition.timeout_ms)
                    || definition.operations.len() > 64
                    || definition
                        .operations
                        .iter()
                        .any(|op| !policy.allowed_operations.contains(op))
                {
                    return Err(Error::Policy(
                        "invalid hook name, timeout or operation selector",
                    ));
                }
                let handler = self
                    .handlers
                    .get(&definition.handler)
                    .ok_or(Error::Policy("unknown hook handler"))?;
                handler.validate(definition.stage, &definition.config)?;
                Ok(BoundHook {
                    definition: definition.clone(),
                    handler: handler.clone(),
                })
            })
            .collect()
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub(crate) struct BoundHook {
    definition: HookDefinition,
    handler: Arc<dyn PermissionHook>,
}

pub(crate) async fn run(
    hooks: &[BoundHook],
    stage: HookStage,
    policy: &Policy,
    plan: &Plan,
    response: Option<&Response>,
) -> Result<(), Error> {
    for hook in hooks {
        let definition = &hook.definition;
        if definition.stage != stage
            || !definition.operations.is_empty()
                && !definition.operations.iter().any(|op| op == &plan.operation)
        {
            continue;
        }
        let context = HookContext {
            policy,
            plan,
            response,
        };
        let outcome = tokio::time::timeout(
            Duration::from_millis(definition.timeout_ms),
            std::panic::AssertUnwindSafe(hook.handler.check(context, &definition.config))
                .catch_unwind(),
        )
        .await;
        match outcome {
            Ok(Ok(Ok(HookDecision::Allow))) => {}
            Ok(Ok(Ok(HookDecision::Deny))) => {
                return Err(Error::HookDenied {
                    hook: definition.name.clone(),
                    stage,
                });
            }
            _ => {
                return Err(Error::HookUnavailable {
                    hook: definition.name.clone(),
                    stage,
                });
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PauseConfig {
    path: PathBuf,
}

struct PauseSwitch;

#[async_trait]
impl PermissionHook for PauseSwitch {
    fn validate(&self, stage: HookStage, config: &Value) -> Result<(), Error> {
        let config: PauseConfig = serde_json::from_value(config.clone())
            .map_err(|_| Error::Policy("invalid pause_switch config"))?;
        if stage != HookStage::BeforeExecute
            || !config.path.is_absolute()
            || config.path.file_name().is_none()
            || config
                .path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(Error::Policy(
                "pause_switch requires before_execute and an absolute file path",
            ));
        }
        Ok(())
    }

    async fn check(
        &self,
        _context: HookContext<'_>,
        config: &Value,
    ) -> Result<HookDecision, HookFailure> {
        let config: PauseConfig =
            serde_json::from_value(config.clone()).map_err(|_| HookFailure)?;
        let parent = config.path.parent().ok_or(HookFailure)?;
        if !tokio::fs::metadata(parent)
            .await
            .map_err(|_| HookFailure)?
            .is_dir()
        {
            return Err(HookFailure);
        }
        // symlink_metadata also counts a dangling symlink as a pause marker.
        // The marker's contents and target are never opened.
        match tokio::fs::symlink_metadata(&config.path).await {
            Ok(_) => Ok(HookDecision::Deny),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HookDecision::Allow),
            Err(_) => Err(HookFailure),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkersConfig {
    markers: Vec<String>,
}

struct ResponseMarkers;

#[async_trait]
impl PermissionHook for ResponseMarkers {
    fn validate(&self, stage: HookStage, config: &Value) -> Result<(), Error> {
        let config: MarkersConfig = serde_json::from_value(config.clone())
            .map_err(|_| Error::Policy("invalid response_markers config"))?;
        if stage != HookStage::AfterResponse
            || config.markers.is_empty()
            || config.markers.len() > 16
            || config
                .markers
                .iter()
                .any(|marker| marker.is_empty() || marker.len() > 128)
        {
            return Err(Error::Policy(
                "response_markers requires after_response and 1–16 markers of 1–128 bytes",
            ));
        }
        Ok(())
    }

    async fn check(
        &self,
        context: HookContext<'_>,
        config: &Value,
    ) -> Result<HookDecision, HookFailure> {
        let config: MarkersConfig =
            serde_json::from_value(config.clone()).map_err(|_| HookFailure)?;
        let body = &context.response.ok_or(HookFailure)?.body;
        // This is a literal-byte demonstration, not a document/DLP parser.
        // Overlap covers markers spanning chunks; yields make deadlines useful.
        for marker in config.markers {
            for start in (0..body.len()).step_by(64 * 1024) {
                let end = body.len().min(start + 64 * 1024 + marker.len() - 1);
                if memchr::memmem::find(&body[start..end], marker.as_bytes()).is_some() {
                    return Ok(HookDecision::Deny);
                }
                tokio::task::yield_now().await;
            }
        }
        Ok(HookDecision::Allow)
    }
}
