//! Presentation metadata is not a serialization policy. Values are admitted separately.
pub const VERSION: &str = "service-history-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Created,
    Updated,
    Enabled,
    Disabled,
    Deleted,
    CredentialReplaced,
    CredentialRemoved,
    CredentialReauthorized,
    CredentialBindingChanged,
    RoutingChanged,
    SshChanged,
}

#[derive(Clone, Copy)]
pub struct Definition {
    pub code: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub group: &'static str,
}

impl Action {
    pub const ALL: [Self; 11] = [
        Self::Created,
        Self::Updated,
        Self::Enabled,
        Self::Disabled,
        Self::Deleted,
        Self::CredentialReplaced,
        Self::CredentialRemoved,
        Self::CredentialReauthorized,
        Self::CredentialBindingChanged,
        Self::RoutingChanged,
        Self::SshChanged,
    ];

    pub const fn definition(self) -> Definition {
        let (code, label, description) = match self {
            Self::Created => (
                "service.created",
                "Service created",
                "A service instance was created.",
            ),
            Self::Updated => (
                "service.updated",
                "Service updated",
                "Service configuration changed.",
            ),
            Self::Enabled => (
                "service.enabled",
                "Service enabled",
                "The service was enabled.",
            ),
            Self::Disabled => (
                "service.disabled",
                "Service disabled",
                "The service was disabled.",
            ),
            Self::Deleted => (
                "service.deleted",
                "Service deleted",
                "The service instance was deleted.",
            ),
            Self::CredentialRemoved => (
                "service.credential_removed",
                "Credential removed",
                "The backing credential was removed.",
            ),
            Self::CredentialReplaced => (
                "service.credential_replaced",
                "Credential replaced",
                "Credential material or its binding was replaced.",
            ),
            Self::CredentialReauthorized => (
                "service.credential_reauthorized",
                "Credential reauthorized",
                "A new OAuth authorization completed.",
            ),
            Self::CredentialBindingChanged => (
                "service.credential_binding_changed",
                "Credential source changed",
                "The service switched between user and platform credentials.",
            ),
            Self::RoutingChanged => (
                "service.routing_changed",
                "Routing changed",
                "The service route changed.",
            ),
            Self::SshChanged => (
                "service.ssh_changed",
                "SSH configuration changed",
                "SSH configuration changed.",
            ),
        };
        Definition {
            code,
            label,
            description,
            group: "Service history",
        }
    }
}

pub fn action_label(code: &str) -> &'static str {
    Action::ALL
        .into_iter()
        .map(Action::definition)
        .find(|d| d.code == code)
        .map_or("Service updated", |d| d.label)
}

macro_rules! fields {
    ($($code:literal => $label:literal),* $(,)?) => {
        pub const FIELDS: &[Definition] = &[$(Definition { code: $code, label: $label,
            description: $label, group: "Service settings" }),*];
    };
}
fields! {
    "label" => "Label", "slug" => "Service slug", "url" => "Endpoint URL",
    "endpoint_id" => "Endpoint", "api_key_id" => "Credential connection",
    "auth_method" => "Authentication method", "auth_key_name" => "Authentication key name",
    "credential_binding" => "Credential source", "credential_type" => "Credential type",
    "credential_source" => "OAuth application source", "credential" => "Credential",
    "oauth_application" => "OAuth application", "token_scopes" => "OAuth scopes",
    "node_id" => "Node route", "node_priority" => "Node priority",
    "is_active" => "Enabled", "admin_only" => "Admin only",
    "service_type" => "Service type", "ssh_auth_mode" => "SSH authentication mode",
    "ssh_config" => "SSH configuration", "identity_propagation_mode" => "Identity propagation",
    "identity_include_user_id" => "Include user ID", "identity_include_email" => "Include email",
    "identity_include_name" => "Include name", "identity_jwt_audience" => "Identity audience",
    "forward_access_token" => "Forward access token", "inject_delegation_token" => "Inject delegation token",
    "delegation_token_scope" => "Delegation scope", "custom_user_agent" => "Custom User-Agent",
    "default_request_headers" => "Default request headers", "ws_frame_injections" => "WebSocket authentication rules",
    "openapi_spec_url" => "OpenAPI specification", "recommended_skills" => "Recommended skills",
    "source_app_id" => "Initiating application", "connection_id" => "Provider connection",
    "provider_config_id" => "Provider", "catalog_service_id" => "Catalog service",
}

pub fn field(code: &str) -> Option<&'static Definition> {
    FIELDS.iter().find(|d| d.code == code)
}
