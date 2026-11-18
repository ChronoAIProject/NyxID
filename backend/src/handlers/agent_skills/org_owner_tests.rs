use super::*;
use crate::{
    models::catalog_skill_revision::SkillReference,
    services::{
        assistant_nyxagent as engine, feature_flag_service as flags, org_agent_tests::Fixture,
        user_api_key_service as keys,
    },
    test_utils::*,
};
use mongodb::bson::{self, doc};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};
use uuid::Uuid;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

async fn connection(
    f: &Fixture,
    owner: &str,
    catalog: &str,
    url: &str,
    credential: &str,
) -> String {
    let endpoint = Uuid::new_v4().to_string();
    let id = Uuid::new_v4().to_string();
    let key = keys::create_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        owner,
        keys::CreateApiKeyParams {
            label: "Test Ornn",
            credential_type: "bearer",
            credential,
            access_token: None,
            refresh_token: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: None,
            source_id: None,
        },
    )
    .await
    .unwrap();
    f.state
        .db
        .collection(crate::models::user_endpoint::COLLECTION_NAME)
        .insert_one(test_user_endpoint(
            &endpoint,
            owner,
            "Test Ornn",
            url,
            None,
            Some(catalog),
        ))
        .await
        .unwrap();
    let mut service = test_user_service(&id, owner, "ornn-test", &endpoint, Some(catalog), None);
    service.api_key_id = Some(key.id);
    service.auth_method = "header".into();
    service.auth_key_name = "X-Ornn-Credential".into();
    service.identity_propagation_mode = "jwt".into();
    f.state
        .db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(service)
        .await
        .unwrap();
    id
}

async fn preview(f: &Fixture, agent: &str, skill: &str) -> AppResult<Json<SkillTestResponse>> {
    Box::pin(preview_attached(
        State(f.state.clone()),
        test_auth_user(&f.admin),
        Path((agent.into(), skill.into())),
    ))
    .await
    .inspect_err(|error| eprintln!("Attached preview refused: {}", json!(error.response_body())))
}

async fn runtime(f: &Fixture, chat: &ChatAuthority, skill: &str) -> (Value, bool) {
    Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        chat,
        "nyxid__skill_read",
        &json!({"skill":skill,"path":"SKILL.md","offset":0}),
    ))
    .await
}

fn assert_reason(error: &Value, reason: &str) {
    assert_eq!(error["details"]["reason"], reason, "{error}");
    let serialized = error.to_string();
    for private in [
        "org-test-token",
        "personal-test-token",
        "private-package-content",
        "Ornn-private-error",
    ] {
        assert!(!serialized.contains(private), "refusal leaked private data");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn org_bound_skill_preview_runtime_use_owner_connection_and_refuse_revocation() {
    let f = Fixture::new("org_bound_skill_2361").await;
    let (mut agent, thread) = f.create().await;
    flags::set_platform_override(
        &f.state.db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.admin,
    )
    .await
    .unwrap();
    let upstream = MockServer::start().await;
    let mut catalog = test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = upstream.uri();
    catalog.identity_propagation_mode = "jwt".into();
    // This fixture uses an org-owned credential. The auto-connected helper's
    // default would classify a non-none injector as a catalog master key.
    catalog.requires_user_credential = true;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    crate::services::approval_service::set_service_approval_config(
        &f.state.db,
        &f.admin,
        &catalog.id,
        &catalog.name,
        Some(false),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    // The actor's own working connection must never shadow or rescue the org.
    connection(
        &f,
        &f.admin,
        &catalog.id,
        &upstream.uri(),
        "personal-test-token",
    )
    .await;
    let org_connection =
        connection(&f, &f.org, &catalog.id, &upstream.uri(), "org-test-token").await;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("SKILL.md", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"private-package-content").unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let pin = SkillReference {
        source: "ornn".into(),
        skill_id: Uuid::new_v4().to_string(),
        name: "private-guide".into(),
        version: "1.0".into(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        dependencies: vec![],
        disabled: false,
    };
    agent.skills = vec![pin.clone()];
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$set":{"skills":bson::to_bson(&agent.skills).unwrap(),"skills_revision":1}},
        )
        .await
        .unwrap();
    let allowed = Arc::new(AtomicBool::new(true));
    let corrupt = Arc::new(AtomicBool::new(false));
    let renamed = Arc::new(AtomicBool::new(false));
    let current_name = renamed.clone();
    // Ornn authority: 0 = org-only reader, 1 = creator, 2 = direct user grantee.
    let directly_visible = Arc::new(AtomicU8::new(0));
    let direct = directly_visible.clone();
    let (grant, corrupt_archive) = (allowed.clone(), corrupt.clone());
    let actor = f.admin.clone();
    let identity_key = f.state.jwt_keys.decoding.clone();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&[upstream.uri()]);
    validation.set_issuer(&[&f.state.config.jwt_issuer]);
    let reference = pin.clone();
    let expected_org = f.org.clone();
    let callback = axum::Router::new()
        .route("/api/v1/orgs", axum::routing::get(org_membership_lookup))
        .with_state(f.state.clone());
    let callback_runtime = tokio::runtime::Handle::current();
    Mock::given(method("GET")).respond_with(move |request: &wiremock::Request| {
        let signed_actor = request.headers.get("x-nyxid-identity-token")
            .and_then(|header| header.to_str().ok())
            .and_then(|token| jsonwebtoken::decode::<Value>(token, &identity_key, &validation).ok())
            .is_some_and(|token| token.claims["sub"].as_str() == Some(actor.as_str()));
        let org_credential = request.headers.get("x-ornn-credential")
            .is_some_and(|header| header == "org-test-token");
        // Match Ornn's real object ACL: identity alone does not establish an
        // org grant. It calls NyxID /orgs with the forwarded Authorization.
        let membership = request.headers.get("authorization").is_some_and(|bearer| {
            // Wiremock owns a single-thread runtime: run the async callback
            // on the test runtime, then wait for its result on this mock thread.
            let callback = callback.clone();
            let expected_org = expected_org.clone();
            let bearer = bearer.clone();
            futures::executor::block_on(callback_runtime.spawn(async move {
                use tower::ServiceExt;
                let response = callback.clone().oneshot(Request::builder()
                    .uri("/api/v1/orgs").header(http::header::AUTHORIZATION, bearer)
                    .body(Body::empty()).unwrap()).await.unwrap();
                let status = response.status();
                let body = to_bytes(response.into_body(), 4096).await.unwrap();
                let data: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(status, http::StatusCode::OK, "org callback refused: {data}");
                data["orgs"].as_array().is_some_and(|orgs| orgs.len() == 1
                    && orgs[0]["id"] == expected_org && orgs[0]["your_role"] == "member")
            })).unwrap()
        });
        assert!(signed_actor, "Ornn did not receive the expected signed actor");
        assert!(org_credential, "Ornn did not receive the org credential");
        assert!(membership, "Ornn did not receive the expected live org membership");
        if !(direct.load(Ordering::SeqCst) != 0 || (membership && grant.load(Ordering::SeqCst))) {
            return ResponseTemplate::new(403).set_body_json(json!({"detail":"Ornn-private-error"}));
        }
        let root = format!("/api/v1/skills/{}", reference.skill_id);
        if request.url.path() == root && request.url.query_pairs().any(|(k,v)| k == "version" && v == "1.0") {
            ResponseTemplate::new(200).set_body_json(json!({"data":{
                "guid":reference.skill_id,"name":if current_name.load(Ordering::SeqCst) { "renamed-guide" } else { &reference.name },"version":reference.version,
                "description":"Guide", "skillHash":reference.sha256, "isPrivate":true,
                "createdBy":if direct.load(Ordering::SeqCst) == 1 { actor.as_str() } else { "another-person" },
                "grants":if grant.load(Ordering::SeqCst) { json!([{"type":"org","id":expected_org,"level":"read"}]) }
                    else if direct.load(Ordering::SeqCst) == 2 { json!([{"type":"user","id":actor,"level":"read"}]) }
                    else { json!([]) },
            }}))
        } else if request.url.path() == format!("{root}/closure") {
            ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}}))
        } else if request.url.path() == format!("{root}/versions/1.0/download") {
            ResponseTemplate::new(200).set_body_bytes(if corrupt_archive.load(Ordering::SeqCst) {
                b"wrong-pinned-version".to_vec()
            } else { bytes.clone() })
        } else {
            ResponseTemplate::new(404)
        }
    }).mount(&upstream).await;
    let thread = engine::begin_turn(
        &f.state.db,
        &f.admin,
        &engine::TurnRequest {
            conversation_id: Some(thread.id),
            text: "Read the guide".into(),
            agent_id: None,
            model: None,
            access_mode: None,
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let chat = acks::for_key(&f.state.db, &f.admin, Some(&thread.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(chat.turn_live);
    // Exercise an ordinary member, not an organization administrator. The
    // existing thread must retain access after demotion and lose it on removal.
    f.state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(
            doc! {"org_user_id":&f.org,"member_user_id":&f.admin},
            doc! {"$set":{"role":"member"}},
        )
        .await
        .unwrap();
    assert!(
        !f.key(&thread)
            .await
            .allowed_service_ids
            .contains(&org_connection)
    );
    let Json(result) = preview(&f, &agent.id, &pin.skill_id).await.unwrap();
    assert!(result.readable);
    assert_eq!(result.excerpt, "private-package-content");
    let before_runtime = upstream.received_requests().await.unwrap().len();
    let (page, error) = runtime(&f, &chat, &pin.skill_id).await;
    assert!(!error, "{page}");
    assert_eq!(page["content"], result.excerpt);
    // Runtime reads only the exact pin; Ornn decides visibility. No extra
    // metadata/ACL fetch is needed before checking the downloaded bytes.
    let calls = upstream.received_requests().await.unwrap();
    assert_eq!(calls.len(), before_runtime + 1);
    assert_eq!(
        calls.last().unwrap().url.path(),
        format!("/api/v1/skills/{}/versions/1.0/download", pin.skill_id)
    );
    // Every call retained the acting person as principal and the org credential.
    assert!(calls.len() >= 5);
    for call in calls {
        assert_eq!(call.headers["x-ornn-credential"], "org-test-token");
        assert!(
            call.headers["authorization"]
                .to_str()
                .unwrap()
                .starts_with("Bearer ey")
        );
        assert!(!call.url.query_pairs().any(|(name, _)| name == "_nyxid_via"));
    }
    // Ornn metadata names are current even on pinned versions. A display-name
    // change must not make preview disagree with an unchanged package read.
    renamed.store(true, Ordering::SeqCst);
    assert!(
        preview(&f, &agent.id, &pin.skill_id)
            .await
            .unwrap()
            .0
            .readable
    );
    let (page, failed) = runtime(&f, &chat, &pin.skill_id).await;
    assert!(!failed, "{page}");
    renamed.store(false, Ordering::SeqCst);
    let before = upstream.received_requests().await.unwrap().len();
    let (error, failed) = runtime(&f, &chat, &Uuid::new_v4().to_string()).await;
    assert!(failed);
    assert_reason(&error, "skill_not_bound");
    let (error, failed) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__skill_read",
        &json!({"skill":pin.skill_id,"dependency":Uuid::new_v4().to_string()}),
    ))
    .await;
    assert!(failed);
    assert_reason(&error, "skill_not_bound");
    let mut guest = chat.clone();
    guest.guest = true;
    assert!(runtime(&f, &guest, &pin.skill_id).await.1);
    assert_eq!(upstream.received_requests().await.unwrap().len(), before);

    // No org connection: preview cannot certify the actor's personal fallback.
    let connections = f
        .state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME);
    connections
        .update_one(
            doc! {"_id":&org_connection},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    let error = preview(&f, &agent.id, &pin.skill_id).await.unwrap_err();
    assert_reason(
        &json!(error.response_body()),
        "owner_connection_unavailable",
    );
    let (error, failed) = runtime(&f, &chat, &pin.skill_id).await;
    assert!(failed);
    assert_reason(&error, "owner_connection_unavailable");
    assert_eq!(upstream.received_requests().await.unwrap().len(), before);
    connections
        .update_one(
            doc! {"_id":&org_connection},
            doc! {"$set":{"is_active":true}},
        )
        .await
        .unwrap();

    // A configured Authorization credential must not overwrite the lookup
    // capability or silently substitute its principal.
    connections
        .update_one(
            doc! {"_id":&org_connection},
            doc! {"$set":{"auth_method":"bearer"}},
        )
        .await
        .unwrap();
    let error = preview(&f, &agent.id, &pin.skill_id).await.unwrap_err();
    assert_reason(
        &json!(error.response_body()),
        "owner_connection_unavailable",
    );
    let (error, failed) = runtime(&f, &chat, &pin.skill_id).await;
    assert!(failed);
    assert_reason(&error, "owner_connection_unavailable");
    assert_eq!(upstream.received_requests().await.unwrap().len(), before);
    connections
        .update_one(
            doc! {"_id":&org_connection},
            doc! {"$set":{"auth_method":"header"}},
        )
        .await
        .unwrap();

    // The narrow callback must never cause minting a general delegation token.
    connections
        .update_one(
            doc! {"_id":&org_connection},
            doc! {"$set":{"inject_delegation_token":true}},
        )
        .await
        .unwrap();
    let (error, failed) = runtime(&f, &chat, &pin.skill_id).await;
    assert!(failed);
    assert_reason(&error, "owner_connection_unavailable");
    assert_eq!(upstream.received_requests().await.unwrap().len(), before);
    connections
        .update_one(
            doc! {"_id":&org_connection},
            doc! {"$set":{"inject_delegation_token":false}},
        )
        .await
        .unwrap();

    // Ornn revokes the private object's org read grant. Never try personal auth.
    allowed.store(false, Ordering::SeqCst);
    let error = preview(&f, &agent.id, &pin.skill_id).await.unwrap_err();
    assert_reason(&json!(error.response_body()), "ornn_principal_denied");
    let (error, failed) = runtime(&f, &chat, &pin.skill_id).await;
    assert!(failed);
    assert_reason(&error, "ornn_principal_denied");
    assert_eq!(error["error_code"], 1002);
    assert_eq!(
        upstream.received_requests().await.unwrap().len(),
        before + 2
    );
    // Ornn remains authoritative: creator and direct-user access both keep
    // working through the same org connection without any org grant.
    for access in [1, 2] {
        directly_visible.store(access, Ordering::SeqCst);
        let result = preview(&f, &agent.id, &pin.skill_id).await.unwrap().0;
        assert!(result.readable);
        assert_eq!(result.excerpt, "private-package-content");
        let before_runtime = upstream.received_requests().await.unwrap().len();
        let (page, failed) = runtime(&f, &chat, &pin.skill_id).await;
        assert!(!failed, "access={access}: {page}");
        assert_eq!(page["content"], result.excerpt);
        assert_eq!(
            upstream.received_requests().await.unwrap().len(),
            before_runtime + 1
        );
    }
    directly_visible.store(0, Ordering::SeqCst);
    allowed.store(true, Ordering::SeqCst);

    // A successful HTTP download of changed bytes is still not this pin.
    corrupt.store(true, Ordering::SeqCst);
    let error = preview(&f, &agent.id, &pin.skill_id).await.unwrap_err();
    assert_reason(&json!(error.response_body()), "pinned_version_mismatch");
    let (error, failed) = runtime(&f, &chat, &pin.skill_id).await;
    assert!(failed);
    assert_reason(&error, "pinned_version_mismatch");
    corrupt.store(false, Ordering::SeqCst);

    // Removing the enabled binding takes effect on the next read.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$set":{"skills.0.disabled":true}},
        )
        .await
        .unwrap();
    let (error, failed) = runtime(&f, &chat, &pin.skill_id).await;
    assert!(failed);
    assert_reason(&error, "skill_not_bound");
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$set":{"skills.0.disabled":false}},
        )
        .await
        .unwrap();

    let before = upstream.received_requests().await.unwrap().len();
    f.revoke(&f.admin).await;
    assert!(preview(&f, &agent.id, &pin.skill_id).await.is_err());
    assert!(runtime(&f, &chat, &pin.skill_id).await.1);
    assert_eq!(upstream.received_requests().await.unwrap().len(), before);
    f.state.db.drop().await.unwrap();
}
