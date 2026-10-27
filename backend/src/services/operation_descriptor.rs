use crate::models::service_approval_config::ApprovalVerb;
use crate::services::action_description;
use regex::Regex;
use std::sync::OnceLock;

const MAX_STORED_RESOURCE_LEN: usize = 200;

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Protocol {
    Http,
    Llm,
    Mcp,
    Ssh,
}

impl Protocol {
    pub fn summary_prefix(&self) -> &'static str {
        match self {
            Self::Http => "proxy",
            Self::Llm => "llm",
            Self::Mcp => "mcp",
            Self::Ssh => "ssh",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationDescriptor {
    pub protocol: Protocol,
    pub verb: ApprovalVerb,
    pub method: Option<String>,
    pub resource: Option<String>,
    pub summary: String,
}

impl OperationDescriptor {
    pub fn operation_summary(&self) -> String {
        if self.protocol == Protocol::Ssh
            && self.method.as_deref() == Some("TUNNEL")
            && self.resource.as_deref().unwrap_or_default().is_empty()
        {
            return "ssh:tunnel".to_string();
        }

        let method = self
            .method
            .as_deref()
            .unwrap_or_else(|| self.protocol.summary_prefix());
        match self
            .resource
            .as_deref()
            .filter(|resource| !resource.is_empty())
        {
            Some(resource) => format!("{}:{} {}", self.protocol.summary_prefix(), method, resource),
            None => format!("{}:{method}", self.protocol.summary_prefix()),
        }
    }

    #[allow(dead_code)]
    pub fn normalized_method(&self) -> Option<String> {
        self.method
            .as_deref()
            .map(|method| method.trim().to_ascii_lowercase())
            .filter(|method| !method.is_empty())
    }

    #[allow(dead_code)]
    pub fn normalized_resource(&self) -> Option<String> {
        self.resource
            .as_deref()
            .map(|resource| match self.protocol {
                Protocol::Ssh => resource.trim().to_string(),
                _ => normalize_resource(resource),
            })
            .filter(|resource| !resource.is_empty())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshOperationKind {
    Exec,
    Tunnel,
}

pub fn build_http_descriptor(method: &str, path: &str, body: Option<&[u8]>) -> OperationDescriptor {
    build_http_like_descriptor(Protocol::Http, method, path, body)
}

pub fn build_llm_descriptor(method: &str, path: &str, body: Option<&[u8]>) -> OperationDescriptor {
    build_http_like_descriptor(Protocol::Llm, method, path, body)
}

#[allow(dead_code)]
pub fn build_mcp_endpoint_descriptor(method: &str, path: &str) -> OperationDescriptor {
    build_mcp_descriptor(method, path, None)
}

pub fn build_mcp_descriptor(method: &str, path: &str, body: Option<&[u8]>) -> OperationDescriptor {
    let method = normalize_method_for_display(method);
    let resource = normalize_http_path(path);
    OperationDescriptor {
        protocol: Protocol::Mcp,
        verb: derive_verb_from_method(&method),
        method: Some(method.clone()),
        resource: Some(resource.clone()),
        summary: action_description::build_action_description(&method, &resource, body),
    }
}

#[allow(dead_code)]
pub fn build_mcp_meta_descriptor(tool_name: &str) -> OperationDescriptor {
    OperationDescriptor {
        protocol: Protocol::Mcp,
        verb: ApprovalVerb::Read,
        method: Some("tools/call".to_string()),
        resource: Some(tool_name.trim().to_string()),
        summary: format!("MCP meta-tool: {}", tool_name.trim()),
    }
}

pub fn build_ssh_descriptor(kind: SshOperationKind, command: Option<&str>) -> OperationDescriptor {
    match kind {
        SshOperationKind::Exec => {
            let command = sanitize_ssh_command(command.unwrap_or_default());
            OperationDescriptor {
                protocol: Protocol::Ssh,
                verb: ApprovalVerb::Write,
                method: Some("EXEC".to_string()),
                resource: Some(command.clone()),
                summary: truncate_summary(&format!("SSH exec: {command}")),
            }
        }
        SshOperationKind::Tunnel => OperationDescriptor {
            protocol: Protocol::Ssh,
            verb: ApprovalVerb::Write,
            method: Some("TUNNEL".to_string()),
            resource: Some(String::new()),
            summary: "SSH tunnel session".to_string(),
        },
    }
}

/// Reuse the trusted operation contract, including conservative remote marks.
pub fn apply_operation_effects(
    descriptor: &mut OperationDescriptor,
    effects: crate::services::mcp_service::OperationEffects,
    method_override: bool,
) {
    descriptor.verb = if effects.destructive {
        ApprovalVerb::Destructive
    } else if effects.reads && !method_override {
        ApprovalVerb::Read
    } else {
        ApprovalVerb::Write
    };
}

/// Model generation is read-class only under the catalog's inference contract
/// and the same closed, stateless request contract used for assistant consent.
/// Provider storage, resource handles, hosted effects and unknown extensions
/// retain method classification. Service names never establish this authority.
/// Only live guest model transport recognizes Codex correlation metadata here;
/// owner approval verbs retain their pre-extension behavior.
pub fn apply_inference_contract(
    descriptor: &mut OperationDescriptor,
    service: &crate::models::downstream_service::DownstreamService,
    body: Option<&[u8]>,
    method_override: bool,
    guest_model: bool,
) {
    // An override makes even a GET/HEAD ambiguous. This also covers the LLM
    // path, which derives its initial verb directly from the HTTP method.
    if method_override {
        if descriptor.verb != ApprovalVerb::Destructive {
            descriptor.verb = ApprovalVerb::Write;
        }
        return;
    }
    if descriptor.verb != ApprovalVerb::Destructive
        && service.inference.is_some()
        && descriptor.method.as_deref() == Some("POST")
        && body
            .and_then(|bytes| serde_json::from_slice(bytes).ok())
            .is_some_and(|body| {
                let path = descriptor.resource.as_deref().unwrap_or_default();
                if guest_model {
                    crate::services::assistant_service_execution::stateless_model_transport(
                        path, &body,
                    )
                } else {
                    crate::services::assistant_service_execution::stateless_inference(path, &body)
                }
            })
    {
        descriptor.verb = ApprovalVerb::Read;
    }
}

pub fn derive_verb_from_method(method: &str) -> ApprovalVerb {
    match method.trim().to_ascii_uppercase().as_str() {
        "GET" | "HEAD" | "OPTIONS" => ApprovalVerb::Read,
        "DELETE" => ApprovalVerb::Destructive,
        _ => ApprovalVerb::Write,
    }
}

/// Only the opaque smart-HTTP fetch protocol can demote a machine Git POST.
/// Structured bodies must retain their ordinary write/override classification.
pub fn apply_machine_git_contract(
    descriptor: &mut OperationDescriptor,
    headers: &axum::http::HeaderMap,
    method_override: bool,
) {
    let mut content_types = headers.get_all(axum::http::header::CONTENT_TYPE).iter();
    let upload_pack = content_types
        .next()
        .is_some_and(|value| value.as_bytes() == b"application/x-git-upload-pack-request")
        && content_types.next().is_none();
    if !method_override
        && upload_pack
        && descriptor.verb != ApprovalVerb::Destructive
        && descriptor.method.as_deref() == Some("POST")
        && descriptor
            .resource
            .as_deref()
            .is_some_and(|path| path.ends_with("/git-upload-pack"))
    {
        descriptor.verb = ApprovalVerb::Read;
    }
}

fn build_http_like_descriptor(
    protocol: Protocol,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> OperationDescriptor {
    let method = normalize_method_for_display(method);
    let resource = normalize_http_path(path);
    OperationDescriptor {
        protocol,
        verb: derive_verb_from_method(&method),
        method: Some(method.clone()),
        resource: Some(resource.clone()),
        summary: action_description::build_action_description(&method, &resource, body),
    }
}

fn normalize_method_for_display(method: &str) -> String {
    method.trim().to_ascii_uppercase()
}

fn normalize_http_path(path: &str) -> String {
    let without_query = path.split_once('?').map_or(path, |(path, _)| path);
    let trimmed = without_query.trim();
    if trimmed.is_empty() {
        return "/".to_string();
    }
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

#[allow(dead_code)]
fn normalize_resource(resource: &str) -> String {
    resource
        .split_once('?')
        .map_or(resource, |(path, _)| path)
        .trim()
        .to_string()
}

fn sanitize_ssh_command(command: &str) -> String {
    truncate_stored_resource(&redact_ssh_command_secrets(command.trim()))
}

pub(crate) fn redact_ssh_command_secrets(command: &str) -> String {
    let mut redacted = command.to_string();
    for (regex, replacement) in ssh_redaction_patterns() {
        redacted = regex
            .replace_all(&redacted, replacement.as_str())
            .into_owned();
    }
    redacted
}

fn ssh_redaction_patterns() -> &'static [(Regex, String)] {
    static PATTERNS: OnceLock<Vec<(Regex, String)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (
                Regex::new(r"(?i)(^|\s)(-p)(\S+)").expect("valid ssh redaction regex"),
                "$1$2***".to_string(),
            ),
            (
                Regex::new(r"(?i)(--password(?:=|\s+))(\S+)").expect("valid ssh redaction regex"),
                "$1***".to_string(),
            ),
            (
                Regex::new(r#"(?i)(authorization:\s*(?:bearer\s+)?)([^\s"']+)"#)
                    .expect("valid ssh redaction regex"),
                "$1***".to_string(),
            ),
            (
                Regex::new(r#"(?i)(\btoken=)([^\s&;"']+)"#).expect("valid ssh redaction regex"),
                "$1***".to_string(),
            ),
            (
                Regex::new(r#"(?i)(\bapi[-_]?key=)([^\s&;"']+)"#)
                    .expect("valid ssh redaction regex"),
                "$1***".to_string(),
            ),
        ]
    })
}

fn truncate_stored_resource(resource: &str) -> String {
    if resource.len() <= MAX_STORED_RESOURCE_LEN {
        return resource.to_string();
    }
    let mut end = MAX_STORED_RESOURCE_LEN - 3;
    while !resource.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &resource[..end])
}

fn truncate_summary(summary: &str) -> String {
    if summary.len() <= 200 {
        return summary.to_string();
    }
    let mut end = 197;
    while !summary.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &summary[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_git_classification_requires_upload_pack_without_overrides() {
        use axum::http::{HeaderMap, HeaderValue, header::CONTENT_TYPE};
        let classify = |content_type: Option<&str>,
                        header: Option<&str>,
                        query: Option<&str>,
                        body: &[u8],
                        path: &str| {
            let mut headers = HeaderMap::new();
            if let Some(value) = content_type {
                headers.insert(CONTENT_TYPE, HeaderValue::from_str(value).unwrap());
            }
            if let Some(name) = header {
                headers.insert(
                    axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    HeaderValue::from_static("DELETE"),
                );
            }
            let method_override = crate::services::mcp_service::http_carries_method_override(
                "POST", &headers, query, body,
            );
            let mut operation = build_http_descriptor("POST", path, Some(body));
            apply_operation_effects(
                &mut operation,
                crate::services::mcp_service::operation_effects(
                    &reqwest::Method::POST,
                    Default::default(),
                ),
                method_override,
            );
            apply_machine_git_contract(&mut operation, &headers, method_override);
            operation.verb
        };
        let git = Some("application/x-git-upload-pack-request");
        let path = "/owner/repo.git/git-upload-pack";
        assert_eq!(classify(git, None, None, b"0000", path), ApprovalVerb::Read);
        for header in [
            "x-http-method-override",
            "x-method-override",
            "x-method",
            "x_http_method_override",
        ] {
            assert_eq!(
                classify(git, Some(header), None, b"0000", path),
                ApprovalVerb::Write
            );
        }
        for query in [
            "_method=DELETE",
            "%5fHttpMethod=DELETE",
            "method=DELETE",
            "x-http-method-override=DELETE",
        ] {
            assert_eq!(
                classify(git, None, Some(query), b"0000", path),
                ApprovalVerb::Write
            );
        }
        for (content_type, body) in [
            ("application/json", br#"{"_method":"DELETE"}"#.as_slice()),
            (
                "application/x-www-form-urlencoded",
                b"_method=DELETE".as_slice(),
            ),
            ("application/json", b"{}".as_slice()),
            ("text/plain", b"_method=DELETE".as_slice()),
        ] {
            assert_eq!(
                classify(Some(content_type), None, None, body, path),
                ApprovalVerb::Write
            );
        }
        for content_type in [
            None,
            Some("application/octet-stream"),
            Some("application/x-git-upload-pack-request; charset=utf-8"),
        ] {
            assert_eq!(
                classify(content_type, None, None, b"0000", path),
                ApprovalVerb::Write
            );
        }
        assert_eq!(
            classify(git, None, None, b"0000", "/owner/repo.git/git-receive-pack"),
            ApprovalVerb::Write
        );
        let mut headers = HeaderMap::new();
        headers.append(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-git-upload-pack-request"),
        );
        headers.append(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut operation = build_http_descriptor("POST", path, None);
        apply_machine_git_contract(&mut operation, &headers, false);
        assert_eq!(operation.verb, ApprovalVerb::Write);
        headers.remove(CONTENT_TYPE);
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-git-upload-pack-request"),
        );
        operation.verb = ApprovalVerb::Destructive;
        apply_machine_git_contract(&mut operation, &headers, false);
        assert_eq!(operation.verb, ApprovalVerb::Destructive);
    }

    #[test]
    fn owner_1995_codex_metadata_preserves_owner_approval_verbs() {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.inference = Some(
            serde_json::from_value(serde_json::json!({"wire_protocol":"openai_responses"}))
                .unwrap(),
        );
        let body = serde_json::to_vec(
            &serde_json::json!({"model":"m","store":false,"input":"hello",
            "client_metadata":{"thread_id":"correlation","turn_id":"turn"}}),
        )
        .unwrap();
        for llm in [false, true] {
            for (guest, method_override, expected) in [
                (false, false, ApprovalVerb::Write),
                (true, false, ApprovalVerb::Read),
                (true, true, ApprovalVerb::Write),
            ] {
                let mut descriptor = if llm {
                    build_llm_descriptor("POST", "responses", Some(&body))
                } else {
                    build_http_descriptor("POST", "responses", Some(&body))
                };
                apply_inference_contract(
                    &mut descriptor,
                    &service,
                    Some(&body),
                    method_override,
                    guest,
                );
                assert_eq!(descriptor.verb, expected);
            }
        }
    }

    #[test]
    fn llm_method_overrides_never_retain_read_approval_classification() {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        for inference in [false, true] {
            service.inference =
                inference.then_some(crate::models::downstream_service::ServiceInference {
                    wire_protocol:
                        crate::models::downstream_service::InferenceWireProtocol::OpenaiCompletions,
                    model_list: true,
                    realtime: false,
                    voice: None,
                });
            for method in ["GET", "HEAD", "OPTIONS"] {
                let mut descriptor = build_llm_descriptor(method, "/v1/models", None);
                apply_inference_contract(&mut descriptor, &service, None, false, false);
                assert_eq!(descriptor.verb, ApprovalVerb::Read);
                apply_inference_contract(&mut descriptor, &service, None, true, false);
                assert_eq!(descriptor.verb, ApprovalVerb::Write);
            }
            let mut descriptor = build_llm_descriptor("DELETE", "/v1/files/item", None);
            apply_inference_contract(&mut descriptor, &service, None, true, false);
            assert_eq!(descriptor.verb, ApprovalVerb::Destructive);
        }
    }

    #[test]
    fn unknown_verbs_and_untrusted_read_marks_cannot_bypass_write_approval() {
        for method in ["CONNECT", "TRACE", "UNKNOWN", ""] {
            assert_eq!(derive_verb_from_method(method), ApprovalVerb::Write);
        }
        let mut descriptor = build_mcp_descriptor("POST", "/search", None);
        let mut metadata = crate::services::mcp_service::McpDurableEndpointMetadata {
            risk: Some(crate::models::service_endpoint::EndpointRisk::Read),
            ..Default::default()
        };
        let method = reqwest::Method::POST;
        apply_operation_effects(
            &mut descriptor,
            crate::services::mcp_service::operation_effects(&method, metadata),
            false,
        );
        assert_eq!(descriptor.verb, ApprovalVerb::Write);
        metadata.catalog_contract = true;
        apply_operation_effects(
            &mut descriptor,
            crate::services::mcp_service::operation_effects(&method, metadata),
            false,
        );
        assert_eq!(descriptor.verb, ApprovalVerb::Read);
        apply_operation_effects(
            &mut descriptor,
            crate::services::mcp_service::operation_effects(&method, metadata),
            true,
        );
        assert_eq!(descriptor.verb, ApprovalVerb::Write);
    }

    #[test]
    fn http_descriptor_derives_read_write_destructive_verbs() {
        assert_eq!(
            build_http_descriptor("GET", "/v1/models", None).verb,
            ApprovalVerb::Read
        );
        assert_eq!(
            build_http_descriptor("post", "/v1/chat/completions", None).verb,
            ApprovalVerb::Write
        );
        assert_eq!(
            build_http_descriptor("DELETE", "/v1/files/file-1", None).verb,
            ApprovalVerb::Destructive
        );
    }

    #[test]
    fn http_descriptor_normalizes_path_and_strips_query_for_resource() {
        let descriptor = build_http_descriptor("GET", "v1/models?limit=10", None);

        assert_eq!(descriptor.method.as_deref(), Some("GET"));
        assert_eq!(descriptor.resource.as_deref(), Some("/v1/models"));
        assert_eq!(descriptor.normalized_method().as_deref(), Some("get"));
        assert_eq!(
            descriptor.normalized_resource().as_deref(),
            Some("/v1/models")
        );
        assert_eq!(descriptor.operation_summary(), "proxy:GET /v1/models");
    }

    #[test]
    fn llm_descriptor_reuses_action_description_summary() {
        let body = serde_json::to_vec(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "secret"}]
        }))
        .unwrap();

        let descriptor = build_llm_descriptor("POST", "/openai/v1/chat/completions", Some(&body));

        assert_eq!(descriptor.protocol, Protocol::Llm);
        assert_eq!(descriptor.verb, ApprovalVerb::Write);
        assert!(descriptor.summary.contains("model: gpt-4"));
        assert!(descriptor.summary.contains("1 messages"));
        assert!(!descriptor.summary.contains("secret"));
        assert_eq!(
            descriptor.operation_summary(),
            "llm:POST /openai/v1/chat/completions"
        );
    }

    #[test]
    fn ssh_tunnel_descriptor_is_coarse() {
        let descriptor = build_ssh_descriptor(SshOperationKind::Tunnel, None);

        assert_eq!(descriptor.protocol, Protocol::Ssh);
        assert_eq!(descriptor.verb, ApprovalVerb::Write);
        assert_eq!(descriptor.method.as_deref(), Some("TUNNEL"));
        assert_eq!(descriptor.resource.as_deref(), Some(""));
        assert_eq!(descriptor.operation_summary(), "ssh:tunnel");
    }

    #[test]
    fn ssh_exec_descriptor_carries_command_for_later_rule_matching() {
        let descriptor = build_ssh_descriptor(SshOperationKind::Exec, Some("git push origin main"));

        assert_eq!(descriptor.method.as_deref(), Some("EXEC"));
        assert_eq!(descriptor.resource.as_deref(), Some("git push origin main"));
        assert_eq!(
            descriptor.normalized_resource().as_deref(),
            Some("git push origin main")
        );
        assert_eq!(
            descriptor.operation_summary(),
            "ssh:EXEC git push origin main"
        );
        assert_eq!(descriptor.summary, "SSH exec: git push origin main");
    }

    #[test]
    fn mcp_endpoint_descriptor_reuses_http_verb_logic() {
        let descriptor = build_mcp_endpoint_descriptor("delete", "/repos/{owner}/{repo}");

        assert_eq!(descriptor.protocol, Protocol::Mcp);
        assert_eq!(descriptor.verb, ApprovalVerb::Destructive);
        assert_eq!(descriptor.method.as_deref(), Some("DELETE"));
        assert_eq!(
            descriptor.resource.as_deref(),
            Some("/repos/{owner}/{repo}")
        );
    }

    #[test]
    fn mcp_descriptor_summarizes_body_with_existing_scrubber() {
        let body = serde_json::to_vec(&serde_json::json!({
            "model": "gpt-4.1",
            "api_key": "secret",
            "messages": [{"role": "user", "content": "private"}]
        }))
        .unwrap();

        let descriptor = build_mcp_descriptor("post", "/v1/chat/completions", Some(&body));

        assert_eq!(descriptor.verb, ApprovalVerb::Write);
        assert!(descriptor.summary.contains("model: gpt-4.1"));
        assert!(descriptor.summary.contains("1 messages"));
        assert!(!descriptor.summary.contains("secret"));
        assert!(!descriptor.summary.contains("private"));
    }

    #[test]
    fn mcp_meta_descriptor_is_read_only() {
        let descriptor = build_mcp_meta_descriptor("nyx__search_tools");

        assert_eq!(descriptor.protocol, Protocol::Mcp);
        assert_eq!(descriptor.verb, ApprovalVerb::Read);
        assert_eq!(descriptor.method.as_deref(), Some("tools/call"));
        assert_eq!(descriptor.resource.as_deref(), Some("nyx__search_tools"));
    }

    #[test]
    fn ssh_exec_descriptor_redacts_common_secret_patterns() {
        let descriptor = build_ssh_descriptor(
            SshOperationKind::Exec,
            Some(
                "mysql -pPASS --password hunter2 curl -H 'Authorization: Bearer abc-secret' \
                 token=token-secret api_key=apikey-secret api-key=apikey2-secret",
            ),
        );
        let resource = descriptor.resource.as_deref().unwrap();

        assert!(resource.contains("-p***"));
        assert!(resource.contains("--password ***"));
        assert!(resource.contains("Authorization: Bearer ***"));
        assert!(resource.contains("token=***"));
        assert!(resource.contains("api_key=***"));
        assert!(resource.contains("api-key=***"));
        assert!(!resource.contains("PASS"));
        assert!(!resource.contains("hunter2"));
        assert!(!resource.contains("abc-secret"));
        assert!(!resource.contains("token-secret"));
        assert!(!resource.contains("apikey-secret"));
        assert!(!resource.contains("apikey2-secret"));
        assert_eq!(descriptor.summary, format!("SSH exec: {resource}"));
    }

    #[test]
    fn ssh_exec_descriptor_truncates_stored_resource() {
        let long = format!("echo {}", "a".repeat(400));
        let descriptor = build_ssh_descriptor(SshOperationKind::Exec, Some(&long));
        let resource = descriptor.resource.as_deref().unwrap();

        assert!(resource.len() <= MAX_STORED_RESOURCE_LEN);
        assert!(resource.ends_with("..."));
        assert!(descriptor.summary.len() <= 200);
    }
}
