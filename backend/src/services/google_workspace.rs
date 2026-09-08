//! Product presets sharing the existing Google OAuth client.

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{ProxyOperationPolicy, ProxyOperationRule};

pub const DRIVE: &str = "https://www.googleapis.com/auth/drive";
pub const CALENDAR: &str = "https://www.googleapis.com/auth/calendar";
pub const MANAGED_SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    DRIVE,
    "https://www.googleapis.com/auth/drive.readonly",
    "https://www.googleapis.com/auth/drive.file",
    CALENDAR,
    "https://www.googleapis.com/auth/calendar.readonly",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoogleProduct {
    Workspace,
    Calendar,
    Drive,
}

impl GoogleProduct {
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "api-google-workspace" => Some(Self::Workspace),
            "api-google-calendar" => Some(Self::Calendar),
            "api-google-drive" => Some(Self::Drive),
            _ => None,
        }
    }

    pub fn default_scopes(self) -> Vec<String> {
        let mut scopes = vec!["openid", "email", "profile"];
        if self != Self::Calendar {
            scopes.push(DRIVE);
        }
        if self != Self::Drive {
            scopes.push(CALENDAR);
        }
        scopes.into_iter().map(String::from).collect()
    }

    pub fn allowed_scopes(self) -> Vec<String> {
        MANAGED_SCOPES
            .iter()
            .filter(|scope| match self {
                Self::Workspace => true,
                Self::Calendar => !scope.starts_with(DRIVE),
                Self::Drive => !scope.starts_with(CALENDAR),
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
        Ok(())
    }

    pub fn spec_key(self) -> &'static str {
        match self {
            Self::Workspace => "google-workspace",
            Self::Calendar => "google-calendar",
            Self::Drive => "google-drive",
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
        ] {
            product
                .validate_scopes(Some(&product.default_scopes().join(" ")))
                .unwrap();
            assert!(
                product
                    .validate_scopes(Some("https://www.googleapis.com/auth/gmail.modify"))
                    .is_err()
            );
            assert!(
                product
                    .validate_scopes(Some("https://www.googleapis.com/auth/cloud-platform"))
                    .is_err()
            );
        }
        assert!(
            GoogleProduct::Calendar
                .validate_scopes(Some(DRIVE))
                .is_err()
        );
        assert!(
            GoogleProduct::Drive
                .validate_scopes(Some(CALENDAR))
                .is_err()
        );
    }

    #[test]
    fn google_product_policies_enforce_both_proxy_path_forms() {
        for product in [
            GoogleProduct::Workspace,
            GoogleProduct::Calendar,
            GoogleProduct::Drive,
        ] {
            let policy = product.operation_policy().unwrap();
            for (method, path, allowed) in [
                (
                    "POST",
                    "/calendar/v3/calendars",
                    product != GoogleProduct::Drive,
                ),
                (
                    "PATCH",
                    "/calendar/v3/calendars/user@example.com/events/event1",
                    product != GoogleProduct::Drive,
                ),
                (
                    "POST",
                    "/calendar/v3/freeBusy",
                    product != GoogleProduct::Drive,
                ),
                (
                    "POST",
                    "/drive/v3/files",
                    product != GoogleProduct::Calendar,
                ),
                (
                    "PATCH",
                    "/upload/drive/v3/files/file1",
                    product != GoogleProduct::Calendar,
                ),
                (
                    "GET",
                    "/drive/v3/files/file1/export",
                    product != GoogleProduct::Calendar,
                ),
                (
                    "DELETE",
                    "/drive/v3/files/file1",
                    product != GoogleProduct::Calendar,
                ),
                ("GET", "/gmail/v1/users/me/messages", false),
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
