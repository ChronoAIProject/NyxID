//! Server allowlist for explicit client analytics. Raw errors, page URLs,
//! searches, form contents, profile labels and message IDs are not collected.
use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum ClientEventName {
    #[serde(rename = "ui.dialog_opened")]
    UiDialogOpened,
    #[serde(rename = "ui.dialog_step_completed")]
    UiDialogStepCompleted,
    #[serde(rename = "ui.dialog_abandoned")]
    UiDialogAbandoned,
    #[serde(rename = "ui.provider_connect_initiated")]
    UiProviderConnectInitiated,
    #[serde(rename = "ui.secret_copied")]
    UiSecretCopied,
    #[serde(rename = "ui.inline_edit_started")]
    UiInlineEditStarted,
    #[serde(rename = "ui.list_filtered")]
    UiListFiltered,
    #[serde(rename = "ui.list_searched")]
    UiListSearched,
    #[serde(rename = "ui.nav_target_opened")]
    UiNavTargetOpened,
    #[serde(rename = "ui.docs_opened")]
    UiDocsOpened,
    #[serde(rename = "ui.external_link_opened")]
    UiExternalLinkOpened,
    #[serde(rename = "ui.destructive_confirmed")]
    UiDestructiveConfirmed,
    #[serde(rename = "ui.decision_made")]
    UiDecisionMade,
    #[serde(rename = "ui.preference_toggled")]
    UiPreferenceToggled,
    #[serde(rename = "ui.assistant_feedback_submitted")]
    UiAssistantFeedbackSubmitted,
    #[serde(rename = "mobile.deep_link_opened")]
    MobileDeepLinkOpened,
    #[serde(rename = "mobile.approval_viewed")]
    MobileApprovalViewed,
    #[serde(rename = "mobile.push_received")]
    MobilePushReceived,
    #[serde(rename = "mobile.biometric_prompted")]
    MobileBiometricPrompted,
    #[serde(rename = "mobile.biometric_result")]
    MobileBiometricResult,
    #[serde(rename = "ui.mobile_dialog_opened")]
    UiMobileDialogOpened,
    #[serde(rename = "ui.mobile_dialog_step_completed")]
    UiMobileDialogStepCompleted,
    #[serde(rename = "ui.mobile_dialog_abandoned")]
    UiMobileDialogAbandoned,
    #[serde(rename = "ui.mobile_provider_connect_initiated")]
    UiMobileProviderConnectInitiated,
    #[serde(rename = "ui.mobile_inline_edit_started")]
    UiMobileInlineEditStarted,
    #[serde(rename = "ui.mobile_list_filtered")]
    UiMobileListFiltered,
    #[serde(rename = "ui.mobile_nav_target_opened")]
    UiMobileNavTargetOpened,
    #[serde(rename = "ui.mobile_legal_page_opened")]
    UiMobileLegalPageOpened,
    #[serde(rename = "ui.mobile_destructive_confirmed")]
    UiMobileDestructiveConfirmed,
    #[serde(rename = "ui.mobile_decision_made")]
    UiMobileDecisionMade,
    #[serde(rename = "ui.mobile_preference_toggled")]
    UiMobilePreferenceToggled,
    #[serde(rename = "cli.command_invoked")]
    CliCommandInvoked,
}

impl ClientEventName {
    pub fn name(self) -> &'static str {
        match self {
            Self::UiDialogOpened => "ui.dialog_opened",
            Self::UiDialogStepCompleted => "ui.dialog_step_completed",
            Self::UiDialogAbandoned => "ui.dialog_abandoned",
            Self::UiProviderConnectInitiated => "ui.provider_connect_initiated",
            Self::UiSecretCopied => "ui.secret_copied",
            Self::UiInlineEditStarted => "ui.inline_edit_started",
            Self::UiListFiltered => "ui.list_filtered",
            Self::UiListSearched => "ui.list_searched",
            Self::UiNavTargetOpened => "ui.nav_target_opened",
            Self::UiDocsOpened => "ui.docs_opened",
            Self::UiExternalLinkOpened => "ui.external_link_opened",
            Self::UiDestructiveConfirmed => "ui.destructive_confirmed",
            Self::UiDecisionMade => "ui.decision_made",
            Self::UiPreferenceToggled => "ui.preference_toggled",
            Self::UiAssistantFeedbackSubmitted => "ui.assistant_feedback_submitted",
            Self::MobileDeepLinkOpened => "mobile.deep_link_opened",
            Self::MobileApprovalViewed => "mobile.approval_viewed",
            Self::MobilePushReceived => "mobile.push_received",
            Self::MobileBiometricPrompted => "mobile.biometric_prompted",
            Self::MobileBiometricResult => "mobile.biometric_result",
            Self::UiMobileDialogOpened => "ui.mobile_dialog_opened",
            Self::UiMobileDialogStepCompleted => "ui.mobile_dialog_step_completed",
            Self::UiMobileDialogAbandoned => "ui.mobile_dialog_abandoned",
            Self::UiMobileProviderConnectInitiated => "ui.mobile_provider_connect_initiated",
            Self::UiMobileInlineEditStarted => "ui.mobile_inline_edit_started",
            Self::UiMobileListFiltered => "ui.mobile_list_filtered",
            Self::UiMobileNavTargetOpened => "ui.mobile_nav_target_opened",
            Self::UiMobileLegalPageOpened => "ui.mobile_legal_page_opened",
            Self::UiMobileDestructiveConfirmed => "ui.mobile_destructive_confirmed",
            Self::UiMobileDecisionMade => "ui.mobile_decision_made",
            Self::UiMobilePreferenceToggled => "ui.mobile_preference_toggled",
            Self::CliCommandInvoked => "cli.command_invoked",
        }
    }
    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::UiDialogOpened => &["dialog_id", "entry_point"],
            Self::UiDialogStepCompleted => &["dialog_id", "step", "total_steps"],
            Self::UiDialogAbandoned => &["dialog_id", "final_step", "duration_ms"],
            Self::UiProviderConnectInitiated => &["provider_slug", "method"],
            Self::UiSecretCopied => &["secret_type", "context"],
            Self::UiInlineEditStarted => &["domain", "field"],
            Self::UiListFiltered => &["list", "result_count"],
            Self::UiListSearched => &["list", "result_count"],
            Self::UiNavTargetOpened => &["target", "source"],
            Self::UiDocsOpened => &["page"],
            Self::UiExternalLinkOpened => &["url_domain"],
            Self::UiDestructiveConfirmed => &["domain", "action", "sub_domain"],
            Self::UiDecisionMade => &["domain", "decision", "decision_ms"],
            Self::UiPreferenceToggled => &["name", "value"],
            Self::UiAssistantFeedbackSubmitted => &["vote"],
            Self::MobileDeepLinkOpened => &["link_type"],
            Self::MobileApprovalViewed => &["service_slug", "mode"],
            Self::MobilePushReceived => &["type", "app_state"],
            Self::MobileBiometricPrompted => &["reason"],
            Self::MobileBiometricResult => &["reason", "outcome"],
            Self::UiMobileDialogOpened => &["dialog_id", "entry_point"],
            Self::UiMobileDialogStepCompleted => &["dialog_id", "step", "total_steps"],
            Self::UiMobileDialogAbandoned => &["dialog_id", "final_step", "duration_ms"],
            Self::UiMobileProviderConnectInitiated => &["provider", "method"],
            Self::UiMobileInlineEditStarted => &["domain", "field"],
            Self::UiMobileListFiltered => &["list", "result_count"],
            Self::UiMobileNavTargetOpened => &["target", "source"],
            Self::UiMobileLegalPageOpened => &["page"],
            Self::UiMobileDestructiveConfirmed => &["domain", "action"],
            Self::UiMobileDecisionMade => &["domain", "decision", "decision_ms"],
            Self::UiMobilePreferenceToggled => &["name", "value"],
            Self::CliCommandInvoked => &[
                "command_group",
                "subcommand",
                "exit_code",
                "duration_ms",
                "os",
                "arch",
            ],
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientEvent {
    pub name: ClientEventName,
    pub props: BTreeMap<String, Value>,
}

impl std::fmt::Debug for ClientEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientEvent")
            .field("name", &self.name)
            .field("props", &"[REDACTED]")
            .finish()
    }
}

impl ClientEvent {
    fn valid_property(&self, field: &str, value: &Value) -> bool {
        let known = |values: &[&str]| value.as_str().is_some_and(|v| values.contains(&v));
        let integer = |max: u64| value.as_u64().is_some_and(|v| v <= max);
        let mobile = self.name.name().starts_with("ui.mobile_");
        match field {
            "domain" => {
                if mobile {
                    known(&[
                        "auth",
                        "approvals",
                        "account",
                        "settings",
                        "activity",
                        "legal",
                        "nav",
                    ])
                } else {
                    known(&[
                        "keys",
                        "services",
                        "endpoints",
                        "catalog",
                        "api_keys",
                        "agent_bindings",
                        "approvals",
                        "nodes",
                        "channels",
                        "notifications",
                        "developer_apps",
                        "oauth",
                        "ssh",
                        "auth",
                        "mfa",
                        "settings",
                        "admin",
                        "account",
                        "nav",
                        "docs",
                    ])
                }
            }
            "sub_domain" => known(&[
                "users",
                "roles",
                "groups",
                "service_accounts",
                "audit_log",
                "invite_codes",
                "orgs",
                "devices",
                "sessions",
                "profile",
                "mfa",
            ]),
            "dialog_id" => {
                if mobile {
                    known(&[
                        "login",
                        "register",
                        "mfa_setup",
                        "mfa_verify",
                        "forgot_password",
                        "biometric_enable",
                        "approval_detail",
                        "profile_edit",
                        "change_password",
                        "delete_account_confirm",
                        "other",
                    ])
                } else {
                    known(&[
                        "login",
                        "register",
                        "forgot_password",
                        "mfa_enroll",
                        "mfa_verify",
                        "add_key",
                        "edit_key",
                        "add_service",
                        "edit_service",
                        "add_endpoint",
                        "edit_endpoint",
                        "add_api_key",
                        "edit_api_key",
                        "add_agent_binding",
                        "add_channel_bot",
                        "edit_channel_bot",
                        "add_node",
                        "register_node_wizard",
                        "add_developer_app",
                        "edit_developer_app",
                        "oauth_consent",
                        "approval_mode_edit",
                        "admin_suspend_user",
                        "admin_edit_user",
                        "admin_add_role",
                        "admin_add_group",
                        "admin_add_service_account",
                        "admin_add_invite_code",
                        "delete_account_confirm",
                        "other",
                    ])
                }
            }
            "action" => {
                if mobile {
                    known(&[
                        "delete_account",
                        "revoke_session",
                        "disconnect_device",
                        "wipe",
                        "cancel_registration",
                    ])
                } else {
                    known(&[
                        "delete",
                        "revoke",
                        "rotate",
                        "suspend",
                        "unsuspend",
                        "disconnect",
                        "wipe",
                    ])
                }
            }
            "method" => {
                if mobile {
                    known(&["oauth", "device_code", "token_exchange", "password"])
                } else {
                    known(&["oauth", "device_code", "api_key"])
                }
            }
            "source" => {
                if mobile {
                    known(&["tab", "header", "back", "deep_link"])
                } else {
                    known(&["sidebar", "breadcrumb", "tab", "quick_link"])
                }
            }
            "decision" => known(&["approve", "deny", "skip", "defer"]),
            "secret_type" => known(&[
                "api_key",
                "client_id",
                "client_secret",
                "ca_key",
                "curl_example",
                "one_time_recovery_code",
                "other",
            ]),
            "context" => known(&["creation_modal", "detail_page", "settings"]),
            "link_type" => known(&["challenge", "approval", "other"]),
            "type" => known(&["approval_request", "other"]),
            "app_state" => known(&["foreground", "background"]),
            "reason" => known(&["app_open", "approval_decision"]),
            "outcome" => known(&["success", "failed", "cancelled", "unavailable"]),
            "provider" => known(&["google", "github", "apple"]),
            "provider_slug" => known(&["google", "github", "apple"]),
            "vote" => known(&["up", "down"]),
            "mode" => known(&["per_request", "grant"]),
            "page" => known(&[
                "privacy",
                "terms",
                "encryption",
                "credential-nodes",
                "mcp-proxy",
                "the-proxy",
                "broker-model",
                "oauth-oidc",
            ]),
            "list" => known(&[
                "keys",
                "services",
                "endpoints",
                "catalog",
                "api_keys",
                "agent_bindings",
                "approvals",
                "nodes",
                "channels",
                "notifications",
                "developer_apps",
                "oauth",
                "ssh",
                "auth",
                "mfa",
                "settings",
                "admin",
                "account",
                "nav",
                "docs",
                "activity",
                "sessions",
            ]),
            "os" => known(&[
                "linux", "macos", "windows", "freebsd", "openbsd", "netbsd", "android", "ios",
            ]),
            "arch" => known(&[
                "x86",
                "x86_64",
                "aarch64",
                "arm",
                "riscv64",
                "powerpc64",
                "s390x",
            ]),
            "command_group" => known(&[
                "auth",
                "user",
                "service",
                "cli",
                "catalog",
                "tools",
                "pool",
                "trigger",
                "billing",
                "provider",
                "api_key",
                "org",
                "node",
                "device",
                "node_credential",
                "proxy",
                "public",
                "ssh",
                "openclaw",
                "mcp",
                "notification",
                "oauth",
                "approval",
                "endpoint",
                "external_key",
                "service_account",
                "developer_app",
                "ai_setup",
                "channel_bot",
                "channel_event",
                "admin",
                "telemetry",
                "oracle",
                "repo",
                "pairing",
            ]),
            "subcommand" => known(&[
                "login",
                "logout",
                "register",
                "verify_email",
                "forgot_password",
                "reset_password",
                "whoami",
                "status",
                "connect",
                "doctor",
                "profile",
                "mfa",
                "session",
                "subcommand",
                "list",
                "update",
                "repo",
                "info",
            ]),
            "step" | "final_step" | "total_steps" => integer(100),
            "result_count" => integer(1_000_000),
            "duration_ms" | "decision_ms" => integer(86_400_000),
            "exit_code" => integer(255),
            "name" => known(&[
                "analytics",
                "theme",
                "biometric",
                "push_notifications",
                "push_enabled",
                "telegram_enabled",
                "theme_system",
            ]),
            "value" => match self.props.get("name").and_then(Value::as_str) {
                Some(
                    "analytics" | "biometric" | "push_notifications" | "push_enabled"
                    | "telegram_enabled" | "theme_system",
                ) => value.is_boolean(),
                Some("theme") => known(&["light", "dark", "system"]),
                _ => false,
            },
            _ => false,
        }
    }

    pub fn properties(&self) -> AppResult<Value> {
        if self.props.keys().any(|key| key.starts_with('$'))
            || self.props.len() > 12
            || serde_json::to_vec(self).map_or(true, |v| v.len() > 4096)
        {
            return Err(AppError::ValidationError(
                "Analytics event exceeds limits".into(),
            ));
        }
        let mut result = json!({});
        for field in self.name.fields() {
            let Some(value) = self.props.get(*field) else {
                continue;
            };
            // Each field has its own type and finite taxonomy. Free-form
            // labels, service slugs, hostnames and URLs are intentionally
            // omitted, even if the client schema still carries them locally.
            let allowed = self.valid_property(field, value);
            if allowed {
                result[*field] = value.clone();
            }
        }
        super::scrub::scrub_value(&mut result);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_names_and_drops_uncollected_or_secret_fields() {
        assert!(
            serde_json::from_value::<ClientEvent>(json!({"name":"$identify", "props":{}})).is_err()
        );
        let event: ClientEvent = serde_json::from_value(json!({"name":"ui.list_searched", "props":{
            "list":"keys", "filter":"private query", "result_count":3, "credential":"distinctive-secret"
        }})).unwrap();
        assert_eq!(
            event.properties().unwrap(),
            json!({"list":"keys", "result_count":3})
        );
        let event: ClientEvent = serde_json::from_value(
            json!({"name":"ui.docs_opened", "props":{"page":"https://x.test/?token=secret"}}),
        )
        .unwrap();
        assert_eq!(event.properties().unwrap(), json!({}));
        assert!(!format!("{event:?}").contains("secret"));
    }
}

#[cfg(test)]
mod taxonomy_tests {
    use super::*;
    fn properties(name: &str, props: Value) -> Value {
        serde_json::from_value::<ClientEvent>(json!({"name":name,"props":props}))
            .unwrap()
            .properties()
            .unwrap()
    }
    #[test]
    fn strings_are_field_specific_and_arbitrary_urls_labels_and_slugs_are_omitted() {
        assert_eq!(
            properties(
                "ui.dialog_opened",
                json!({"dialog_id":"add_key", "entry_point":"https://example.com"})
            ),
            json!({"dialog_id":"add_key"})
        );
        for value in [
            "distinctive-secret",
            "https://example.com",
            "https://example.com/no-query",
            "user.example.com",
        ] {
            assert_eq!(
                properties("ui.docs_opened", json!({"page":value})),
                json!({})
            );
            assert_eq!(
                properties(
                    "ui.destructive_confirmed",
                    json!({"domain":value,"action":value})
                ),
                json!({})
            );
            assert_eq!(
                properties("ui.external_link_opened", json!({"url_domain":value})),
                json!({})
            );
            assert_eq!(
                properties(
                    "ui.provider_connect_initiated",
                    json!({"provider_slug":value,"method":value})
                ),
                json!({})
            );
            assert_eq!(
                properties("mobile.approval_viewed", json!({"service_slug":value})),
                json!({})
            );
        }
        assert_eq!(
            properties(
                "ui.provider_connect_initiated",
                json!({"provider_slug":"google","method":"oauth"})
            ),
            json!({"provider_slug":"google","method":"oauth"})
        );
        assert_eq!(
            properties(
                "ui.mobile_destructive_confirmed",
                json!({"domain":"account","action":"delete_account"})
            ),
            json!({"domain":"account","action":"delete_account"})
        );
        assert_eq!(
            properties(
                "ui.destructive_confirmed",
                json!({"domain":"account","action":"delete_account"})
            ),
            json!({"domain":"account"})
        );
    }
    #[test]
    fn numeric_and_boolean_properties_have_specific_types_and_bounds() {
        for invalid in [
            json!(-1),
            json!(1.5),
            json!("1"),
            json!(true),
            json!(86_400_001),
        ] {
            assert_eq!(
                properties("ui.dialog_abandoned", json!({"duration_ms":invalid})),
                json!({})
            );
        }
        assert_eq!(
            properties(
                "ui.dialog_step_completed",
                json!({"step":101,"total_steps":true,"dialog_id":3})
            ),
            json!({})
        );
        assert_eq!(
            properties(
                "ui.dialog_abandoned",
                json!({"final_step":2,"duration_ms":50})
            ),
            json!({"final_step":2,"duration_ms":50})
        );
        assert_eq!(
            properties(
                "ui.preference_toggled",
                json!({"name":"theme","value":"dark"})
            ),
            json!({"name":"theme","value":"dark"})
        );
        assert_eq!(
            properties(
                "ui.preference_toggled",
                json!({"name":"analytics","value":true})
            ),
            json!({"name":"analytics","value":true})
        );
        assert_eq!(
            properties(
                "ui.preference_toggled",
                json!({"name":"analytics","value":"secret"})
            ),
            json!({"name":"analytics"})
        );
    }
}
