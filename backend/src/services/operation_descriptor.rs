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

pub fn derive_verb_from_method(method: &str) -> ApprovalVerb {
    match method.trim().to_ascii_uppercase().as_str() {
        "GET" | "HEAD" | "OPTIONS" => ApprovalVerb::Read,
        "DELETE" => ApprovalVerb::Destructive,
        _ => ApprovalVerb::Write,
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
