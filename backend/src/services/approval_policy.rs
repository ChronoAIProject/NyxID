use globset::{Glob, GlobBuilder, GlobMatcher};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::models::service_approval_config::{
    ApprovalEffect, ApprovalMode, ApprovalRule, ApprovalVerb, ServiceApprovalConfig,
};
use crate::services::operation_descriptor::{OperationDescriptor, Protocol};

pub const MAX_APPROVAL_RULES: usize = 50;
pub const MAX_RULE_METHODS: usize = 16;
pub const MAX_RULE_RESOURCE_PATTERN_LEN: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalPolicyDecision {
    pub effect: ApprovalEffect,
    pub mode: ApprovalMode,
    pub grant_scope: Option<String>,
}

pub fn evaluate(
    config: &ServiceApprovalConfig,
    descriptor: &OperationDescriptor,
) -> ApprovalPolicyDecision {
    for rule in &config.rules {
        if rule_matches(rule, descriptor) {
            return ApprovalPolicyDecision {
                effect: rule.effect.clone(),
                mode: rule.mode.clone(),
                // Rule matches mint the rule pattern so one approval covers
                // the user's declared operation class; defaults stay concrete.
                grant_scope: require_approval_scope(&rule.effect, || {
                    scope_from_rule(descriptor, rule)
                }),
            };
        }
    }

    if let Some(effect) = &config.default_effect {
        return ApprovalPolicyDecision {
            effect: effect.clone(),
            mode: config.approval_mode.clone(),
            grant_scope: require_approval_scope(effect, || concrete_scope(descriptor)),
        };
    }

    if config.rules.is_empty() {
        if config.approval_required {
            return ApprovalPolicyDecision {
                effect: ApprovalEffect::RequireApproval,
                mode: config.approval_mode.clone(),
                grant_scope: None,
            };
        }
        return ApprovalPolicyDecision {
            effect: ApprovalEffect::AutoAllow,
            mode: config.approval_mode.clone(),
            grant_scope: None,
        };
    }

    ApprovalPolicyDecision {
        effect: ApprovalEffect::AutoAllow,
        mode: config.approval_mode.clone(),
        grant_scope: None,
    }
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

fn rule_matches(rule: &ApprovalRule, descriptor: &OperationDescriptor) -> bool {
    methods_match(&rule.methods, descriptor)
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
            methods: methods.iter().map(|method| (*method).to_string()).collect(),
            resource_pattern: "*".to_string(),
            verbs,
            effect,
            mode,
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

        let decision = evaluate(&cfg, &post_descriptor());

        assert_eq!(decision.effect, ApprovalEffect::AutoAllow);
        assert_eq!(decision.grant_scope, None);
    }

    #[test]
    fn empty_rules_without_default_uses_legacy_binary_fallback() {
        let required = evaluate(
            &config(true, ApprovalMode::Grant, vec![], None),
            &post_descriptor(),
        );
        assert_eq!(required.effect, ApprovalEffect::RequireApproval);
        assert_eq!(required.mode, ApprovalMode::Grant);
        assert_eq!(required.grant_scope, None);

        let allowed = evaluate(
            &config(false, ApprovalMode::Grant, vec![], None),
            &post_descriptor(),
        );
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

        let decision = evaluate(&cfg, &get_descriptor());

        assert_eq!(decision.effect, ApprovalEffect::AutoAllow);
        assert_eq!(decision.grant_scope, None);
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

        let decision = evaluate(&cfg, &post_descriptor());

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

        let decision = evaluate(&cfg, &post_descriptor());

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
        );
        let non_matching = evaluate(
            &cfg,
            &github_contents_descriptor("/repos/nyx/issues/1/comments"),
        );

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
        );
        let single = evaluate(
            &cfg,
            &github_contents_descriptor("/repos/nyx/contents/main.rs"),
        );

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

        assert_eq!(evaluate(&cfg, &nested).effect, ApprovalEffect::AutoAllow);
        assert_eq!(
            evaluate(&cfg, &single).effect,
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
            evaluate(&cfg, &push).effect,
            ApprovalEffect::RequireApproval
        );
        assert_eq!(evaluate(&cfg, &status).effect, ApprovalEffect::AutoAllow);
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
            evaluate(&cfg, &descriptor).effect,
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
            evaluate(&cfg, &matching).effect,
            ApprovalEffect::RequireApproval
        );
        assert_eq!(
            evaluate(&cfg, &non_matching).effect,
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

        let decision = evaluate(&cfg, &post_descriptor());

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
}
