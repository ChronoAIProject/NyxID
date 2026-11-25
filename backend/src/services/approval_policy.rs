use globset::{Glob, GlobBuilder, GlobMatcher};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::models::service_approval_config::{
    ApprovalEffect, ApprovalMode, ApprovalRule, ApprovalVerb, ServiceApprovalConfig,
};
use crate::services::operation_descriptor::{OperationDescriptor, Protocol};

/// Global preference is distinct from the per-service binary/rule policy.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GlobalApprovalMode {
    WritesOnly,
    All,
    Off,
}

impl GlobalApprovalMode {
    /// Normalize update intent without confusing an omitted update with Writes only.
    pub fn from_update(
        legacy: Option<bool>,
        mode: Option<Self>,
    ) -> crate::errors::AppResult<Option<Self>> {
        if let (Some(legacy), Some(mode)) = (legacy, mode)
            && mode.stored() != Some(legacy)
        {
            return Err(crate::errors::AppError::ValidationError(
                "approval_required conflicts with global_approval_mode".to_string(),
            ));
        }
        Ok(mode.or_else(|| legacy.map(|value| Self::from_stored(Some(value)))))
    }

    pub fn from_stored(preference: Option<bool>) -> Self {
        match preference {
            Some(true) => Self::All,
            Some(false) => Self::Off,
            None => Self::WritesOnly,
        }
    }

    pub fn stored(self) -> Option<bool> {
        match self {
            Self::All => Some(true),
            Self::Off => Some(false),
            Self::WritesOnly => None,
        }
    }

    pub fn requires(self, verb: &ApprovalVerb) -> bool {
        match self {
            Self::All => true,
            Self::Off => false,
            Self::WritesOnly => !matches!(verb, ApprovalVerb::Read),
        }
    }
}

pub const MAX_APPROVAL_RULES: usize = 50;
pub const MAX_RULE_METHODS: usize = 16;
pub const MAX_RULE_RESOURCE_PATTERN_LEN: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalPolicyDecision {
    pub effect: ApprovalEffect,
    pub mode: ApprovalMode,
    pub grant_scope: Option<String>,
}

pub const MAX_RULE_API_KEYS: usize = 20;
const MAX_RULE_API_KEY_ID_LEN: usize = 64;

/// Policy for one call, honouring rules limited to specific NyxID API keys.
///
/// A `None` result means the config has no opinion about this caller: nothing matched,
/// there is no explicit default, and every rule is limited to other keys. The
/// caller then applies whatever it would have applied without this config.
pub fn evaluate(
    config: &ServiceApprovalConfig,
    descriptor: &OperationDescriptor,
    api_key_id: Option<&str>,
) -> Option<ApprovalPolicyDecision> {
    if config.disabled {
        return None;
    }
    for rule in &config.rules {
        if rule_matches(rule, descriptor, api_key_id) {
            return Some(ApprovalPolicyDecision {
                effect: rule.effect.clone(),
                mode: rule.mode.clone(),
                // Rule matches mint the rule pattern so one approval covers
                // the user's declared operation class; defaults stay concrete.
                grant_scope: require_approval_scope(&rule.effect, || {
                    scope_from_rule(descriptor, rule)
                }),
            });
        }
    }

    if let Some(effect) = &config.default_effect {
        return Some(ApprovalPolicyDecision {
            effect: effect.clone(),
            mode: config.approval_mode.clone(),
            grant_scope: require_approval_scope(effect, || concrete_scope(descriptor)),
        });
    }

    if config.rules.is_empty() {
        return Some(ApprovalPolicyDecision {
            effect: if config.approval_required {
                ApprovalEffect::RequireApproval
            } else {
                ApprovalEffect::AutoAllow
            },
            mode: config.approval_mode.clone(),
            grant_scope: None,
        });
    }

    if config.rules.iter().all(|rule| !rule.api_key_ids.is_empty()) {
        return None;
    }

    Some(ApprovalPolicyDecision {
        effect: ApprovalEffect::AutoAllow,
        mode: config.approval_mode.clone(),
        grant_scope: None,
    })
}

/// Separately authored marker. Existing unmarked rules keep their mode semantics.
pub(crate) fn mandatory_review(
    config: &ServiceApprovalConfig,
    descriptor: &OperationDescriptor,
) -> bool {
    !config.disabled
        && config
            .rules
            .iter()
            .find(|rule| rule_matches(rule, descriptor, None))
            .is_some_and(|rule| rule.mandatory_review)
}

pub fn grant_scope_covers(scope: Option<&str>, descriptor: &OperationDescriptor) -> bool {
    let Some(scope) = scope else {
        return true;
    };
    let Some(parsed) = ParsedScope::parse(scope) else {
        return false;
    };
    if scope_protocol_family(parsed.protocol) != Some(protocol_scope_label(&descriptor.protocol)) {
        return false;
    }

    method_set_matches(&parsed.methods, descriptor)
        && verb_set_matches(&parsed.verbs, descriptor)
        && resource_pattern_matches(parsed.resource_pattern, descriptor)
}

pub fn validate_rules(rules: &[ApprovalRule]) -> Result<(), String> {
    if rules.iter().any(|rule| rule.mandatory_review) {
        return Err(
            "Mandatory organization review is reader-only until execution enforcement is released"
                .into(),
        );
    }
    if rules.len() > MAX_APPROVAL_RULES {
        return Err(format!(
            "approval rules cannot exceed {MAX_APPROVAL_RULES} entries"
        ));
    }

    for (idx, rule) in rules.iter().enumerate() {
        if rule.methods.len() > MAX_RULE_METHODS {
            return Err(format!(
                "approval rule {idx} cannot list more than {MAX_RULE_METHODS} methods"
            ));
        }
        let resource_pattern = normalized_rule_resource_pattern(&rule.resource_pattern);
        if resource_pattern.len() > MAX_RULE_RESOURCE_PATTERN_LEN {
            return Err(format!(
                "approval rule {idx} resource_pattern exceeds {MAX_RULE_RESOURCE_PATTERN_LEN} characters"
            ));
        }
        validate_methods(idx, &rule.methods)?;
        if rule.api_key_ids.len() > MAX_RULE_API_KEYS {
            return Err(format!(
                "approval rule {idx} cannot list more than {MAX_RULE_API_KEYS} API keys"
            ));
        }
        if rule
            .api_key_ids
            .iter()
            .any(|id| id.trim().is_empty() || id.len() > MAX_RULE_API_KEY_ID_LEN)
        {
            return Err(format!("approval rule {idx} lists an invalid API key id"));
        }
        // A remembered approval is recorded per account, not per key, so a
        // key-limited rule that remembered would let other keys reuse it.
        if !rule.api_key_ids.is_empty()
            && rule.effect == ApprovalEffect::RequireApproval
            && rule.mode == ApprovalMode::Grant
        {
            return Err(format!(
                "approval rule {idx} is limited to API keys and must ask every time"
            ));
        }
        compile_resource_pattern(&resource_pattern, true)
            .and_then(|_| compile_resource_pattern(&resource_pattern, false))
            .map_err(|e| format!("approval rule {idx} resource_pattern is invalid: {e}"))?;
    }

    Ok(())
}

pub fn concrete_scope(descriptor: &OperationDescriptor) -> String {
    build_scope(
        protocol_scope_label(&descriptor.protocol),
        descriptor
            .normalized_method()
            .unwrap_or_else(|| "*".to_string()),
        descriptor.verb.as_str().to_string(),
        descriptor
            .normalized_resource()
            .unwrap_or_else(|| "*".to_string()),
    )
}

/// Whether a rule concerns this credential, independent of the operation shape.
/// Opaque streams and service-wide evidence must check every applicable rule.
pub(crate) fn rule_applies_to_key(rule: &ApprovalRule, api_key_id: Option<&str>) -> bool {
    rule.api_key_ids.is_empty()
        || api_key_id.is_some_and(|key| rule.api_key_ids.iter().any(|id| id == key))
}

/// Whether a config can override implicit defaults for this credential.
pub(crate) fn has_policy_for_key(config: &ServiceApprovalConfig, api_key_id: Option<&str>) -> bool {
    !config.disabled
        && (config.rules.is_empty()
            || config.default_effect.is_some()
            || config
                .rules
                .iter()
                .any(|rule| rule_applies_to_key(rule, api_key_id)))
}

fn rule_matches(
    rule: &ApprovalRule,
    descriptor: &OperationDescriptor,
    api_key_id: Option<&str>,
) -> bool {
    rule_applies_to_key(rule, api_key_id)
        && methods_match(&rule.methods, descriptor)
        && verbs_match(&rule.verbs, descriptor)
        && resource_pattern_matches(&rule.resource_pattern, descriptor)
}

fn methods_match(methods: &[String], descriptor: &OperationDescriptor) -> bool {
    if methods.is_empty() || methods.iter().any(|method| method.trim() == "*") {
        return true;
    }
    let Some(request_method) = descriptor.normalized_method() else {
        return false;
    };
    methods
        .iter()
        .map(|method| method.trim().to_ascii_lowercase())
        .any(|method| method == request_method)
}

fn verbs_match(verbs: &[ApprovalVerb], descriptor: &OperationDescriptor) -> bool {
    verbs.is_empty() || verbs.iter().any(|verb| verb == &descriptor.verb)
}

fn require_approval_scope(
    effect: &ApprovalEffect,
    build: impl FnOnce() -> String,
) -> Option<String> {
    if effect == &ApprovalEffect::RequireApproval {
        Some(build())
    } else {
        None
    }
}

fn scope_from_rule(descriptor: &OperationDescriptor, rule: &ApprovalRule) -> String {
    build_scope(
        protocol_scope_label(&descriptor.protocol),
        normalized_method_set(&rule.methods),
        normalized_verb_set(&rule.verbs),
        normalized_rule_resource_pattern(&rule.resource_pattern),
    )
}

fn build_scope(protocol: &str, methods: String, verbs: String, resource_pattern: String) -> String {
    format!("v1:{protocol}:{methods}:{verbs}:{resource_pattern}")
}

fn protocol_scope_label(protocol: &Protocol) -> &'static str {
    match protocol {
        Protocol::Http | Protocol::Llm | Protocol::Mcp => "http",
        Protocol::Ssh => "ssh",
    }
}

fn scope_protocol_family(protocol: &str) -> Option<&'static str> {
    match protocol {
        "http" | "llm" | "mcp" => Some("http"),
        "ssh" => Some("ssh"),
        _ => None,
    }
}

fn normalized_method_set(methods: &[String]) -> String {
    let mut values: Vec<String> = methods
        .iter()
        .map(|method| method.trim().to_ascii_lowercase())
        .filter(|method| !method.is_empty())
        .collect();
    if values.is_empty() || values.iter().any(|method| method == "*") {
        return "*".to_string();
    }
    values.sort();
    values.dedup();
    values.join(",")
}

fn normalized_verb_set(verbs: &[ApprovalVerb]) -> String {
    if verbs.is_empty() {
        return "*".to_string();
    }
    let mut values: Vec<&str> = verbs.iter().map(ApprovalVerb::as_str).collect();
    values.sort();
    values.dedup();
    values.join(",")
}

fn normalized_rule_resource_pattern(resource_pattern: &str) -> String {
    let trimmed = resource_pattern.trim();
    if trimmed.is_empty() {
        "*".to_string()
    } else {
        trimmed.to_string()
    }
}

fn validate_methods(rule_idx: usize, methods: &[String]) -> Result<(), String> {
    if methods.is_empty() {
        return Ok(());
    }
    let has_wildcard = methods.iter().any(|method| method.trim() == "*");
    if has_wildcard && methods.len() > 1 {
        return Err(format!(
            "approval rule {rule_idx} cannot combine '*' with explicit methods"
        ));
    }
    for method in methods {
        let normalized = method.trim().to_ascii_uppercase();
        if !matches!(
            normalized.as_str(),
            "*" | "GET"
                | "POST"
                | "PUT"
                | "PATCH"
                | "DELETE"
                | "HEAD"
                | "OPTIONS"
                | "EXEC"
                | "TUNNEL"
        ) {
            return Err(format!(
                "approval rule {rule_idx} contains unsupported method '{method}'"
            ));
        }
    }
    Ok(())
}

struct ParsedScope<'a> {
    protocol: &'a str,
    methods: Vec<&'a str>,
    verbs: Vec<&'a str>,
    resource_pattern: &'a str,
}

impl<'a> ParsedScope<'a> {
    fn parse(scope: &'a str) -> Option<Self> {
        let mut parts = scope.splitn(5, ':');
        let version = parts.next()?;
        if version != "v1" {
            return None;
        }
        let protocol = parts.next()?;
        let methods = split_scope_set(parts.next()?);
        let verbs = split_scope_set(parts.next()?);
        let resource_pattern = parts.next()?;
        Some(Self {
            protocol,
            methods,
            verbs,
            resource_pattern,
        })
    }
}

fn split_scope_set(raw: &str) -> Vec<&str> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect()
}

fn method_set_matches(methods: &[&str], descriptor: &OperationDescriptor) -> bool {
    if methods.is_empty() || methods.contains(&"*") {
        return true;
    }
    let Some(method) = descriptor.normalized_method() else {
        return false;
    };
    methods.iter().any(|candidate| *candidate == method)
}

fn verb_set_matches(verbs: &[&str], descriptor: &OperationDescriptor) -> bool {
    verbs.is_empty()
        || verbs.contains(&"*")
        || verbs.iter().any(|verb| *verb == descriptor.verb.as_str())
}

fn resource_pattern_matches(pattern: &str, descriptor: &OperationDescriptor) -> bool {
    let pattern = normalized_rule_resource_pattern(pattern);
    if pattern == "*" {
        return true;
    }
    let Some(resource) = descriptor.normalized_resource() else {
        return false;
    };
    cached_resource_matcher(
        &pattern,
        literal_separator_for_protocol(&descriptor.protocol),
    )
    .is_ok_and(|glob| glob.is_match(resource))
}

fn literal_separator_for_protocol(protocol: &Protocol) -> bool {
    !matches!(protocol, Protocol::Ssh)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct GlobCacheKey {
    pattern: String,
    literal_separator: bool,
}

fn glob_matcher_cache() -> &'static Mutex<HashMap<GlobCacheKey, GlobMatcher>> {
    static CACHE: OnceLock<Mutex<HashMap<GlobCacheKey, GlobMatcher>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_resource_matcher(
    pattern: &str,
    literal_separator: bool,
) -> Result<GlobMatcher, globset::Error> {
    const MAX_CACHED_GLOBS: usize = 1024;

    let key = GlobCacheKey {
        pattern: normalized_rule_resource_pattern(pattern),
        literal_separator,
    };
    if let Some(matcher) = glob_matcher_cache()
        .lock()
        .expect("glob matcher cache poisoned")
        .get(&key)
        .cloned()
    {
        return Ok(matcher);
    }

    let matcher = compile_resource_pattern(&key.pattern, key.literal_separator)?.compile_matcher();
    let mut cache = glob_matcher_cache()
        .lock()
        .expect("glob matcher cache poisoned");
    if cache.len() < MAX_CACHED_GLOBS {
        cache.insert(key, matcher.clone());
    }
    Ok(matcher)
}

fn compile_resource_pattern(
    pattern: &str,
    literal_separator: bool,
) -> Result<Glob, globset::Error> {
    let pattern = normalized_rule_resource_pattern(pattern);
    GlobBuilder::new(&pattern)
        .literal_separator(literal_separator)
        .backslash_escape(true)
        .build()
}

/// Key-limited Ask is an explicit protection even in assistant Full access.
/// The live turn's stateless model-call exemption remains separately documented.
pub(crate) fn key_requires_fresh_decision(
    config: &ServiceApprovalConfig,
    descriptor: &OperationDescriptor,
    key: Option<&str>,
) -> bool {
    !config.disabled
        && config
            .rules
            .iter()
            .find(|rule| rule_matches(rule, descriptor, key))
            .is_some_and(|rule| {
                !rule.api_key_ids.is_empty()
                    && rule.effect == ApprovalEffect::RequireApproval
                    && rule.mode == ApprovalMode::PerRequest
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn config(
        approval_required: bool,
        approval_mode: ApprovalMode,
        rules: Vec<ApprovalRule>,
        default_effect: Option<ApprovalEffect>,
    ) -> ServiceApprovalConfig {
        let now = Utc::now();
        ServiceApprovalConfig {
            disabled: false,
            grant_expiry_days: None,
            id: "cfg-1".to_string(),
            user_id: "user-1".to_string(),
            service_id: "svc-1".to_string(),
            service_name: "Service".to_string(),
            approval_required,
            approval_mode,
            rules,
            default_effect,
            created_at: now,
            updated_at: now,
        }
    }

    fn rule(
        methods: &[&str],
        verbs: Vec<ApprovalVerb>,
        effect: ApprovalEffect,
        mode: ApprovalMode,
    ) -> ApprovalRule {
        ApprovalRule {
            mandatory_review: false,
            methods: methods.iter().map(|method| (*method).to_string()).collect(),
            resource_pattern: "*".to_string(),
            verbs,
            effect,
            mode,
            api_key_ids: vec![],
        }
    }

    fn rule_with_pattern(
        methods: &[&str],
        resource_pattern: &str,
        verbs: Vec<ApprovalVerb>,
        effect: ApprovalEffect,
        mode: ApprovalMode,
    ) -> ApprovalRule {
        ApprovalRule {
            resource_pattern: resource_pattern.to_string(),
            ..rule(methods, verbs, effect, mode)
        }
    }

    fn post_descriptor() -> OperationDescriptor {
        crate::services::operation_descriptor::build_http_descriptor(
            "POST",
            "/v1/chat/completions",
            None,
        )
    }

    fn get_descriptor() -> OperationDescriptor {
        crate::services::operation_descriptor::build_http_descriptor("GET", "/v1/models", None)
    }

    fn github_contents_descriptor(path: &str) -> OperationDescriptor {
        crate::services::operation_descriptor::build_http_descriptor("PUT", path, None)
    }

    #[test]
    fn mandatory_review_is_additive_and_authoring_stays_disabled() {
        let legacy = serde_json::json!({"methods":["POST"],"mode":"per_request"});
        let mut rule: ApprovalRule = serde_json::from_value(legacy).unwrap();
        assert!(!rule.mandatory_review);
        assert!(validate_rules(&[rule.clone()]).is_ok());
        assert!(
            serde_json::to_value(&rule)
                .unwrap()
                .get("mandatory_review")
                .is_none()
        );
        rule.mandatory_review = true;
        let decoded: ApprovalRule = bson::from_document(bson::to_document(&rule).unwrap()).unwrap();
        assert!(decoded.mandatory_review);
        assert!(validate_rules(&[decoded]).is_err());
    }

    #[test]
    fn first_matching_rule_wins() {
        let cfg = config(
            false,
            ApprovalMode::PerRequest,
            vec![
                rule(
                    &["POST"],
                    vec![],
                    ApprovalEffect::AutoAllow,
                    ApprovalMode::PerRequest,
                ),
                rule(
                    &["POST"],
                    vec![],
                    ApprovalEffect::RequireApproval,
                    ApprovalMode::Grant,
                ),
            ],
            Some(ApprovalEffect::Deny),
        );

        let decision = evaluate(&cfg, &post_descriptor(), None).expect("policy decision");

        assert_eq!(decision.effect, ApprovalEffect::AutoAllow);
        assert_eq!(decision.grant_scope, None);
    }

    #[test]
    fn empty_rules_without_default_uses_legacy_binary_fallback() {
        let required = evaluate(
            &config(true, ApprovalMode::Grant, vec![], None),
            &post_descriptor(),
            None,
        )
        .expect("policy decision");
        assert_eq!(required.effect, ApprovalEffect::RequireApproval);
        assert_eq!(required.mode, ApprovalMode::Grant);
        assert_eq!(required.grant_scope, None);

        let allowed = evaluate(
            &config(false, ApprovalMode::Grant, vec![], None),
            &post_descriptor(),
            None,
        )
        .expect("policy decision");
        assert_eq!(allowed.effect, ApprovalEffect::AutoAllow);
        assert_eq!(allowed.grant_scope, None);
    }

    #[test]
    fn non_empty_rules_without_default_auto_allows_unmatched_operations() {
        let cfg = config(
            true,
            ApprovalMode::Grant,
            vec![rule(
                &["POST"],
                vec![],
                ApprovalEffect::RequireApproval,
                ApprovalMode::Grant,
            )],
            None,
        );

        let decision = evaluate(&cfg, &get_descriptor(), None).expect("policy decision");

        assert_eq!(decision.effect, ApprovalEffect::AutoAllow);
        assert_eq!(decision.grant_scope, None);
    }

    fn keyed(mut rule: ApprovalRule, keys: &[&str]) -> ApprovalRule {
        rule.api_key_ids = keys.iter().map(|key| (*key).to_string()).collect();
        rule
    }

    #[test]
    fn key_limited_rule_applies_only_to_listed_keys() {
        let ask = keyed(
            rule(
                &[],
                vec![],
                ApprovalEffect::RequireApproval,
                ApprovalMode::PerRequest,
            ),
            &["key-a"],
        );
        let cfg = config(false, ApprovalMode::PerRequest, vec![ask], None);

        let listed = evaluate(&cfg, &post_descriptor(), Some("key-a")).expect("decision");
        assert_eq!(listed.effect, ApprovalEffect::RequireApproval);
        // Other keys and callers without a key get no opinion, so the caller's
        // ordinary default applies instead of a silent allow.
        assert_eq!(evaluate(&cfg, &post_descriptor(), Some("key-b")), None);
        assert_eq!(evaluate(&cfg, &post_descriptor(), None), None);
    }

    #[test]
    fn key_limited_rule_falls_through_to_general_rules_and_defaults() {
        let block_for_a = keyed(
            rule(&[], vec![], ApprovalEffect::Deny, ApprovalMode::PerRequest),
            &["key-a"],
        );
        let ask_everyone = rule(
            &[],
            vec![],
            ApprovalEffect::RequireApproval,
            ApprovalMode::PerRequest,
        );
        let cfg = config(
            false,
            ApprovalMode::PerRequest,
            vec![block_for_a.clone(), ask_everyone],
            None,
        );
        assert_eq!(
            evaluate(&cfg, &post_descriptor(), Some("key-a"))
                .unwrap()
                .effect,
            ApprovalEffect::Deny
        );
        assert_eq!(
            evaluate(&cfg, &post_descriptor(), Some("key-b"))
                .unwrap()
                .effect,
            ApprovalEffect::RequireApproval
        );

        let with_default = config(
            false,
            ApprovalMode::PerRequest,
            vec![block_for_a],
            Some(ApprovalEffect::RequireApproval),
        );
        assert_eq!(
            evaluate(&with_default, &post_descriptor(), Some("key-b"))
                .unwrap()
                .effect,
            ApprovalEffect::RequireApproval
        );
    }

    #[test]
    fn key_limited_rules_are_bounded_and_cannot_remember() {
        let ok = keyed(
            rule(
                &[],
                vec![],
                ApprovalEffect::RequireApproval,
                ApprovalMode::PerRequest,
            ),
            &["key-a"],
        );
        assert!(validate_rules(std::slice::from_ref(&ok)).is_ok());

        let remembers = keyed(
            rule(
                &[],
                vec![],
                ApprovalEffect::RequireApproval,
                ApprovalMode::Grant,
            ),
            &["key-a"],
        );
        assert!(
            validate_rules(&[remembers])
                .unwrap_err()
                .contains("ask every time")
        );

        let blank = keyed(ok.clone(), &[" "]);
        assert!(validate_rules(&[blank]).is_err());

        let many: Vec<String> = (0..=MAX_RULE_API_KEYS)
            .map(|i| format!("key-{i}"))
            .collect();
        let too_many = keyed(ok, &many.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(validate_rules(&[too_many]).is_err());
    }

    #[test]
    fn key_limit_is_omitted_from_storage_when_empty() {
        let plain = rule(
            &[],
            vec![],
            ApprovalEffect::RequireApproval,
            ApprovalMode::PerRequest,
        );
        assert!(
            !bson::to_document(&plain)
                .unwrap()
                .contains_key("api_key_ids")
        );
        let limited = keyed(plain, &["key-a"]);
        let doc = bson::to_document(&limited).unwrap();
        assert_eq!(
            bson::from_document::<ApprovalRule>(doc)
                .unwrap()
                .api_key_ids,
            vec!["key-a"]
        );
    }

    #[test]
    fn explicit_default_effect_is_used_when_no_rule_matches() {
        let cfg = config(
            false,
            ApprovalMode::Grant,
            vec![rule(
                &["DELETE"],
                vec![],
                ApprovalEffect::Deny,
                ApprovalMode::PerRequest,
            )],
            Some(ApprovalEffect::RequireApproval),
        );

        let decision = evaluate(&cfg, &post_descriptor(), None).expect("policy decision");

        assert_eq!(decision.effect, ApprovalEffect::RequireApproval);
        assert_eq!(decision.mode, ApprovalMode::Grant);
        assert_eq!(
            decision.grant_scope.as_deref(),
            Some("v1:http:post:write:/v1/chat/completions")
        );
    }

    #[test]
    fn methods_and_verbs_must_both_match() {
        let cfg = config(
            false,
            ApprovalMode::PerRequest,
            vec![
                rule(
                    &["GET"],
                    vec![ApprovalVerb::Write],
                    ApprovalEffect::Deny,
                    ApprovalMode::PerRequest,
                ),
                rule(
                    &["POST"],
                    vec![ApprovalVerb::Write],
                    ApprovalEffect::RequireApproval,
                    ApprovalMode::Grant,
                ),
            ],
            None,
        );

        let decision = evaluate(&cfg, &post_descriptor(), None).expect("policy decision");

        assert_eq!(decision.effect, ApprovalEffect::RequireApproval);
        assert_eq!(decision.mode, ApprovalMode::Grant);
        assert_eq!(
            decision.grant_scope.as_deref(),
            Some("v1:http:post:write:*")
        );
    }

    #[test]
    fn resource_glob_must_match_when_present() {
        let cfg = config(
            false,
            ApprovalMode::Grant,
            vec![rule_with_pattern(
                &["PUT"],
                "/repos/*/contents/**",
                vec![ApprovalVerb::Write],
                ApprovalEffect::RequireApproval,
                ApprovalMode::Grant,
            )],
            None,
        );

        let matching = evaluate(
            &cfg,
            &github_contents_descriptor("/repos/nyx/contents/src/main.rs"),
            None,
        )
        .expect("policy decision");
        let non_matching = evaluate(
            &cfg,
            &github_contents_descriptor("/repos/nyx/issues/1/comments"),
            None,
        )
        .expect("policy decision");

        assert_eq!(matching.effect, ApprovalEffect::RequireApproval);
        assert_eq!(
            matching.grant_scope.as_deref(),
            Some("v1:http:put:write:/repos/*/contents/**")
        );
        assert_eq!(non_matching.effect, ApprovalEffect::AutoAllow);
    }

    #[test]
    fn single_segment_star_does_not_cross_path_separator() {
        let cfg = config(
            false,
            ApprovalMode::PerRequest,
            vec![rule_with_pattern(
                &["PUT"],
                "/repos/*/contents/*",
                vec![],
                ApprovalEffect::RequireApproval,
                ApprovalMode::PerRequest,
            )],
            None,
        );

        let nested = evaluate(
            &cfg,
            &github_contents_descriptor("/repos/nyx/contents/src/main.rs"),
            None,
        )
        .expect("policy decision");
        let single = evaluate(
            &cfg,
            &github_contents_descriptor("/repos/nyx/contents/main.rs"),
            None,
        )
        .expect("policy decision");

        assert_eq!(nested.effect, ApprovalEffect::AutoAllow);
        assert_eq!(single.effect, ApprovalEffect::RequireApproval);
    }

    #[test]
    fn http_star_globs_do_not_cross_path_separator() {
        let cfg = config(
            false,
            ApprovalMode::PerRequest,
            vec![rule_with_pattern(
                &["GET"],
                "/repos/*",
                vec![ApprovalVerb::Read],
                ApprovalEffect::RequireApproval,
                ApprovalMode::PerRequest,
            )],
            None,
        );
        let nested =
            crate::services::operation_descriptor::build_http_descriptor("GET", "/repos/a/b", None);
        let single =
            crate::services::operation_descriptor::build_http_descriptor("GET", "/repos/a", None);

        assert_eq!(
            evaluate(&cfg, &nested, None)
                .expect("policy decision")
                .effect,
            ApprovalEffect::AutoAllow
        );
        assert_eq!(
            evaluate(&cfg, &single, None)
                .expect("policy decision")
                .effect,
            ApprovalEffect::RequireApproval
        );
    }

    #[test]
    fn command_globs_match_ssh_exec_resources() {
        let cfg = config(
            false,
            ApprovalMode::PerRequest,
            vec![rule_with_pattern(
                &["EXEC"],
                "git push*",
                vec![ApprovalVerb::Write],
                ApprovalEffect::RequireApproval,
                ApprovalMode::PerRequest,
            )],
            None,
        );
        let push = crate::services::operation_descriptor::build_ssh_descriptor(
            crate::services::operation_descriptor::SshOperationKind::Exec,
            Some("git push origin main"),
        );
        let status = crate::services::operation_descriptor::build_ssh_descriptor(
            crate::services::operation_descriptor::SshOperationKind::Exec,
            Some("git status"),
        );

        assert_eq!(
            evaluate(&cfg, &push, None).expect("policy decision").effect,
            ApprovalEffect::RequireApproval
        );
        assert_eq!(
            evaluate(&cfg, &status, None)
                .expect("policy decision")
                .effect,
            ApprovalEffect::AutoAllow
        );
    }

    #[test]
    fn ssh_star_globs_match_slashes_inside_commands() {
        let cfg = config(
            false,
            ApprovalMode::PerRequest,
            vec![rule_with_pattern(
                &["EXEC"],
                "rm *",
                vec![ApprovalVerb::Write],
                ApprovalEffect::RequireApproval,
                ApprovalMode::PerRequest,
            )],
            None,
        );
        let descriptor = crate::services::operation_descriptor::build_ssh_descriptor(
            crate::services::operation_descriptor::SshOperationKind::Exec,
            Some("rm /etc/passwd"),
        );

        assert_eq!(
            evaluate(&cfg, &descriptor, None)
                .expect("policy decision")
                .effect,
            ApprovalEffect::RequireApproval
        );
    }

    #[test]
    fn ssh_absolute_command_globs_match_expected_prefix() {
        let cfg = config(
            false,
            ApprovalMode::PerRequest,
            vec![rule_with_pattern(
                &["EXEC"],
                "/usr/bin/*",
                vec![ApprovalVerb::Write],
                ApprovalEffect::RequireApproval,
                ApprovalMode::PerRequest,
            )],
            None,
        );
        let matching = crate::services::operation_descriptor::build_ssh_descriptor(
            crate::services::operation_descriptor::SshOperationKind::Exec,
            Some("/usr/bin/python --version"),
        );
        let non_matching = crate::services::operation_descriptor::build_ssh_descriptor(
            crate::services::operation_descriptor::SshOperationKind::Exec,
            Some("/bin/python --version"),
        );

        assert_eq!(
            evaluate(&cfg, &matching, None)
                .expect("policy decision")
                .effect,
            ApprovalEffect::RequireApproval
        );
        assert_eq!(
            evaluate(&cfg, &non_matching, None)
                .expect("policy decision")
                .effect,
            ApprovalEffect::AutoAllow
        );
    }

    #[test]
    fn deny_is_a_first_class_effect() {
        let cfg = config(
            false,
            ApprovalMode::PerRequest,
            vec![rule(
                &["POST"],
                vec![ApprovalVerb::Write],
                ApprovalEffect::Deny,
                ApprovalMode::PerRequest,
            )],
            None,
        );

        let decision = evaluate(&cfg, &post_descriptor(), None).expect("policy decision");

        assert_eq!(decision.effect, ApprovalEffect::Deny);
        assert_eq!(decision.grant_scope, None);
    }

    #[test]
    fn scoped_grant_matches_only_covered_operations() {
        let write_scope = Some("v1:http:post:write:*");

        assert!(grant_scope_covers(write_scope, &post_descriptor()));
        assert!(!grant_scope_covers(write_scope, &get_descriptor()));
        assert!(grant_scope_covers(None, &get_descriptor()));
    }

    #[test]
    fn http_family_grants_cover_mcp_for_same_endpoint() {
        let http_scope = Some("v1:http:delete:destructive:/repos/*");
        let mcp_descriptor = crate::services::operation_descriptor::build_mcp_descriptor(
            "DELETE",
            "/repos/acme",
            None,
        );

        assert!(grant_scope_covers(http_scope, &mcp_descriptor));
    }

    #[test]
    fn mcp_family_grants_cover_http_for_same_endpoint() {
        let legacy_mcp_scope = Some("v1:mcp:delete:destructive:/repos/*");
        let http_descriptor = crate::services::operation_descriptor::build_http_descriptor(
            "DELETE",
            "/repos/acme",
            None,
        );

        assert!(grant_scope_covers(legacy_mcp_scope, &http_descriptor));
    }

    #[test]
    fn ssh_grants_do_not_cover_http_descriptors() {
        let ssh_scope = Some("v1:ssh:exec:write:*");

        assert!(!grant_scope_covers(ssh_scope, &post_descriptor()));
    }

    #[test]
    fn concrete_scope_matches_exact_method_verb_resource() {
        let scope = Some("v1:http:post:write:/v1/chat/completions");

        assert!(grant_scope_covers(scope, &post_descriptor()));
        assert!(!grant_scope_covers(scope, &get_descriptor()));
    }

    #[test]
    fn glob_scoped_grant_covers_matching_resources_only() {
        let scope = Some("v1:http:put:write:/repos/*/contents/**");

        assert!(grant_scope_covers(
            scope,
            &github_contents_descriptor("/repos/nyx/contents/src/main.rs")
        ));
        assert!(!grant_scope_covers(
            scope,
            &github_contents_descriptor("/repos/nyx/issues/1/comments")
        ));
    }

    #[test]
    fn validates_method_sets_and_rule_count() {
        let invalid = vec![rule(
            &["POST", "*"],
            vec![],
            ApprovalEffect::AutoAllow,
            ApprovalMode::PerRequest,
        )];
        assert!(validate_rules(&invalid).is_err());

        let invalid_method = vec![rule(
            &["BREW"],
            vec![],
            ApprovalEffect::AutoAllow,
            ApprovalMode::PerRequest,
        )];
        assert!(validate_rules(&invalid_method).is_err());

        let too_many = vec![
            rule(
                &["GET"],
                vec![],
                ApprovalEffect::AutoAllow,
                ApprovalMode::PerRequest,
            );
            MAX_APPROVAL_RULES + 1
        ];
        assert!(validate_rules(&too_many).is_err());

        let invalid_glob = vec![rule_with_pattern(
            &["GET"],
            "[",
            vec![],
            ApprovalEffect::AutoAllow,
            ApprovalMode::PerRequest,
        )];
        assert!(validate_rules(&invalid_glob).is_err());
    }
    #[test]
    fn opaque_policy_key_selection_preserves_general_defaults() {
        for effect in [
            ApprovalEffect::Deny,
            ApprovalEffect::RequireApproval,
            ApprovalEffect::AutoAllow,
        ] {
            let limited = keyed(
                rule(&[], vec![], effect.clone(), ApprovalMode::PerRequest),
                &["key-a"],
            );
            let mut cfg = config(true, ApprovalMode::PerRequest, vec![limited], None);
            assert!(has_policy_for_key(&cfg, Some("key-a")));
            for other in [Some("key-b"), None] {
                assert!(!has_policy_for_key(&cfg, other));
                assert!(!rule_applies_to_key(&cfg.rules[0], other));
                assert_eq!(evaluate(&cfg, &post_descriptor(), other), None);
            }
            cfg.default_effect = Some(ApprovalEffect::RequireApproval);
            assert!(has_policy_for_key(&cfg, Some("key-b")));
            assert_eq!(
                evaluate(&cfg, &post_descriptor(), Some("key-b"))
                    .unwrap()
                    .effect,
                ApprovalEffect::RequireApproval
            );
            cfg.rules[0].api_key_ids.clear();
            for key in [Some("key-a"), Some("key-b"), None] {
                assert!(has_policy_for_key(&cfg, key));
                assert_eq!(
                    evaluate(&cfg, &post_descriptor(), key).unwrap().effect,
                    effect
                );
            }
        }
    }
}
