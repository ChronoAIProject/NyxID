const ROUTE: &str = "/api/v1/admin/warm-pools/pool_1";
const BODY: &[u8] = br#"{"expected_revision":1,"target_replicas":3}"#;
const KEY: &str = "warm-pool-resize-test";
const RUNTIME_ROUTE: &str = "/api/v1/admin/sandboxes/sandbox_1/runtime-update";
const RUNTIME_BODY: &[u8] = br#"{"target_version":"registry.example/runtime@sha256:1234"}"#;

#[test]
fn runtime_update_is_exact_and_requires_its_own_capability() {
    let operation = classify("POST", RUNTIME_ROUTE, RUNTIME_BODY).unwrap();
    assert_eq!(operation, Operation::UpdateSandboxRuntime);
    assert_eq!(operation.admin_permission(), Some("cma:admin:control"));
    assert_eq!(
        Operation::ResizeWarmPool.admin_permission(),
        Some("cma:admin:settings")
    );
    for (permission, allowed) in [
        ("cma:admin:control", true),
        ("cma:*", true),
        ("cma:admin:settings", false),
        ("cma:sandbox:use", false),
        ("*", false),
        ("admin", false),
    ] {
        assert_eq!(
            require_cma_admin_permission(
                &[permission.into()],
                operation.admin_permission().unwrap()
            )
            .is_ok(),
            allowed
        );
    }
    assert!(
        require_cma_admin_permission(&["cma:admin:control".into()], "cma:admin:settings").is_err()
    );
    for body in [
        "{}",
        "[]",
        r#"{"target_version":null}"#,
        r#"{"target_version":1}"#,
        r#"{"target_version":""}"#,
        r#"{"target_version":"   "}"#,
        r#"{"target_version":"x\ny"}"#,
        r#"{"target_version":"v2","force":true}"#,
        r#"{"target_version":"v2","target_version":"v3"}"#,
    ] {
        assert!(
            classify("POST", RUNTIME_ROUTE, body.as_bytes()).is_err(),
            "{body}"
        );
    }
    for (version, allowed) in [("x".repeat(512), true), ("x".repeat(513), false)] {
        let body = serde_json::to_vec(&serde_json::json!({"target_version":version})).unwrap();
        assert_eq!(classify("POST", RUNTIME_ROUTE, &body).is_ok(), allowed);
    }
    for route in [
        "/api/v1/admin/sandboxes/sandbox_1/restart",
        "/api/v1/admin/sandboxes/sandbox_1/runtime-update/",
        "/api/v1/admin/sandboxes/%73andbox_1/runtime-update",
        "/api/v1/admin/sandboxes/../runtime-update",
        "/api/v1/admin/sandboxes/sandbox_1/runtime-update?force=1",
        "/api/v1/sandboxes/sandbox_1/runtime-update",
    ] {
        assert!(classify("POST", route, RUNTIME_BODY).is_err(), "{route}");
    }
    for method in ["PUT", "PATCH", "DELETE"] {
        assert!(classify(method, RUNTIME_ROUTE, RUNTIME_BODY).is_err());
    }
}

#[tokio::test]
async fn runtime_update_checks_control_without_resolving_a_personal_sandbox() {
    let f = Fixture::new().await;
    set_permission(&f, "cma:admin:settings").await;
    assert!(matches!(
        operator_permit(&f, "POST", RUNTIME_ROUTE, RUNTIME_BODY).await,
        Err(AppError::Forbidden(_))
    ));
    set_permission(&f, "cma:admin:control").await;
    let permit = operator_permit(&f, "POST", RUNTIME_ROUTE, RUNTIME_BODY)
        .await
        .unwrap();
    assert!(permit.sandbox.is_none() && permit.creation.is_none());
    let request = || {
        reqwest::Client::new()
            .post(format!("{}{RUNTIME_ROUTE}", f.server.uri()))
            .header("Idempotency-Key", KEY)
            .body(RUNTIME_BODY.to_vec())
            .build()
            .unwrap()
    };
    permit.before_send(&request()).await.unwrap();
    for collection in [
        COLLECTION_NAME,
        crate::models::assistant_cma_sandbox::COLLECTION_NAME,
    ] {
        assert_eq!(
            f.db.collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    // Neither the remote Sandbox nor its owner's personal connection is read.
    assert!(f.server.received_requests().await.unwrap().is_empty());
    for change in ["method", "sandbox", "target_version", "key"] {
        let mut changed = request();
        match change {
            "method" => *changed.method_mut() = reqwest::Method::PUT,
            "sandbox" => changed
                .url_mut()
                .set_path("/api/v1/admin/sandboxes/sandbox_2/runtime-update"),
            "target_version" => {
                *changed.body_mut() = Some(br#"{"target_version":"v3"}"#.to_vec().into())
            }
            "key" => {
                changed
                    .headers_mut()
                    .insert("idempotency-key", "changed-key".parse().unwrap());
            }
            _ => unreachable!(),
        }
        assert!(permit.before_send(&changed).await.is_err(), "{change}");
    }
    set_permission(&f, "cma:admin:settings").await;
    assert!(matches!(
        permit.before_send(&request()).await,
        Err(AppError::Forbidden(_))
    ));
    f.db.drop().await.unwrap();
}

#[test]
fn warm_pool_resize_accepts_only_the_exact_put_and_strict_cas_body() {
    for (revision, replicas) in [(1, 0), (1, 3), (i64::MAX as u64 - 1, 1000)] {
        let body = serde_json::to_vec(&serde_json::json!({
            "expected_revision":revision,"target_replicas":replicas
        }))
        .unwrap();
        assert_eq!(
            classify("PUT", ROUTE, &body).unwrap(),
            Operation::ResizeWarmPool
        );
    }
    for body in [
        "{}",
        "[]",
        r#"{"expected_revision":0,"target_replicas":3}"#,
        r#"{"expected_revision":9223372036854775807,"target_replicas":3}"#,
        r#"{"expected_revision":18446744073709551616,"target_replicas":3}"#,
        r#"{"expected_revision":1,"target_replicas":1001}"#,
        r#"{"expected_revision":1,"target_replicas":4294967296}"#,
        r#"{"expected_revision":1,"target_replicas":-1}"#,
        r#"{"expected_revision":-1,"target_replicas":3}"#,
        r#"{"expected_revision":1.0,"target_replicas":3}"#,
        r#"{"expected_revision":"1","target_replicas":3}"#,
        r#"{"expected_revision":true,"target_replicas":3}"#,
        r#"{"expected_revision":1,"target_replicas":null}"#,
        r#"{"expected_revision":1,"target_replicas":3,"image":"other"}"#,
        r#"{"expected_revision":1,"expected_revision":2,"target_replicas":3}"#,
        r#"{"expected_revision":1,"target_replicas":3,"target_replicas":0}"#,
    ] {
        assert!(classify("PUT", ROUTE, body.as_bytes()).is_err(), "{body}");
    }
    for route in [
        "/api/v1/admin/warm-pools",
        "/api/v1/admin/warm-pools/pool_1/",
        "/api/v1/admin/warm-pools/pool_1/resize",
        "/api/v1/admin/warm-pools/..",
        "/api/v1/admin/warm-pools/%70ool_1",
        "/api/v1/admin/warm-pools/pool_1?target=3",
        "/api/v1/admin/warm-pools/pool_1#suffix",
        "/api/v1/admin/warm-pools/pool\\1",
        "/api/v1/admin/sandboxes/pool_1",
        "//api/v1/admin/warm-pools/pool_1",
    ] {
        assert!(classify("PUT", route, BODY).is_err(), "{route}");
    }
    for method in ["POST", "PATCH", "DELETE"] {
        assert!(classify(method, ROUTE, BODY).is_err(), "{method}");
    }
    assert_eq!(classify("GET", ROUTE, b"").unwrap(), Operation::Read);
    assert_eq!(
        classify("POST", "/api/v1/sandboxes", b"{}").unwrap(),
        Operation::CreateSandbox
    );
}

#[test]
fn warm_pool_resize_requires_explicit_cma_permission_not_generic_authority() {
    for permissions in [vec!["cma:admin:settings"], vec!["cma:*", "unrelated"]] {
        assert!(
            require_cma_admin_permission(
                &permissions
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                "cma:admin:settings",
            )
            .is_ok()
        );
    }
    for permissions in [
        vec![],
        vec!["*"],
        vec!["admin"],
        vec!["cma:admin:*"],
        vec!["provider:*"],
        vec!["cma:agent:manage"],
    ] {
        assert!(
            require_cma_admin_permission(
                &permissions
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                "cma:admin:settings",
            )
            .is_err()
        );
    }
}

async fn set_permission(f: &Fixture, permission: &str) {
    use crate::models::{role, user};
    let now = Utc::now();
    let role = role::Role {
        id: "warm-pool-operator".into(),
        name: "Warm pool operator".into(),
        slug: "warm-pool-operator".into(),
        description: None,
        permissions: vec![permission.into()],
        is_default: false,
        is_system: false,
        client_id: None,
        created_at: now,
        updated_at: now,
    };
    f.db.collection::<role::Role>(role::COLLECTION_NAME)
        .replace_one(doc! {"_id":&role.id}, &role)
        .upsert(true)
        .await
        .unwrap();
    f.db.collection::<user::User>(user::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"role_ids":[&role.id]}})
        .await
        .unwrap();
}

async fn resize_permit(f: &Fixture) -> AppResult<CmaDispatchPermit> {
    operator_permit(f, "PUT", ROUTE, BODY).await
}

async fn operator_permit(
    f: &Fixture,
    method: &str,
    route: &str,
    body: &[u8],
) -> AppResult<CmaDispatchPermit> {
    admit(
        &f.db,
        &f.keys,
        &f.jwt,
        &f.config,
        &f.owner,
        true,
        &f.target,
        method,
        route,
        None,
        body,
        &[("Idempotency-Key".into(), KEY.into())],
        false,
    )
    .await
}

fn resize_request(f: &Fixture) -> reqwest::Request {
    reqwest::Client::new()
        .put(format!("{}{ROUTE}", f.server.uri()))
        .header("Idempotency-Key", KEY)
        .body(BODY.to_vec())
        .build()
        .unwrap()
}

#[tokio::test]
async fn warm_pool_resize_bypasses_account_slot_without_changing_it() {
    let f = Fixture::new().await;
    set_permission(&f, "cma:admin:settings").await;
    let slots =
        f.db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME);
    // A deliberately opaque occupied personal slot must never be loaded or
    // recovered by an administrative operation.
    let occupied = doc! {"_id":"occupied","user_id":&f.owner,"occupies_account_slot":true};
    slots.insert_one(&occupied).await.unwrap();
    let permit = resize_permit(&f).await.unwrap();
    assert!(permit.sandbox.is_none());
    assert!(permit.creation.is_none());
    permit.before_send(&resize_request(&f)).await.unwrap();
    assert_eq!(slots.find_one(doc! {}).await.unwrap().unwrap(), occupied);
    assert_eq!(
        f.db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    // CMA's accepted/conflict response is passed through, with no creation
    // observation or retry on a stale expected_revision.
    for status in [202, 409] {
        Mock::given(method("PUT"))
            .and(path(ROUTE))
            .respond_with(ResponseTemplate::new(status).set_body_string("resize result"))
            .expect(1)
            .mount(&f.server)
            .await;
        let response = reqwest::Client::new()
            .execute(resize_request(&f))
            .await
            .unwrap();
        let response = permit.observe_response(response).await.unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.text().await.unwrap(), "resize result");
        f.server.verify().await;
        f.server.reset().await;
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn warm_pool_resize_rechecks_live_grants_at_dispatch() {
    let f = Fixture::new().await;
    assert!(matches!(
        resize_permit(&f).await,
        Err(AppError::Forbidden(_))
    ));
    set_permission(&f, "*").await;
    assert!(matches!(
        resize_permit(&f).await,
        Err(AppError::Forbidden(_))
    ));
    set_permission(&f, "cma:*").await;
    let permit = resize_permit(&f).await.unwrap();
    permit.before_send(&resize_request(&f)).await.unwrap();
    set_permission(&f, "cma:agent:manage").await;
    assert!(matches!(
        permit.before_send(&resize_request(&f)).await,
        Err(AppError::Forbidden(_))
    ));
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn warm_pool_resize_binds_wire_request_and_transport() {
    let f = Fixture::new().await;
    set_permission(&f, "cma:admin:settings").await;
    let permit = resize_permit(&f).await.unwrap();
    for change in ["method", "path", "body", "query", "key", "duplicate_key"] {
        let mut request = resize_request(&f);
        match change {
            "method" => *request.method_mut() = reqwest::Method::POST,
            "path" => request
                .url_mut()
                .set_path("/api/v1/admin/warm-pools/pool_2"),
            "body" => {
                *request.body_mut() =
                    Some(br#"{"expected_revision":1,"target_replicas":4}"#.to_vec().into());
            }
            "query" => request.url_mut().set_query(Some("target=0")),
            "key" => {
                request.headers_mut().remove("idempotency-key");
            }
            "duplicate_key" => {
                request
                    .headers_mut()
                    .append("idempotency-key", KEY.parse().unwrap());
            }
            _ => unreachable!(),
        }
        assert!(permit.before_send(&request).await.is_err(), "{change}");
    }
    for (personal, unsupported_transport, query, headers) in [
        (
            false,
            false,
            None,
            vec![("Idempotency-Key".into(), KEY.into())],
        ),
        (
            true,
            true,
            None,
            vec![("Idempotency-Key".into(), KEY.into())],
        ),
        (
            true,
            false,
            Some("target=3"),
            vec![("Idempotency-Key".into(), KEY.into())],
        ),
        (true, false, None, vec![]),
        (
            true,
            false,
            None,
            vec![
                ("Idempotency-Key".into(), KEY.into()),
                ("idempotency-key".into(), KEY.into()),
            ],
        ),
        (
            true,
            false,
            None,
            vec![
                ("Idempotency-Key".into(), KEY.into()),
                (RECOVERY_SUBMISSION_HEADER.into(), "old-create".into()),
            ],
        ),
    ] {
        assert!(
            admit(
                &f.db,
                &f.keys,
                &f.jwt,
                &f.config,
                &f.owner,
                personal,
                &f.target,
                "PUT",
                ROUTE,
                query,
                BODY,
                &headers,
                unsupported_transport,
            )
            .await
            .is_err()
        );
    }
    // The personal Agent file-upload exemption must not apply to a pool whose
    // opaque id happens to be "files".
    assert!(matches!(
        admit(
            &f.db,
            &f.keys,
            &f.jwt,
            &f.config,
            &f.owner,
            true,
            &f.target,
            "PUT",
            "/api/v1/admin/warm-pools/files",
            None,
            BODY,
            &[],
            false,
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.db.drop().await.unwrap();
}
