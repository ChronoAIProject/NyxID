//! Declarative Google REST permissions. Resource semantics live in explicit
//! adapters (for example Drive ancestry); this adapter constrains wire inputs.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use http::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error, ParameterLocation, Plan, Policy, ProviderAdapter, Request, ResourceBoundary, Response,
    Transport, ValueRule, matches_value, parse_json,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiOperation {
    pub id: String,
    pub origin: String,
    pub method: String,
    pub path: String,
    /// Every template variable is mandatory and limited to exact values.
    #[serde(default)]
    pub path_parameters: BTreeMap<String, ValueRule>,
    /// Omitted parameters are forbidden, including Google system parameters.
    #[serde(default)]
    pub query_parameters: BTreeMap<String, QueryParameter>,
    /// None forbids a body; Some requires a body matching this closed schema.
    pub body: Option<JsonShape>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryParameter {
    #[serde(default)]
    pub required: bool,
    pub rule: ValueRule,
}

/// Small, bounded schema vocabulary. Object properties are always closed,
/// including objects nested in arrays (such as Workspace batchUpdate requests).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum JsonShape {
    Object {
        properties: BTreeMap<String, JsonShape>,
        #[serde(default)]
        required: Vec<String>,
    },
    Array {
        items: Box<JsonShape>,
        max_items: usize,
    },
    String {
        max_length: usize,
    },
    Integer {
        minimum: i64,
        maximum: i64,
    },
    Boolean,
    Exact {
        value: Value,
    },
    OneOf {
        values: Vec<Value>,
    },
}

impl JsonShape {
    fn validate(&self, depth: usize, remaining: &mut usize) -> Result<(), Error> {
        if depth > 12 || *remaining == 0 {
            return Err(Error::Policy("body schema exceeds depth or node limit"));
        }
        *remaining -= 1;
        match self {
            Self::Object {
                properties,
                required,
            } => {
                if properties.len() > 64
                    || required.len() > properties.len()
                    || required.iter().any(|key| !properties.contains_key(key))
                    || properties
                        .keys()
                        .any(|key| key.is_empty() || key.len() > 256)
                {
                    return Err(Error::Policy("invalid body object schema"));
                }
                for child in properties.values() {
                    child.validate(depth + 1, remaining)?;
                }
            }
            Self::Array { items, max_items } => {
                if *max_items > 1000 {
                    return Err(Error::Policy("maximum 1000 array items"));
                }
                items.validate(depth + 1, remaining)?;
            }
            Self::String { max_length } if *max_length > crate::MAX_BODY_BYTES => {
                return Err(Error::Policy("string limit exceeds request limit"));
            }
            Self::Integer { minimum, maximum } if minimum > maximum => {
                return Err(Error::Policy("invalid integer bounds"));
            }
            Self::OneOf { values } if values.is_empty() || values.len() > 100 => {
                return Err(Error::Policy("one_of requires 1–100 values"));
            }
            _ => {}
        }
        Ok(())
    }

    fn accepts(&self, value: &Value) -> bool {
        match self {
            Self::Object {
                properties,
                required,
            } => value.as_object().is_some_and(|object| {
                required.iter().all(|key| object.contains_key(key))
                    && object.iter().all(|(key, value)| {
                        properties.get(key).is_some_and(|rule| rule.accepts(value))
                    })
            }),
            Self::Array { items, max_items } => value.as_array().is_some_and(|array| {
                array.len() <= *max_items && array.iter().all(|value| items.accepts(value))
            }),
            Self::String { max_length } => value
                .as_str()
                .is_some_and(|s| s.chars().count() <= *max_length),
            Self::Integer { minimum, maximum } => value
                .as_i64()
                .is_some_and(|n| (*minimum..=*maximum).contains(&n)),
            Self::Boolean => value.is_boolean(),
            Self::Exact { value: expected } => value == expected,
            Self::OneOf { values } => values.contains(value),
        }
    }
}

pub struct GoogleApiAdapter;

/// Eligibility is Google API DNS ownership; permission is an exact origin in
/// each operation plus the host's independently pinned credential recipient.
pub fn is_google_api_origin(origin: &str) -> bool {
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.port().is_none()
        && url.origin().ascii_serialization() == origin
        && url.host_str().is_some_and(|host| {
            host.strip_suffix(".googleapis.com").is_some_and(|prefix| {
                !prefix.is_empty()
                    && prefix.split('.').all(|label| {
                        !label.is_empty()
                            && !label.starts_with('-')
                            && !label.ends_with('-')
                            && label
                                .bytes()
                                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                    })
            })
        })
}

fn operations(policy: &Policy) -> Result<&[ApiOperation], Error> {
    match &policy.resource {
        ResourceBoundary::GoogleApi { operations } if policy.provider == "google" => Ok(operations),
        _ => Err(Error::Policy(
            "expected google provider and google_api boundary",
        )),
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn path_is_canonical(path: &str, template: bool) -> bool {
    path.starts_with('/')
        && path.len() <= 2048
        && path.len() > 1
        && path.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'/' | b'-'
                        | b'.'
                        | b'_'
                        | b'~'
                        | b'!'
                        | b'$'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'+'
                        | b','
                        | b';'
                        | b'='
                        | b':'
                        | b'@'
                )
                || (template && matches!(b, b'{' | b'}'))
        })
        && path[1..]
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// One whole-segment variable, optionally followed by an AIP custom verb.
fn variable(segment: &str) -> Option<(&str, &str)> {
    let rest = segment.strip_prefix('{')?;
    let (name, suffix) = rest.split_once('}')?;
    if !identifier(name) || !(suffix.is_empty() || suffix.strip_prefix(':').is_some_and(identifier))
    {
        return None;
    }
    Some((name, suffix))
}

fn exact_strings(rule: &ValueRule) -> Option<Vec<&str>> {
    match rule {
        ValueRule::Exact { value } => Some(vec![value.as_str()?]),
        ValueRule::OneOf { values } if !values.is_empty() && values.len() <= 100 => {
            values.iter().map(Value::as_str).collect()
        }
        _ => None,
    }
}

fn reserved_query(name: &str) -> bool {
    !identifier(name)
        || name.to_ascii_lowercase().starts_with("_nyxid")
        || matches!(
            name.to_ascii_lowercase().as_str(),
            "access_token"
                | "oauth_token"
                | "key"
                | "callback"
                | "uploadtype"
                | "upload_protocol"
                | "httpmethod"
                | "method"
                | "_method"
        )
}

impl ApiOperation {
    fn validate(&self) -> Result<(), Error> {
        if !identifier(&self.id)
            || !is_google_api_origin(&self.origin)
            || !matches!(
                self.method.as_str(),
                "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
            )
            || !path_is_canonical(&self.path, true)
            || self.path_parameters.len() > 16
            || self.query_parameters.len() > 32
        {
            return Err(Error::Policy(
                "invalid API operation identity, origin, method or path",
            ));
        }
        let mut variables = BTreeSet::new();
        for segment in self.path[1..].split('/') {
            if let Some((name, _)) = variable(segment) {
                if !variables.insert(name) {
                    return Err(Error::Policy("duplicate path variable"));
                }
            } else if segment.contains(['{', '}']) {
                return Err(Error::Policy("invalid path template"));
            }
        }
        if variables.len() != self.path_parameters.len()
            || variables
                .iter()
                .any(|name| !self.path_parameters.contains_key(*name))
        {
            return Err(Error::Policy(
                "every path variable requires an exact or one_of rule",
            ));
        }
        for rule in self.path_parameters.values() {
            if exact_strings(rule).is_none_or(|values| {
                values.iter().any(|value| {
                    value.contains('/') || !path_is_canonical(&format!("/{value}"), false)
                })
            }) {
                return Err(Error::Policy(
                    "path variables require literal exact or one_of strings",
                ));
            }
        }
        for (name, parameter) in &self.query_parameters {
            if reserved_query(name)
                || matches!(&parameter.rule, ValueRule::Exact { value } if !value.is_string())
                || matches!(&parameter.rule, ValueRule::OneOf { values } if values.is_empty() || values.len() > 100 || values.iter().any(|v| !v.is_string()))
                || (name.eq_ignore_ascii_case("alt")
                    && exact_strings(&parameter.rule)
                        .is_none_or(|values| values.iter().any(|v| *v != "json")))
            {
                return Err(Error::Policy("invalid or reserved query parameter rule"));
            }
        }
        if let Some(body) = &self.body {
            if matches!(self.method.as_str(), "GET" | "HEAD") {
                return Err(Error::Policy("read operations cannot declare a body"));
            }
            body.validate(0, &mut 512)?;
        }
        Ok(())
    }

    fn matches(&self, request: &Request) -> bool {
        if self.method != request.method.as_str() {
            return false;
        }
        let template: Vec<_> = self.path[1..].split('/').collect();
        let actual: Vec<_> = request.path[1..].split('/').collect();
        template.len() == actual.len()
            && template.iter().zip(actual).all(|(pattern, part)| {
                if let Some((name, suffix)) = variable(pattern) {
                    part.strip_suffix(suffix).is_some_and(|value| {
                        self.path_parameters
                            .get(name)
                            .is_some_and(|rule| matches_value(rule, &Value::String(value.into())))
                    })
                } else {
                    *pattern == part
                }
            })
    }

    fn check_inputs(&self, request: &Request) -> Result<(), Error> {
        if request
            .query
            .keys()
            .any(|name| !self.query_parameters.contains_key(name))
        {
            return Err(Error::Denied("query parameter is not allowed"));
        }
        for (name, parameter) in &self.query_parameters {
            match request.query.get(name) {
                Some(value) if matches_value(&parameter.rule, &Value::String(value.clone())) => {}
                None if !parameter.required => {}
                _ => return Err(Error::Denied("query parameter violates its rule")),
            }
        }
        match (&self.body, request.body.is_empty()) {
            (None, true) => Ok(()),
            (Some(shape), false) => {
                if request
                    .content_type
                    .as_deref()
                    .and_then(|s| s.split(';').next())
                    .map(str::trim)
                    != Some("application/json")
                {
                    return Err(Error::Denied("body requires application/json"));
                }
                if !shape.accepts(&parse_json(&request.body)?) {
                    return Err(Error::Denied("body violates its closed schema"));
                }
                Ok(())
            }
            _ => Err(Error::Denied("body is missing or forbidden")),
        }
    }
}

#[async_trait]
impl ProviderAdapter for GoogleApiAdapter {
    fn validate_policy(&self, policy: &Policy) -> Result<(), Error> {
        let operations = operations(policy)?;
        let ids: BTreeSet<_> = operations.iter().map(|op| &op.id).collect();
        if operations.is_empty()
            || operations.len() > 64
            || ids.len() != operations.len()
            || policy.allowed_operations.iter().collect::<BTreeSet<_>>() != ids
            || policy.allowed_operations.len() != ids.len()
        {
            return Err(Error::Policy(
                "declare exactly the allowed operations, without duplicate IDs",
            ));
        }
        for operation in operations {
            operation.validate()?;
            if let Some(constraints) = policy.constraints.get(&operation.id) {
                for constraint in constraints {
                    match constraint.location {
                        ParameterLocation::Query
                            if !operation.query_parameters.contains_key(&constraint.name) =>
                        {
                            return Err(Error::Policy(
                                "constraint names an undeclared query parameter",
                            ));
                        }
                        ParameterLocation::Body if operation.body.is_none() => {
                            return Err(Error::Policy("body constraint on a bodyless operation"));
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    fn plan(&self, policy: &Policy, mut request: Request) -> Result<Plan, Error> {
        if !path_is_canonical(&request.path, false) {
            return Err(Error::Denied("noncanonical API path"));
        }
        let mut candidates = operations(policy)?.iter().filter(|op| op.matches(&request));
        let operation = candidates
            .next()
            .ok_or(Error::Denied("API operation or resource is not allowed"))?;
        if candidates.next().is_some() {
            return Err(Error::Denied("ambiguous API operation"));
        }
        operation.check_inputs(&request)?;
        request.content_type = (!request.body.is_empty()).then(|| "application/json".into());
        Ok(Plan {
            operation: operation.id.clone(),
            origin: Some(operation.origin.clone()),
            request,
            resources: vec![],
        })
    }

    async fn verify(&self, _: &Policy, _: &Plan, _: &dyn Transport) -> Result<(), Error> {
        // Exact resource IDs were checked above. This makes no ancestry or
        // response-membership claim for arbitrary Google services.
        Ok(())
    }

    fn sanitize_response(&self, plan: &Plan, response: Response) -> Result<Response, Error> {
        if response.status == 204 || plan.request.method == Method::HEAD {
            return Ok(Response {
                status: response.status,
                content_type: "application/json".into(),
                body: Default::default(),
            });
        }
        if response.content_type.split(';').next().map(str::trim) != Some("application/json") {
            return Err(Error::Verification);
        }
        let json = parse_json(&response.body).map_err(|_| Error::Verification)?;
        Ok(Response {
            status: response.status,
            content_type: "application/json".into(),
            body: serde_json::to_vec(&json)
                .map_err(|_| Error::Verification)?
                .into(),
        })
    }
}
