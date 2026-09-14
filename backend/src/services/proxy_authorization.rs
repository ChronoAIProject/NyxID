use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    DownstreamService, ProxyOperationPolicy, ProxyOperationRule, ProxyPathConstraint,
};

/// A normalized downstream path used for both policy matching and forwarding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalPath {
    segments: Vec<String>,
}

impl CanonicalPath {
    /// Construct from an Axum-decoded REST wildcard after the raw OriginalUri
    /// has passed `validate_requested_proxy_path`. Reject every remaining
    /// percent sign, including literal data, to forbid another decoding layer.
    pub fn from_rest_decoded(path: &str) -> AppResult<Self> {
        Self::from_decoded(path)
    }

    /// Construct from a caller-supplied MCP generic path. Percent signs are
    /// rejected because MCP arguments are literal rather than router-decoded.
    pub fn from_mcp_literal(path: &str) -> AppResult<Self> {
        Self::from_decoded(path)
    }

    /// Construct from a typed MCP endpoint after its declared path parameters
    /// have been substituted and encoded by the existing argument builder.
    pub fn from_mcp_built(path: &str) -> AppResult<Self> {
        reject_common_ambiguity(path, false)?;
        let decoded = percent_decode_path(path)?;
        Self::from_decoded(&decoded)
    }

    fn from_decoded(path: &str) -> AppResult<Self> {
        reject_common_ambiguity(path, true)?;
        let trimmed = path.strip_prefix('/').unwrap_or(path);
        if trimmed.is_empty() {
            return Ok(Self { segments: vec![] });
        }
        let segments = trimmed.split('/').map(str::to_string).collect();
        Ok(Self { segments })
    }

    #[cfg(test)]
    pub fn as_policy_path(&self) -> String {
        if self.segments.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", self.segments.join("/"))
        }
    }
}

fn reject_common_ambiguity(path: &str, reject_percent: bool) -> AppResult<()> {
    if path.contains('?')
        || path.contains('#')
        || path.contains('\\')
        || (reject_percent && path.contains('%'))
        || path.bytes().any(|byte| byte < b' ' || byte == 0x7f)
        || path.starts_with("//")
        || path.contains("//")
        || (path.len() > 1 && path.ends_with('/'))
    {
        return Err(AppError::BadRequest("Invalid proxy path".to_string()));
    }

    let trimmed = path.strip_prefix('/').unwrap_or(path);
    if trimmed
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(AppError::BadRequest("Invalid proxy path".to_string()));
    }
    Ok(())
}

fn percent_decode_path(path: &str) -> AppResult<String> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err(AppError::BadRequest("Invalid proxy path".to_string()));
        }
        let high = hex_value(bytes[index + 1])
            .ok_or_else(|| AppError::BadRequest("Invalid proxy path".to_string()))?;
        let low = hex_value(bytes[index + 2])
            .ok_or_else(|| AppError::BadRequest("Invalid proxy path".to_string()))?;
        let byte = (high << 4) | low;
        if matches!(byte, b'/' | b'\\' | b'%' | 0) {
            return Err(AppError::BadRequest("Invalid proxy path".to_string()));
        }
        decoded.push(byte);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| AppError::BadRequest("Invalid proxy path".to_string()))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub fn normalize_policy(policy: ProxyOperationPolicy) -> AppResult<ProxyOperationPolicy> {
    if policy.rules.len() > 256 {
        return Err(AppError::ValidationError(
            "proxy_operation_policy must not exceed 256 rules".to_string(),
        ));
    }
    let mut rules = Vec::with_capacity(policy.rules.len());
    for rule in policy.rules {
        let method = rule.method.trim().to_ascii_uppercase();
        if !matches!(
            method.as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
        ) {
            return Err(AppError::ValidationError(format!(
                "Unsupported proxy operation method: {}",
                rule.method
            )));
        }
        validate_rule(&rule)?;
        rules.push(ProxyOperationRule {
            method,
            path_template: rule.path_template,
            path_parameter_constraints: rule.path_parameter_constraints,
        });
    }
    Ok(ProxyOperationPolicy { rules })
}

pub(crate) fn validate_template(template: &str) -> AppResult<()> {
    if template.len() > 2048 {
        return Err(AppError::ValidationError(
            "proxy operation path template exceeds 2048 bytes".to_string(),
        ));
    }
    if template.contains(' ') || !template.starts_with('/') {
        return Err(AppError::ValidationError(
            "proxy operation paths must be root-anchored".to_string(),
        ));
    }
    reject_common_ambiguity(template, true).map_err(|_| {
        AppError::ValidationError("Invalid proxy operation path template".to_string())
    })?;
    if template == "/" {
        return Ok(());
    }
    for segment in template.trim_start_matches('/').split('/') {
        validate_template_segment(segment)?;
    }
    Ok(())
}

fn invalid_template() -> AppError {
    AppError::ValidationError("Invalid proxy operation path template".to_string())
}

/// Split a path-template segment into its resource part and optional AIP
/// custom-method verb: `{documentId}:batchUpdate` -> `("{documentId}", Some("batchUpdate"))`.
///
/// Shared so every consumer of the template grammar (proxy authorization and
/// durable operation grants) agrees on what a segment means.
pub(crate) fn split_template_segment(segment: &str) -> (&str, Option<&str>) {
    match segment.split_once(':') {
        Some((resource, verb)) => (resource, Some(verb)),
        None => (segment, None),
    }
}

/// Strip a custom-method verb from an actual (non-template) segment, yielding
/// the resource part. Returns `None` when the suffix differs or the resource
/// part is empty. The parameter constraint validates the capture.
pub(crate) fn strip_custom_method<'a>(actual: &'a str, verb: &str) -> Option<&'a str> {
    let resource = actual.strip_suffix(verb)?.strip_suffix(':')?;
    (!resource.is_empty()).then_some(resource)
}

/// A template segment is a literal, a whole parameter (`{name}`), or an AIP
/// custom method: a resource part, one `:`, and a literal verb
/// (`{documentId}:batchUpdate`, `spreadsheets:batchGet`).
fn validate_template_segment(segment: &str) -> AppResult<()> {
    let (resource, verb) = split_template_segment(segment);

    if let Some(verb) = verb {
        // One colon only, and the verb is a bare literal: it is compared
        // exactly, so it must never carry a parameter or further structure.
        if verb.is_empty() || !verb.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            return Err(invalid_template());
        }
        if resource.is_empty() {
            return Err(invalid_template());
        }
    }

    if resource.starts_with('{') || resource.ends_with('}') {
        let well_formed = resource.starts_with('{')
            && resource.ends_with('}')
            && resource.len() > 2
            && !resource[1..resource.len() - 1]
                .chars()
                .any(|ch| !ch.is_ascii_alphanumeric() && ch != '_');
        if !well_formed {
            return Err(invalid_template());
        }
    } else if resource.contains(['*', '[', ']', '(', ')', '|', '{', '}']) {
        return Err(invalid_template());
    }
    Ok(())
}

/// Read the parameter-level extension, outside the restricted JSON Schema subset.
/// The same rule builder is used by product policies and selected MCP endpoints.
pub(crate) fn rule_from_endpoint(
    method: &str,
    path: &str,
    parameters: Option<&serde_json::Value>,
) -> AppResult<ProxyOperationRule> {
    let mut rule = ProxyOperationRule {
        method: method.trim().to_ascii_uppercase(),
        path_template: path.to_string(),
        ..Default::default()
    };
    for parameter in parameters
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(constraint) = parameter.get("x-nyxid-path-constraint") else {
            continue;
        };
        let name = parameter["name"].as_str().ok_or_else(invalid_template)?;
        if parameter["in"] != "path" {
            return Err(invalid_template());
        }
        let constraint =
            serde_json::from_value(constraint.clone()).map_err(|_| invalid_template())?;
        if rule
            .path_parameter_constraints
            .insert(name.to_string(), constraint)
            .is_some()
        {
            return Err(invalid_template());
        }
    }
    validate_rule(&rule)?;
    Ok(rule)
}

fn parameter_name(resource: &str) -> Option<&str> {
    resource.strip_prefix('{')?.strip_suffix('}')
}

fn validate_rule(rule: &ProxyOperationRule) -> AppResult<()> {
    validate_template(&rule.path_template)?;
    for name in rule.path_parameter_constraints.keys() {
        if !rule
            .path_template
            .split('/')
            .any(|segment| parameter_name(split_template_segment(segment).0) == Some(name.as_str()))
        {
            return Err(invalid_template());
        }
    }
    Ok(())
}

/// Bind decoded captures only after matching the selected template and its
/// value grammars. Callers must never decode these values again.
pub(crate) fn match_path_arguments(
    rule: &ProxyOperationRule,
    path: &CanonicalPath,
) -> Option<std::collections::BTreeMap<String, String>> {
    validate_rule(rule).ok()?;
    let template_segments: Vec<&str> = rule
        .path_template
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if template_segments.len() != path.segments.len() {
        return None;
    }
    let mut arguments = std::collections::BTreeMap::new();
    for (template, actual) in template_segments.iter().zip(&path.segments) {
        let (resource, verb) = split_template_segment(template);
        let value = match verb {
            Some(verb) => strip_custom_method(actual, verb)?,
            None => actual.as_str(),
        };
        if let Some(name) = parameter_name(resource) {
            if !parameter_matches(rule.path_parameter_constraints.get(name), value) {
                return None;
            }
            if let Some(previous) = arguments.insert(name.to_string(), value.to_string())
                && previous != value
            {
                return None;
            }
        } else if resource != value {
            return None;
        }
    }
    Some(arguments)
}

pub(crate) fn rule_matches(rule: &ProxyOperationRule, method: &str, path: &CanonicalPath) -> bool {
    rule.method == method && match_path_arguments(rule, path).is_some()
}

/// Preserve only the custom-method suffix declared by the matched rule.
/// Colons inside a constrained parameter remain encoded as data (as in
/// Google's generated Sheets clients), so they cannot become method delimiters.
pub(crate) fn rule_forwarding_path(
    rule: &ProxyOperationRule,
    method: &str,
    path: &CanonicalPath,
) -> Option<String> {
    if !rule_matches(rule, method, path) {
        return None;
    }
    Some(
        rule.path_template
            .trim_start_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .zip(&path.segments)
            .map(|(template, actual)| {
                Some(match split_template_segment(template).1 {
                    Some(verb) => format!(
                        "{}:{verb}",
                        urlencoding::encode(strip_custom_method(actual, verb)?)
                    ),
                    None => urlencoding::encode(actual).into_owned(),
                })
            })
            .collect::<Option<Vec<_>>>()?
            .join("/"),
    )
}

fn parameter_matches(constraint: Option<&ProxyPathConstraint>, value: &str) -> bool {
    if matches!(value, "" | "." | "..")
        || value.contains(['%', '/', '\\', '?', '#'])
        || value.chars().any(char::is_control)
    {
        return false;
    }
    match constraint {
        Some(ProxyPathConstraint::SheetsA1Range) => sheets_a1_range(value),
        Some(ProxyPathConstraint::DiscordEmoji) => discord_emoji(value),
        None => !value.contains(':') && !value.chars().any(char::is_whitespace),
    }
}

fn discord_emoji(value: &str) -> bool {
    if emojis::get(value).is_some() {
        return true;
    }
    let Some((name, id)) = value.split_once(':') else {
        return false;
    };
    (2..=32).contains(&name.len())
        && name
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
        && (1..=20).contains(&id.len())
        && id.bytes().all(|ch| ch.is_ascii_digit())
        && id.parse::<u64>().is_ok_and(|id| id > 0)
}

/// A1 coordinates, named ranges and sheet names. Colons join coordinates and
/// are encoded as data on forwarding, including column names such as `get`.
/// Quoted titles may contain spaces, Unicode, exclamation marks and doubled
/// apostrophes.
fn sheets_a1_range(value: &str) -> bool {
    fn identifier(value: &str) -> bool {
        let mut chars = value.chars();
        chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
    }
    fn sheet(value: &str) -> bool {
        if let Some(quoted) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
            !quoted.is_empty()
                && !quoted.contains([':', '/', '\\', '?', '*', '[', ']'])
                && !quoted.replace("''", "").contains('\'')
        } else {
            identifier(value)
        }
    }
    fn coordinate(value: &str) -> bool {
        let value = value.strip_prefix('$').unwrap_or(value);
        let letters = value.bytes().take_while(u8::is_ascii_alphabetic).count();
        // Sheets has at most 18,278 columns (ZZZ).
        if letters > 3 {
            return false;
        }
        let rest = &value[letters..];
        let row = rest.strip_prefix('$').unwrap_or(rest);
        if row.is_empty() {
            return letters > 0 && rest.is_empty();
        }
        row.bytes().all(|c| c.is_ascii_digit()) && !row.starts_with('0')
    }
    if value.starts_with('\'') && value.ends_with('\'') {
        return sheet(value);
    }
    let range = if let Some((title, range)) = value.rsplit_once('!') {
        if !sheet(title) {
            return false;
        }
        range
    } else {
        value
    };
    if let Some((start, end)) = range.split_once(':') {
        coordinate(start) && coordinate(end)
    } else {
        coordinate(range) || identifier(range)
    }
}

pub fn authorize_proxy_operation(
    service: &DownstreamService,
    method: &str,
    path: &CanonicalPath,
) -> AppResult<String> {
    authorize_proxy_operation_fields(
        &service.id,
        &service.slug,
        service.proxy_operation_policy.as_ref(),
        method,
        path,
    )
}

pub fn authorize_proxy_operation_fields(
    service_id: &str,
    service_slug: &str,
    policy: Option<&ProxyOperationPolicy>,
    method: &str,
    path: &CanonicalPath,
) -> AppResult<String> {
    let method = method.trim().to_ascii_uppercase();
    let forwarding_path = match policy {
        Some(policy) => policy
            .rules
            .iter()
            .find_map(|rule| rule_forwarding_path(rule, &method, path)),
        None => Some(
            path.segments
                .iter()
                .map(|s| urlencoding::encode(s))
                .collect::<Vec<_>>()
                .join("/"),
        ),
    };
    if let Some(path) = forwarding_path {
        return Ok(path);
    }

    tracing::warn!(
        service_id = %service_id,
        service_slug = %service_slug,
        method = %method,
        reason = "operation_not_allowlisted",
        "Proxy operation authorization denied"
    );
    Err(AppError::NotFound(
        "Service operation not found".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_emoji_constraint_is_scoped_and_encodes_colons_as_data() {
        let parameters = serde_json::json!([{
            "name":"emoji", "in":"path", "x-nyxid-path-constraint":"discord_emoji"
        }]);
        let rule = rule_from_endpoint(
            "PUT",
            "/channels/{id}/reactions/{emoji}/@me",
            Some(&parameters),
        )
        .unwrap();
        for emoji in [
            "smile:12345",
            "ab:18446744073709551615",
            "👍",
            "👍🏽",
            "👩‍💻",
            "🇸🇬",
            "1️⃣",
            "*️⃣",
            "❤️",
            "❤",
        ] {
            let literal = format!("/channels/123/reactions/{emoji}/@me");
            let encoded = format!("/channels/123/reactions/{}/@me", urlencoding::encode(emoji));
            for path in [
                CanonicalPath::from_rest_decoded(&literal).unwrap(),
                CanonicalPath::from_mcp_literal(&literal).unwrap(),
                CanonicalPath::from_mcp_built(&encoded).unwrap(),
            ] {
                let forward = rule_forwarding_path(&rule, "PUT", &path).unwrap();
                assert_eq!(
                    forward,
                    encoded.trim_start_matches('/').replace("@me", "%40me")
                );
                let url =
                    url::Url::parse(&format!("https://discord.com/api/v10/{forward}")).unwrap();
                assert_eq!(url.origin().ascii_serialization(), "https://discord.com");
                assert!(url.query().is_none() && url.fragment().is_none());
            }
        }
        for value in [
            "a:b:c",
            ":12345",
            "ab:",
            "ab:xyz",
            "ab:１２",
            "ab:0",
            "ab:18446744073709551616",
            "a:12345",
            "ab :12345",
            "100%",
            "ab%3A12345",
            "ab%253A12345",
            "plain",
            "👍👍",
            "👍:other",
            "ab:1/other",
        ] {
            assert!(
                !parameter_matches(Some(&ProxyPathConstraint::DiscordEmoji), value),
                "{value}"
            );
        }
        assert!(discord_emoji(&format!("{}:1", "a".repeat(32))));
        assert!(!discord_emoji(&format!("{}:1", "a".repeat(33))));
        let other_id =
            CanonicalPath::from_mcp_literal("/channels/ab:123/reactions/smile:12345/@me").unwrap();
        assert!(!rule_matches(&rule, "PUT", &other_id));
        let ordinary = ProxyOperationRule {
            path_parameter_constraints: Default::default(),
            ..rule
        };
        let path =
            CanonicalPath::from_mcp_literal("/channels/123/reactions/smile:12345/@me").unwrap();
        assert!(!rule_matches(&ordinary, "PUT", &path));
    }

    #[test]
    fn discord_number_sign_keycap_keeps_the_existing_transport_restriction() {
        assert!(emojis::get("#️⃣").is_some());
        for path in [
            "/channels/123/reactions/#️⃣/@me",
            "/channels/123/reactions/%23%EF%B8%8F%E2%83%A3/@me",
        ] {
            assert!(crate::services::proxy_service::validate_requested_proxy_path(path).is_err());
            assert!(CanonicalPath::from_mcp_built(path).is_err());
        }
    }

    #[test]
    fn legacy_policy_serialization_preserves_approval_digest_input() {
        let original = serde_json::json!({"method": "GET", "path_template": "/files/{id}"});
        let rule: ProxyOperationRule = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(serde_json::to_value(rule).unwrap(), original);
    }

    #[test]
    fn policy_paths_decode_once_and_never_accept_residual_percent_signs() {
        for path in [
            "/v1/documents/id%3AbatchUpdate",
            "/v1/documents/id%253AbatchUpdate",
            "/v1/documents/id%2Fother:batchUpdate",
            "/v1/documents/id%5Cother:batchUpdate",
            "/v1/documents/id%252Fother:batchUpdate",
            "/v1/documents/id%25:batchUpdate",
        ] {
            assert!(
                CanonicalPath::from_rest_decoded(path).is_err(),
                "REST {path}"
            );
            assert!(
                CanonicalPath::from_mcp_literal(path).is_err(),
                "generic {path}"
            );
        }
        // Typed paths contain exactly one encoding pass from build_proxy_args.
        let typed = CanonicalPath::from_mcp_built("/v1/documents/id%3AbatchUpdate").unwrap();
        assert_eq!(
            rule_forwarding_path(&docs_policy().rules[1], "POST", &typed).unwrap(),
            "v1/documents/id:batchUpdate"
        );
        for path in [
            "/v1/documents/id%253AbatchUpdate",
            "/v1/documents/id%25253AbatchUpdate",
            "/v1/documents/id%2Fother:batchUpdate",
            "/v1/documents/id%5Cother:batchUpdate",
            "/v1/documents/id%252Fother:batchUpdate",
            "/v1/documents/id%00:batchUpdate",
        ] {
            assert!(CanonicalPath::from_mcp_built(path).is_err(), "typed {path}");
        }
    }

    #[test]
    fn sheets_titles_with_literal_percent_are_deliberately_rejected() {
        let range = "'Q1 100%'!A1:B2";
        assert!(!parameter_matches(
            Some(&ProxyPathConstraint::SheetsA1Range),
            range
        ));
        for suffix in ["", ":append", ":clear"] {
            let decoded = format!("/v4/spreadsheets/id/values/{range}{suffix}");
            assert!(matches!(
                CanonicalPath::from_rest_decoded(&decoded),
                Err(AppError::BadRequest(_))
            ));
            assert!(matches!(
                CanonicalPath::from_mcp_literal(&decoded),
                Err(AppError::BadRequest(_))
            ));
            let encoded = format!(
                "/v4/spreadsheets/id/values/{}{suffix}",
                urlencoding::encode(range)
            );
            assert!(matches!(
                CanonicalPath::from_mcp_built(&encoded),
                Err(AppError::BadRequest(_))
            ));
        }
    }

    #[test]
    fn existing_google_policies_keep_spaces_and_percent_out_of_ordinary_ids() {
        use crate::services::google_workspace::GoogleProduct;
        for (product, prefix) in [
            (GoogleProduct::Workspace, "/drive/v3/files"),
            (GoogleProduct::Drive, "/drive/v3/files"),
            (GoogleProduct::Calendar, "/calendar/v3/calendars"),
            (GoogleProduct::Gmail, "/gmail/v1/users/me/messages"),
        ] {
            let policy = product.operation_policy().unwrap();
            let plain = CanonicalPath::from_rest_decoded(&format!("{prefix}/id")).unwrap();
            authorize_proxy_operation_fields("s", "s", Some(&policy), "GET", &plain).unwrap();
            let spaced = CanonicalPath::from_rest_decoded(&format!("{prefix}/id value")).unwrap();
            assert!(matches!(
                authorize_proxy_operation_fields("s", "s", Some(&policy), "GET", &spaced),
                Err(AppError::NotFound(_))
            ));
            assert!(matches!(
                CanonicalPath::from_rest_decoded(&format!("{prefix}/id%value")),
                Err(AppError::BadRequest(_))
            ));
        }
    }

    #[test]
    fn sheets_range_constraint_is_explicit_and_scoped_to_its_parameter() {
        let policy = crate::services::google_workspace::GoogleProduct::Sheets
            .operation_policy()
            .unwrap();
        for range in [
            "Sheet1!A1:B2",
            "'Quarter 1'!$A$1:$B$2",
            "'O''Brien!'!A:A",
            "A5:A",
            "1:10",
            "A1",
            "Revenue",
            "'Sheet 1'",
            "数据!A1:B2",
            "a1:b2",
            "'Sheet!Name'",
        ] {
            for (method, suffix) in [
                ("GET", ""),
                ("PUT", ""),
                ("POST", ":append"),
                ("POST", ":clear"),
            ] {
                let path = CanonicalPath::from_mcp_literal(&format!(
                    "/v4/spreadsheets/id/values/{range}{suffix}"
                ))
                .unwrap();
                let forwarded =
                    authorize_proxy_operation_fields("s", "s", Some(&policy), method, &path)
                        .unwrap_or_else(|e| panic!("{method} {range}{suffix}: {e}"));
                assert_eq!(
                    forwarded.matches(':').count(),
                    usize::from(!suffix.is_empty()),
                    "only the declared method delimiter can stay literal"
                );
                assert_eq!(
                    urlencoding::decode(&forwarded).unwrap(),
                    path.as_policy_path().trim_start_matches('/')
                );
                // An ordinary wildcard never inherits range punctuation permission.
                let plain = ProxyOperationRule {
                    method: method.into(),
                    path_template: format!(
                        "/v4/spreadsheets/{{spreadsheetId}}/values/{{range}}{suffix}"
                    ),
                    ..Default::default()
                };
                if range.contains(':') || range.contains(' ') {
                    assert!(!rule_matches(&plain, method, &path));
                }
            }
        }
        for range in [
            "A1:clear",
            "A1:append",
            "A1:B2:clear",
            "A1:B2:other",
            "A1:",
            ":B2",
            "A0:B2",
            "A1:B0",
            "A1:AAAA",
            "Sheet 1!A1:B2",
            "'Bad:Title'!A1:B2",
            "'unmatched!A1:B2",
        ] {
            let path =
                CanonicalPath::from_mcp_literal(&format!("/v4/spreadsheets/id/values/{range}"))
                    .unwrap();
            assert!(
                authorize_proxy_operation_fields("s", "s", Some(&policy), "GET", &path).is_err(),
                "{range}"
            );
        }
        let path = CanonicalPath::from_mcp_literal("/v4/spreadsheets/A1:B2/values/A1").unwrap();
        assert!(authorize_proxy_operation_fields("s", "s", Some(&policy), "GET", &path).is_err());
    }

    #[test]
    fn a1_column_names_cannot_become_custom_method_delimiters() {
        let parameters = serde_json::json!([{"name":"range", "in":"path", "x-nyxid-path-constraint":"sheets_a1_range"}]);
        let rule = rule_from_endpoint("GET", "/values/{range}", Some(&parameters)).unwrap();
        let path = CanonicalPath::from_mcp_literal("/values/A1:get").unwrap();
        assert_eq!(
            rule_forwarding_path(&rule, "GET", &path).unwrap(),
            "values/A1%3Aget"
        );
    }

    #[test]
    fn unknown_or_misplaced_path_constraints_fail_closed() {
        for parameters in [
            serde_json::json!([{"name":"range", "in":"path", "x-nyxid-path-constraint":"anything"}]),
            serde_json::json!([{"name":"range", "in":"query", "x-nyxid-path-constraint":"sheets_a1_range"}]),
            serde_json::json!([{"name":"missing", "in":"path", "x-nyxid-path-constraint":"sheets_a1_range"}]),
        ] {
            assert!(rule_from_endpoint("GET", "/values/{range}", Some(&parameters)).is_err());
        }
    }

    fn duffel_policy() -> ProxyOperationPolicy {
        normalize_policy(ProxyOperationPolicy {
            rules: vec![
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/offer_requests".into(),
                    ..Default::default()
                },
                ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/air/offers".into(),
                    ..Default::default()
                },
                ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/air/offers/{id}".into(),
                    ..Default::default()
                },
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/orders".into(),
                    ..Default::default()
                },
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/payments".into(),
                    ..Default::default()
                },
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/order_cancellations".into(),
                    ..Default::default()
                },
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/order_cancellations/{id}/actions/confirm".into(),
                    ..Default::default()
                },
            ],
        })
        .expect("valid policy")
    }

    #[test]
    fn duffel_policy_allows_only_declared_operations() {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.proxy_operation_policy = Some(duffel_policy());
        let allowed = [
            ("POST", "/air/offer_requests"),
            ("GET", "/air/offers"),
            ("GET", "/air/offers/off_123"),
            ("POST", "/air/orders"),
            ("POST", "/air/payments"),
            ("POST", "/air/order_cancellations"),
            ("POST", "/air/order_cancellations/orc_123/actions/confirm"),
        ];
        for (method, path) in allowed {
            let path = CanonicalPath::from_mcp_literal(path).expect("canonical path");
            authorize_proxy_operation(&service, method, &path).unwrap_or_else(|error| {
                panic!("{method} {} was denied: {error}", path.as_policy_path())
            });
        }
        for (method, path) in [
            ("GET", "/air/orders"),
            ("GET", "/air/orders/ord_123"),
            ("POST", "/identity/component_client_keys"),
        ] {
            let path = CanonicalPath::from_mcp_literal(path).expect("canonical path");
            assert!(
                authorize_proxy_operation(&service, method, &path).is_err(),
                "{method} {} was allowed",
                path.as_policy_path()
            );
        }
    }

    #[test]
    fn missing_policy_preserves_passthrough_behavior() {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.proxy_operation_policy = None;
        let path = CanonicalPath::from_mcp_literal("/any/current/operation").unwrap();
        authorize_proxy_operation(&service, "DELETE", &path)
            .expect("a row without a policy must preserve current passthrough behavior");
    }

    #[test]
    fn present_empty_policy_denies_every_operation() {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.proxy_operation_policy = Some(ProxyOperationPolicy { rules: vec![] });
        let path = CanonicalPath::from_mcp_literal("/any/current/operation").unwrap();
        assert!(authorize_proxy_operation(&service, "GET", &path).is_err());
    }

    #[test]
    fn canonical_paths_reject_ambiguous_spellings() {
        for path in [
            "/air//offers",
            "/air/offers/",
            "/air/../orders",
            "/air/%2Forders",
            "/air\\orders",
        ] {
            assert!(
                CanonicalPath::from_mcp_literal(path).is_err(),
                "accepted {path}"
            );
        }
        assert_ne!(
            CanonicalPath::from_mcp_literal("/air/offers").unwrap(),
            CanonicalPath::from_mcp_literal("/Air/offers").unwrap()
        );
    }

    #[test]
    fn rest_and_mcp_canonicalizers_forward_equivalent_paths_identically() {
        let rest = CanonicalPath::from_rest_decoded("air/offers/off_123").unwrap();
        let mcp = CanonicalPath::from_mcp_literal("/air/offers/off_123").unwrap();
        assert_eq!(rest, mcp);
        assert_eq!(
            authorize_proxy_operation_fields("s", "s", Some(&duffel_policy()), "GET", &rest)
                .unwrap(),
            authorize_proxy_operation_fields("s", "s", Some(&duffel_policy()), "GET", &mcp)
                .unwrap()
        );
    }

    fn docs_policy() -> ProxyOperationPolicy {
        normalize_policy(ProxyOperationPolicy {
            rules: vec![
                ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/v1/documents/{documentId}".into(),
                    ..Default::default()
                },
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/v1/documents/{documentId}:batchUpdate".into(),
                    ..Default::default()
                },
            ],
        })
        .expect("valid policy")
    }

    /// Google's Docs/Sheets/Slides APIs address custom methods with a colon.
    /// Percent-encoding it makes Google answer 400, so the colon has to survive
    /// canonicalization and forwarding intact.
    #[test]
    fn aip_custom_methods_survive_canonicalization_and_forwarding() {
        let path = CanonicalPath::from_mcp_built("/v1/documents/abc:batchUpdate").unwrap();
        assert_eq!(
            rule_forwarding_path(&docs_policy().rules[1], "POST", &path).unwrap(),
            "v1/documents/abc:batchUpdate"
        );
        assert_eq!(path.as_policy_path(), "/v1/documents/abc:batchUpdate");

        let rest = CanonicalPath::from_rest_decoded("v1/documents/abc:batchUpdate").unwrap();
        assert_eq!(
            rule_forwarding_path(&docs_policy().rules[1], "POST", &rest).unwrap(),
            rule_forwarding_path(&docs_policy().rules[1], "POST", &path).unwrap()
        );
    }

    #[test]
    fn everything_except_the_colon_is_still_encoded() {
        let path = CanonicalPath::from_rest_decoded("v1/documents/a@b:batchUpdate").unwrap();
        assert_eq!(
            rule_forwarding_path(&docs_policy().rules[1], "POST", &path).unwrap(),
            "v1/documents/a%40b:batchUpdate"
        );
    }

    #[test]
    fn custom_method_policy_matches_only_the_declared_verb() {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.proxy_operation_policy = Some(docs_policy());

        let allowed = CanonicalPath::from_mcp_literal("/v1/documents/abc:batchUpdate").unwrap();
        authorize_proxy_operation(&service, "POST", &allowed).expect("declared verb is allowed");

        for (method, path) in [
            // A different custom verb on the same resource is a different operation.
            ("POST", "/v1/documents/abc:batchDelete"),
            // The bare resource is not the custom method.
            ("POST", "/v1/documents/abc"),
            // An empty resource part must never satisfy the parameter.
            ("POST", "/v1/documents/:batchUpdate"),
            // A second colon must not be absorbed into the resource.
            ("POST", "/v1/documents/abc:x:batchUpdate"),
        ] {
            let path = CanonicalPath::from_mcp_literal(path).unwrap();
            assert!(
                authorize_proxy_operation(&service, method, &path).is_err(),
                "{method} {} was allowed",
                path.as_policy_path()
            );
        }
    }

    /// Now that `:` is forwarded literally, a bare `{id}` must not be a hole
    /// through which an un-allowlisted custom method reaches the upstream.
    #[test]
    fn a_bare_parameter_does_not_smuggle_a_custom_method() {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.proxy_operation_policy = Some(docs_policy());

        let smuggled = CanonicalPath::from_mcp_literal("/v1/documents/abc:delete").unwrap();
        assert!(
            authorize_proxy_operation(&service, "GET", &smuggled).is_err(),
            "GET /v1/documents/{{documentId}} must not match a colon custom method"
        );

        let plain = CanonicalPath::from_mcp_literal("/v1/documents/abc").unwrap();
        authorize_proxy_operation(&service, "GET", &plain).expect("the plain resource still works");
    }

    #[test]
    fn custom_method_templates_reject_malformed_verbs() {
        for template in [
            "/v1/documents/{documentId}:",
            "/v1/documents/:batchUpdate",
            "/v1/documents/{documentId}:batch:update",
            "/v1/documents/{documentId}:batch-update",
            "/v1/documents/{documentId}:{verb}",
        ] {
            assert!(
                normalize_policy(ProxyOperationPolicy {
                    rules: vec![ProxyOperationRule {
                        method: "POST".into(),
                        path_template: template.into(),
                        ..Default::default()
                    }]
                })
                .is_err(),
                "accepted {template}"
            );
        }
        normalize_policy(ProxyOperationPolicy {
            rules: vec![ProxyOperationRule {
                method: "POST".into(),
                path_template: "/v1/documents/{documentId}:batchUpdate".into(),
                ..Default::default()
            }],
        })
        .expect("a well-formed custom method is accepted");
    }

    #[test]
    fn template_grammar_is_deliberately_small() {
        for template in [
            "air/offers",
            "/air/**",
            "/air/[a-z]",
            "/air/{id}/",
            "/air/{bad-name}",
        ] {
            assert!(
                normalize_policy(ProxyOperationPolicy {
                    rules: vec![ProxyOperationRule {
                        method: "GET".into(),
                        path_template: template.into(),
                        ..Default::default()
                    }]
                })
                .is_err(),
                "accepted {template}"
            );
        }
    }
}
