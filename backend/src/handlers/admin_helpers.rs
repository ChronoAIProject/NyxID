use axum::http::{HeaderMap, header};
use mongodb::bson::doc;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, role_service};

/// Check that the authenticated user has admin (write) privileges.
///
/// Admin access is determined by platform RBAC role membership. This is the
/// canonical admin check for write paths. For read-only admin access, use
/// [`require_admin_or_operator`].
pub async fn require_admin(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    let user_id = auth_user.user_id.to_string();
    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let platform_role = role_service::resolve_platform_role(&state.db, &user_model).await?;
    if !platform_role.is_admin() {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    Ok(())
}

/// Check that the authenticated user has at least read-only platform admin
/// access — either `admin` (full admin) or `operator` (read-only) platform
/// RBAC role membership.
///
/// Use this on admin GET handlers that should be accessible to operator-role
/// users (strategy / share-ops accounts that need cross-org platform data
/// without write privileges). Write handlers must keep using
/// [`require_admin`].
///
/// `endpoint_marker` is a short, stable identifier for the calling handler
/// (e.g. `"admin.users.list"`, `"admin.invite_codes.list"`). It is written
/// to the audit entry's `event_data` so the audit trail can answer
/// "operator X read endpoint Y at time T" — necessary because issue #715
/// requires that operator reads are auditable, and HTTP access logs are
/// not always retained alongside the audit log. Use a dot-namespaced
/// `admin.<resource>.<action>` form so log queries can filter by prefix.
///
/// Operator (non-admin) reads are audited via a fire-and-forget
/// `admin.read.by_operator` entry. Full admins are not audited here —
/// the existing per-handler write-audit entries already cover their
/// activity, and adding one extra row per admin GET would balloon the
/// audit volume without proportional value.
pub async fn require_admin_or_operator(
    state: &AppState,
    auth_user: &AuthUser,
    endpoint_marker: &'static str,
) -> AppResult<()> {
    let user_id = auth_user.user_id.to_string();
    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let platform_role = role_service::resolve_platform_role(&state.db, &user_model).await?;
    if !platform_role.has_admin_read() {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    if matches!(platform_role, crate::models::user::PlatformRole::Operator) {
        audit_service::log_for_user(
            state.db.clone(),
            auth_user,
            "admin.read.by_operator",
            Some(serde_json::json!({ "endpoint": endpoint_marker })),
        );
    }

    Ok(())
}

pub fn extract_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

/// Validate that a slug matches the required format: lowercase alphanumeric,
/// hyphens, and underscores only.
pub fn validate_slug(slug: &str) -> AppResult<()> {
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(AppError::ValidationError(
            "Slug must contain only lowercase alphanumeric characters, hyphens, or underscores"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserType;
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use uuid::Uuid;

    async fn insert_user(db: &mongodb::Database, is_admin: bool, is_operator: bool) -> String {
        role_service::seed_system_roles(db)
            .await
            .expect("seed platform roles");
        let platform_role_ids = role_service::get_platform_role_ids(db)
            .await
            .expect("platform role ids");
        let id = Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        if is_admin {
            user.role_ids.push(platform_role_ids.admin);
        } else if is_operator {
            user.role_ids.push(platform_role_ids.operator);
        }
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert test user");
        id
    }

    #[tokio::test]
    async fn require_admin_rejects_plain_user() {
        let Some(db) = connect_test_database("admin_helpers_user").await else {
            eprintln!("skipping require_admin test: no local MongoDB available");
            return;
        };
        let user_id = insert_user(&db, false, false).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = require_admin(&state, &auth)
            .await
            .expect_err("plain user should be rejected");
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn require_admin_rejects_operator() {
        let Some(db) = connect_test_database("admin_helpers_operator_write").await else {
            eprintln!("skipping require_admin operator test: no local MongoDB available");
            return;
        };
        let user_id = insert_user(&db, false, true).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        // Operator is read-only — must not be allowed through the write helper.
        let err = require_admin(&state, &auth)
            .await
            .expect_err("operator should be rejected by require_admin (write)");
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn require_admin_accepts_admin() {
        let Some(db) = connect_test_database("admin_helpers_admin").await else {
            eprintln!("skipping require_admin admin test: no local MongoDB available");
            return;
        };
        let user_id = insert_user(&db, true, false).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        require_admin(&state, &auth)
            .await
            .expect("admin should pass require_admin");
    }

    #[tokio::test]
    async fn require_admin_or_operator_accepts_operator() {
        let Some(db) = connect_test_database("admin_helpers_operator_read").await else {
            eprintln!("skipping require_admin_or_operator test: no local MongoDB available");
            return;
        };
        let user_id = insert_user(&db, false, true).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        require_admin_or_operator(&state, &auth, "test.helper")
            .await
            .expect("operator should pass require_admin_or_operator");
    }

    #[tokio::test]
    async fn require_admin_or_operator_accepts_admin() {
        let Some(db) = connect_test_database("admin_helpers_or_operator_admin").await else {
            eprintln!("skipping require_admin_or_operator admin test: no local MongoDB available");
            return;
        };
        let user_id = insert_user(&db, true, false).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        require_admin_or_operator(&state, &auth, "test.helper")
            .await
            .expect("admin should pass require_admin_or_operator");
    }

    #[tokio::test]
    async fn require_admin_or_operator_rejects_plain_user() {
        let Some(db) = connect_test_database("admin_helpers_or_operator_user").await else {
            eprintln!("skipping require_admin_or_operator user test: no local MongoDB available");
            return;
        };
        let user_id = insert_user(&db, false, false).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = require_admin_or_operator(&state, &auth, "test.helper")
            .await
            .expect_err("plain user should be rejected");
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn operator_read_writes_audit_entry() {
        use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
        use std::time::Duration;

        let Some(db) = connect_test_database("admin_helpers_operator_audit").await else {
            eprintln!("skipping operator-audit test: no local MongoDB available");
            return;
        };
        let user_id = insert_user(&db, false, true).await;
        let state = test_app_state(db.clone());
        let auth = test_auth_user(&user_id);

        require_admin_or_operator(&state, &auth, "test.endpoint.marker")
            .await
            .expect("operator read should succeed");

        // The audit write is fire-and-forget on a tokio::spawn; give it a
        // moment to land before asserting. 250ms is generous in practice.
        tokio::time::sleep(Duration::from_millis(250)).await;

        let entry = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! {
                "user_id": &user_id,
                "event_type": "admin.read.by_operator",
            })
            .await
            .expect("query audit log")
            .expect("operator read should leave an admin.read.by_operator audit entry");

        // The audit row must carry the endpoint marker so the audit trail
        // can answer "operator X read endpoint Y at time T". Without this,
        // operator reads of /admin/users vs /admin/audit-log are
        // indistinguishable.
        let endpoint = entry
            .event_data
            .as_ref()
            .and_then(|v| v.get("endpoint"))
            .and_then(|v| v.as_str());
        assert_eq!(
            endpoint,
            Some("test.endpoint.marker"),
            "audit entry must record the calling endpoint marker, got {endpoint:?}"
        );
    }

    // ── Pure function tests ──

    #[test]
    fn extract_ip_from_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
        assert_eq!(extract_ip(&headers), Some("1.2.3.4".to_string()));
    }

    #[test]
    fn extract_ip_single_value() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "10.0.0.1".parse().unwrap());
        assert_eq!(extract_ip(&headers), Some("10.0.0.1".to_string()));
    }

    #[test]
    fn extract_ip_missing_header() {
        let headers = HeaderMap::new();
        assert!(extract_ip(&headers).is_none());
    }

    #[test]
    fn extract_ip_empty_value() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "".parse().unwrap());
        assert!(extract_ip(&headers).is_none());
    }

    #[test]
    fn extract_ip_trims_whitespace() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "  9.8.7.6  , 1.2.3.4".parse().unwrap());
        assert_eq!(extract_ip(&headers), Some("9.8.7.6".to_string()));
    }

    #[test]
    fn extract_user_agent_present() {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, "Mozilla/5.0".parse().unwrap());
        assert_eq!(
            extract_user_agent(&headers),
            Some("Mozilla/5.0".to_string())
        );
    }

    #[test]
    fn extract_user_agent_missing() {
        let headers = HeaderMap::new();
        assert!(extract_user_agent(&headers).is_none());
    }

    #[test]
    fn validate_slug_valid() {
        assert!(validate_slug("my-service").is_ok());
        assert!(validate_slug("my_service_2").is_ok());
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("123").is_ok());
    }

    #[test]
    fn validate_slug_rejects_empty() {
        assert!(validate_slug("").is_err());
    }

    #[test]
    fn validate_slug_rejects_uppercase() {
        assert!(validate_slug("MyService").is_err());
    }

    #[test]
    fn validate_slug_rejects_spaces() {
        assert!(validate_slug("my service").is_err());
    }

    #[test]
    fn validate_slug_rejects_special_chars() {
        assert!(validate_slug("my.service").is_err());
        assert!(validate_slug("my/service").is_err());
        assert!(validate_slug("my@service").is_err());
    }

    #[tokio::test]
    async fn admin_read_does_not_write_operator_audit_entry() {
        use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
        use std::time::Duration;

        let Some(db) = connect_test_database("admin_helpers_admin_no_audit").await else {
            eprintln!("skipping admin-no-audit test: no local MongoDB available");
            return;
        };
        let user_id = insert_user(&db, true, false).await;
        let state = test_app_state(db.clone());
        let auth = test_auth_user(&user_id);

        require_admin_or_operator(&state, &auth, "test.helper")
            .await
            .expect("admin read should succeed");
        tokio::time::sleep(Duration::from_millis(250)).await;

        let entry = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! {
                "user_id": &user_id,
                "event_type": "admin.read.by_operator",
            })
            .await
            .expect("query audit log");
        assert!(
            entry.is_none(),
            "admin reads must not be tagged as operator reads"
        );
    }
}
