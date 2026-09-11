//! Product presets sharing the existing Google OAuth client.

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{ProxyOperationPolicy, ProxyOperationRule};

pub const DRIVE: &str = "https://www.googleapis.com/auth/drive";
pub const CALENDAR: &str = "https://www.googleapis.com/auth/calendar";
pub const GMAIL_READONLY: &str = "https://www.googleapis.com/auth/gmail.readonly";
pub const GMAIL_SEND: &str = "https://www.googleapis.com/auth/gmail.send";
pub const MANAGED_SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    DRIVE,
    "https://www.googleapis.com/auth/drive.readonly",
    "https://www.googleapis.com/auth/drive.file",
    GMAIL_READONLY,
    GMAIL_SEND,
    CALENDAR,
    "https://www.googleapis.com/auth/calendar.readonly",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoogleProduct {
    Workspace,
    Calendar,
    Drive,
    Gmail,
}

impl GoogleProduct {
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "api-google-workspace" => Some(Self::Workspace),
            "api-google-calendar" => Some(Self::Calendar),
            "api-google-drive" => Some(Self::Drive),
            "api-google-gmail" => Some(Self::Gmail),
            _ => None,
        }
    }

    pub fn default_scopes(self) -> Vec<String> {
        let mut scopes = vec!["openid", "email", "profile"];
        match self {
            Self::Workspace => scopes.extend([DRIVE, CALENDAR, GMAIL_READONLY, GMAIL_SEND]),
            Self::Calendar => scopes.push(CALENDAR),
            Self::Drive => scopes.push(DRIVE),
            Self::Gmail => scopes.extend([GMAIL_READONLY, GMAIL_SEND]),
        }
        scopes.into_iter().map(String::from).collect()
    }

    pub fn required_scopes(self) -> &'static [&'static str] {
        match self {
            Self::Workspace | Self::Gmail => &[GMAIL_SEND],
            Self::Calendar | Self::Drive => &[],
        }
    }

    pub fn validate_required_scopes(self, scopes: Option<&str>) -> AppResult<()> {
        for required in self.required_scopes() {
            if !scopes
                .unwrap_or_default()
                .split_whitespace()
                .any(|scope| scope == *required)
            {
                return Err(AppError::ValidationError(format!(
                    "Gmail send permission ({required}) is required for this Google service. \
                     Reconnect and approve Gmail sending access."
                )));
            }
        }
        Ok(())
    }

    pub fn allowed_scopes(self) -> Vec<String> {
        MANAGED_SCOPES
            .iter()
            .filter(|scope| {
                let identity = matches!(
                    **scope,
                    "openid"
                        | "email"
                        | "profile"
                        | "https://www.googleapis.com/auth/userinfo.email"
                        | "https://www.googleapis.com/auth/userinfo.profile"
                );
                identity
                    || match self {
                        Self::Workspace => true,
                        Self::Calendar => scope.starts_with(CALENDAR),
                        Self::Drive => scope.starts_with(DRIVE),
                        Self::Gmail => matches!(**scope, GMAIL_READONLY | GMAIL_SEND),
                    }
            })
            .map(|scope| (*scope).to_string())
            .collect()
    }

    pub fn validate_scopes(self, scopes: Option<&str>) -> AppResult<()> {
        let allowed = self.allowed_scopes();
        if let Some(scope) = scopes.and_then(|scopes| {
            scopes
                .split_whitespace()
                .find(|scope| !allowed.iter().any(|s| s == scope))
        }) {
            return Err(AppError::ValidationError(format!(
                "Scope {scope} is not supported by this Google service"
            )));
        }
        self.validate_required_scopes(scopes)
    }

    pub fn spec_key(self) -> &'static str {
        match self {
            Self::Workspace => "google-workspace",
            Self::Calendar => "google-calendar",
            Self::Drive => "google-drive",
            Self::Gmail => "google-gmail",
        }
    }

    /// The operation catalog also defines the proxy boundary, including for
    /// tokens whose Google grant contains permissions from another product.
    pub fn operation_policy(self) -> AppResult<ProxyOperationPolicy> {
        let spec = super::catalog_spec_registry::spec_for_key(self.spec_key())
            .ok_or_else(|| AppError::Internal("Missing Google product spec".into()))?;
        let paths = spec["paths"]
            .as_object()
            .ok_or_else(|| AppError::Internal("Missing Google product paths".into()))?;
        let rules = paths
            .iter()
            .flat_map(|(path, item)| {
                ["get", "post", "put", "patch", "delete"]
                    .into_iter()
                    .filter(move |method| item.get(*method).is_some())
                    .map(move |method| ProxyOperationRule {
                        method: method.to_ascii_uppercase(),
                        path_template: path.clone(),
                    })
            })
            .collect();
        super::proxy_authorization::normalize_policy(ProxyOperationPolicy { rules })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::proxy_authorization::{CanonicalPath, authorize_proxy_operation_fields};

    #[test]
    fn google_product_scopes_exclude_other_apis() {
        for product in [
            GoogleProduct::Workspace,
            GoogleProduct::Calendar,
            GoogleProduct::Drive,
            GoogleProduct::Gmail,
        ] {
            product
                .validate_scopes(Some(&product.default_scopes().join(" ")))
                .unwrap();
            for scope in [
                "https://www.googleapis.com/auth/gmail.modify",
                "https://www.googleapis.com/auth/gmail.compose",
                "https://mail.google.com/",
                "https://www.googleapis.com/auth/cloud-platform",
            ] {
                assert!(product.validate_scopes(Some(scope)).is_err());
            }
            for (scope, allowed) in [
                (
                    DRIVE,
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Drive),
                ),
                (
                    CALENDAR,
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Calendar),
                ),
                (
                    GMAIL_READONLY,
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Gmail),
                ),
                (
                    GMAIL_SEND,
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Gmail),
                ),
            ] {
                assert_eq!(
                    product
                        .validate_scopes(Some(&format!(
                            "{} {scope}",
                            product.required_scopes().join(" ")
                        )))
                        .is_ok(),
                    allowed,
                    "{product:?} {scope}"
                );
            }
            assert_eq!(
                product.default_scopes().contains(&GMAIL_SEND.to_string()),
                matches!(product, GoogleProduct::Workspace | GoogleProduct::Gmail)
            );
        }
    }

    #[test]
    fn gmail_send_is_required_for_workspace_and_gmail_authorization() {
        for product in [GoogleProduct::Workspace, GoogleProduct::Gmail] {
            for scopes in [
                None,
                Some(""),
                Some(GMAIL_READONLY),
                Some("openid email profile"),
            ] {
                assert!(product.validate_scopes(scopes).is_err());
                assert!(product.validate_required_scopes(scopes).is_err());
            }
            product.validate_scopes(Some(GMAIL_SEND)).unwrap();
            // Google may return broader grants from the shared project.
            product
                .validate_required_scopes(Some(&format!("{GMAIL_SEND} {DRIVE}")))
                .unwrap();
        }
        for product in [GoogleProduct::Drive, GoogleProduct::Calendar] {
            product.validate_required_scopes(None).unwrap();
        }
    }

    #[test]
    fn google_product_policies_enforce_both_proxy_path_forms() {
        for product in [
            GoogleProduct::Workspace,
            GoogleProduct::Calendar,
            GoogleProduct::Drive,
            GoogleProduct::Gmail,
        ] {
            let policy = product.operation_policy().unwrap();
            for (method, path, allowed) in [
                (
                    "POST",
                    "/calendar/v3/calendars",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Calendar),
                ),
                (
                    "PATCH",
                    "/calendar/v3/calendars/user@example.com/events/event1",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Calendar),
                ),
                (
                    "POST",
                    "/calendar/v3/freeBusy",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Calendar),
                ),
                (
                    "POST",
                    "/drive/v3/files",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Drive),
                ),
                (
                    "PATCH",
                    "/upload/drive/v3/files/file1",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Drive),
                ),
                (
                    "GET",
                    "/drive/v3/files/file1/export",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Drive),
                ),
                (
                    "DELETE",
                    "/drive/v3/files/file1",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Drive),
                ),
                (
                    "GET",
                    "/gmail/v1/users/me/messages",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Gmail),
                ),
                (
                    "GET",
                    "/gmail/v1/users/me/messages/message1",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Gmail),
                ),
                (
                    "POST",
                    "/gmail/v1/users/me/messages/send",
                    matches!(product, GoogleProduct::Workspace | GoogleProduct::Gmail),
                ),
                ("DELETE", "/gmail/v1/users/me/messages/message1", false),
                ("POST", "/gmail/v1/users/me/messages/message1/trash", false),
                ("POST", "/gmail/v1/users/me/messages/message1/modify", false),
                ("POST", "/gmail/v1/users/me/messages/batchDelete", false),
                ("POST", "/gmail/v1/users/me/messages/batchModify", false),
                ("DELETE", "/gmail/v1/users/me/threads/thread1", false),
                ("POST", "/gmail/v1/users/me/threads/thread1/trash", false),
                ("POST", "/gmail/v1/users/me/threads/thread1/modify", false),
                ("POST", "/gmail/v1/users/me/drafts", false),
                ("DELETE", "/gmail/v1/users/me/drafts/draft1", false),
                ("POST", "/gmail/v1/users/me/labels", false),
                ("GET", "/gmail/v1/users/other@example.com/messages", false),
                ("POST", "/batch/gmail/v1", false),
                ("POST", "/batch", false),
                ("POST", "/batch/drive/v3", false),
                ("POST", "/calendar/v3/calendars/primary/acl", false),
            ] {
                for path in [
                    CanonicalPath::from_rest_decoded(path).unwrap(),
                    CanonicalPath::from_mcp_literal(path).unwrap(),
                ] {
                    assert_eq!(
                        authorize_proxy_operation_fields(
                            "google",
                            "google",
                            Some(&policy),
                            method,
                            &path
                        )
                        .is_ok(),
                        allowed,
                        "{product:?} {method} {path:?}"
                    );
                }
            }
        }
    }
}
