//! Experimental permission engine. All transports must call `Engine::execute`;
//! provider adapters interpret resources, while the engine owns common limits.

pub mod drive;
pub mod google;
pub mod hooks;
pub mod http_transport;
pub mod mock;
pub mod server;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use bytes::Bytes;
use http::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid permission policy: {0}")]
    Policy(&'static str),
    #[error("request denied: {0}")]
    Denied(&'static str),
    #[error("resource verification unavailable")]
    Verification,
    #[error("upstream request failed or was rejected")]
    Upstream,
    #[error("permission request timed out")]
    Timeout,
    #[error("response exceeds the proof-of-concept byte limit")]
    ResponseTooLarge,
    #[error("write outcome is uncertain; inspect the resource before retrying")]
    OutcomeUncertain,
    #[error("permission evaluator is at capacity")]
    Busy,
    #[error("request denied by {stage} hook '{hook}'")]
    HookDenied {
        hook: String,
        stage: hooks::HookStage,
    },
    #[error("{stage} hook '{hook}' failed or timed out")]
    HookUnavailable {
        hook: String,
        stage: hooks::HookStage,
    },
    #[error(
        "upstream accepted the write, but its response was withheld; inspect state before retrying"
    )]
    WriteAcceptedResponseWithheld,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u8,
    pub name: String,
    pub provider: String,
    pub resource: ResourceBoundary,
    pub allowed_operations: Vec<String>,
    #[serde(default)]
    pub constraints: BTreeMap<String, Vec<ParameterConstraint>>,
    #[serde(default)]
    pub hooks: Vec<hooks::HookDefinition>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceBoundary {
    GoogleDriveFolder {
        root_folder_id: String,
        include_descendants: bool,
    },
    GoogleApi {
        operations: Vec<google::ApiOperation>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterConstraint {
    pub location: ParameterLocation,
    pub name: String,
    #[serde(default)]
    pub required: bool,
    pub rule: ValueRule,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterLocation {
    Query,
    Body,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueRule {
    Exact { value: Value },
    OneOf { values: Vec<Value> },
    MaxInteger { value: u64 },
    MaxLength { value: usize },
}

#[derive(Clone, Debug)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub body: Bytes,
    pub content_type: Option<String>,
}

impl Request {
    pub fn get(path: impl Into<String>) -> Self {
        Self {
            method: Method::GET,
            path: path.into(),
            query: BTreeMap::new(),
            body: Bytes::new(),
            content_type: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Bytes,
}

#[derive(Clone, Debug)]
pub struct ResourceCheck {
    pub id: String,
    pub must_be_folder: bool,
    pub allow_root: bool,
}

pub struct Plan {
    pub operation: String,
    /// A policy-owned recipient, never supplied by the calling agent.
    pub origin: Option<String>,
    pub request: Request,
    pub resources: Vec<ResourceCheck>,
}

#[async_trait]
pub trait Transport: Send + Sync {
    /// Must pin the destination and credential, reject redirects, bound the
    /// response, and never forward caller-controlled authentication headers.
    async fn send(&self, request: &Request) -> Result<Response, Error>;

    /// Hosts supporting generic APIs must attest to the final selected origin.
    /// Existing transports fail closed until they implement that contract.
    async fn send_to(&self, request: &Request, origin: Option<&str>) -> Result<Response, Error> {
        if origin.is_some() {
            return Err(Error::Denied("transport cannot enforce the API origin"));
        }
        self.send(request).await
    }
}

pub fn adapter_for_policy(policy: &Policy) -> Result<Arc<dyn ProviderAdapter>, Error> {
    match (&*policy.provider, &policy.resource) {
        ("google_drive", ResourceBoundary::GoogleDriveFolder { .. }) => {
            Ok(Arc::new(drive::DriveAdapter))
        }
        ("google", ResourceBoundary::GoogleApi { .. }) => Ok(Arc::new(google::GoogleApiAdapter)),
        _ => Err(Error::Policy("provider and resource boundary do not match")),
    }
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn validate_policy(&self, policy: &Policy) -> Result<(), Error>;
    fn plan(&self, policy: &Policy, request: Request) -> Result<Plan, Error>;
    async fn verify(
        &self,
        policy: &Policy,
        plan: &Plan,
        transport: &dyn Transport,
    ) -> Result<(), Error>;
    fn sanitize_response(&self, plan: &Plan, response: Response) -> Result<Response, Error>;
}

pub struct Engine {
    policy: Policy,
    adapter: Arc<dyn ProviderAdapter>,
    transport: Arc<dyn Transport>,
    concurrency: Semaphore,
    hooks: Vec<hooks::BoundHook>,
}

impl Engine {
    pub fn new(
        policy: Policy,
        adapter: Arc<dyn ProviderAdapter>,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, Error> {
        Self::new_with_hooks(policy, adapter, transport, hooks::HookRegistry::builtins())
    }

    /// Hooks are installed by the trusted host. Requests cannot select handlers,
    /// replace configuration, change prepared inputs, or override a base denial.
    pub fn new_with_hooks(
        policy: Policy,
        adapter: Arc<dyn ProviderAdapter>,
        transport: Arc<dyn Transport>,
        registry: hooks::HookRegistry,
    ) -> Result<Self, Error> {
        if policy.version != 1 || policy.name.is_empty() || policy.name.len() > 128 {
            return Err(Error::Policy(
                "expected version 1 and a name of 1–128 bytes",
            ));
        }
        if policy.allowed_operations.is_empty() || policy.allowed_operations.len() > 64 {
            return Err(Error::Policy("expected 1–64 allowed operations"));
        }
        if policy.constraints.len() > 64 {
            return Err(Error::Policy("too many operation constraints"));
        }
        if serde_json::to_vec(&policy)
            .map_err(|_| Error::Policy("invalid policy"))?
            .len()
            > 128 * 1024
        {
            return Err(Error::Policy("policy exceeds 128 KiB"));
        }
        for (operation, constraints) in &policy.constraints {
            if !policy.allowed_operations.contains(operation) || constraints.len() > 32 {
                return Err(Error::Policy(
                    "constraints must name an allowed operation; maximum 32 each",
                ));
            }
            for constraint in constraints {
                if constraint.name.len() > 256
                    || matches!(constraint.location, ParameterLocation::Body)
                        && !valid_pointer(&constraint.name)
                {
                    return Err(Error::Policy(
                        "invalid constraint parameter or JSON Pointer",
                    ));
                }
                if matches!(&constraint.rule, ValueRule::OneOf { values } if values.is_empty() || values.len() > 100)
                {
                    return Err(Error::Policy("one_of requires 1–100 values"));
                }
            }
        }
        adapter.validate_policy(&policy)?;
        let hooks = registry.bind(&policy)?;
        Ok(Self {
            policy,
            adapter,
            transport,
            concurrency: Semaphore::new(32),
            hooks,
        })
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub async fn execute(&self, request: Request) -> Result<Response, Error> {
        let mut dispatched = false;
        let mut write_accepted = false;
        let mutating = !matches!(request.method, Method::GET | Method::HEAD);
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            let _permit = self.concurrency.try_acquire().map_err(|_| Error::Busy)?;
            let plan = self.authorize(request).await?;
            dispatched = true;
            let response = self
                .transport
                .send_to(&plan.request, plan.origin.as_deref())
                .await?;
            if response.body.len() > MAX_RESPONSE_BYTES {
                return Err(Error::ResponseTooLarge);
            }
            if !(200..300).contains(&response.status) {
                return Err(Error::Upstream);
            }
            write_accepted = mutating;
            let response = self.adapter.sanitize_response(&plan, response)?;
            hooks::run(
                &self.hooks,
                hooks::HookStage::AfterResponse,
                &self.policy,
                &plan,
                Some(&response),
            )
            .await?;
            Ok(response)
        })
        .await
        .unwrap_or(Err(Error::Timeout));
        if write_accepted && result.is_err() {
            return Err(Error::WriteAcceptedResponseWithheld);
        }
        if dispatched && mutating && result.is_err() {
            return Err(Error::OutcomeUncertain);
        }
        result
    }

    async fn authorize(&self, request: Request) -> Result<Plan, Error> {
        if request.body.len() > MAX_BODY_BYTES {
            return Err(Error::Denied("request body exceeds the policy byte limit"));
        }
        if request
            .query
            .iter()
            .map(|(k, v)| k.len().saturating_add(v.len()))
            .sum::<usize>()
            > 8192
        {
            return Err(Error::Denied("query is too long"));
        }
        let plan = self.adapter.plan(&self.policy, request)?;
        if !self
            .policy
            .allowed_operations
            .iter()
            .any(|op| op == &plan.operation)
        {
            return Err(Error::Denied("operation is not allowed"));
        }
        if let Some(constraints) = self.policy.constraints.get(&plan.operation) {
            enforce_constraints(constraints, &plan.request)?;
        }
        hooks::run(
            &self.hooks,
            hooks::HookStage::BeforeExecute,
            &self.policy,
            &plan,
            None,
        )
        .await?;
        self.adapter
            .verify(&self.policy, &plan, self.transport.as_ref())
            .await?;
        Ok(plan)
    }
}

fn enforce_constraints(
    constraints: &[ParameterConstraint],
    request: &Request,
) -> Result<(), Error> {
    let body = if request.body.is_empty() {
        Value::Null
    } else {
        parse_json(&request.body)?
    };
    for constraint in constraints {
        let value = match constraint.location {
            ParameterLocation::Query => request
                .query
                .get(&constraint.name)
                .cloned()
                .map(Value::String),
            ParameterLocation::Body => body.pointer(&constraint.name).cloned(),
        };
        let Some(value) = value else {
            if constraint.required {
                return Err(Error::Denied("required parameter is missing"));
            }
            continue;
        };
        if !matches_value(&constraint.rule, &value) {
            return Err(Error::Denied("parameter violates its constraint"));
        }
    }
    Ok(())
}

pub(crate) fn matches_value(rule: &ValueRule, value: &Value) -> bool {
    match rule {
        ValueRule::Exact { value: expected } => value == expected,
        ValueRule::OneOf { values } => values.contains(value),
        ValueRule::MaxInteger { value: maximum } => value
            .as_u64()
            .or_else(|| value.as_str().and_then(|s| s.parse::<u64>().ok()))
            .is_some_and(|v| v <= *maximum),
        ValueRule::MaxLength { value: maximum } => value
            .as_str()
            .is_some_and(|s| s.chars().count() <= *maximum),
    }
}

fn valid_pointer(pointer: &str) -> bool {
    (pointer.is_empty() || pointer.starts_with('/'))
        && pointer
            .split('~')
            .skip(1)
            .all(|part| part.starts_with('0') || part.starts_with('1'))
}

/// JSON duplicate keys must not acquire different meanings across parsers.
pub fn parse_json(bytes: &[u8]) -> Result<Value, Error> {
    struct Unique(Value);
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Unique;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("JSON with unique object keys")
                }
                fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Unique, E> {
                    serde_json::Number::from_f64(v)
                        .map(|n| Unique(Value::Number(n)))
                        .ok_or_else(|| E::custom("invalid number"))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = Vec::new();
                    while let Some(Unique(value)) = seq.next_element()? {
                        values.push(value);
                    }
                    Ok(Unique(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some((key, Unique(value))) = map.next_entry::<String, Unique>()? {
                        if values.insert(key, value).is_some() {
                            return Err(serde::de::Error::custom("duplicate key"));
                        }
                    }
                    Ok(Unique(Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    serde_json::from_slice::<Unique>(bytes)
        .map(|v| v.0)
        .map_err(|_| Error::Denied("invalid or ambiguous JSON"))
}

pub fn parse_query(query: &str) -> Result<BTreeMap<String, String>, Error> {
    if query.len() > 8192 {
        return Err(Error::Denied("query is too long"));
    }
    let mut result = BTreeMap::new();
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let decode = |raw: &str| {
            for (index, byte) in raw.bytes().enumerate() {
                if byte == b'%'
                    && (raw
                        .as_bytes()
                        .get(index + 1)
                        .is_none_or(|b| !b.is_ascii_hexdigit())
                        || raw
                            .as_bytes()
                            .get(index + 2)
                            .is_none_or(|b| !b.is_ascii_hexdigit()))
                {
                    return Err(Error::Denied("invalid query encoding"));
                }
            }
            urlencoding::decode(&raw.replace('+', " "))
                .map(|s| s.into_owned())
                .map_err(|_| Error::Denied("invalid query encoding"))
        };
        if result.insert(decode(key)?, decode(value)?).is_some() {
            return Err(Error::Denied("duplicate query parameter"));
        }
    }
    Ok(result)
}
