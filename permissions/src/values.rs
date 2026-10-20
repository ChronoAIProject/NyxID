//! Transport-neutral input limits for one operation. The Google API adapter and
//! NyxID's scoped operations share these rules and this evaluator, so a value
//! limit means the same thing on every route that enforces it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error, ValueRule,
    google::{JsonShape, QueryParameter, exact_strings, path_is_canonical, reserved_query},
    matches_value, parse_json, parse_query,
};

/// Optional limits on one operation's inputs. An absent part is unconstrained;
/// a present query map is closed and a present body schema requires JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRules {
    /// Path variables limited to literal exact or one_of strings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub path: BTreeMap<String, ValueRule>,
    /// Every accepted query parameter, when present; undeclared ones are refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<BTreeMap<String, QueryParameter>>,
    /// A closed JSON body schema, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<JsonShape>,
}

impl InputRules {
    pub fn is_empty(&self) -> bool {
        self.path.is_empty() && self.query.is_none() && self.body.is_none()
    }

    /// Validate against the operation's method and template variable names.
    pub fn validate(&self, method: &str, variables: &BTreeSet<&str>) -> Result<(), Error> {
        if self.path.len() > 16
            || self
                .path
                .keys()
                .any(|name| !variables.contains(name.as_str()))
        {
            return Err(Error::Policy("path rules must name template variables"));
        }
        validate_path_rules(self.path.values())?;
        if let Some(query) = &self.query {
            validate_query_rules(query)?;
        }
        if let Some(body) = &self.body {
            if matches!(method, "GET" | "HEAD") {
                return Err(Error::Policy("read operations cannot declare a body"));
            }
            body.validate(0, &mut 512)?;
        }
        Ok(())
    }

    /// Check bound path variables, the raw query string and the request body.
    pub fn check(
        &self,
        path: &BTreeMap<String, String>,
        query: Option<&str>,
        body: &[u8],
        content_type: Option<&str>,
    ) -> Result<(), Error> {
        for (name, rule) in &self.path {
            let value = path
                .get(name)
                .ok_or(Error::Denied("path variable is missing"))?;
            if !matches_value(rule, &Value::String(value.clone())) {
                return Err(Error::Denied("path variable violates its rule"));
            }
        }
        if let Some(rules) = &self.query {
            check_query(rules, &parse_query(query.unwrap_or(""))?)?;
        }
        if let Some(shape) = &self.body {
            check_json_body(shape, body, content_type)?;
        }
        Ok(())
    }
}

pub(crate) fn validate_path_rules<'a>(
    rules: impl IntoIterator<Item = &'a ValueRule>,
) -> Result<(), Error> {
    for rule in rules {
        if exact_strings(rule).is_none_or(|values| {
            values
                .iter()
                .any(|value| value.contains('/') || !path_is_canonical(&format!("/{value}"), false))
        }) {
            return Err(Error::Policy(
                "path variables require literal exact or one_of strings",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_query_rules(rules: &BTreeMap<String, QueryParameter>) -> Result<(), Error> {
    if rules.len() > 32 {
        return Err(Error::Policy("at most 32 query parameters"));
    }
    for (name, parameter) in rules {
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
    Ok(())
}

/// A closed query: every parameter is declared and satisfies its rule.
pub(crate) fn check_query(
    rules: &BTreeMap<String, QueryParameter>,
    query: &BTreeMap<String, String>,
) -> Result<(), Error> {
    if query.keys().any(|name| !rules.contains_key(name)) {
        return Err(Error::Denied("query parameter is not allowed"));
    }
    for (name, parameter) in rules {
        match query.get(name) {
            Some(value) if matches_value(&parameter.rule, &Value::String(value.clone())) => {}
            None if !parameter.required => {}
            _ => return Err(Error::Denied("query parameter violates its rule")),
        }
    }
    Ok(())
}

/// A non-empty `application/json` body accepted by the closed schema.
pub(crate) fn check_json_body(
    shape: &JsonShape,
    body: &[u8],
    content_type: Option<&str>,
) -> Result<(), Error> {
    if body.is_empty() {
        return Err(Error::Denied("body is missing or forbidden"));
    }
    if content_type
        .and_then(|s| s.split(';').next())
        .map(str::trim)
        != Some("application/json")
    {
        return Err(Error::Denied("body requires application/json"));
    }
    if !shape.accepts(&parse_json(body)?) {
        return Err(Error::Denied("body violates its closed schema"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::JsonShape;
    use serde_json::json;

    fn vars<'a>(names: &[&'a str]) -> BTreeSet<&'a str> {
        names.iter().copied().collect()
    }

    #[test]
    fn validation_refuses_unknown_variables_reserved_query_and_read_bodies() {
        let path = InputRules {
            path: [("missing".into(), ValueRule::Exact { value: json!("x") })].into(),
            ..Default::default()
        };
        assert!(path.validate("GET", &vars(&["id"])).is_err());
        let slash = InputRules {
            path: [(
                "id".into(),
                ValueRule::Exact {
                    value: json!("a/b"),
                },
            )]
            .into(),
            ..Default::default()
        };
        assert!(slash.validate("GET", &vars(&["id"])).is_err());
        for reserved in ["access_token", "key", "_method", "_nyxid_via"] {
            let query = InputRules {
                query: Some(
                    [(
                        reserved.into(),
                        QueryParameter {
                            required: false,
                            rule: ValueRule::MaxLength { value: 4 },
                        },
                    )]
                    .into(),
                ),
                ..Default::default()
            };
            assert!(query.validate("GET", &vars(&[])).is_err(), "{reserved}");
        }
        let body = InputRules {
            body: Some(JsonShape::Boolean),
            ..Default::default()
        };
        assert!(body.validate("GET", &vars(&[])).is_err());
        assert!(body.validate("POST", &vars(&[])).is_ok());
        assert!(InputRules::default().is_empty());
    }

    #[test]
    fn checks_are_closed_and_require_json_bodies() {
        let rules = InputRules {
            path: [(
                "id".into(),
                ValueRule::OneOf {
                    values: vec![json!("1"), json!("2")],
                },
            )]
            .into(),
            query: Some(
                [(
                    "limit".into(),
                    QueryParameter {
                        required: true,
                        rule: ValueRule::MaxInteger { value: 10 },
                    },
                )]
                .into(),
            ),
            body: Some(JsonShape::Object {
                properties: [("ok".into(), JsonShape::Boolean)].into(),
                required: vec![],
            }),
        };
        let path: BTreeMap<String, String> = [("id".into(), "2".into())].into();
        let json = Some("application/json");
        assert!(
            rules
                .check(&path, Some("limit=10"), br#"{"ok":true}"#, json)
                .is_ok()
        );
        assert!(
            rules
                .check(&path, Some("limit=11"), br#"{"ok":true}"#, json)
                .is_err()
        );
        assert!(
            rules
                .check(&path, Some("limit=1&x=1"), br#"{"ok":true}"#, json)
                .is_err()
        );
        assert!(
            rules
                .check(&path, Some("limit=1&limit=2"), br#"{}"#, json)
                .is_err()
        );
        assert!(
            rules
                .check(&path, Some("limit=1"), br#"{"ok":1}"#, json)
                .is_err()
        );
        assert!(
            rules
                .check(
                    &path,
                    Some("limit=1"),
                    br#"{"ok":true}"#,
                    Some("text/plain")
                )
                .is_err()
        );
        assert!(
            rules
                .check(&path, Some("limit=1"), br#"{"ok":true,"ok":false}"#, json)
                .is_err()
        );
        let other: BTreeMap<String, String> = [("id".into(), "3".into())].into();
        assert!(
            rules
                .check(&other, Some("limit=1"), br#"{}"#, json)
                .is_err()
        );
    }
}
