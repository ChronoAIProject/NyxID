use std::net::IpAddr;

use chrono::Utc;
use mongodb::bson::{self, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::anonymous_endpoint_usage::{
    AnonymousEndpointUsage, COLLECTION_NAME as ANONYMOUS_ENDPOINT_USAGE,
};
use crate::models::downstream_service::{
    AnonymousEndpointRule, COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};

pub const MAX_ANONYMOUS_ENDPOINTS_PER_SERVICE: usize = 100;
pub const MAX_ANONYMOUS_PATH_PATTERN_LEN: usize = 512;
pub const MAX_ANONYMOUS_METHOD_LEN: usize = 16;
pub const DEFAULT_PUBLIC_PROXY_MAX_BODY_SIZE: usize = 1_048_576;
pub const DEFAULT_PUBLIC_PROXY_RATE_LIMIT_PER_MINUTE: u32 = 60;
pub const DEFAULT_PUBLIC_MCP_RATE_LIMIT_PER_MINUTE: u32 = 30;

const PUBLIC_AUDIT_PATH_MAX_LEN: usize = 256;
const PUBLIC_AUDIT_IP_MAX_LEN: usize = 64;
const PUBLIC_AUDIT_UA_MAX_LEN: usize = 256;

const SAFE_RESPONSE_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-encoding",
    "content-language",
    "content-disposition",
    "cache-control",
    "etag",
    "last-modified",
    "x-request-id",
    "x-correlation-id",
    "accept-ranges",
    "content-range",
];

const STRIPPED_RESPONSE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authenticate",
    "proxy-authorization",
    "set-cookie",
    "cookie",
    "www-authenticate",
    "x-nyxid-access-token",
    "x-nyxid-refresh-token",
    "x-nyxid-session",
    "x-nyxid-agent-id",
    "x-nyxid-connection-id",
    "x-nyxid-delegation-token",
    "x-nyxid-identity-token",
];

#[derive(Clone, Debug)]
pub struct AnonymousRuleInput {
    pub enabled: bool,
    pub method: String,
    pub path_pattern: String,
    pub daily_quota: u32,
}

#[derive(Clone, Debug)]
pub struct AnonymousRuleUpdate {
    pub enabled: Option<bool>,
    pub method: Option<String>,
    pub path_pattern: Option<String>,
    pub daily_quota: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct MatchedAnonymousEndpoint {
    pub service: DownstreamService,
    pub rule: AnonymousEndpointRule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicAuditEvent {
    pub service_id: String,
    pub service_slug: String,
    pub rule_id: String,
    pub method: String,
    pub path: String,
    pub response_status: Option<u16>,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    pub quota_used: Option<i64>,
    pub quota_limit: u32,
}

pub fn anonymous_service_is_runtime_safe(service: &DownstreamService) -> bool {
    service.identity_propagation_mode == "none"
        && !service.forward_access_token
        && !service.inject_delegation_token
}

pub fn has_enabled_anonymous_endpoints(service: &DownstreamService) -> bool {
    service.anonymous_endpoints.iter().any(|rule| rule.enabled)
}

pub fn validate_anonymous_service_runtime_safety(service: &DownstreamService) -> AppResult<()> {
    if has_enabled_anonymous_endpoints(service) && !anonymous_service_is_runtime_safe(service) {
        return Err(AppError::AnonymousIncompatibleService(
            "Enabled anonymous endpoints require identity_propagation_mode='none', forward_access_token=false, and inject_delegation_token=false".to_string(),
        ));
    }
    Ok(())
}

pub fn validate_service_update_anonymous_compatibility(
    service: &DownstreamService,
    next_identity_mode: Option<&str>,
    next_forward_access_token: Option<bool>,
    next_inject_delegation_token: Option<bool>,
    next_rules: Option<&[AnonymousEndpointRule]>,
) -> AppResult<()> {
    let has_enabled = next_rules
        .map(|rules| rules.iter().any(|rule| rule.enabled))
        .unwrap_or_else(|| has_enabled_anonymous_endpoints(service));
    if !has_enabled {
        return Ok(());
    }

    let identity_mode = next_identity_mode.unwrap_or(&service.identity_propagation_mode);
    let forward_access_token = next_forward_access_token.unwrap_or(service.forward_access_token);
    let inject_delegation_token =
        next_inject_delegation_token.unwrap_or(service.inject_delegation_token);

    if identity_mode != "none" || forward_access_token || inject_delegation_token {
        return Err(AppError::AnonymousIncompatibleService(
            "Enabled anonymous endpoints require identity_propagation_mode='none', forward_access_token=false, and inject_delegation_token=false".to_string(),
        ));
    }
    Ok(())
}

pub fn build_rule(input: AnonymousRuleInput) -> AppResult<AnonymousEndpointRule> {
    let rule = AnonymousEndpointRule {
        id: Uuid::new_v4().to_string(),
        enabled: input.enabled,
        method: normalize_method(&input.method)?,
        path_pattern: normalize_path_pattern(&input.path_pattern)?,
        daily_quota: validate_daily_quota(input.daily_quota)?,
    };
    Ok(rule)
}

pub fn apply_rule_update(
    existing: &AnonymousEndpointRule,
    update: AnonymousRuleUpdate,
) -> AppResult<AnonymousEndpointRule> {
    Ok(AnonymousEndpointRule {
        id: existing.id.clone(),
        enabled: update.enabled.unwrap_or(existing.enabled),
        method: match update.method {
            Some(method) => normalize_method(&method)?,
            None => existing.method.clone(),
        },
        path_pattern: match update.path_pattern {
            Some(pattern) => normalize_path_pattern(&pattern)?,
            None => existing.path_pattern.clone(),
        },
        daily_quota: match update.daily_quota {
            Some(quota) => validate_daily_quota(quota)?,
            None => existing.daily_quota,
        },
    })
}

pub fn validate_rules_for_service(
    service: &DownstreamService,
    rules: &[AnonymousEndpointRule],
) -> AppResult<()> {
    validate_rule_list(rules)?;
    validate_service_update_anonymous_compatibility(service, None, None, None, Some(rules))
}

pub fn validate_rule_list(rules: &[AnonymousEndpointRule]) -> AppResult<()> {
    if rules.len() > MAX_ANONYMOUS_ENDPOINTS_PER_SERVICE {
        return Err(AppError::ValidationError(format!(
            "anonymous_endpoints must not exceed {MAX_ANONYMOUS_ENDPOINTS_PER_SERVICE} entries"
        )));
    }

    let mut ids = std::collections::HashSet::with_capacity(rules.len());
    for rule in rules {
        if rule.id.trim().is_empty() {
            return Err(AppError::ValidationError(
                "anonymous_endpoints[].id must not be empty".to_string(),
            ));
        }
        if !ids.insert(rule.id.as_str()) {
            return Err(AppError::ValidationError(format!(
                "anonymous_endpoints contains duplicate id '{}'",
                rule.id
            )));
        }
        normalize_method(&rule.method)?;
        normalize_path_pattern(&rule.path_pattern)?;
        validate_daily_quota(rule.daily_quota)?;
    }
    Ok(())
}

pub fn normalize_method(method: &str) -> AppResult<String> {
    let method = method.trim().to_ascii_uppercase();
    if method.is_empty() || method.len() > MAX_ANONYMOUS_METHOD_LEN {
        return Err(AppError::ValidationError(
            "Anonymous endpoint method must be between 1 and 16 characters".to_string(),
        ));
    }
    let valid = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
    if !valid.contains(&method.as_str()) {
        return Err(AppError::ValidationError(format!(
            "Anonymous endpoint method must be one of: {}",
            valid.join(", ")
        )));
    }
    Ok(method)
}

pub fn normalize_path_pattern(path_pattern: &str) -> AppResult<String> {
    let mut pattern = path_pattern.trim().to_string();
    if pattern.is_empty() {
        return Err(AppError::ValidationError(
            "Anonymous endpoint path_pattern must not be empty".to_string(),
        ));
    }
    if pattern.len() > MAX_ANONYMOUS_PATH_PATTERN_LEN {
        return Err(AppError::ValidationError(format!(
            "Anonymous endpoint path_pattern must not exceed {MAX_ANONYMOUS_PATH_PATTERN_LEN} characters"
        )));
    }
    if !pattern.starts_with('/') {
        pattern.insert(0, '/');
    }
    if pattern.contains('\\')
        || pattern.contains('\0')
        || pattern.contains('?')
        || pattern.contains('#')
        || pattern.contains("//")
        || pattern
            .split('/')
            .any(|segment| segment == "." || segment == "..")
    {
        return Err(AppError::ValidationError(
            "Anonymous endpoint path_pattern is invalid".to_string(),
        ));
    }
    if pattern.contains('*') && !pattern.ends_with("/**") {
        return Err(AppError::ValidationError(
            "Anonymous endpoint wildcard must be a trailing /** segment".to_string(),
        ));
    }
    Ok(pattern)
}

pub fn validate_daily_quota(quota: u32) -> AppResult<u32> {
    if quota == 0 {
        return Err(AppError::ValidationError(
            "Anonymous endpoint daily_quota must be at least 1".to_string(),
        ));
    }
    Ok(quota)
}

pub async fn find_matching_enabled_rule(
    db: &mongodb::Database,
    slug: &str,
    method: &str,
    path: &str,
) -> AppResult<MatchedAnonymousEndpoint> {
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! {
            "slug": slug,
            "is_active": true,
            "service_type": "http",
            "anonymous_endpoints": {
                "$elemMatch": {
                    "enabled": true,
                    "method": method.to_ascii_uppercase(),
                }
            }
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Public endpoint not found".to_string()))?;

    let method = normalize_method(method)?;
    let path = normalize_runtime_path(path)?;
    let rule = service
        .anonymous_endpoints
        .iter()
        .find(|rule| {
            rule.enabled && rule.method == method && path_matches(&rule.path_pattern, &path)
        })
        .cloned()
        .ok_or_else(|| AppError::NotFound("Public endpoint not found".to_string()))?;

    validate_anonymous_service_runtime_safety(&service)?;

    Ok(MatchedAnonymousEndpoint { service, rule })
}

pub fn normalize_runtime_path(path: &str) -> AppResult<String> {
    let mut normalized = path.trim_start_matches('/').to_string();
    if normalized.is_empty() {
        normalized = String::new();
    }
    let path = if normalized.is_empty() {
        "/".to_string()
    } else {
        format!("/{normalized}")
    };
    normalize_path_pattern(&path)
}

pub fn path_matches(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/**") {
        if prefix.is_empty() {
            return path.starts_with('/');
        }
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    pattern == path
}

pub async fn increment_daily_usage(
    db: &mongodb::Database,
    service_id: &str,
    rule_id: &str,
    daily_quota: u32,
) -> AppResult<i64> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let id = format!("{service_id}:{rule_id}:{today}");
    let collection = db.collection::<AnonymousEndpointUsage>(ANONYMOUS_ENDPOINT_USAGE);

    // The counter is incremented with an atomic conditional update (no upsert):
    // `{ _id, count < quota }` only matches a doc still under quota. If it
    // matches, the increment is applied and the new count returned. If it does
    // NOT match, we must distinguish two cases:
    //   - the doc exists (count has reached the quota) -> RateLimited (429), or
    //   - the doc is missing (first request of the day) -> create it at count 1.
    // The first-insert path can lose a race against a concurrent first request;
    // that surfaces as an E11000 duplicate-key error, which is NOT a quota hit,
    // so we loop back and retry the conditional increment instead of reporting
    // RateLimited. The loop is bounded; RateLimited is the final fallback.
    const MAX_ATTEMPTS: usize = 3;
    for _ in 0..MAX_ATTEMPTS {
        let now = Utc::now();
        let options = FindOneAndUpdateOptions::builder()
            .upsert(false)
            .return_document(ReturnDocument::After)
            .build();

        let updated = collection
            .find_one_and_update(
                doc! { "_id": &id, "count": { "$lt": i64::from(daily_quota) } },
                doc! {
                    "$inc": { "count": 1_i64 },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) },
                },
            )
            .with_options(options)
            .await?;

        if let Some(usage) = updated {
            return Ok(usage.count);
        }

        // No matching under-quota doc. If a doc already exists for this key, the
        // quota is exhausted; deny with RateLimited.
        let exists = collection.find_one(doc! { "_id": &id }).await?.is_some();
        if exists {
            return Err(AppError::RateLimited);
        }

        // First request of the day: insert a fresh counter at count 1.
        let fresh = AnonymousEndpointUsage {
            id: id.clone(),
            service_id: service_id.to_string(),
            rule_id: rule_id.to_string(),
            day: today.clone(),
            count: 1,
            created_at: now,
            updated_at: now,
        };
        match collection.insert_one(&fresh).await {
            Ok(_) => return Ok(1),
            // A concurrent first request won the insert race. Retry the
            // conditional increment rather than treating this as a quota hit.
            Err(error) if is_duplicate_key_error(&error) => continue,
            Err(error) => return Err(AppError::DatabaseError(error)),
        }
    }

    // All attempts exhausted under contention; fail closed.
    Err(AppError::RateLimited)
}

/// Returns true if the given MongoDB error is an E11000 unique-index violation.
fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    if let mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we)) =
        error.kind.as_ref()
    {
        return we.code == 11000;
    }
    false
}

pub fn is_safe_public_response_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SAFE_RESPONSE_HEADERS.contains(&lower.as_str())
        && !STRIPPED_RESPONSE_HEADERS.contains(&lower.as_str())
        && !lower.starts_with("x-nyxid-")
        && !lower.starts_with("x-powered-by")
}

pub struct PublicAuditInput<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub response_status: Option<u16>,
    pub client_ip: Option<IpAddr>,
    pub user_agent: Option<&'a str>,
    pub quota_used: Option<i64>,
}

pub fn bounded_public_audit_event(
    service: &DownstreamService,
    rule: &AnonymousEndpointRule,
    input: PublicAuditInput<'_>,
) -> PublicAuditEvent {
    PublicAuditEvent {
        service_id: service.id.clone(),
        service_slug: service.slug.clone(),
        rule_id: rule.id.clone(),
        method: truncate_for_audit(input.method, MAX_ANONYMOUS_METHOD_LEN),
        path: truncate_for_audit(input.path, PUBLIC_AUDIT_PATH_MAX_LEN),
        response_status: input.response_status,
        client_ip: input
            .client_ip
            .map(|ip| truncate_for_audit(&ip.to_string(), PUBLIC_AUDIT_IP_MAX_LEN)),
        user_agent: input
            .user_agent
            .map(|ua| truncate_for_audit(ua, PUBLIC_AUDIT_UA_MAX_LEN)),
        quota_used: input.quota_used,
        quota_limit: rule.daily_quota,
    }
}

pub fn truncate_for_audit(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub fn public_audit_json(event: PublicAuditEvent) -> serde_json::Value {
    serde_json::json!({
        "service_id": event.service_id,
        "service_slug": event.service_slug,
        "rule_id": event.rule_id,
        "method": event.method,
        "path": event.path,
        "response_status": event.response_status,
        "client_ip": event.client_ip,
        "user_agent": event.user_agent,
        "quota_used": event.quota_used,
        "quota_limit": event.quota_limit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compatible_service() -> DownstreamService {
        DownstreamService {
            id: "svc-1".to_string(),
            name: "Service".to_string(),
            slug: "svc".to_string(),
            description: None,
            base_url: "https://example.com".to_string(),
            service_type: "http".to_string(),
            visibility: "public".to_string(),
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            credential_encrypted: vec![],
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: None,
            oauth_client_id: None,
            service_category: "internal".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: "admin".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            provider_config_id: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            anonymous_endpoints: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn path_matching_supports_exact_and_trailing_wildcard() {
        assert!(path_matches("/public/a", "/public/a"));
        assert!(!path_matches("/public/a", "/public/a/b"));
        assert!(path_matches("/public/**", "/public"));
        assert!(path_matches("/public/**", "/public/a/b"));
        assert!(!path_matches("/public/**", "/private/a"));
    }

    #[test]
    fn disabled_rules_do_not_require_identity_safety() {
        let mut service = compatible_service();
        service.identity_propagation_mode = "headers".to_string();
        service.anonymous_endpoints = vec![AnonymousEndpointRule {
            id: "draft".to_string(),
            enabled: false,
            method: "GET".to_string(),
            path_pattern: "/public/**".to_string(),
            daily_quota: 1,
        }];

        assert!(validate_anonymous_service_runtime_safety(&service).is_ok());
    }

    #[test]
    fn enabled_rules_reject_identity_exposure() {
        let mut service = compatible_service();
        service.forward_access_token = true;
        service.anonymous_endpoints = vec![AnonymousEndpointRule {
            id: "enabled".to_string(),
            enabled: true,
            method: "GET".to_string(),
            path_pattern: "/public/**".to_string(),
            daily_quota: 1,
        }];

        let err = validate_anonymous_service_runtime_safety(&service).unwrap_err();
        assert!(matches!(err, AppError::AnonymousIncompatibleService(_)));
    }

    #[test]
    fn response_header_sanitizer_strips_auth_and_session_headers() {
        assert!(is_safe_public_response_header("content-type"));
        assert!(!is_safe_public_response_header("authorization"));
        assert!(!is_safe_public_response_header("set-cookie"));
        assert!(!is_safe_public_response_header("x-nyxid-agent-id"));
        assert!(!is_safe_public_response_header("www-authenticate"));
    }

    #[test]
    fn audit_event_bounds_user_agent_and_path() {
        let service = compatible_service();
        let rule = AnonymousEndpointRule {
            id: "rule-1".to_string(),
            enabled: true,
            method: "GET".to_string(),
            path_pattern: "/public/**".to_string(),
            daily_quota: 10,
        };
        let event = bounded_public_audit_event(
            &service,
            &rule,
            PublicAuditInput {
                method: "GET",
                path: &"x".repeat(400),
                response_status: Some(200),
                client_ip: None,
                user_agent: Some(&"u".repeat(500)),
                quota_used: Some(1),
            },
        );

        assert_eq!(event.path.len(), PUBLIC_AUDIT_PATH_MAX_LEN);
        assert_eq!(event.user_agent.unwrap().len(), PUBLIC_AUDIT_UA_MAX_LEN);
    }

    #[test]
    fn duplicate_key_detector_only_matches_e11000() {
        // A non-write error (e.g. a custom error) must not be treated as a
        // duplicate-key violation.
        let custom = mongodb::error::Error::custom("boom");
        assert!(!is_duplicate_key_error(&custom));
    }

    /// The first request of the day creates the counter and returns 1; the
    /// over-quota request returns `RateLimited` (HTTP 429), never a raw
    /// database error (HTTP 500).
    #[tokio::test]
    async fn increment_daily_usage_first_then_rate_limited_at_quota() {
        let Some(db) = crate::test_utils::connect_test_database("anon_usage_quota").await else {
            return;
        };

        // First request of the day with no existing doc -> creates counter, count 1.
        let first = increment_daily_usage(&db, "svc-1", "rule-1", 2)
            .await
            .expect("first request creates counter");
        assert_eq!(first, 1, "first request of the day returns count 1");

        // Second request still under quota (quota = 2) -> count 2.
        let second = increment_daily_usage(&db, "svc-1", "rule-1", 2)
            .await
            .expect("second request still under quota");
        assert_eq!(second, 2);

        // Third request is at/over quota -> RateLimited (429), NOT a 500/E11000.
        let err = increment_daily_usage(&db, "svc-1", "rule-1", 2)
            .await
            .expect_err("over-quota request must be denied");
        assert!(
            matches!(err, AppError::RateLimited),
            "over-quota request must be RateLimited (429), got {err:?}"
        );

        // A subsequent over-quota request stays RateLimited (idempotent denial).
        let err = increment_daily_usage(&db, "svc-1", "rule-1", 2)
            .await
            .expect_err("still over quota");
        assert!(matches!(err, AppError::RateLimited), "got {err:?}");
    }

    /// A quota of 1 is denied on the very second call with RateLimited.
    #[tokio::test]
    async fn increment_daily_usage_quota_one_denies_second_call() {
        let Some(db) = crate::test_utils::connect_test_database("anon_usage_quota1").await else {
            return;
        };

        let first = increment_daily_usage(&db, "svc-2", "rule-2", 1)
            .await
            .expect("first request within quota");
        assert_eq!(first, 1);

        let err = increment_daily_usage(&db, "svc-2", "rule-2", 1)
            .await
            .expect_err("second request must be denied");
        assert!(
            matches!(err, AppError::RateLimited),
            "got {err:?}, expected RateLimited"
        );
    }
}
