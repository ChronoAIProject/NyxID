use super::build_router_with_state;
use crate::AppState;
use crate::crypto::jwt;
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::services::key_service::{self, CreatedApiKey};
use crate::test_utils::{connect_test_database, test_app_state, test_membership, test_user};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    state: AppState,
    app: Router,
    org: User,
    admin: String,
    member: String,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let db = connect_test_database(name)
            .await
            .expect("org router tests require MongoDB; tests must not skip");
        db.run_command(bson::doc! { "ping": 1 }).await.unwrap();
        eprintln!("MongoDB connected; executing org router test: {name}");
        let org = test_user(&Uuid::new_v4().to_string(), UserType::Org);
        let admin = Uuid::new_v4().to_string();
        let member = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                org.clone(),
                test_user(&admin, UserType::Person),
                test_user(&member, UserType::Person),
            ])
            .await
            .unwrap();
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_many([
                test_membership(&org.id, &admin, OrgRole::Admin, None),
                test_membership(&org.id, &member, OrgRole::Member, Some(vec![])),
            ])
            .await
            .unwrap();
        let state = test_app_state(db);
        let (_, private) = build_router_with_state(state.clone());
        let app = private.with_state(state.clone());
        Self {
            state,
            app,
            org,
            admin,
            member,
        }
    }

    async fn key(&self, owner: &str) -> CreatedApiKey {
        key_service::create_api_key(
            &self.state.db,
            owner,
            "org reader",
            "proxy",
            None,
            None,
            Some(&[]),
            Some(&[]),
            Some(false),
            Some(false),
            Some(false),
            None,
            None,
            Some("codex"),
            None,
        )
        .await
        .unwrap()
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        credential: Option<(&str, &str)>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some((header, value)) = credential {
            request = request.header(header, value);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, headers, body)
    }
}

#[tokio::test]
async fn non_api_key_authentication_classes_preserve_org_behavior() {
    let f = Fixture::new("org_auth_classes").await;
    for (scope, expected) in [
        ("account:read", StatusCode::OK),
        ("proxy:*", StatusCode::FORBIDDEN),
    ] {
        let token = jwt::generate_delegated_access_token(
            &f.state.jwt_keys,
            &f.state.config,
            &Uuid::parse_str(&f.admin).unwrap(),
            scope,
            "org-reader",
            60,
            None,
        )
        .unwrap();
        let bearer = format!("Bearer {token}");
        for path in f.read_paths() {
            let (status, _, body) = f
                .request("GET", &path, Some(("authorization", &bearer)))
                .await;
            assert_eq!(status, expected, "delegated scope {scope}, {path}: {body}");
        }
        for (method, path) in f.human_only_routes() {
            let (status, _, _) = f
                .request(method, &path, Some(("authorization", &bearer)))
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "delegated {method} {path}");
        }
    }
    let (token, _) = jwt::generate_service_account_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::new_v4().to_string(),
        "account:read",
        60,
        0,
    )
    .unwrap();
    for (method, path) in f
        .read_paths()
        .into_iter()
        .map(|path| ("GET", path))
        .chain(f.human_only_routes())
    {
        let (status, _, body) = f
            .request(
                method,
                &path,
                Some(("authorization", &format!("Bearer {token}"))),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "service account {method} {path}"
        );
        assert_eq!(
            body["message"],
            "Forbidden: Service accounts cannot access this endpoint"
        );
    }

    let key = f.key(&f.admin).await;
    let token = jwt::generate_relay_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&f.admin).unwrap(),
        "account:read",
        None,
        &jwt::RelayAgentScope {
            api_key_id: key.id,
            api_key_name: key.name,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: false,
            allow_all_nodes: false,
        },
    )
    .unwrap();
    for (method, path) in f
        .read_paths()
        .into_iter()
        .map(|path| ("GET", path))
        .chain(f.human_only_routes())
    {
        let (status, _, _) = f
            .request(
                method,
                &path,
                Some(("authorization", &format!("Bearer {token}"))),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "relay {method} {path}");
    }
    f.state.db.drop().await.unwrap();
}

impl Fixture {
    fn read_paths(&self) -> Vec<String> {
        vec![
            "/api/v1/orgs".into(),
            format!("/api/v1/orgs/{}", self.org.id),
            format!("/api/v1/orgs/{}/authorization", self.org.id),
            format!("/api/v1/orgs/{}/members", self.org.id),
            format!(
                "/api/v1/orgs/{}/members/{}/authorization",
                self.org.id, self.member
            ),
            format!("/api/v1/orgs/{}/role-scopes", self.org.id),
        ]
    }

    fn human_only_routes(&self) -> Vec<(&'static str, String)> {
        let org = &self.org.id;
        let member = &self.member;
        vec![
            ("POST", "/api/v1/orgs".into()),
            ("POST", "/api/v1/orgs/join/test-nonce".into()),
            ("PATCH", format!("/api/v1/orgs/{org}")),
            ("DELETE", format!("/api/v1/orgs/{org}")),
            ("POST", format!("/api/v1/orgs/{org}/members")),
            ("PATCH", format!("/api/v1/orgs/{org}/members/{member}")),
            ("DELETE", format!("/api/v1/orgs/{org}/members/{member}")),
            ("PUT", format!("/api/v1/orgs/{org}/role-scopes/admin")),
            ("DELETE", format!("/api/v1/orgs/{org}/role-scopes/admin")),
            ("GET", format!("/api/v1/orgs/{org}/invites")),
            ("POST", format!("/api/v1/orgs/{org}/invites")),
            ("DELETE", format!("/api/v1/orgs/{org}/invites/test-invite")),
            ("PATCH", "/api/v1/users/me/primary-org".into()),
        ]
    }
}

#[tokio::test]
async fn person_owned_restricted_keys_read_all_six_routes_with_both_headers() {
    let f = Fixture::new("org_person_reads").await;
    for (owner, role) in [(&f.admin, "admin"), (&f.member, "member")] {
        let key = f.key(owner).await;
        assert!(!key.allow_all_services);
        assert!(key.allowed_service_ids.is_empty());
        assert_eq!(key.scopes, "proxy"); // No account:read or read scope.
        let bearer = format!("Bearer {}", key.full_key);
        for credential in [
            ("x-api-key", key.full_key.as_str()),
            ("authorization", bearer.as_str()),
        ] {
            for path in f.read_paths() {
                let (status, _, body) = f.request("GET", &path, Some(credential)).await;
                let expected = if role == "member" && path.ends_with("/role-scopes") {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::OK
                };
                assert_eq!(status, expected, "{path}: {body}");
                if path == "/api/v1/orgs" {
                    assert_eq!(body["orgs"].as_array().unwrap().len(), 1);
                    assert_eq!(body["orgs"][0]["id"], f.org.id);
                    assert_eq!(body["orgs"][0]["your_role"], role);
                }
            }
            for suffix in ["", "/authorization"] {
                let path = format!("/api/v1/orgs/{}{suffix}", f.org.slug.as_deref().unwrap());
                let (status, _, body) = f.request("GET", &path, Some(credential)).await;
                assert_eq!(status, StatusCode::OK, "{path}: {body}");
                assert_eq!(body["id"], f.org.id);
                assert_eq!(body["your_role"], role);
            }
        }
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_owned_keys_read_own_org_as_direct_admin_without_membership() {
    use crate::models::feature_flag_override::{
        COLLECTION_NAME as FLAGS, FeatureFlagOverride, FlagTargetKind,
    };
    use crate::models::org_membership::MemberScopeSource;
    use crate::services::org_invite_service;

    let f = Fixture::new("org_direct_reads").await;
    let key = f.key(&f.org.id).await;
    assert_eq!(
        f.state
            .db
            .collection::<OrgMembership>(MEMBERSHIPS)
            .count_documents(bson::doc! {"member_user_id": &f.org.id})
            .await
            .unwrap(),
        0
    );
    // Even malformed legacy primary-org state cannot project an org as primary.
    f.state
        .db
        .collection::<User>(USERS)
        .update_one(
            bson::doc! {"_id": &f.org.id},
            bson::doc! {"$set": {"primary_org_id": &f.org.id}},
        )
        .await
        .unwrap();
    let invite = org_invite_service::create_invite(
        &f.state.db,
        &f.org.id,
        &f.admin,
        OrgRole::Member,
        MemberScopeSource::Inherit,
        None,
        None,
    )
    .await
    .unwrap();
    org_invite_service::create_invite(
        &f.state.db,
        &f.org.id,
        &f.admin,
        OrgRole::Member,
        MemberScopeSource::Inherit,
        None,
        Some(chrono::Duration::hours(-1)),
    )
    .await
    .unwrap();
    // Role resolution must use Admin; user resolution must use the org actor,
    // with neither lookup depending on an org-as-member row.
    let now = chrono::Utc::now();
    f.state
        .db
        .collection::<FeatureFlagOverride>(FLAGS)
        .insert_many([
            FeatureFlagOverride {
                definition_id: None,
                id: Uuid::new_v4().to_string(),
                org_user_id: Some(f.org.id.clone()),
                flag_key: "experimental:ai-assistant".into(),
                target_kind: FlagTargetKind::Role,
                target_key: Some("admin".into()),
                enabled: true,
                created_at: now,
                updated_at: now,
                updated_by: f.admin.clone(),
            },
            FeatureFlagOverride {
                definition_id: None,
                id: Uuid::new_v4().to_string(),
                org_user_id: Some(f.org.id.clone()),
                flag_key: "example_ui".into(),
                target_kind: FlagTargetKind::User,
                target_key: Some(f.org.id.clone()),
                enabled: true,
                created_at: now,
                updated_at: now,
                updated_by: f.admin.clone(),
            },
        ])
        .await
        .unwrap();
    let bearer = format!("Bearer {}", key.full_key);
    for credential in [
        ("x-api-key", key.full_key.as_str()),
        ("authorization", bearer.as_str()),
    ] {
        let mut paths = f.read_paths();
        paths.extend(
            ["", "/authorization"]
                .map(|suffix| format!("/api/v1/orgs/{}{suffix}", f.org.slug.as_deref().unwrap())),
        );
        for path in paths {
            let (status, _, body) = f.request("GET", &path, Some(credential)).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            assert!(
                !body.to_string().contains(&invite.nonce),
                "org metadata must not expose invite nonces"
            );
            if path == "/api/v1/orgs" {
                assert_eq!(body["orgs"].as_array().unwrap().len(), 1);
                assert_eq!(body["orgs"][0]["id"], f.org.id);
                assert_eq!(body["orgs"][0]["your_role"], "admin");
            } else if body.get("your_role").is_some() {
                assert_eq!(body["your_role"], "admin");
                assert_eq!(body["is_primary"], false);
                assert_eq!(body["active_invite_count"], 1);
                assert_eq!(body["member_count"], 2);
                if let Some(features) = body["enabled_features"].as_array() {
                    assert!(features.contains(&Value::from("experimental:ai-assistant")));
                    assert!(features.contains(&Value::from("example_ui")));
                }
            } else if path.ends_with("/members") {
                assert_eq!(body["members"].as_array().unwrap().len(), 2);
            } else if path.ends_with("/role-scopes") {
                assert_eq!(body["role_scopes"].as_array().unwrap().len(), 3);
            }
        }
    }
    // Direct read projection must not widen the membership-only write helper.
    assert!(matches!(
        crate::handlers::orgs::require_org_admin(&f.state.db, &f.org.id, &f.org.id).await,
        Err(crate::errors::AppError::OrgRoleInsufficient(_))
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn membership_acl_distinguishes_unrelated_missing_revoked_and_viewer() {
    let f = Fixture::new("org_membership_acl").await;
    let other = test_user(&Uuid::new_v4().to_string(), UserType::Org);
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(&other)
        .await
        .unwrap();
    let missing = Uuid::new_v4().to_string();
    for owner in [&f.member, &f.org.id] {
        let key = f.key(owner).await;
        for (target_label, id, expected, code) in [
            ("unrelated org", &other.id, StatusCode::FORBIDDEN, 8102),
            ("missing org", &missing, StatusCode::NOT_FOUND, 8101),
        ] {
            for (route_label, suffix) in [
                ("org detail", "".to_string()),
                ("org authorization", "/authorization".into()),
                ("member list", "/members".into()),
                (
                    "member authorization",
                    format!("/members/{}/authorization", f.member),
                ),
            ] {
                let path = format!("/api/v1/orgs/{id}{suffix}");
                let (status, _, body) = f
                    .request("GET", &path, Some(("x-api-key", &key.full_key)))
                    .await;
                assert_eq!(status, expected, "{target_label} {route_label}: {body}");
                assert_eq!(body["error_code"], code);
            }
        }
    }
    let key = f.key(&f.member).await;
    f.state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .update_one(
            bson::doc! {"org_user_id": &f.org.id, "member_user_id": &f.member},
            bson::doc! {"$set": {"role": "viewer"}},
        )
        .await
        .unwrap();
    let (status, _, _) = f
        .request(
            "GET",
            &format!("/api/v1/orgs/{}/members", f.org.id),
            Some(("x-api-key", &key.full_key)),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    f.state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .update_one(
            bson::doc! {"org_user_id": &f.org.id, "member_user_id": &f.member},
            bson::doc! {"$set": {"revoked_at": bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let (status, _, body) = f
        .request("GET", "/api/v1/orgs", Some(("x-api-key", &key.full_key)))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["orgs"], serde_json::json!([]));
    let (status, _, body) = f
        .request(
            "GET",
            &format!("/api/v1/orgs/{}/members", f.org.id),
            Some(("x-api-key", &key.full_key)),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error_code"], 8102);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn api_keys_cannot_reach_org_writes_invites_or_primary_org() {
    let f = Fixture::new("org_human_routes").await;
    for owner in [&f.admin, &f.org.id] {
        let key = f.key(owner).await;
        let bearer = format!("Bearer {}", key.full_key);
        for credential in [
            ("x-api-key", key.full_key.as_str()),
            ("authorization", bearer.as_str()),
        ] {
            for (method, path) in f.human_only_routes() {
                let (status, _, body) = f.request(method, &path, Some(credential)).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {body}");
                assert_eq!(
                    body["message"], "Forbidden: API keys cannot access this endpoint",
                    "{method} {path}"
                );
            }
        }
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn scheduled_invocation_keys_cannot_read_org_metadata() {
    use crate::models::api_key::{ApiKey, COLLECTION_NAME as KEYS};
    let f = Fixture::new("org_scheduled_key").await;
    let key = f.key(&f.admin).await;
    f.state.db.collection::<ApiKey>(KEYS).update_one(
        bson::doc! {"_id": &key.id},
        bson::doc! {"$set": {"purpose": "scheduled_invocation", "scheduled_write_enabled": true}},
    ).await.unwrap();
    let bearer = format!("Bearer {}", key.full_key);
    for credential in [
        ("x-api-key", key.full_key.as_str()),
        ("authorization", bearer.as_str()),
    ] {
        for path in f.read_paths() {
            let (status, _, body) = f.request("GET", &path, Some(credential)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
            assert_eq!(
                body["error_code"], 9009,
                "must reject with DurableGrantMismatch"
            );
        }
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn merged_org_methods_preserve_allow_headers_and_human_writes() {
    let f = Fixture::new("org_method_merge").await;
    let key = f.key(&f.admin).await;
    for (path, expected) in [
        ("/api/v1/orgs".to_string(), vec!["GET", "HEAD", "POST"]),
        (
            format!("/api/v1/orgs/{}", f.org.id),
            vec!["DELETE", "GET", "HEAD", "PATCH"],
        ),
        (
            format!("/api/v1/orgs/{}/members", f.org.id),
            vec!["GET", "HEAD", "POST"],
        ),
    ] {
        let (status, headers, _) = f.request("PUT", &path, None).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "PUT {path}");
        let mut allowed: Vec<_> = headers["allow"]
            .to_str()
            .unwrap()
            .split(',')
            .map(str::trim)
            .collect();
        allowed.sort_unstable();
        assert_eq!(allowed, expected, "PUT {path}");
    }
    let token = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&f.admin).unwrap(),
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/orgs/{}", f.org.id))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"display_name":"Updated by human"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (status, _, body) = f
        .request(
            "GET",
            &format!("/api/v1/orgs/{}", f.org.id),
            Some(("x-api-key", &key.full_key)),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["display_name"], "Updated by human");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_reads_require_live_keys_and_support_agent_login_children() {
    use crate::models::api_key::{ApiKey, COLLECTION_NAME as KEYS};
    use crate::models::api_key_credential::{ApiKeyCredential, COLLECTION_NAME as CREDENTIALS};
    use crate::services::api_key_credential_service;

    let f = Fixture::new("org_live_keys").await;
    for owner in [&f.admin, &f.org.id] {
        let key = f.key(owner).await;
        let parent = f
            .state
            .db
            .collection::<ApiKey>(KEYS)
            .find_one(bson::doc! {"_id": &key.id})
            .await
            .unwrap()
            .unwrap();
        let child_id = Uuid::new_v4().to_string();
        let secret = api_key_credential_service::generate_secret();
        let mut session = f.state.db.client().start_session().await.unwrap();
        api_key_credential_service::issue(
            &f.state.db,
            &parent,
            &child_id,
            &Uuid::new_v4().to_string(),
            "org reader login",
            chrono::Utc::now() + chrono::Duration::days(1),
            &secret,
            &mut session,
        )
        .await
        .unwrap();
        f.state
            .db
            .collection::<ApiKeyCredential>(CREDENTIALS)
            .update_one(
                bson::doc! {"_id": &child_id},
                bson::doc! {"$set": {"is_active": true}},
            )
            .await
            .unwrap();
        let bearer = format!("Bearer {}", secret.as_str());
        let credential = Some(("authorization", bearer.as_str()));
        for path in f.read_paths() {
            let (status, _, body) = f.request("GET", &path, credential).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
        }
        let (status, _, body) = f.request("POST", "/api/v1/orgs", credential).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body["message"],
            "Forbidden: API keys cannot access this endpoint"
        );
        // Parent revocation invalidates both primary and child credentials live.
        f.state
            .db
            .collection::<ApiKey>(KEYS)
            .update_one(
                bson::doc! {"_id": &key.id},
                bson::doc! {"$set": {"is_active": false}},
            )
            .await
            .unwrap();
        let (status, _, _) = f.request("GET", "/api/v1/orgs", credential).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _, _) = f
            .request("GET", "/api/v1/orgs", Some(("x-api-key", &key.full_key)))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        f.state.db.collection::<ApiKey>(KEYS).update_one(
            bson::doc! {"_id": &key.id},
            bson::doc! {"$set": {"is_active": true, "expires_at": bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::hours(1))}},
        ).await.unwrap();
        let (status, _, _) = f.request("GET", "/api/v1/orgs", credential).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _, _) = f
            .request("GET", "/api/v1/orgs", Some(("x-api-key", &key.full_key)))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    for path in f.read_paths() {
        let (status, _, _) = f.request("GET", &path, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn w8_id_inactive_org_membership_walkers_and_domain_listings_deny() {
    use crate::models::feature_flag_override::{FeatureFlagOverride, FlagTargetKind};
    use crate::services::{
        catalog_service, channel_bot_service, feature_flag_service, node_service, org_service,
        proxy_service, user_service_service,
    };
    use crate::test_utils::{
        test_auto_connected_catalog_service, test_user_endpoint, test_user_service,
    };
    use bson::{Document, doc};

    let f = Fixture::new("w8_id_inactive_org_domains").await;
    let db = &f.state.db;
    let active_org = test_user(&Uuid::new_v4().to_string(), UserType::Org);
    db.collection::<User>(USERS)
        .insert_one(&active_org)
        .await
        .unwrap();
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(
            &active_org.id,
            &f.admin,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    db.collection::<User>(USERS)
        .update_one(
            doc! {"_id": &f.admin},
            doc! {"$set": {"primary_org_id": &f.org.id}},
        )
        .await
        .unwrap();
    let mut services = Vec::new();
    let now = bson::DateTime::now();
    for (index, org) in [&f.org, &active_org].into_iter().enumerate() {
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &org.id,
            "org endpoint",
            "https://example.com",
            None,
            None,
        );
        db.collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .insert_one(&endpoint)
        .await
        .unwrap();
        let mut catalog = test_auto_connected_catalog_service();
        catalog.slug = format!("llm-w8-private-catalog-{index}");
        catalog.visibility = "private".into();
        catalog.owner_user_id = Some(org.id.clone());
        catalog.created_by = org.id.clone();
        let provider_id = Uuid::new_v4().to_string();
        db.collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
            .insert_one(
                doc! {"_id": &provider_id, "slug": format!("w8-provider-{index}"),
                "name": "org LLM provider", "provider_type": "api_key", "is_active": true,
                "created_by": &f.admin, "created_at": now, "updated_at": now},
            )
            .await
            .unwrap();
        catalog.provider_config_id = Some(provider_id);
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
        let service = test_user_service(
            &Uuid::new_v4().to_string(),
            &org.id,
            "w8-shared-proxy",
            &endpoint.id,
            Some(&catalog.id),
            None,
        );
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_one(&service)
        .await
        .unwrap();
        db.collection::<Document>(crate::models::node::COLLECTION_NAME)
            .insert_one(doc! {"_id": Uuid::new_v4().to_string(), "user_id": &org.id,
            "name": "org node", "status": "offline", "auth_token_hash": "fixture",
            "is_active": true, "created_at": now, "updated_at": now})
            .await
            .unwrap();
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(doc! {"_id": Uuid::new_v4().to_string(), "user_id": &org.id,
                "platform": "telegram", "label": "org bot", "platform_bot_id": "fixture",
                "platform_bot_username": "fixture", "webhook_registered": false,
                "webhook_secret_hash": "fixture", "status": "active", "is_active": true,
                "bot_token_encrypted": bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1,2,3]},
                "created_at": now, "updated_at": now})
            .await.unwrap();
        db.collection::<Document>(crate::models::service_approval_config::COLLECTION_NAME)
            .insert_one(doc! {"_id": Uuid::new_v4().to_string(), "user_id": &org.id,
            "service_id": &catalog.id, "service_name": "org policy", "approval_required": true,
            "created_at": now, "updated_at": now})
            .await
            .unwrap();
        services.push(service);
    }
    db.collection::<FeatureFlagOverride>(crate::models::feature_flag_override::COLLECTION_NAME)
        .insert_one(FeatureFlagOverride {
            id: Uuid::new_v4().to_string(),
            definition_id: None,
            org_user_id: Some(f.org.id.clone()),
            flag_key: "example_ui".into(),
            target_kind: FlagTargetKind::Org,
            target_key: None,
            enabled: true,
            created_at: now.to_chrono(),
            updated_at: now.to_chrono(),
            updated_by: f.admin.clone(),
        })
        .await
        .unwrap();
    let token = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&f.admin).unwrap(),
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let bearer = format!("Bearer {token}");
    for is_active in [true, false, true] {
        db.collection::<User>(USERS)
            .update_one(
                doc! {"_id": &f.org.id},
                doc! {"$set": {"is_active": is_active}},
            )
            .await
            .unwrap();
        let mut expected = std::collections::BTreeSet::from([active_org.id.clone()]);
        if is_active {
            expected.insert(f.org.id.clone());
        }
        for include_revoked in [false, true] {
            let rows = org_service::list_memberships_for_member(db, &f.admin, include_revoked)
                .await
                .unwrap();
            assert_eq!(
                rows.into_iter()
                    .map(|m| m.org_user_id)
                    .collect::<std::collections::BTreeSet<_>>(),
                expected
            );
        }
        let rows = org_service::find_active_memberships_with_timeout(db, &f.admin)
            .await
            .unwrap();
        assert_eq!(
            rows.iter()
                .map(|m| m.org_user_id.clone())
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        assert_eq!(
            rows[0].org_user_id,
            if is_active { &f.org.id } else { &active_org.id }.as_str()
        );
        for actor in [&f.admin, &f.member] {
            assert_eq!(
                org_service::is_member(db, actor, &f.org.id).await.unwrap(),
                is_active
            );
            assert_eq!(
                org_service::is_admin(db, actor, &f.org.id).await.unwrap(),
                is_active && actor == &f.admin
            );
            assert_eq!(
                org_service::get_active_membership(db, &f.org.id, actor)
                    .await
                    .unwrap()
                    .is_some(),
                is_active
            );
        }
        assert_eq!(
            org_service::list_members_for_org(db, &f.org.id, false)
                .await
                .unwrap()
                .len(),
            if is_active { 2 } else { 0 }
        );
        assert_eq!(
            org_service::list_admin_user_ids(db, &f.org.id)
                .await
                .unwrap(),
            if is_active {
                vec![f.admin.clone()]
            } else {
                vec![]
            }
        );
        let nodes = node_service::list_user_nodes(db, &f.admin).await.unwrap();
        assert_eq!(
            nodes
                .into_iter()
                .map(|n| n.node.user_id)
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        let bots = channel_bot_service::list_all_bots(db, &f.admin)
            .await
            .unwrap();
        assert_eq!(
            bots.into_iter()
                .map(|b| b.user_id)
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        let visible = user_service_service::list_user_services_with_sources(db, &f.admin)
            .await
            .unwrap();
        assert_eq!(
            visible
                .into_iter()
                .map(|s| s.service.user_id)
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        let catalog = catalog_service::list_catalog_all(db, &f.state.encryption_keys, &f.admin)
            .await
            .unwrap();
        assert_eq!(catalog.len(), expected.len());
        assert_eq!(
            catalog.iter().any(|s| s.slug == "llm-w8-private-catalog-0"),
            is_active
        );
        assert!(catalog.iter().any(|s| s.slug == "llm-w8-private-catalog-1"));
        let llm = crate::services::llm_gateway_service::get_llm_status(
            db,
            &f.admin,
            &f.state.config.base_url,
        )
        .await
        .unwrap();
        assert_eq!(llm.providers.len(), 2);
        // Provider metadata remains discoverable; a disabled org contributes no
        // usable credentials, while the other active org stays ready.
        for provider in llm.providers {
            assert_eq!(
                provider.status,
                if provider.provider_slug == "w8-provider-0" && !is_active {
                    "not_connected"
                } else {
                    "ready"
                }
            );
        }
        assert_eq!(
            feature_flag_service::resolve_personal_features(db, &f.admin)
                .await
                .unwrap()
                .contains(&"example_ui".to_string()),
            is_active
        );
        let (status, _, policies) = f
            .request(
                "GET",
                "/api/v1/approvals/service-configs",
                Some(("authorization", &bearer)),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{policies}");
        let policy_orgs = policies["dominant_org_policies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["org_id"].as_str().unwrap().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(policy_orgs, expected);
        for (path, field) in [
            ("/api/v1/keys", "keys"),
            ("/api/v1/user-services", "services"),
        ] {
            let (status, _, body) = f
                .request("GET", path, Some(("authorization", &bearer)))
                .await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            let ids = body[field]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<std::collections::BTreeSet<_>>();
            assert!(ids.contains(services[1].id.as_str()));
            assert_eq!(ids.contains(services[0].id.as_str()), is_active);
        }
        let resolution = proxy_service::resolve_proxy_target_from_user_service(
            db,
            &f.state.encryption_keys,
            &f.state.node_ws_manager,
            &f.admin,
            Some("w8-shared-proxy"),
            None,
            proxy_service::ProxyExecutionContext::new(
                None,
                crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            resolution.user_service_id,
            services[if is_active { 0 } else { 1 }].id
        );
        let (status, _, listed) = f
            .request("GET", "/api/v1/orgs", Some(("authorization", &bearer)))
            .await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(listed["orgs"].as_array().unwrap().len(), 2);
        let org = listed["orgs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == f.org.id)
            .unwrap();
        assert_eq!(org["is_active"], is_active);
        // The other person remains unable to inherit the second org's resources.
        assert!(matches!(
            org_service::resolve_owner_access(db, &f.member, &active_org.id)
                .await
                .unwrap(),
            org_service::OwnerAccess::Forbidden
        ));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn w8_id_inactive_owners_refuse_keys_and_service_accounts_without_direct_escape() {
    use crate::mw::auth::{AuthMethod, AuthUser};
    use crate::services::{auth_service, org_service, service_account_service};
    use bson::doc;
    let f = Fixture::new("w8_id_inactive_owner_auth").await;
    let key = f.key(&f.org.id).await;
    let identity = Router::new().route("/identity", axum::routing::get(|auth: AuthUser| async move {
        axum::Json(serde_json::json!({"subject":auth.user_id,"owner":auth.proxy_resolution_user_id(),"sa":auth.auth_method == AuthMethod::ServiceAccount}))
    })).with_state(f.state.clone());
    for owner in [&f.org.id, &f.member] {
        let (sa, secret) = service_account_service::create_service_account(
            &f.state.db,
            "owner activity test",
            None,
            "account:read",
            &[],
            None,
            owner,
        )
        .await
        .unwrap();
        let minted = service_account_service::authenticate_client_credentials(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &sa.client_id,
            &secret,
            None,
        )
        .await
        .unwrap();
        for is_active in [true, false, true] {
            f.state
                .db
                .collection::<User>(USERS)
                .update_one(doc! {"_id": owner}, doc! {"$set": {"is_active": is_active}})
                .await
                .unwrap();
            let response = identity
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/identity")
                        .header("authorization", format!("Bearer {}", minted.access_token))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if is_active {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                }
            );
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                    .unwrap();
            if is_active {
                assert_eq!(body["subject"], sa.id);
                assert_eq!(body["owner"], owner.as_str());
                assert_eq!(body["sa"], true);
                // The authenticated SA id cannot hit the owner==actor shortcut.
                assert!(matches!(
                    org_service::resolve_owner_access(&f.state.db, &sa.id, owner)
                        .await
                        .unwrap(),
                    org_service::OwnerAccess::Forbidden
                ));
            }
            if owner == &f.org.id {
                let bearer = format!("Bearer {}", key.full_key);
                for credential in [
                    ("x-api-key", key.full_key.as_str()),
                    ("authorization", bearer.as_str()),
                ] {
                    let (status, _, _) = f.request("GET", "/api/v1/orgs", Some(credential)).await;
                    assert_eq!(
                        status,
                        if is_active {
                            StatusCode::OK
                        } else {
                            StatusCode::UNAUTHORIZED
                        }
                    );
                }
            }
        }
    }
    // Even a malformed legacy org with a valid password hash cannot log in;
    // Direct is safe only behind the verified authentication boundary.
    let password = "w8-id-org-password";
    f.state.db.collection::<User>(USERS).update_one(
        doc! {"_id": &f.org.id},doc! {"$set":{"password_hash":crate::crypto::password::hash_password(password).unwrap()}},
    ).await.unwrap();
    assert!(
        auth_service::authenticate_user(&f.state.db, &f.org.email, password)
            .await
            .is_err()
    );
    assert_eq!(
        org_service::resolve_owner_access(&f.state.db, &f.org.id, &f.org.id)
            .await
            .unwrap(),
        org_service::OwnerAccess::Direct
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn w8_id_platform_admin_can_dismantle_inactive_org_while_org_admin_is_denied() {
    use crate::services::{org_service, ownership_transfer_service as transfers};
    use bson::{Document, doc};
    let f = Fixture::new("w8_id_inactive_org_dismantle").await;
    let mut platform_admin = test_user(&Uuid::new_v4().to_string(), UserType::Person);
    platform_admin.is_admin = true;
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(&platform_admin)
        .await
        .unwrap();
    // Production seeds RBAC roles and backfills legacy flags before serving.
    // Pin the original fixture failure through the unchanged HEAD admin helper:
    // is_admin alone, with no seeded roles, is not a platform-admin identity.
    let fixture_error = crate::handlers::admin_helpers::require_admin(
        &f.state,
        &crate::test_utils::test_auth_user(&platform_admin.id),
    )
    .await
    .expect_err("an unseeded RBAC fixture cannot reach the status mutation");
    assert!(
        matches!(&fixture_error, crate::errors::AppError::Internal(message)
            if message == "Platform system role 'admin' is missing; startup seed did not complete"),
        "{fixture_error:?}"
    );
    eprintln!("Original unseeded fixture error: {fixture_error:?}");
    crate::services::role_service::seed_system_roles(&f.state.db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
        .await
        .unwrap();
    f.state
        .db
        .collection::<User>(USERS)
        .update_one(
            doc! {"_id": &platform_admin.id},
            doc! {"$addToSet": {"role_ids": &roles.admin}},
        )
        .await
        .unwrap();
    let token = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&platform_admin.id).unwrap(),
        "read write admin",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/admin/users/{}/status", f.org.id))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"is_active":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let org = org_service::get_org_user(&f.state.db, &f.org.id)
        .await
        .unwrap();
    assert!(!org.is_active);
    assert!(
        crate::handlers::orgs::require_org_admin(&f.state.db, &f.admin, &f.org.id)
            .await
            .is_err()
    );
    let bot_id = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    f.state.db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(doc! {"_id": &bot_id, "user_id": &f.org.id,"platform":"telegram","label":"to transfer",
            "credential_source":"user","bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1,2,3]},
            "platform_bot_id":"fixture","platform_bot_username":"fixture","webhook_registered":false,
            "webhook_secret_hash":"fixture","status":"active","is_active":true,"created_at":now,"updated_at":now})
        .await.unwrap();
    assert!(
        transfers::preview(
            &f.state.db,
            &f.admin,
            transfers::ResourceKind::ChannelBot,
            &bot_id,
            &f.member,
            5
        )
        .await
        .is_err()
    );
    let preview = transfers::preview(
        &f.state.db,
        &platform_admin.id,
        transfers::ResourceKind::ChannelBot,
        &bot_id,
        &f.member,
        5,
    )
    .await
    .unwrap();
    assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
    let moved = transfers::transfer(
        &f.state.db,
        transfers::TransferCommand {
            actor: &platform_admin.id,
            api_key_id: None,
            first_party_human: true,
            kind: transfers::ResourceKind::ChannelBot,
            resource_id: &bot_id,
            destination: &f.member,
            request_id: &Uuid::new_v4().to_string(),
            expected_version: &preview.version,
            capacity: 5,
        },
    )
    .await
    .unwrap();
    assert_eq!(moved.previous_owner_user_id, f.org.id);
    assert_eq!(moved.new_owner_user_id, f.member);
    let (status, _, _) = f
        .request(
            "DELETE",
            &format!("/api/v1/admin/users/{}", f.org.id),
            Some(("authorization", &format!("Bearer {token}"))),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        f.state
            .db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &f.org.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id":&bot_id,"user_id":&f.member})
            .await
            .unwrap()
            .is_some()
    );
    f.state.db.drop().await.unwrap();
}
