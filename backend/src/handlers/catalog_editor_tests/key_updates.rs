use super::*;
use axum::{body::Body, http::Request};
use tower::ServiceExt;

async fn raw_put(
    f: &Fixture,
    id: &str,
    bearer: &str,
    body: String,
    headers: &[(&str, &str)],
) -> axum::response::Response {
    let (public, private) = crate::routes::build_router_with_state(f.state.clone());
    let mut req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/keys/{id}"))
        .header("authorization", format!("Bearer {bearer}"));
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    public
        .merge(private)
        .layer(axum::extract::DefaultBodyLimit::max(1_048_576))
        .with_state(f.state.clone())
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

async fn assert_no_skill_writes(f: &Fixture) {
    let saved = f
        .state
        .db
        .collection::<DownstreamService>(CATALOG)
        .find_one(doc! {"_id": &f.service.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.skills_revision, 0);
    for collection in [
        crate::models::catalog_skill_revision::COLLECTION_NAME,
        crate::models::catalog_skill_revision::OPERATIONS,
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

fn reference(version: &str) -> serde_json::Value {
    json!({"source":"ornn", "skill_id":"immutable-test-skill", "name":"ops/setup",
        "version":version, "sha256":"a".repeat(64), "dependencies":[]})
}

#[tokio::test]
async fn key_put_assigns_catalog_refs_with_the_existing_client_body() {
    let f = fixture("key_put_scope_editor", false).await;
    save_editor_scopes(&f, "catalog:skills:read catalog:skills:write").await;
    let bearer = token(&f, None).await;
    let future = add_catalog_service(&f, "future-target").await;
    f.state
        .db
        .collection::<Document>(CATALOG)
        .update_one(doc! {"_id": &future.id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    for id in [&f.service.id, &future.id] {
        let path = format!("/api/v1/keys/{id}");
        let refs = json!([reference("1.0")]);
        let body = json!({"recommended_skill_refs":refs});
        let (status, saved) = request(&f.state, "PUT", &path, &bearer, Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(saved["resource_type"], "catalog_service");
        assert_eq!(saved["id"], *id);
        assert_eq!(saved["recommended_skill_refs"], refs);
        assert_eq!(saved["recommended_skills"], json!(["ops/setup"]));
        assert_eq!(saved["skills_revision"], 1);
        for field in [
            "credential",
            "credential_encrypted",
            "base_url",
            "node_id",
            "auth_method",
        ] {
            assert!(saved.get(field).is_none());
        }
        let (status, retry) = request(&f.state, "PUT", &path, &bearer, Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(retry, saved);
        let (status, read) = request(&f.state, "GET", &path, &bearer, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(read, saved);
        let (status, changed) = request(
            &f.state,
            "PUT",
            &path,
            &bearer,
            Some(json!({"recommended_skill_refs":[reference("1.1")]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(changed["skills_revision"], 2);
        assert_ne!(
            changed["skills_manifest_digest"],
            saved["skills_manifest_digest"]
        );
        let (status, cleared) = request(
            &f.state,
            "PUT",
            &path,
            &bearer,
            Some(json!({"recommended_skill_refs":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cleared["recommended_skill_refs"], json!([]));
        assert_eq!(cleared["recommended_skills"], json!([]));
        assert_eq!(cleared["skills_revision"], 3);
    }
    let (status, list) = request(&f.state, "GET", "/api/v1/keys", &bearer, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["keys"].as_array().unwrap().len(), 2);
    for row in list["keys"].as_array().unwrap() {
        assert_eq!(row["skills_revision"], 3);
        assert_eq!(row["recommended_skill_refs"], json!([]));
    }
    for collection in [
        crate::models::catalog_skill_revision::COLLECTION_NAME,
        crate::models::catalog_skill_revision::OPERATIONS,
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            6
        );
    }
}

#[tokio::test]
async fn key_put_legacy_and_scope_editors_keep_independent_write_authority() {
    let (f, role_id, bearer) = editor("key_put_legacy_authority").await;
    let path = format!("/api/v1/keys/{}", f.service.id);
    let body = json!({"recommended_skill_refs":[reference("1.0")]});
    assert_eq!(
        request(&f.state, "PUT", &path, &bearer, Some(body.clone()))
            .await
            .0,
        StatusCode::OK
    );
    f.state
        .db
        .collection::<Document>(crate::models::role::COLLECTION_NAME)
        .update_one(
            doc! {"_id":role_id},
            doc! {"$set":{"permissions":[READ_PERMISSION]}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "PUT", &path, &bearer, Some(body.clone()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    save_editor_scopes(&f, "catalog:skills:write").await;
    let writer = token(&f, None).await;
    let response = raw_put(
        &f,
        &f.service.id,
        &writer,
        body.to_string(),
        &[("content-type", "application/json")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(
        request(&f.state, "GET", &path, &writer, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&f.state, "GET", "/api/v1/keys", &writer, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    save_editor_scopes(&f, "catalog:skills:read").await;
    let readonly = token(&f, None).await;
    for credential in [&writer, &readonly] {
        for body in ["{}".to_string(), "x".repeat(70_001)] {
            assert_eq!(
                raw_put(
                    &f,
                    &f.service.id,
                    credential,
                    body,
                    &[("content-type", "application/json")]
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
        }
    }
}

#[tokio::test]
async fn key_put_rejects_settings_invalid_refs_and_wrong_targets_without_mutations() {
    let (f, _, bearer) = editor("key_put_body_boundaries").await;
    let path = format!("/api/v1/keys/{}", f.service.id);
    let before = f
        .state
        .db
        .collection::<Document>(CATALOG)
        .find_one(doc! {"_id":&f.service.id})
        .await
        .unwrap()
        .unwrap();
    for body in [
        json!({}),
        json!({"recommended_skill_refs":null}),
        json!({"recommended_skill_refs":{}}),
    ] {
        assert_eq!(
            request(&f.state, "PUT", &path, &bearer, Some(body)).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for field in [
        "credential",
        "endpoint_url",
        "auth_method",
        "node_id",
        "label",
        "icon_url",
        "is_active",
        "identity_propagation_mode",
        "use_platform_key",
        "default_request_headers",
        "recommended_skills",
        "clear_refs",
        "base_revision",
        "request_id",
    ] {
        let mut body = json!({"recommended_skill_refs":[reference("1.0")]});
        body[field] = serde_json::Value::Null;
        assert_eq!(
            request(&f.state, "PUT", &path, &bearer, Some(body)).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for (field, value) in [("version", "latest"), ("sha256", "ABC"), ("source", "")] {
        let mut pin = reference("1.0");
        pin[field] = json!(value);
        assert_eq!(
            request(
                &f.state,
                "PUT",
                &path,
                &bearer,
                Some(json!({"recommended_skill_refs":[pin]}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        request(
            &f.state,
            "PUT",
            &path,
            &bearer,
            Some(json!({"recommended_skill_refs":[reference("1.0"),reference("1.0")]}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    for (body, headers, status) in [
        (
            "{",
            vec![("content-type", "application/json")],
            StatusCode::BAD_REQUEST,
        ),
        (
            "{\"recommended_skill_refs\":[],\"recommended_skill_refs\":[]}",
            vec![("content-type", "application/json")],
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "{\"recommended_skill_refs\":[]}",
            vec![],
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            "{\"recommended_skill_refs\":[]}",
            vec![
                ("content-type", "application/json"),
                ("upgrade", "websocket"),
            ],
            StatusCode::FORBIDDEN,
        ),
    ] {
        assert_eq!(
            raw_put(&f, &f.service.id, &bearer, body.into(), &headers)
                .await
                .status(),
            status
        );
    }
    assert_eq!(
        raw_put(
            &f,
            &f.service.id,
            &bearer,
            " ".repeat(70_001),
            &[("content-type", "application/json")]
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let private_id = add_personal_connection(&f).await;
    for id in [private_id, Uuid::new_v4().to_string()] {
        for target in [
            format!("/api/v1/keys/{id}"),
            format!("/api/v1/catalog-curation/services/{id}/skills"),
        ] {
            let mut body = json!({"recommended_skill_refs":[]});
            if target.contains("catalog-curation") {
                body["base_revision"] = json!(0);
                body["request_id"] = json!(Uuid::new_v4().to_string());
            }
            assert_eq!(
                request(&f.state, "PUT", &target, &bearer, Some(body))
                    .await
                    .0,
                StatusCode::NOT_FOUND
            );
        }
    }
    for target in [
        "/api/v1/keys".to_string(),
        format!("/api/v1/keys/{}", f.service.slug),
        format!("{path}/authorization"),
    ] {
        let status = request(
            &f.state,
            "PUT",
            &target,
            &bearer,
            Some(json!({"recommended_skill_refs":[]})),
        )
        .await
        .0;
        assert!(!status.is_success());
    }
    assert_no_skill_writes(&f).await;
    let after = f
        .state
        .db
        .collection::<Document>(CATALOG)
        .find_one(doc! {"_id":&f.service.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn key_put_general_and_curation_accounts_cannot_use_catalog_alias() {
    for curated in [false, true] {
        let f = fixture("key_put_other_purposes", curated).await;
        let bearer = token(&f, None).await;
        let (status, body) = request(
            &f.state,
            "PUT",
            &format!("/api/v1/keys/{}", f.service.id),
            &bearer,
            Some(json!({"recommended_skill_refs":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error_code"], 1002);
        assert_no_skill_writes(&f).await;
    }
}

#[tokio::test]
async fn key_put_checks_live_token_expiry_revocation_generation_and_mixed_headers() {
    let (f, _, bearer) = editor("key_put_live_token").await;
    let body = json!({"recommended_skill_refs":[reference("1.0")]}).to_string();
    assert_eq!(
        raw_put(
            &f,
            &f.service.id,
            &bearer,
            body.clone(),
            &[
                ("content-type", "application/json"),
                ("x-api-key", "invalid-test-key")
            ]
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let tokens = f.state.db.collection::<Document>(TOKENS);
    let original = tokens
        .find_one(doc! {"service_account_id":&f.sa.id})
        .await
        .unwrap()
        .unwrap();
    for update in [
        doc! {"revoked":true},
        doc! {"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::hours(1))},
        doc! {"credential_generation":999_i64},
    ] {
        tokens
            .update_one(
                doc! {"_id":original.get("_id").unwrap()},
                doc! {"$set":update},
            )
            .await
            .unwrap();
        assert_eq!(
            raw_put(
                &f,
                &f.service.id,
                &bearer,
                body.clone(),
                &[("content-type", "application/json")]
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        tokens
            .replace_one(doc! {"_id":original.get("_id").unwrap()}, original.clone())
            .await
            .unwrap();
    }
    let mut forged = bearer.clone().into_bytes();
    let position = forged.iter().rposition(|b| *b == b'.').unwrap() + 1;
    forged[position] = if forged[position] == b'A' { b'B' } else { b'A' };
    assert_eq!(
        raw_put(
            &f,
            &f.service.id,
            &String::from_utf8(forged).unwrap(),
            body,
            &[("content-type", "application/json")]
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_no_skill_writes(&f).await;
}

#[tokio::test]
async fn key_put_and_curation_race_without_rebasing_the_compatibility_request() {
    use crate::services::api_key_mutation_service::TransactionCollisionHook;
    use crate::services::catalog_skill_service::COLLISION;
    let (f, _, bearer) = editor("key_put_collision").await;
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let alias_path = format!("/api/v1/keys/{}", f.service.id);
    let curation_path = format!("/api/v1/catalog-curation/services/{}/skills", f.service.id);
    let (left,right) = tokio::time::timeout(std::time::Duration::from_secs(30),async {
        tokio::join!(
            COLLISION.scope(TransactionCollisionHook::new(barrier.clone()),request(&f.state,"PUT",&alias_path,&bearer,Some(json!({"recommended_skill_refs":[reference("1.0")]})))),
            COLLISION.scope(TransactionCollisionHook::new(barrier),request(&f.state,"PUT",&curation_path,&bearer,Some(json!({"recommended_skill_refs":[reference("2.0")],"base_revision":0,"request_id":Uuid::new_v4().to_string()}))))
        )
    }).await.unwrap();
    let mut statuses = [left.0.as_u16(), right.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    for collection in [
        crate::models::catalog_skill_revision::COLLECTION_NAME,
        crate::models::catalog_skill_revision::OPERATIONS,
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn key_put_revocation_during_transaction_leaves_no_partial_write() {
    use crate::services::catalog_skill_service::AUTHORITY_PAUSE;
    for revoke in ["scope", "token", "account", "role"] {
        let (f, role_id, bearer) = editor("key_put_revoke_race").await;
        if revoke != "role" {
            save_editor_scopes(&f, SCOPES).await;
        }
        let reached = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let resume = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let path = format!("/api/v1/keys/{}", f.service.id);
        let writer = AUTHORITY_PAUSE.scope(
            (reached.clone(), resume.clone(), once),
            request(
                &f.state,
                "PUT",
                &path,
                &bearer,
                Some(json!({"recommended_skill_refs":[reference("1.0")]})),
            ),
        );
        let revoker = async {
            reached.wait().await;
            let (collection, filter, change) = match revoke {
                "scope" => (
                    ACCOUNTS,
                    doc! {"_id":&f.sa.id},
                    doc! {"allowed_scopes":"catalog:skills:read"},
                ),
                "token" => (
                    TOKENS,
                    doc! {"service_account_id":&f.sa.id},
                    doc! {"revoked":true},
                ),
                "account" => (ACCOUNTS, doc! {"_id":&f.sa.id}, doc! {"is_active":false}),
                _ => (
                    crate::models::role::COLLECTION_NAME,
                    doc! {"_id":&role_id},
                    doc! {"permissions":[READ_PERMISSION]},
                ),
            };
            f.state
                .db
                .collection::<Document>(collection)
                .update_many(filter, doc! {"$set":change})
                .await
                .unwrap();
            resume.wait().await;
        };
        let ((status, _), ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            tokio::join!(writer, revoker)
        })
        .await
        .unwrap();
        assert!([StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN].contains(&status));
        assert_no_skill_writes(&f).await;
    }
}

#[tokio::test]
async fn key_put_preserves_human_settings_and_null_headers() {
    let f = fixture("key_put_human_compatibility", false).await;
    let id = add_personal_connection(&f).await;
    let path = format!("/api/v1/keys/{id}");
    let (status, saved) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({
            "label":"Renamed", "recommended_skill_refs":[reference("1.0")],
            "default_request_headers":[{"name":"x-compatibility","value":"kept"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["label"], "Renamed");
    assert!(saved["recommended_skill_refs"].is_null());
    let headers = saved["default_request_headers"].clone();
    assert_eq!(headers[0]["value"], "kept");
    let (status, saved) = request(
        &f.state,
        "PUT",
        "/api/v1/keys/private-connection",
        &f.human_token,
        Some(json!({"label":"Again"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["default_request_headers"], headers);
    let (status, saved) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"default_request_headers":null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(saved["default_request_headers"].is_null());
    let (status, saved) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"icon_url":"https://example.com/icon.png", "recommended_skill_refs":[]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["icon_url"], "https://example.com/icon.png");
    // Above the SA cap but below the existing human cap, an ignored field stays harmless.
    let (status, _) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"label":"Large human body","ignored":"x".repeat(70_001)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        raw_put(
            &f,
            &id,
            &f.human_token,
            " ".repeat(1_048_577),
            &[("content-type", "application/json")]
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_no_skill_writes(&f).await;
}

#[tokio::test]
async fn key_put_keeps_delegated_and_relay_writes_denied() {
    use crate::crypto::jwt::{self, RelayAgentScope};
    let f = fixture("key_put_token_classes", false).await;
    let owner = Uuid::parse_str(&f.owner).unwrap();
    let delegated = jwt::generate_delegated_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &owner,
        "account:read",
        "test-client",
        60,
        None,
    )
    .unwrap();
    let relay = jwt::generate_relay_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &owner,
        "proxy",
        None,
        &RelayAgentScope {
            api_key_id: Uuid::new_v4().to_string(),
            api_key_name: "test-relay".into(),
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: true,
        },
    )
    .unwrap();
    for bearer in [delegated, relay] {
        let (status, body) = request(
            &f.state,
            "PUT",
            &format!("/api/v1/keys/{}", f.service.id),
            &bearer,
            Some(json!({"recommended_skill_refs":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error_code"], 1002);
    }
    assert_no_skill_writes(&f).await;
}

#[tokio::test]
async fn key_put_recommendations_reach_consumers_and_preserve_instance_overrides() {
    let (f, _, bearer) = editor("key_put_consumer_inheritance").await;
    let id = add_personal_connection(&f).await;
    let personal = f
        .state
        .db
        .collection::<Document>(USER_SERVICES)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<Document>(USER_SERVICES)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"catalog_service_id":&f.service.id}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(ENDPOINTS)
        .update_one(
            doc! {"_id":personal.get_str("endpoint_id").unwrap()},
            doc! {"$set":{"catalog_service_id":&f.service.id}},
        )
        .await
        .unwrap();
    let refs = json!([reference("1.0")]);
    let (status, assigned) = request(
        &f.state,
        "PUT",
        &format!("/api/v1/keys/{}", f.service.id),
        &bearer,
        Some(json!({"recommended_skill_refs":refs})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let path = format!("/api/v1/keys/{id}");
    let (status, read) = request(&f.state, "GET", &path, &f.human_token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["recommended_skill_refs"], refs);
    assert_eq!(
        read["skills_manifest_digest"],
        assigned["skills_manifest_digest"]
    );
    let (status, _) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"recommended_skills":["personal/override"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, read) = request(&f.state, "GET", &path, &f.human_token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["recommended_skills"], json!(["personal/override"]));
    assert!(read["recommended_skill_refs"].is_null());
    assert!(read["skills_revision"].is_null());
}

#[tokio::test]
async fn key_put_shares_curation_budget_and_identical_retry_is_free() {
    let (f, _, bearer) = editor("key_put_shared_budget").await;
    f.state.db.collection::<Document>(ACCOUNTS).update_one(doc! {"_id":&f.sa.id},doc! {"$set":{
        "rate_limit_override":1_i64,"catalog_editor_writes_used":0_i64,
        "catalog_editor_write_window":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::hours(1))
    }}).await.unwrap();
    let path = format!("/api/v1/keys/{}", f.service.id);
    let body = json!({"recommended_skill_refs":[reference("1.0")]});
    for _ in 0..2 {
        let (status, saved) = request(&f.state, "PUT", &path, &bearer, Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(saved["skills_revision"], 1);
    }
    assert_eq!(
        request(
            &f.state,
            "PUT",
            &path,
            &bearer,
            Some(json!({"recommended_skill_refs":[reference("1.1")]}))
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(request(&f.state,"PUT",&format!("/api/v1/catalog-curation/services/{}/skills",f.service.id),&bearer,Some(json!({"recommended_skill_refs":[],"base_revision":1,"request_id":Uuid::new_v4().to_string()}))).await.0,StatusCode::TOO_MANY_REQUESTS);
    let account = f
        .state
        .db
        .collection::<Document>(ACCOUNTS)
        .find_one(doc! {"_id":&f.sa.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.get_i64("catalog_editor_writes_used").unwrap(), 1);
    for collection in [
        crate::models::catalog_skill_revision::COLLECTION_NAME,
        crate::models::catalog_skill_revision::OPERATIONS,
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }
}
