//! Metadata-only projections of current scope grants and exact recorded requests.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use bson::{Document, doc};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::Database;
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    errors::{AppError, AppResult},
    models::{
        agent_service_binding::AgentServiceBinding,
        api_key::{ApiKey, ApiKeyPurpose},
        audit_log::AuditLog,
        user_service::UserService,
    },
    mw::auth::{AuthMethod, AuthUser},
    services::{
        audit_service, key_service,
        org_service::{self, OwnerAccess},
    },
};

const KEY_LIMIT: usize = 500;
const REQUEST_LIMIT: i64 = 3;

#[derive(Clone, Serialize, ToSchema)]
pub struct ConnectionActivity {
    pub access: AccessSummary,
    pub activity: ActivitySummary,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct AccessSummary {
    /// Counts cover the visible key inventory, never other members' personal keys.
    pub visibility: String,
    pub keys: Vec<AccessKey>,
    pub total: usize,
    pub truncated: bool,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct AccessKey {
    pub id: String,
    pub name: String,
    pub platform: Option<String>,
    pub owner_id: String,
    pub permission: String,
    pub credential_override: bool,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct ActivitySummary {
    pub visibility: String,
    pub period_days: u32,
    /// Historical events and uninstrumented ingress cannot prove a complete period.
    pub tracking: String,
    pub request_count: u64,
    pub requests: Vec<RecentRequest>,
    pub truncated: bool,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct RecentRequest {
    pub id: String,
    pub execution_id: Option<String>,
    pub caller: RequestCaller,
    pub occurred_at: DateTime<Utc>,
    pub outcome: String,
    pub response_status: Option<u16>,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct RequestCaller {
    pub id: Option<String>,
    pub kind: String,
    pub name: String,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
}

fn can_execute(access: &OwnerAccess, service: &UserService) -> bool {
    access.allows_resource(&service.id)
        && match access {
            OwnerAccess::Direct | OwnerAccess::AsOrgAdmin { .. } => true,
            OwnerAccess::AsOrgMember { role, .. } => {
                super::user_service_service::role_can_proxy_service(*role, service)
            }
            OwnerAccess::Forbidden => false,
        }
}

fn permission(key: &ApiKey, effective_ids: &[String], service_id: &str) -> Option<&'static str> {
    if key.purpose != ApiKeyPurpose::General
        || !key.is_active
        || key.expires_at.is_some_and(|expiry| expiry <= Utc::now())
        || !crate::mw::auth::scope_allows_llm_proxy(&key.scopes)
    {
        return None;
    }
    if key.allow_all_services {
        Some("all_services")
    } else if key.allowed_service_ids.iter().any(|id| id == service_id) {
        Some("selected_service")
    } else if effective_ids.iter().any(|id| id == service_id) {
        Some("platform_services")
    } else {
        None
    }
}

/// The handler must apply its token's service allowlist before passing rows here.
/// This layer independently checks owner/member visibility and scopes key inventory.
pub async fn insights(
    db: &Database,
    actor_id: &str,
    services: &[UserService],
) -> AppResult<HashMap<String, ConnectionActivity>> {
    if services.len() > 100 {
        return Err(AppError::ValidationError(
            "At most 100 connections may be requested".into(),
        ));
    }
    let mut owner_access = HashMap::new();
    for service in services {
        if !owner_access.contains_key(&service.user_id) {
            owner_access.insert(
                service.user_id.clone(),
                org_service::resolve_owner_access(db, actor_id, &service.user_id).await?,
            );
        }
    }
    let visible: Vec<_> = services
        .iter()
        .filter(|service| {
            owner_access
                .get(&service.user_id)
                .is_some_and(|access| access.can_read() && access.allows_resource(&service.id))
        })
        .collect();
    if visible.is_empty() {
        return Ok(HashMap::new());
    }
    let mut key_owners = HashSet::from([actor_id.to_owned()]);
    for service in &visible {
        if owner_access[&service.user_id].can_write() {
            key_owners.insert(service.user_id.clone());
        }
    }
    let mut keys: Vec<ApiKey> = db.collection("api_keys")
        .find(doc! { "user_id": { "$in": key_owners.into_iter().collect::<Vec<_>>() }, "is_active": true, "$or": [{ "expires_at": null }, { "expires_at": { "$gt": bson::DateTime::now() } }] })
        .sort(doc! { "name": 1, "_id": 1 })
        .limit((KEY_LIMIT + 1) as i64)
        .max_time(Duration::from_secs(3))
        .await?.try_collect().await?;
    let keys_truncated = keys.len() > KEY_LIMIT;
    keys.truncate(KEY_LIMIT);
    let ids: Vec<_> = visible.iter().map(|s| s.id.as_str()).collect();
    let key_ids: Vec<_> = keys.iter().map(|k| k.id.as_str()).collect();
    let bindings: Vec<AgentServiceBinding> = db
        .collection("agent_service_bindings")
        .find(doc! { "api_key_id": { "$in": key_ids }, "user_service_id": { "$in": &ids } })
        .max_time(Duration::from_secs(3))
        .await?
        .try_collect()
        .await?;
    let overrides: HashSet<_> = bindings
        .iter()
        .map(|b| {
            (
                b.api_key_id.as_str(),
                b.user_service_id.as_str(),
                b.user_id.as_str(),
            )
        })
        .collect();
    // Reuse the runtime expansion, once per owner, for dynamic platform grants.
    let mut platform_grants: HashMap<String, Vec<String>> = HashMap::new();
    for key in &keys {
        if key.allow_auto_connected_services
            && !key.allow_all_services
            && !platform_grants.contains_key(&key.user_id)
        {
            let mut grant_key = key.clone();
            grant_key.allowed_service_ids.clear();
            platform_grants.insert(
                key.user_id.clone(),
                key_service::effective_allowed_service_ids(db, &grant_key).await?,
            );
        }
    }
    let mut result = HashMap::new();
    let mut managed_ids = Vec::new();
    let mut own_ids = Vec::new();
    for service in &visible {
        let access = &owner_access[&service.user_id];
        let managed = access.can_write();
        if managed {
            managed_ids.push(service.id.as_str());
        } else {
            own_ids.push(service.id.as_str());
        }
        let mut allowed = Vec::new();
        for key in &keys {
            let key_access = if key.user_id == service.user_id {
                Some(&OwnerAccess::Direct)
            } else if key.user_id == actor_id {
                Some(access)
            } else {
                None
            };
            if !key_access.is_some_and(|a| can_execute(a, service)) {
                continue;
            }
            let effective = platform_grants
                .get(&key.user_id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let effective = if key.allow_auto_connected_services {
                effective
            } else {
                &[]
            };
            if let Some(reason) = permission(key, effective, &service.id) {
                allowed.push(AccessKey {
                    id: key.id.clone(),
                    name: key.name.clone(),
                    platform: key.platform.clone(),
                    owner_id: key.user_id.clone(),
                    permission: reason.into(),
                    credential_override: key.user_id == service.user_id
                        && overrides.contains(&(
                            key.id.as_str(),
                            service.id.as_str(),
                            service.user_id.as_str(),
                        )),
                });
            }
        }
        result.insert(
            service.id.clone(),
            ConnectionActivity {
                access: AccessSummary {
                    visibility: if managed { "managed_keys" } else { "own_keys" }.into(),
                    total: allowed.len(),
                    keys: allowed,
                    truncated: keys_truncated,
                },
                activity: ActivitySummary {
                    visibility: if managed {
                        "all_requests"
                    } else {
                        "own_requests"
                    }
                    .into(),
                    period_days: 30,
                    tracking: "partial".into(),
                    request_count: 0,
                    requests: Vec::new(),
                    truncated: false,
                },
            },
        );
    }
    // Apply privacy before aggregation: service visibility does not reveal other
    // org members' callers. No legacy catalog-only event is allocated to a row.
    let filter = request_filter(actor_id, &managed_ids, &own_ids);
    let groups: Vec<Document> = db.collection::<Document>("audit_log").aggregate(vec![
        doc! { "$match": filter },
        doc! { "$sort": { "created_at": -1, "_id": -1 } },
        doc! { "$group": { "_id": "$event_data.user_service_id", "count": { "$sum": 1 }, "requests": { "$firstN": { "input": "$$ROOT", "n": REQUEST_LIMIT } } } },
    ]).max_time(Duration::from_secs(3)).await?.try_collect().await?;
    let mut events = Vec::new();
    for group in groups {
        let Some(summary) = group.get_str("_id").ok().and_then(|id| result.get_mut(id)) else {
            continue;
        };
        summary.activity.request_count = group
            .get_i64("count")
            .or_else(|_| group.get_i32("count").map(i64::from))
            .unwrap_or_default()
            .max(0) as u64;
        if let Ok(rows) = group.get_array("requests") {
            for row in rows {
                if let Ok(event) = bson::from_bson::<AuditLog>(row.clone()) {
                    events.push(event);
                }
            }
        }
        summary.activity.truncated = summary.activity.request_count > REQUEST_LIMIT as u64;
    }
    let mut app_ids = HashSet::new();
    let mut subject_ids = HashSet::new();
    for event in &events {
        if let Some(app) = event_string(event, "oauth_client_id")
            .or_else(|| event_string(event, "acting_client_id"))
        {
            app_ids.insert(app);
        }
        if let Some(subject) = &event.user_id {
            subject_ids.insert(subject.as_str());
        }
    }
    let apps = names(
        db,
        "oauth_clients",
        "client_name",
        app_ids.into_iter().collect(),
    )
    .await?;
    let subjects: Vec<_> = subject_ids.into_iter().collect();
    let users = names(db, "users", "display_name", subjects.clone()).await?;
    let accounts = names(db, "service_accounts", "name", subjects).await?;
    for event in events {
        let Some(summary) =
            event_string(&event, "user_service_id").and_then(|id| result.get_mut(id))
        else {
            continue;
        };
        summary
            .activity
            .requests
            .push(recent_request(&event, actor_id, &apps, &users, &accounts));
    }
    Ok(result)
}

fn request_filter(actor_id: &str, managed: &[&str], own: &[&str]) -> Document {
    doc! {
        "event_type": "service_request",
        "created_at": { "$gte": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::days(30)) },
        "$or": [
            { "event_data.user_service_id": { "$in": managed } },
            { "event_data.user_service_id": { "$in": own }, "user_id": actor_id },
        ],
    }
}

async fn names(
    db: &Database,
    collection: &str,
    field: &str,
    ids: Vec<&str>,
) -> AppResult<HashMap<String, String>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<Document> = db
        .collection::<Document>(collection)
        .find(doc! { "_id": { "$in": ids } })
        .projection(doc! { "_id": 1, field: 1 })
        .max_time(Duration::from_secs(3))
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some((
                row.get_str("_id").ok()?.into(),
                row.get_str(field).ok()?.into(),
            ))
        })
        .collect())
}

fn event_string<'a>(event: &'a AuditLog, key: &str) -> Option<&'a str> {
    event.event_data.as_ref()?.get(key)?.as_str()
}

fn recent_request(
    event: &AuditLog,
    viewer: &str,
    apps: &HashMap<String, String>,
    users: &HashMap<String, String>,
    accounts: &HashMap<String, String>,
) -> RecentRequest {
    let app_id = event_string(event, "oauth_client_id")
        .or_else(|| event_string(event, "acting_client_id"))
        .map(str::to_owned);
    let app_name = app_id.as_ref().and_then(|id| apps.get(id)).cloned();
    let subject = event.user_id.clone();
    let (kind, id, name) = if let Some(key_id) = &event.api_key_id {
        (
            "agent_key",
            Some(key_id.clone()),
            event
                .api_key_name
                .clone()
                .unwrap_or_else(|| "Agent key".into()),
        )
    } else if event_string(event, "auth_kind") == Some("service_account") {
        (
            "service_account",
            subject.clone(),
            subject
                .as_ref()
                .and_then(|id| accounts.get(id))
                .cloned()
                .unwrap_or_else(|| "Service account".into()),
        )
    } else if app_id.is_some() {
        (
            "oauth_app",
            app_id.clone(),
            app_name.clone().unwrap_or_else(|| "OAuth app".into()),
        )
    } else if let Some(kind @ ("session" | "access_token" | "relay" | "delegated")) =
        event_string(event, "auth_kind")
    {
        (
            kind,
            subject.clone(),
            if subject.as_deref() == Some(viewer) {
                "You".into()
            } else {
                subject
                    .as_ref()
                    .and_then(|id| users.get(id))
                    .cloned()
                    .unwrap_or_else(|| {
                        if kind == "session" {
                            "User session"
                        } else {
                            "Authenticated user"
                        }
                        .into()
                    })
            },
        )
    } else {
        ("unknown", subject, "Client not recorded".into())
    };
    RecentRequest {
        id: event.id.clone(),
        execution_id: event_string(event, "execution_id").map(str::to_owned),
        caller: RequestCaller {
            id,
            kind: kind.into(),
            name,
            app_id,
            app_name,
        },
        occurred_at: event.created_at,
        outcome: event_string(event, "outcome").unwrap_or("unknown").into(),
        response_status: event
            .event_data
            .as_ref()
            .and_then(|v| v.get("response_status"))
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok()),
    }
}

/// One exact connection event per resolved execution, including early returns.
/// A response header is not stream completion, so successful HTTP responses keep
/// the explicit `response_received` state and WS upgrades use `connection_opened`.
pub struct RequestAudit {
    db: Database,
    actor: audit_service::AuditActor,
    event: serde_json::Value,
}

#[derive(Clone)]
pub struct RequestAttribution {
    pub actor: audit_service::AuditActor,
    pub auth_kind: String,
    pub oauth_client_id: Option<String>,
    pub acting_client_id: Option<String>,
    pub api_key_credential_id: Option<String>,
}

fn admission_denial_status(error: &AppError) -> Option<u16> {
    match error {
        AppError::InsufficientCredits
        | AppError::WalletSuspended
        | AppError::PlanEntitlementRequired(_) => Some(402),
        AppError::Forbidden(_)
        | AppError::ApiKeyScopeForbidden(_)
        | AppError::OrgRoleInsufficient(_) => Some(403),
        _ => None,
    }
}

impl RequestAudit {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: &Database,
        auth: &AuthUser,
        service_id: Option<&str>,
        catalog_id: &str,
        owner_id: &str,
        execution_id: &str,
        credential_class: crate::models::usage_meter::CredentialClass,
    ) -> Self {
        Self::from_attribution(
            db,
            RequestAttribution {
                actor: audit_service::AuditActor::from_auth_user(auth),
                auth_kind: match auth.auth_method {
                    AuthMethod::Session => "session",
                    AuthMethod::AccessToken => "access_token",
                    AuthMethod::ApiKey => "api_key",
                    AuthMethod::ServiceAccount => "service_account",
                    AuthMethod::Delegated => "delegated",
                    AuthMethod::Relay => "relay",
                }
                .into(),
                oauth_client_id: auth.oauth_client_id.clone(),
                acting_client_id: auth.acting_client_id.clone(),
                api_key_credential_id: auth.api_key_credential_id.clone(),
            },
            service_id,
            catalog_id,
            owner_id,
            execution_id,
            credential_class,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_attribution(
        db: &Database,
        attribution: RequestAttribution,
        service_id: Option<&str>,
        catalog_id: &str,
        owner_id: &str,
        execution_id: &str,
        credential_class: crate::models::usage_meter::CredentialClass,
    ) -> Self {
        Self {
            db: db.clone(),
            actor: attribution.actor,
            event: serde_json::json!({
                "user_service_id": service_id, "service_id": catalog_id, "owner_user_id": owner_id,
                "execution_id": execution_id, "billing_request_id": execution_id,
                "credential_class": credential_class, "oauth_client_id": attribution.oauth_client_id,
                "acting_client_id": attribution.acting_client_id, "api_key_credential_id": attribution.api_key_credential_id,
                "auth_kind": attribution.auth_kind,
                "outcome": "unknown",
            }),
        }
    }

    pub fn response(&mut self, status: u16) {
        self.event["response_status"] = status.into();
        self.event["outcome"] = match status {
            101 => "connection_opened",
            400..=599 => "failed",
            _ => "response_received",
        }
        .into();
    }

    pub fn denied(&mut self, status: u16) {
        self.event["response_status"] = status.into();
        self.event["outcome"] = "denied".into();
        self.event["dispatch_state"] = "not_dispatched".into();
    }

    /// Call only at admission gates before dispatch. Transient database/billing
    /// failures remain unknown and must not be turned into provider failures.
    pub fn admission_error(&mut self, error: &AppError) {
        if let Some(status) = admission_denial_status(error) {
            self.denied(status);
        }
    }
}

impl Drop for RequestAudit {
    fn drop(&mut self) {
        if self.event["user_service_id"].is_null() {
            return;
        }
        audit_service::log_async(
            self.db.clone(),
            Some(self.actor.user_id.clone()),
            "service_request".into(),
            Some(self.event.clone()),
            self.actor.ip_address.clone(),
            self.actor.user_agent.clone(),
            self.actor.api_key_id.clone(),
            self.actor.api_key_name.clone(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ApiKey {
        bson::from_document(doc! {
            "_id": "key", "user_id": "owner", "name": "CI agent", "key_prefix": "prefix",
            "key_hash": "hash", "scopes": "proxy", "is_active": true,
            "created_at": bson::DateTime::now(), "allow_all_services": false,
            "allowed_service_ids": ["selected"], "allow_auto_connected_services": true,
        })
        .unwrap()
    }

    fn event() -> AuditLog {
        bson::from_document(doc! {
            "_id": "event", "user_id": "owner", "event_type": "service_request",
            "event_data": { "user_service_id": "connection", "execution_id": "execution", "auth_kind": "session", "outcome": "response_received", "response_status": 200 },
            "created_at": bson::DateTime::now(),
        }).unwrap()
    }

    #[test]
    fn scope_grants_distinguish_explicit_dynamic_and_broad_permission() {
        let mut key = key();
        let effective = vec!["dynamic".into()];
        assert_eq!(
            permission(&key, &effective, "selected"),
            Some("selected_service")
        );
        assert_eq!(
            permission(&key, &effective, "dynamic"),
            Some("platform_services")
        );
        assert_eq!(permission(&key, &effective, "other"), None);
        key.allow_all_services = true;
        assert_eq!(permission(&key, &effective, "other"), Some("all_services"));
        key.is_active = false;
        assert_eq!(permission(&key, &effective, "other"), None);
        key.is_active = true;
        key.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
        assert_eq!(permission(&key, &effective, "other"), None);
        key.expires_at = None;
        key.purpose = ApiKeyPurpose::ScheduledInvocation;
        assert_eq!(permission(&key, &effective, "other"), None);
        key.purpose = ApiKeyPurpose::General;
        key.scopes = "read write".into();
        assert_eq!(permission(&key, &effective, "other"), None);
    }

    #[test]
    fn member_activity_is_actor_scoped_before_counting() {
        let filter = request_filter("viewer", &["personal"], &["org"]);
        let branches = filter.get_array("$or").unwrap();
        assert!(!branches[0].as_document().unwrap().contains_key("user_id"));
        assert_eq!(
            branches[1]
                .as_document()
                .unwrap()
                .get_str("user_id")
                .unwrap(),
            "viewer"
        );
        assert_eq!(filter.get_str("event_type").unwrap(), "service_request");
        assert!(!filter.to_string().contains("service_slug"));
    }

    #[test]
    fn caller_identity_does_not_infer_apps_or_success_from_provisioning() {
        let mut event = event();
        let empty = HashMap::new();
        let own = recent_request(&event, "owner", &empty, &empty, &empty);
        assert_eq!(own.caller.kind, "session");
        assert_eq!(own.caller.name, "You");
        assert_eq!(own.outcome, "response_received");
        let other = recent_request(&event, "viewer", &empty, &empty, &empty);
        assert_eq!(other.caller.name, "User session");
        for kind in ["access_token", "relay", "delegated"] {
            event.event_data =
                Some(serde_json::json!({ "source_app_name": "Heca", "auth_kind": kind }));
            let own = recent_request(&event, "owner", &empty, &empty, &empty);
            assert_eq!(own.caller.kind, kind);
            assert_eq!(own.caller.name, "You");
            assert_eq!(own.caller.id.as_deref(), Some("owner"));
            assert!(own.caller.app_id.is_none());
            assert!(own.caller.app_name.is_none());
            assert_eq!(own.outcome, "unknown");
            let names = HashMap::from([("owner".into(), "Alicia".into())]);
            let shared = recent_request(&event, "viewer", &empty, &names, &empty);
            assert_eq!(shared.caller.kind, kind);
            assert_eq!(shared.caller.name, "Alicia");
        }
        event.event_data = Some(serde_json::json!({ "source_app_name": "Heca" }));
        let unknown = recent_request(&event, "owner", &empty, &empty, &empty);
        assert_eq!(unknown.caller.kind, "unknown");
        assert_eq!(unknown.caller.name, "Client not recorded");
        event.api_key_id = Some("agent".into());
        event.api_key_name = Some("Codex CI".into());
        event.event_data =
            Some(serde_json::json!({ "oauth_client_id": "app", "auth_kind": "relay" }));
        let apps = HashMap::from([("app".into(), "Release app".into())]);
        let agent = recent_request(&event, "owner", &apps, &empty, &empty);
        assert_eq!(agent.caller.kind, "agent_key");
        assert_eq!(agent.caller.name, "Codex CI");
        assert_eq!(agent.caller.app_name.as_deref(), Some("Release app"));
    }

    #[test]
    fn admission_failures_only_claim_denial_when_the_gate_confirms_it() {
        assert_eq!(
            admission_denial_status(&AppError::InsufficientCredits),
            Some(402)
        );
        assert_eq!(
            admission_denial_status(&AppError::WalletSuspended),
            Some(402)
        );
        assert_eq!(
            admission_denial_status(&AppError::Forbidden("policy".into())),
            Some(403)
        );
        assert_eq!(
            admission_denial_status(&AppError::BillingProviderUnavailable("temporary".into())),
            None
        );
        assert_eq!(
            admission_denial_status(&AppError::Internal("database".into())),
            None
        );
    }

    #[tokio::test]
    async fn exact_connection_activity_and_key_inventory_apply_independent_org_visibility() {
        use crate::{
            models::{org_membership::OrgRole, user::UserType},
            test_utils::{connect_test_database, test_membership, test_user, test_user_service},
        };
        let db = connect_test_database("service_insights_acl").await.unwrap();
        audit_service::init_audit_chain_hmac_key(zeroize::Zeroizing::new([7; 32]));
        db.collection("users")
            .insert_many([
                test_user("viewer", UserType::Person),
                test_user("other", UserType::Person),
                test_user("org", UserType::Org),
            ])
            .await
            .unwrap();
        let membership = test_membership("org", "viewer", OrgRole::Member, Some(vec!["a".into()]));
        db.collection::<crate::models::org_membership::OrgMembership>("org_memberships")
            .insert_one(&membership)
            .await
            .unwrap();
        let a = test_user_service("a", "org", "same-catalog-a", "ep", Some("catalog"), None);
        let b = test_user_service("b", "org", "same-catalog-b", "ep", Some("catalog"), None);
        let mut personal = key();
        personal.id = "personal-key".into();
        personal.user_id = "viewer".into();
        personal.allow_all_services = true;
        let mut org = personal.clone();
        org.id = "org-key".into();
        org.user_id = "org".into();
        let mut hidden = personal.clone();
        hidden.id = "other-private-key".into();
        hidden.user_id = "other".into();
        db.collection("api_keys")
            .insert_many([personal, org, hidden])
            .await
            .unwrap();
        db.collection::<Document>("agent_service_bindings").insert_one(doc! {
            "_id": "binding", "api_key_id": "org-key", "user_service_id": "a", "user_api_key_id": "external-secret",
            "user_id": "org", "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        }).await.unwrap();
        for (actor_id, connection, event_type) in [
            ("viewer", Some("a"), "service_request"),
            ("other", Some("a"), "service_request"),
            ("viewer", Some("b"), "service_request"),
            ("viewer", None, "proxy_request"),
        ] {
            audit_service::log_actor_event(db.clone(), &audit_service::AuditActor {
                user_id: actor_id.into(), ip_address: None, user_agent: None,
                api_key_id: None, api_key_name: None,
            }, event_type, Some(serde_json::json!({
                "user_service_id": connection, "service_id": "catalog", "auth_kind": "session", "outcome": "response_received",
            }))).await.unwrap();
        }
        let member = insights(&db, "viewer", &[a.clone(), b.clone()])
            .await
            .unwrap();
        assert!(!member.contains_key("b"));
        assert_eq!(member["a"].access.visibility, "own_keys");
        assert_eq!(
            member["a"]
                .access
                .keys
                .iter()
                .map(|k| k.id.as_str())
                .collect::<Vec<_>>(),
            ["personal-key"]
        );
        assert_eq!(member["a"].activity.visibility, "own_requests");
        assert_eq!(member["a"].activity.request_count, 1);
        assert_eq!(member["a"].activity.requests[0].caller.name, "You");
        assert_eq!(member["a"].activity.tracking, "partial");

        db.collection::<Document>("org_memberships")
            .update_one(
                doc! { "_id": &membership.id },
                doc! { "$set": { "role": "admin" } },
            )
            .await
            .unwrap();
        let admin = insights(&db, "viewer", &[a, b]).await.unwrap();
        assert!(!admin.contains_key("b"));
        assert_eq!(admin["a"].activity.request_count, 2);
        assert_eq!(admin["a"].access.total, 2);
        assert!(
            admin["a"]
                .access
                .keys
                .iter()
                .any(|k| k.id == "org-key" && k.credential_override)
        );
        assert!(
            !admin["a"]
                .access
                .keys
                .iter()
                .any(|k| k.id == "other-private-key")
        );
        assert!(
            admin["a"]
                .activity
                .requests
                .iter()
                .any(|r| r.caller.name == "Test User")
        );
        let serialized = serde_json::to_string(&admin).unwrap();
        for secret_field in ["external-secret", "key_hash", "key_prefix"] {
            assert!(!serialized.contains(secret_field));
        }
    }

    #[tokio::test]
    async fn request_capture_keeps_exact_identity_and_billing_link_without_claiming_stream_completion()
     {
        use crate::{
            models::usage_meter::CredentialClass,
            test_utils::{connect_test_database, test_auth_user},
        };
        let db = connect_test_database("service_request_capture")
            .await
            .unwrap();
        audit_service::init_audit_chain_hmac_key(zeroize::Zeroizing::new([7; 32]));
        let mut auth = test_auth_user(&uuid::Uuid::new_v4().to_string());
        auth.auth_method = AuthMethod::ApiKey;
        auth.api_key_id = Some("agent-key".into());
        auth.api_key_name = Some("Codex CI".into());
        auth.api_key_credential_id = Some("login-credential".into());
        auth.oauth_client_id = Some("verified-app".into());
        let mut audit = RequestAudit::new(
            &db,
            &auth,
            Some("exact-connection"),
            "catalog",
            "owner",
            "execution",
            CredentialClass::AgentOverrideUserOwned,
        );
        audit.response(200);
        drop(audit);
        let event = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(event) = db
                    .collection::<AuditLog>("audit_log")
                    .find_one(doc! { "event_type": "service_request" })
                    .await
                    .unwrap()
                {
                    break event;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let data = event.event_data.unwrap();
        assert_eq!(event.api_key_id.as_deref(), Some("agent-key"));
        assert!(event.seq.is_some());
        assert_eq!(data["user_service_id"], "exact-connection");
        assert_eq!(data["service_id"], "catalog");
        assert_eq!(data["execution_id"], data["billing_request_id"]);
        assert_eq!(data["oauth_client_id"], "verified-app");
        assert_eq!(data["api_key_credential_id"], "login-credential");
        assert_eq!(data["credential_class"], "agent_override_user_owned");
        assert_eq!(data["outcome"], "response_received");
        assert_eq!(
            db.collection::<Document>("audit_log")
                .count_documents(doc! { "event_type": "service_request" })
                .await
                .unwrap(),
            1
        );
    }
}
