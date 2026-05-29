use crate::services::action_description;

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
pub enum Verb {
    Read,
    Write,
    Destructive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationDescriptor {
    pub protocol: Protocol,
    pub verb: Verb,
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
            .map(normalize_resource)
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
    let method = normalize_method_for_display(method);
    let resource = normalize_http_path(path);
    OperationDescriptor {
        protocol: Protocol::Mcp,
        verb: derive_verb_from_method(&method),
        method: Some(method.clone()),
        resource: Some(resource.clone()),
        summary: action_description::build_action_description(&method, &resource, None),
    }
}

pub fn build_ssh_descriptor(kind: SshOperationKind, command: Option<&str>) -> OperationDescriptor {
    match kind {
        SshOperationKind::Exec => {
            let command = command.unwrap_or_default().trim().to_string();
            OperationDescriptor {
                protocol: Protocol::Ssh,
                verb: Verb::Write,
                method: Some("EXEC".to_string()),
                resource: Some(command.clone()),
                summary: truncate_summary(&format!("SSH exec: {command}")),
            }
        }
        SshOperationKind::Tunnel => OperationDescriptor {
            protocol: Protocol::Ssh,
            verb: Verb::Write,
            method: Some("TUNNEL".to_string()),
            resource: Some(String::new()),
            summary: "SSH tunnel session".to_string(),
        },
    }
}

pub fn derive_verb_from_method(method: &str) -> Verb {
    match method.trim().to_ascii_uppercase().as_str() {
        "GET" | "HEAD" | "OPTIONS" => Verb::Read,
        "DELETE" => Verb::Destructive,
        _ => Verb::Write,
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
            Verb::Read
        );
        assert_eq!(
            build_http_descriptor("post", "/v1/chat/completions", None).verb,
            Verb::Write
        );
        assert_eq!(
            build_http_descriptor("DELETE", "/v1/files/file-1", None).verb,
            Verb::Destructive
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
        assert_eq!(descriptor.verb, Verb::Write);
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
        assert_eq!(descriptor.verb, Verb::Write);
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
        assert_eq!(descriptor.verb, Verb::Destructive);
        assert_eq!(descriptor.method.as_deref(), Some("DELETE"));
        assert_eq!(
            descriptor.resource.as_deref(),
            Some("/repos/{owner}/{repo}")
        );
    }
}
