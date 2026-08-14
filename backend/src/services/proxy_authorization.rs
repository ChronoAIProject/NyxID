use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    DownstreamService, ProxyOperationPolicy, ProxyOperationRule,
};

/// A normalized downstream path used for both policy matching and forwarding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalPath {
    segments: Vec<String>,
}

impl CanonicalPath {
    /// Construct from an Axum-decoded REST wildcard after the raw OriginalUri
    /// has passed `validate_requested_proxy_path`.
    pub fn from_rest_decoded(path: &str) -> AppResult<Self> {
        Self::from_decoded(path, false)
    }

    /// Construct from a caller-supplied MCP generic path. Percent signs are
    /// rejected because MCP arguments are literal rather than router-decoded.
    pub fn from_mcp_literal(path: &str) -> AppResult<Self> {
        Self::from_decoded(path, true)
    }

    /// Construct from a typed MCP endpoint after its declared path parameters
    /// have been substituted and encoded by the existing argument builder.
    pub fn from_mcp_built(path: &str) -> AppResult<Self> {
        reject_common_ambiguity(path, false)?;
        let decoded = percent_decode_path(path)?;
        Self::from_decoded(&decoded, true)
    }

    fn from_decoded(path: &str, reject_percent: bool) -> AppResult<Self> {
        reject_common_ambiguity(path, reject_percent)?;
        let trimmed = path.strip_prefix('/').unwrap_or(path);
        if trimmed.is_empty() {
            return Ok(Self { segments: vec![] });
        }
        let segments = trimmed.split('/').map(str::to_string).collect();
        Ok(Self { segments })
    }

    pub fn as_policy_path(&self) -> String {
        if self.segments.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", self.segments.join("/"))
        }
    }

    /// Deterministically encode the exact canonical segments for forwarding.
    pub fn forwarding_path(&self) -> String {
        self.segments
            .iter()
            .map(|segment| urlencoding::encode(segment).into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }
}

fn reject_common_ambiguity(path: &str, reject_percent: bool) -> AppResult<()> {
    if path.contains('?')
        || path.contains('#')
        || path.contains('\\')
        || (reject_percent && path.contains('%'))
        || path.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
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
        validate_template(&rule.path_template)?;
        rules.push(ProxyOperationRule {
            method,
            path_template: rule.path_template,
        });
    }
    Ok(ProxyOperationPolicy { rules })
}

fn validate_template(template: &str) -> AppResult<()> {
    if template.len() > 2048 {
        return Err(AppError::ValidationError(
            "proxy operation path template exceeds 2048 bytes".to_string(),
        ));
    }
    if !template.starts_with('/') {
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
        if segment.starts_with('{') || segment.ends_with('}') {
            if !(segment.starts_with('{')
                && segment.ends_with('}')
                && segment.len() > 2
                && !segment[1..segment.len() - 1]
                    .chars()
                    .any(|ch| !ch.is_ascii_alphanumeric() && ch != '_'))
            {
                return Err(AppError::ValidationError(
                    "Invalid proxy operation path template".to_string(),
                ));
            }
        } else if segment.contains(['*', '[', ']', '(', ')', '|', '{', '}']) {
            return Err(AppError::ValidationError(
                "Invalid proxy operation path template".to_string(),
            ));
        }
    }
    Ok(())
}

fn rule_matches(rule: &ProxyOperationRule, method: &str, path: &CanonicalPath) -> bool {
    if rule.method != method {
        return false;
    }
    let template_segments: Vec<&str> = rule
        .path_template
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    template_segments.len() == path.segments.len()
        && template_segments
            .iter()
            .zip(&path.segments)
            .all(|(template, actual)| {
                (template.starts_with('{') && template.ends_with('}')) || *template == actual
            })
}

pub fn authorize_proxy_operation(
    service: &DownstreamService,
    method: &str,
    path: &CanonicalPath,
) -> AppResult<()> {
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
) -> AppResult<()> {
    let method = method.trim().to_ascii_uppercase();
    let decision = match policy {
        Some(policy) => policy
            .rules
            .iter()
            .any(|rule| rule_matches(rule, &method, path)),
        None => true,
    };
    if decision {
        return Ok(());
    }

    tracing::warn!(
        service_id = %service_id,
        service_slug = %service_slug,
        method = %method,
        path = %path.as_policy_path(),
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

    fn duffel_policy() -> ProxyOperationPolicy {
        normalize_policy(ProxyOperationPolicy {
            rules: vec![
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/offer_requests".into(),
                },
                ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/air/offers".into(),
                },
                ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/air/offers/{id}".into(),
                },
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/orders".into(),
                },
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/payments".into(),
                },
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/order_cancellations".into(),
                },
                ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/air/order_cancellations/{id}/actions/confirm".into(),
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
        assert_eq!(rest.forwarding_path(), mcp.forwarding_path());
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
                        path_template: template.into()
                    }]
                })
                .is_err(),
                "accepted {template}"
            );
        }
    }
}
