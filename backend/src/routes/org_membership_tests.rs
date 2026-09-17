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
            None,
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
