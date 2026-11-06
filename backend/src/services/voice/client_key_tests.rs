//! Real MongoDB tests of the existing-service permission boundary and call identity.
use super::{
    client_key as keys,
    credential_tests::{fixture, platform},
    session,
};
use crate::{
    models::api_key::{ApiKey, COLLECTION_NAME as KEYS},
    mw::auth::AuthUser,
    services::{assistant_authority_tests::Fixture, feature_flag_service as flags, key_service},
};
use mongodb::bson::{self, doc};
use uuid::Uuid;

async fn setup(
    name: &str,
) -> (
    Fixture,
    crate::models::downstream_service::DownstreamService,
    String,
    String,
) {
    let (f, voice) = fixture(name).await;
    crate::services::security_readiness_service::ready_test_replica(&f.state.db).await;
    flags::set_platform_override(
        &f.state.db,
        flags::VOICE_AGENT_KEY_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = super::access::SERVICE_SLUG.into();
    catalog.is_active = true;
    catalog.auth_method = "none".into();
    catalog.requires_user_credential = false;
    catalog.forward_access_token = true;
    catalog.inject_delegation_token = false;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&catalog)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"source":1,"source_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {"source_id":{"$type":"string"}})
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let agent =
        crate::services::assistant_team_service::agent_for_conversation(&f.state.db, &f.row)
            .await
            .unwrap();
    let id = super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url)
        .await
        .unwrap()
        .id;
    let created = key_service::create_api_key(
        &f.state.db,
        &f.owner,
        "panel",
        "read write proxy",
        None,
        None,
        Some(std::slice::from_ref(&id)),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    (f, voice, id, created.full_key)
}
async fn auth(f: &Fixture, raw: &str) -> AuthUser {
    let (_, key, child) = key_service::validate_api_key(&f.state.db, raw)
        .await
        .unwrap();
    crate::mw::auth::api_key_auth_user(&f.state.db, &key, child, None, None)
        .await
        .unwrap()
}
async fn child(f: &Fixture, parent: &str) -> (String, String) {
    let secret = crate::services::api_key_credential_service::generate_secret();
    let id = Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>("api_key_credentials").insert_one(doc! {
        "_id":&id,"api_key_id":parent,"user_id":&f.owner,"secret_hash":crate::crypto::token::hash_token(&secret),
        "secret_prefix":"nyxid_ag_test","label":"board","login_request_id":Uuid::new_v4().to_string(),
        "is_active":true,"created_at":bson::DateTime::now(),
        "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::hours(1)),
    }).await.unwrap();
    (id, secret.to_string())
}

#[tokio::test]
async fn seeded_catalog_enables_owner_publication_and_exact_device_discovery() {
    use crate::handlers::assistant_voice::{publish_voice_service, voice_service};
    use axum::{
        body::{Body, to_bytes},
        extract::{Path, State},
        http::Request,
    };
    use tower::ServiceExt;

    Box::pin(async {
        let (f, _) = fixture("voice_seed_publication").await;
        let mut agent =
            crate::services::assistant_team_service::agent_for_conversation(&f.state.db, &f.row)
                .await
                .unwrap();
        agent.description = "帮助你查看和控制已授权的平板。".into();
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &agent.id},
                doc! {"$set": {"description": &agent.description}},
            )
            .await
            .unwrap();
        let human = crate::test_utils::test_auth_user(&f.owner);
        // Reproduce the missing-catalog failure independently of rollout flags.
        assert!(matches!(
            super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url)
                .await,
            Err(crate::errors::AppError::NotFound(_))
        ));
        super::catalog::seed(&f.state.db, &f.state.config.base_url)
            .await
            .unwrap();
        // The seed publishes no connection and enables no feature.
        assert!(
            super::binding::get(&f.state.db, &f.owner, &agent.id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            publish_voice_service(
                State(f.state.clone()),
                human.clone(),
                Path(agent.id.clone())
            )
            .await,
            Err(crate::errors::AppError::Forbidden(_))
        ));
        crate::services::security_readiness_service::ready_test_replica(&f.state.db).await;
        flags::set_platform_override(
            &f.state.db,
            flags::VOICE_AGENT_KEY_FLAG_KEY,
            &flags::FlagTarget::Global,
            true,
            &f.owner,
        )
        .await
        .unwrap();
        let published = publish_voice_service(
            State(f.state.clone()),
            human.clone(),
            Path(agent.id.clone()),
        )
        .await
        .unwrap()
        .0;
        let read = voice_service(State(f.state.clone()), human, Path(agent.id.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(published, read);
        assert_eq!(read["service"]["agent_id"], agent.id);
        assert_eq!(read["service"]["is_active"], true);
        let service_id = read["service"]["id"].as_str().unwrap().to_string();
        let created = key_service::create_api_key(
            &f.state.db,
            &f.owner,
            "seeded-device",
            "read proxy",
            None,
            None,
            Some(std::slice::from_ref(&service_id)),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        for path in [
            "/api/v1/keys".to_string(),
            format!("/api/v1/proxy/nyxbot-voice/{service_id}/voice/agents"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(&path)
                        .header("authorization", format!("Bearer {}", created.full_key))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
            assert_eq!(status, 200, "{path}: {}", String::from_utf8_lossy(&bytes));
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            if path == "/api/v1/keys" {
                let connections = body["keys"].as_array().unwrap();
                assert_eq!(connections.len(), 1);
                assert_eq!(connections[0]["id"], service_id);
                assert_eq!(
                    connections[0]["catalog_service_slug"],
                    super::access::SERVICE_SLUG
                );
            } else {
                assert_eq!(body["agents"].as_array().unwrap().len(), 1);
                assert_eq!(body["agents"][0]["agent_id"], agent.id);
                assert_eq!(body["agents"][0]["description"], agent.description);
                assert_eq!(body["agents"][0]["service_id"], service_id);
            }
        }
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_key_uses_existing_service_grants_and_no_bot_hardcoding() {
    Box::pin(async {
        let (f, _, service, raw) = setup("voice_key_service_grants").await;
        let a = auth(&f, &raw).await;
        let proof = keys::authorize(&f.state.db, &a, &f.row.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(proof.service_id, service);
        let primary =
            crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.nyxbot_thread)
                .await
                .unwrap();
        // Sharing an owner is not a grant to the Full-access primary NyxBot.
        assert!(keys::authorize(&f.state.db, &a, &primary.id).await.is_err());
        let primary_agent =
            crate::services::assistant_team_service::agent_for_conversation(&f.state.db, &primary)
                .await
                .unwrap();
        let other = super::binding::publish(
            &f.state.db,
            &f.owner,
            &primary_agent.id,
            &f.state.config.base_url,
        )
        .await
        .unwrap();
        assert_ne!(service, other.id);
        assert!(keys::authorize(&f.state.db, &a, &primary.id).await.is_err());
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(
                doc! {"_id":&proof.key_id},
                doc! {"$set":{"allowed_service_ids":[&service, &other.id]}},
            )
            .await
            .unwrap();
        assert!(
            keys::authorize(&f.state.db, &auth(&f, &raw).await, &primary.id)
                .await
                .is_ok()
        );
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(
                doc! {"_id":&proof.key_id},
                doc! {"$set":{"allow_all_services":false,"allowed_service_ids":[]}},
            )
            .await
            .unwrap();
        assert!(
            keys::authorize(&f.state.db, &auth(&f, &raw).await, &f.row.id)
                .await
                .is_err()
        );
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(
                doc! {"_id":&proof.key_id},
                doc! {"$set":{"allowed_service_ids":[&service]}},
            )
            .await
            .unwrap();
        assert!(
            keys::authorize(&f.state.db, &auth(&f, &raw).await, &f.row.id)
                .await
                .is_ok()
        );
        let mut wrong = auth(&f, &raw).await;
        wrong.api_key_purpose = crate::models::api_key::ApiKeyPurpose::ScheduledInvocation;
        // Persisted key purpose, rather than a forged extractor field, is authoritative.
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(
                doc! {"_id":&proof.key_id},
                doc! {"$set":{"purpose":"scheduled_invocation"}},
            )
            .await
            .unwrap();
        assert!(
            keys::authorize(&f.state.db, &wrong, &f.row.id)
                .await
                .is_err()
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}
#[tokio::test]
async fn voice_key_call_binds_primary_or_exact_child_and_rechecks_revocation() {
    Box::pin(async {
        let (f, voice, _, raw) = setup("voice_key_child_binding").await;
        let primary = auth(&f, &raw).await;
        let parent = primary.api_key_id.as_deref().unwrap();
        let (id, secret) = child(&f, parent).await;
        let (_, sibling) = child(&f, parent).await;
        let a = auth(&f, &secret).await;
        let proof = keys::authorize(&f.state.db, &a, &f.row.id).await.unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &Uuid::new_v4().to_string(),
            platform(&voice),
            "fixture".into(),
            "worker",
            proof,
        )
        .await
        .unwrap();
        assert!(keys::own_call(&f.state.db, &a, &call).await.is_ok());
        // A real second authentication updates last_used_at and state_version.
        // It must still be able to open the stream and control its own call.
        let fresh = auth(&f, &secret).await;
        assert!(keys::own_call(&f.state.db, &fresh, &call).await.is_ok());
        assert!(keys::own_call(&f.state.db, &primary, &call).await.is_err());
        assert!(
            keys::own_call(&f.state.db, &auth(&f, &sibling).await, &call)
                .await
                .is_err()
        );
        let segment = super::transcript::Segment {
            fragments: Vec::new(),
            id: Uuid::new_v4().to_string(),
            speaker: super::transcript::Speaker::User,
            text: "Check my authorized task".into(),
            start_ms: 0,
            end_ms: 100,
            sealed: true,
            complete: true,
        };
        super::transcript::persist(&f.state.db, &call, &segment)
            .await
            .unwrap();
        let request = super::transcript::delegate(&f.state.db, &call, "fresh-auth", &segment)
            .await
            .unwrap();
        let _ = auth(&f, &secret).await;
        let mut tx = f.state.db.client().start_session().await.unwrap();
        tx.start_transaction().await.unwrap();
        keys::fence_call(&f.state.db, &call, &mut tx).await.unwrap();
        tx.commit_transaction().await.unwrap();
        // Once Voice task admission fences the child, a concurrent revocation must
        // retry after that admission commits rather than silently passing it.
        let mut admission = f.state.db.client().start_session().await.unwrap();
        admission.start_transaction().await.unwrap();
        keys::fence_call(&f.state.db, &call, &mut admission)
            .await
            .unwrap();
        let mut revocation = f.state.db.client().start_session().await.unwrap();
        revocation.start_transaction().await.unwrap();
        let conflict = f
            .state
            .db
            .collection::<bson::Document>("api_key_credentials")
            .update_one(doc! {"_id":&id}, doc! {"$set":{"is_active":false}})
            .session(&mut revocation)
            .await
            .unwrap_err();
        assert!(
            conflict.contains_label("TransientTransactionError"),
            "{conflict}"
        );
        let _ = revocation.abort_transaction().await;
        admission.commit_transaction().await.unwrap();
        f.state
            .db
            .collection::<bson::Document>("api_key_credentials")
            .update_one(doc! {"_id":&id}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        assert!(keys::revalidate(&f.state.db, &call).await.is_err());
        let mut after_revocation = f.state.db.client().start_session().await.unwrap();
        after_revocation.start_transaction().await.unwrap();
        assert!(
            keys::fence_call(&f.state.db, &call, &mut after_revocation)
                .await
                .is_err()
        );
        after_revocation.abort_transaction().await.unwrap();
        assert!(
            super::transcript::delegate(&f.state.db, &call, "revoked-auth", &segment)
                .await
                .is_err(),
            "Revoked credentials cannot submit new Bot tasks"
        );
        keys::end(&f.state.db, &call).await.unwrap();
        assert_eq!(
            crate::services::assistant_voice::get(&f.state.db, &f.owner, &f.row.id, &request.id)
                .await
                .unwrap()
                .state,
            crate::models::assistant_voice::RequestState::Queued,
            "Ending a revoked call does not cancel an already accepted request"
        );
        assert!(
            session::get(&f.state.db, &f.owner, &f.row.id, &call.id)
                .await
                .unwrap()
                .end_requested
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}
#[tokio::test]
async fn voice_key_rechecks_expiry_scope_catalog_policy_and_service_removal() {
    Box::pin(async {
    let (f,voice,service,raw)=setup("voice_key_live_policy").await;let a=auth(&f,&raw).await;
    let proof=keys::authorize(&f.state.db,&a,&f.row.id).await.unwrap();
    let call=session::admit_for_client_key(&f.state.db,&f.owner,&f.row.id,&Uuid::new_v4().to_string(),platform(&voice),"fixture".into(),"worker",proof).await.unwrap();
    let key=key_service::get_api_key(&f.state.db,&f.owner,a.api_key_id.as_deref().unwrap()).await.unwrap();
    for patch in [doc! {"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))},
        doc! {"scopes":"read"},doc! {"is_active":false},
        doc! {"allow_all_services":false,"allowed_service_ids":[]}] {
        f.state.db.collection::<bson::Document>(KEYS).update_one(doc! {"_id":&key.id},doc! {"$set":patch}).await.unwrap();
        assert!(keys::revalidate(&f.state.db,&call).await.is_err());
        f.state.db.collection::<ApiKey>(KEYS).replace_one(doc! {"_id":&key.id},&key).await.unwrap();
    }
    f.state.db.collection::<bson::Document>("user_services").update_one(doc! {"_id":&service},doc! {"$set":{"is_active":false}}).await.unwrap();
    assert!(keys::revalidate(&f.state.db,&call).await.is_err());
    f.state.db.collection::<bson::Document>("user_services").update_one(doc! {"_id":&service},doc! {"$set":{"is_active":true}}).await.unwrap();
    assert!(keys::revalidate(&f.state.db,&call).await.is_ok());
    flags::set_platform_override(&f.state.db,flags::VOICE_AGENT_KEY_FLAG_KEY,&flags::FlagTarget::Global,false,&f.owner).await.unwrap();
    assert!(keys::revalidate(&f.state.db,&call).await.is_err());
    f.state.db.drop().await.unwrap();
}).await;
}
#[tokio::test]
async fn voice_key_cannot_control_browser_call_or_skip_service_operation_limits() {
    Box::pin(async {
    let (f,voice,service,raw)=setup("voice_key_browser_and_b1").await;let a=auth(&f,&raw).await;
    let call=session::admit(&f.state.db,&f.owner,&f.row.id,&Uuid::new_v4().to_string(),platform(&voice),"fixture".into(),"worker").await.unwrap();
    assert!(keys::own_call(&f.state.db,&a,&call).await.is_err());
    f.state.db.collection::<bson::Document>(KEYS).update_one(doc! {"_id":a.api_key_id.as_deref().unwrap()},doc! {"$set":{"assistant_operation_scopes":{&service:{"revision":1,"operations":[]}}}}).await.unwrap();
    assert!(keys::authorize(&f.state.db,&auth(&f,&raw).await,&f.row.id).await.is_err());
    // The ordinary browser owner retains the existing control authority.
    let human=crate::test_utils::test_auth_user(&f.owner);
    assert!(keys::own_call(&f.state.db,&human,&call).await.is_ok());
    f.state.db.drop().await.unwrap();
}).await;
}

#[tokio::test]
async fn llm_nyx_grant_does_not_delegate_bot_voice() {
    Box::pin(async {
        let (f, _, service, raw) = setup("voice_distinct_service_contract").await;
        let key_id = auth(&f, &raw).await.api_key_id.unwrap();
        f.state.db.collection::<bson::Document>(KEYS).update_one(doc! {"_id":&key_id}, doc! {"$set":{
            "allow_all_services":false,"allowed_service_ids":["stateless-llm-nyx-connection"]
        }}).await.unwrap();
        assert!(keys::authorize(&f.state.db,&auth(&f,&raw).await,&f.nyxbot_thread).await.is_err());
        f.state.db.collection::<bson::Document>(KEYS).update_one(doc! {"_id":&key_id}, doc! {"$set":{
            "allowed_service_ids":[&service]
        }}).await.unwrap();
        assert!(keys::authorize(&f.state.db,&auth(&f,&raw).await,&f.row.id).await.is_ok());
        f.state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn voice_child_http_discovery_and_start_enforce_exact_bot_connection() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    Box::pin(async {
        let (f, voice, service_id, raw) = setup("voice_child_http_scope").await;
        let primary = auth(&f, &raw).await;
        f.state.db.collection::<bson::Document>(KEYS).update_one(
            doc! {"_id":primary.api_key_id.as_deref().unwrap()},
            doc! {"$set":{"scopes":"proxy"}}).await.unwrap();
        let (_, secret) = child(&f, primary.api_key_id.as_deref().unwrap()).await;
        let agent = crate::services::assistant_team_service::agent_for_conversation(&f.state.db, &f.row).await.unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let response = app.clone().oneshot(Request::builder()
            .uri(format!("/api/v1/proxy/nyxbot-voice/{service_id}/voice/agents"))
            .header("authorization", format!("Bearer {secret}"))
            .body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 128*1024).await.unwrap();
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["agents"].as_array().unwrap().len(), 1);
        assert_eq!(body["agents"][0]["agent_id"], agent.id);
        assert_eq!(body["agents"][0]["service_id"], service_id);
        let response = app.clone().oneshot(Request::builder().method("POST")
            .uri(format!("/api/v1/proxy/nyxbot-voice/{service_id}/voice/sessions/{}/control", Uuid::new_v4()))
            .header("authorization", format!("Bearer {secret}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"command_id":Uuid::new_v4(),
                "expected_revision":0,"action":"end"}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), 404, "Proxy-only credential must reach the voice handler");
        let root_agent = crate::services::assistant_team_service::agent_for_conversation(&f.state.db,
            &crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.nyxbot_thread).await.unwrap()).await.unwrap();
        for (method, path, data) in [
            ("GET", format!("voice/options?conversation_id={}", f.nyxbot_thread), serde_json::Value::Null),
            ("POST", format!("conversations/{}/voice-sessions", f.nyxbot_thread), serde_json::json!({
                "client_request_id":Uuid::new_v4(),"preferences":platform(&voice),"sdp_offer":"fixture"})),
            ("PUT", format!("agents/{}/voice-service", agent.id), serde_json::Value::Null),
            ("GET", "dictation/options".into(), serde_json::Value::Null),
        ] {
            let response = app.clone().oneshot(Request::builder().method(method)
                .uri(format!("/api/v1/assistant/nyxagent/{path}"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(data.to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(), 403, "{method} {path}");
        }
        // A connection for another Bot, published later, still needs a Key grant.
        let other = super::binding::publish(&f.state.db, &f.owner, &root_agent.id, &f.state.config.base_url).await.unwrap();
        assert_ne!(service_id, other.id);
        assert!(keys::authorize(&f.state.db, &auth(&f, &secret).await, &f.nyxbot_thread).await.is_err());
        f.state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn voice_publication_is_idempotent_revocable_and_cannot_retarget_a_grant() {
    Box::pin(async {
        let (f, voice, service_id, raw) = setup("voice_bound_publication").await;
        let agent =
            crate::services::assistant_team_service::agent_for_conversation(&f.state.db, &f.row)
                .await
                .unwrap();
        let same =
            super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url)
                .await
                .unwrap();
        assert_eq!(same.id, service_id);
        assert_ne!(same.source.as_deref(), Some("auto_provision"));
        let proof = keys::authorize(&f.state.db, &auth(&f, &raw).await, &f.row.id)
            .await
            .unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &Uuid::new_v4().to_string(),
            platform(&voice),
            "fixture".into(),
            "worker",
            proof,
        )
        .await
        .unwrap();
        assert!(
            crate::services::user_service_service::ensure_service_fields_editable(
                &same,
                &[("identity", true)]
            )
            .is_err()
        );
        assert!(
            crate::services::user_service_service::ensure_user_managed_endpoint(
                &f.state.db,
                &f.owner,
                &same.endpoint_id
            )
            .await
            .is_err()
        );
        crate::services::user_service_service::deactivate_user_service(
            &f.state.db,
            &f.owner,
            &f.owner,
            &service_id,
        )
        .await
        .unwrap();
        assert!(keys::revalidate(&f.state.db, &call).await.is_err());
        let replacement =
            super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url)
                .await
                .unwrap();
        assert_eq!(
            replacement.id, service_id,
            "Disable preserves the exact connection"
        );
        assert!(!replacement.is_active);
        crate::services::user_endpoint_service::delete_service_endpoint(
            &f.state.db,
            &f.owner,
            &replacement.endpoint_id,
            std::slice::from_ref(&service_id),
        )
        .await
        .unwrap();
        let replacement =
            super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url)
                .await
                .unwrap();
        assert_ne!(
            replacement.id, service_id,
            "Delete and republish creates fresh authority"
        );
        assert!(
            keys::authorize(&f.state.db, &auth(&f, &raw).await, &f.row.id)
                .await
                .is_err()
        );
        // Normal all-services semantics are retained; explicit service selection is
        // the existing way to isolate a board's Key from future connections.
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(
                doc! {"_id":auth(&f,&raw).await.api_key_id.unwrap()},
                doc! {"$set":{"allow_all_services":true}},
            )
            .await
            .unwrap();
        assert!(
            keys::authorize(&f.state.db, &auth(&f, &raw).await, &f.row.id)
                .await
                .is_ok()
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn generic_voice_connection_and_auto_connected_scope_delegate_no_bot() {
    Box::pin(async {
        let (f, _, service, raw) = setup("voice_unbound_denied").await;
        f.state.db.collection::<bson::Document>("user_services").update_one(doc! {"_id":&service},
            doc! {"$unset":{"source":"","source_id":""},"$set":{"slug":"nyxbot-voice"}}).await.unwrap();
        assert!(keys::authorize(&f.state.db, &auth(&f,&raw).await, &f.row.id).await.is_err());
        let agent = crate::services::assistant_team_service::agent_for_conversation(&f.state.db, &f.row).await.unwrap();
        super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url).await.unwrap();
        let key = auth(&f, &raw).await.api_key_id.unwrap();
        f.state.db.collection::<bson::Document>(KEYS).update_one(doc! {"_id":key}, doc! {"$set":{
            "allowed_service_ids":[],"allow_all_services":false,"allow_auto_connected_services":true}}).await.unwrap();
        assert!(keys::authorize(&f.state.db, &auth(&f,&raw).await, &f.row.id).await.is_err());
        f.state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn voice_connection_preserves_catalog_operations_without_service_approval() {
    Box::pin(async {
        let (f, _, service, raw) = setup("voice_catalog_without_approval").await;
        let a = auth(&f, &raw).await;
        let identity = super::execution::resolve(&f.state.db, &a, &service)
            .await
            .unwrap();
        let catalog = identity.service.catalog_service_id.as_deref().unwrap();
        crate::services::approval_service::set_service_approval_config(
            &f.state.db,
            &f.owner,
            catalog,
            "Bot voice",
            Some(true),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        // Generic service approval still requires a decision. Voice forwarding
        // uses the exact service grant and the Bot's own realtime authority.
        let descriptor = crate::services::operation_descriptor::build_http_descriptor(
            "POST",
            "/voice/sessions",
            None,
        );
        let (outcome, _) = crate::services::approval_service::evaluate_for_chat(
            &f.state.db,
            &f.owner,
            &f.owner,
            catalog,
            &descriptor,
            Some("api_key"),
            &a.approval_requester_id(),
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            outcome,
            crate::services::approval_service::ApprovalOutcome::NeedsApproval(_)
        ));
        keys::authorize(&f.state.db, &a, &f.row.id).await.unwrap();
        f.state
            .db
            .collection::<bson::Document>("downstream_services")
            .update_one(
                doc! {"_id":catalog},
                doc! {"$set":{"proxy_operation_policy":{"rules":[
                {"method":"GET","path_template":"/voice/agents"}]}}},
            )
            .await
            .unwrap();
        keys::operation(&f.state.db, &a, &f.row.id, "GET", "/voice/agents", false)
            .await
            .unwrap();
        assert!(keys::authorize(&f.state.db, &a, &f.row.id).await.is_err());
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>("approval_requests")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_notifications_keep_exact_child_and_live_grant_authority() {
    Box::pin(async {
        let (f, voice, service, raw) = setup("voice_notice_child").await;
        let parent_auth = auth(&f, &raw).await;
        let parent = parent_auth.api_key_id.as_deref().unwrap();
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(doc! {"_id":parent}, doc! {"$set":{"scopes":"proxy"}})
            .await
            .unwrap();
        let (child_id, secret) = child(&f, parent).await;
        let (_, sibling) = child(&f, parent).await;
        let owner = auth(&f, &secret).await;
        let binding = keys::authorize(&f.state.db, &owner, &f.row.id)
            .await
            .unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &Uuid::new_v4().to_string(),
            platform(&voice),
            "fixture".into(),
            "worker",
            binding,
        )
        .await
        .unwrap();
        let call = session::write(
            &f.state.db,
            &call,
            doc! {"state":"active","protocol":"openai_live"},
        )
        .await
        .unwrap();
        assert!(
            keys::own_call(&f.state.db, &parent_auth, &call)
                .await
                .is_err()
        );
        assert!(
            keys::own_call(&f.state.db, &auth(&f, &sibling).await, &call)
                .await
                .is_err()
        );
        assert!(
            keys::own_call(&f.state.db, &auth(&f, &secret).await, &call)
                .await
                .is_ok()
        );
        let id = Uuid::new_v4().to_string();
        super::announcements::enqueue(&f.state.db, &call, &id, "hello")
            .await
            .unwrap();
        // Exercise the real HTTP middleware with the device's proxy-only scope.
        // A service execution route must not accidentally require management write.
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        for (credential, text, status) in [
            (secret.as_str(), "hello", 200),
            (secret.as_str(), "changed", 409),
            (raw.as_str(), "hello", 403),
            (sibling.as_str(), "hello", 403),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!(
                            "/api/v1/proxy/nyxbot-voice/{service}/voice/sessions/{}/announcements",
                            call.id
                        ))
                        .header("authorization", format!("Bearer {credential}"))
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({"request_id":id,"text":text}).to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
        f.state
            .db
            .collection::<bson::Document>("api_key_credentials")
            .update_one(doc! {"_id":child_id}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        assert!(
            super::announcements::claim(&f.state.db, &call)
                .await
                .is_err()
        );
        assert!(
            super::announcements::enqueue(&f.state.db, &call, &Uuid::new_v4().to_string(), "late")
                .await
                .is_err()
        );
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_voice::REQUESTS)
                .count_documents(doc! {"session_id":&call.id})
                .await
                .unwrap(),
            0
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_discovery_and_start_admission_need_only_live_exact_grants() {
    Box::pin(async {
        let (f, voice, service, raw) = setup("voice_default_grant_admission").await;
        f.state
            .db
            .collection::<bson::Document>("notification_channels")
            .update_one(
                doc! {"user_id":&f.owner},
                doc! {"$unset":{"approval_required":""}},
            )
            .await
            .unwrap();
        let a = auth(&f, &raw).await;
        let discovery = super::application::agents(&f.state, &a, None)
            .await
            .unwrap();
        assert_eq!(discovery["agents"].as_array().unwrap().len(), 1);
        assert_eq!(discovery["agents"][0]["service_id"], service);
        let binding = keys::authorize(&f.state.db, &a, &f.row.id).await.unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &Uuid::new_v4().to_string(),
            platform(&voice),
            "fixture".into(),
            "worker",
            binding,
        )
        .await
        .unwrap();
        keys::revalidate(&f.state.db, &call).await.unwrap();
        // Discovery is read-only and does not also require the start operation.
        let identity = super::execution::resolve(&f.state.db, &a, &service)
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>("downstream_services")
            .update_one(
                doc! {"_id":identity.service.catalog_service_id.as_deref().unwrap()},
                doc! {"$set":{"proxy_operation_policy":{"rules":[
                {"method":"GET","path_template":"/voice/agents"}]}}},
            )
            .await
            .unwrap();
        let read_discovery = super::application::agents(&f.state, &a, None)
            .await
            .unwrap();
        assert_eq!(read_discovery["agents"].as_array().unwrap().len(), 1);
        assert!(keys::authorize(&f.state.db, &a, &f.row.id).await.is_err());
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>("approval_requests")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        // Live operation restrictions also supersede a stale extractor.
        f.state.db.collection::<bson::Document>(KEYS).update_one(
            doc! {"_id":a.api_key_id.as_deref().unwrap()},
            doc! {"$set":{"assistant_operation_scopes":{&service:{"revision":1,"operations":[]}}}},
        ).await.unwrap();
        assert!(
            keys::operation(&f.state.db, &a, &f.row.id, "GET", "/voice/agents", false)
                .await
                .is_err()
        );
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(
                doc! {"_id":a.api_key_id.as_deref().unwrap()},
                doc! {"$set":{"assistant_operation_scopes":{}}},
            )
            .await
            .unwrap();
        // A stale extractor cannot preserve a removed exact service grant.
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(
                doc! {"_id":a.api_key_id.as_deref().unwrap()},
                doc! {"$set":{"allowed_service_ids":[],"allow_all_services":false}},
            )
            .await
            .unwrap();
        assert!(keys::resolve(&f.state.db, &a, &f.row.id).await.is_err());
        assert!(
            super::application::agents(&f.state, &a, None)
                .await
                .is_err()
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_granted_child_runtime_controls_and_announcements_ignore_service_approval() {
    Box::pin(async {
        use axum::{Json, Router, body::{Body, to_bytes}, extract::{WebSocketUpgrade, ws::Message},
            http::Request, routing::{get, post}};
        use serde_json::{Value, json};
        use tower::ServiceExt;
        let (f, voice, service, raw) = setup("voice_no_operation_approval").await;
        crate::services::execution_permission_dispatch::phase_b_tests::enroll(&f.state.db, &f.owner).await;
        f.state.db.collection::<bson::Document>("notification_channels").update_one(
            doc! {"user_id":&f.owner}, doc! {"$set":{"approval_required":true}},
        ).await.unwrap();
        let parent = auth(&f, &raw).await.api_key_id.unwrap();
        f.state.db.collection::<bson::Document>(KEYS).update_one(
            doc! {"_id":&parent}, doc! {"$set":{"scopes":"proxy"}},
        ).await.unwrap();
        let (_, secret) = child(&f, &parent).await;
        let a = auth(&f, &secret).await;
        let identity = super::execution::resolve(&f.state.db, &a, &service).await.unwrap();
        crate::services::approval_service::set_service_approval_config(
            &f.state.db, &f.owner, identity.service.catalog_service_id.as_deref().unwrap(),
            "Voice", None, None, None,
            Some(&crate::models::service_approval_config::ApprovalEffect::RequireApproval),
        ).await.unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        for uri in [format!("/api/v1/proxy/nyxbot-voice/{service}/voice/agents"),
            format!("/api/v1/proxy/nyxbot-voice/{service}/voice/options?conversation_id={}", f.row.id)] {
            let response = app.clone().oneshot(Request::builder().uri(&uri)
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty()).unwrap()).await.unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 128*1024).await.unwrap();
            assert_eq!(status, 200, "{uri}: {}", String::from_utf8_lossy(&bytes));
        }
        // The HTTP service entry reaches the existing transport validator
        // under restrictive service-approval settings, without provider I/O.
        {
            let uri = format!("/api/v1/proxy/nyxbot-voice/{service}/voice/sessions");
            let response = app.clone().oneshot(Request::builder().method("POST").uri(uri)
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"service_id":service,"client_request_id":Uuid::new_v4(),
                    "preferences":platform(&voice),"sdp_offer":"invalid"}).to_string())).unwrap())
                .await.unwrap();
            let status=response.status(); let bytes=to_bytes(response.into_body(),128*1024).await.unwrap();
            assert_eq!(status, 400, "{}", String::from_utf8_lossy(&bytes));
        }
        // Use the production admission and runtime with a loopback provider;
        // fixture injection is test-only and sends no request to a real provider.
        let binding = keys::authorize(&f.state.db, &a, &f.row.id).await.unwrap();
        let resolved = super::credentials::resolve(&f.state, &f.owner, &f.row.id, &platform(&voice))
            .await.unwrap();
        let provider_app = Router::new().route("/v1/live/sessions", post(|| async {
            Json(json!({"session":{"id":"live_key"},"transport":{"sdp":"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"}}))
        })).route("/v1/live/sessions/live_key/attach", get(|ws:WebSocketUpgrade| async move {
            ws.on_upgrade(|mut socket| async move {
                while let Some(Ok(Message::Text(text))) = socket.recv().await {
                    let event:Value = serde_json::from_str(&text).unwrap();
                    if event["type"] == "session.commentary.append" {
                        socket.send(Message::Text(json!({"type":"session.commentary.appended",
                            "client_event_id":event["event_id"]}).to_string().into())).await.unwrap();
                    }
                    if event["type"] == "session.close" {
                        socket.send(Message::Text(json!({"type":"session.closed","usage":{"seconds":4}})
                            .to_string().into())).await.unwrap();
                        break;
                    }
                }
            })
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, provider_app).await.unwrap() });
        let started = super::runtime::start_with_provider(&f.state, super::runtime::StartInput { execution_enrolled: true, responses_billing: None,
            client_key:binding, user:&f.owner, conversation:&f.row.id,
            client_request_id:&Uuid::new_v4().to_string(), preferences:platform(&voice),
            sdp:"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
        }, resolved.identity, resolved.billing, super::openai::OpenAi::fixture(resolved.key,address), None)
            .await.unwrap();
        assert!(!started.session.execution_enrolled, "Device OpenAI voice cannot inherit owner permissions");
        let notice_id = Uuid::new_v4().to_string();
        for (path, data) in [
            (format!("/api/v1/proxy/nyxbot-voice/{service}/voice/sessions/{}/control", started.session.id),
                json!({"command_id":Uuid::new_v4(),"expected_revision":0,"action":"mute"})),
            (format!("/api/v1/proxy/nyxbot-voice/{service}/voice/sessions/{}/announcements", started.session.id),
                json!({"request_id":notice_id,"text":"hello"})),
        ] {
            let response = app.clone().oneshot(Request::builder().method("POST").uri(path)
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(data.to_string())).unwrap()).await.unwrap();
            let status=response.status(); let bytes=to_bytes(response.into_body(),128*1024).await.unwrap();
            assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let notice = f.state.db.collection::<super::announcements::Announcement>(super::announcements::COLLECTION)
                    .find_one(doc! {"session_id":&started.session.id,"request_id":&notice_id}).await.unwrap().unwrap();
                if notice.state == "accepted" { break; }
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        }).await.expect("the sideband must dispatch without a service approval");
        let response = app.oneshot(Request::builder().method("POST")
            .uri(format!("/api/v1/proxy/nyxbot-voice/{service}/voice/sessions/{}/control", started.session.id))
            .header("authorization", format!("Bearer {secret}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({"command_id":Uuid::new_v4(),"expected_revision":1,"action":"end"})
                .to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), 200);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let call = session::get(&f.state.db, &f.owner, &f.row.id, &started.session.id).await.unwrap();
                if !call.live_slot { assert!(call.billing_finalized && call.final_usage_confirmed); break; }
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        }).await.unwrap();
        assert_eq!(f.state.db.collection::<bson::Document>("approval_requests")
            .count_documents(doc! {}).await.unwrap(), 0);
        server.abort();
        f.state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn voice_notification_dispatch_rechecks_its_own_catalog_operation_without_approval() {
    Box::pin(async {
        let (f, voice, service, raw) = setup("voice_notice_live_operation").await;
        let a = auth(&f, &raw).await;
        let proof = keys::authorize(&f.state.db, &a, &f.row.id).await.unwrap();
        let call = session::admit_for_client_key(&f.state.db, &f.owner, &f.row.id,
            &Uuid::new_v4().to_string(), platform(&voice), "fixture".into(), "worker", proof).await.unwrap();
        let call = session::write(&f.state.db, &call, doc! {"state":"active","protocol":"openai_live"}).await.unwrap();
        let notice = super::announcements::enqueue(&f.state.db,&call,&Uuid::new_v4().to_string(),"queued").await.unwrap();
        let identity = super::execution::resolve(&f.state.db,&a,&service).await.unwrap();
        f.state.db.collection::<bson::Document>("downstream_services").update_one(
            doc! {"_id":identity.service.catalog_service_id.as_deref().unwrap()},
            doc! {"$set":{"proxy_operation_policy":{"rules":[{"method":"POST","path_template":"/voice/sessions"}]}}}).await.unwrap();
        keys::revalidate(&f.state.db,&call).await.unwrap();
        assert!(super::announcements::claim(&f.state.db,&call).await.unwrap().is_none());
        let stored = f.state.db.collection::<super::announcements::Announcement>(super::announcements::COLLECTION)
            .find_one(doc! {"_id":notice.id}).await.unwrap().unwrap();
        assert_eq!(stored.state,"cancelled");
        f.state.db.collection::<bson::Document>("downstream_services").update_one(
            doc! {"_id":identity.service.catalog_service_id.as_deref().unwrap()},
            doc! {"$unset":{"proxy_operation_policy":""}}).await.unwrap();
        let notice = super::announcements::enqueue(&f.state.db,&call,&Uuid::new_v4().to_string(),"second").await.unwrap();
        use crate::models::service_approval_config::{ApprovalRule,ApprovalEffect};
        let deny: ApprovalRule = bson::from_document(doc! {
            "methods": ["POST"],
            "resource_pattern": "/voice/sessions/*/announcements",
            "effect": "deny",
        }).unwrap();
        crate::services::approval_service::set_service_approval_config(&f.state.db,&f.owner,
            identity.service.catalog_service_id.as_deref().unwrap(),"Voice",None,None,
            Some(&[deny]),Some(&ApprovalEffect::AutoAllow)).await.unwrap();
        keys::revalidate(&f.state.db,&call).await.unwrap();
        assert!(super::announcements::claim(&f.state.db,&call).await.unwrap().is_none());
        let stored = f.state.db.collection::<super::announcements::Announcement>(super::announcements::COLLECTION)
            .find_one(doc! {"_id":notice.id}).await.unwrap().unwrap();
        assert_eq!(stored.state, "cancelled");
        assert_eq!(f.state.db.collection::<bson::Document>("approval_requests")
            .count_documents(doc! {}).await.unwrap(), 0);
        f.state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn voice_snapshot_concurrent_authentication_retries_without_false_denial() {
    Box::pin(async {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let (f, voice, _, raw) = setup("voice_snapshot_auth_retry").await;
        let a = auth(&f, &raw).await;
        let proof = keys::authorize(&f.state.db, &a, &f.row.id).await.unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &Uuid::new_v4().to_string(),
            platform(&voice),
            "fixture".into(),
            "worker",
            proof,
        )
        .await
        .unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        let db = f.state.db.clone();
        let mut tx = db.client().start_session().await.unwrap();
        tx.start_transaction()
            .and_run2(async move |tx| {
                let result: crate::errors::AppResult<()> = Box::pin(async {
                    db.collection::<bson::Document>(KEYS)
                        .find_one(doc! {"_id":a.api_key_id.as_deref().unwrap()})
                        .session(&mut *tx)
                        .await?;
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        // A real sibling authentication advances last_used_at/state_version
                        // after this transaction has established its older snapshot.
                        key_service::validate_api_key(&db, &raw).await?;
                    }
                    keys::fence_call(&db, &call, tx).await?;
                    db.collection::<bson::Document>("review_effects")
                        .insert_one(doc! {"_id":"one"})
                        .session(&mut *tx)
                        .await?;
                    Ok(())
                })
                .await;
                crate::services::api_key_mutation_service::transaction_result(result)
            })
            .await
            .unwrap();
        assert!(
            observed.load(Ordering::SeqCst) >= 2,
            "The driver must retry the stale snapshot"
        );
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>("review_effects")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_snapshot_concurrent_revocation_prevents_the_effect_after_retry() {
    Box::pin(async {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let (f, voice, _, raw) = setup("voice_snapshot_revoke_retry").await;
        let a = auth(&f, &raw).await;
        let proof = keys::authorize(&f.state.db, &a, &f.row.id).await.unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &Uuid::new_v4().to_string(),
            platform(&voice),
            "fixture".into(),
            "worker",
            proof,
        )
        .await
        .unwrap();
        let first = Arc::new(AtomicBool::new(true));
        let db = f.state.db.clone();
        let mut tx = db.client().start_session().await.unwrap();
        let result = tx
            .start_transaction()
            .and_run2(async move |tx| {
                let result: crate::errors::AppResult<()> = Box::pin(async {
                    db.collection::<bson::Document>(KEYS)
                        .find_one(doc! {"_id":a.api_key_id.as_deref().unwrap()})
                        .session(&mut *tx)
                        .await?;
                    if first.swap(false, Ordering::SeqCst) {
                        db.collection::<bson::Document>(KEYS)
                            .update_one(
                                doc! {"_id":a.api_key_id.as_deref().unwrap()},
                                doc! {"$set":{"is_active":false}},
                            )
                            .await?;
                    }
                    keys::fence_call(&db, &call, tx).await?;
                    db.collection::<bson::Document>("review_effects")
                        .insert_one(doc! {"_id":"forbidden"})
                        .session(&mut *tx)
                        .await?;
                    Ok(())
                })
                .await;
                crate::services::api_key_mutation_service::transaction_result(result)
            })
            .await;
        let error =
            crate::services::api_key_mutation_service::map_transaction_error(result.unwrap_err());
        assert!(crate::services::voice::client_key::permanent_authority_failure(&error));
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>("review_effects")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[test]
fn voice_authority_classification_preserves_uncertain_work() {
    use crate::errors::AppError;
    for error in [
        AppError::Unauthorized("expired".into()),
        AppError::ApiKeyScopeForbidden("grant removed".into()),
        AppError::NotFound("source removed".into()),
        AppError::Forbidden("disabled".into()),
    ] {
        assert!(crate::services::voice::client_key::permanent_authority_failure(&error));
    }
    for error in [
        AppError::Internal("temporary".into()),
        AppError::OAuthAuthorityUnavailable,
        AppError::VoiceProviderUnavailable,
        AppError::ApprovalRequired {
            request_id: "pending".into(),
        },
    ] {
        assert!(!crate::services::voice::client_key::permanent_authority_failure(&error));
    }
}

#[tokio::test]
async fn voice_service_preserves_exact_binding_and_ignores_retargeting_input() {
    Box::pin(async {
        use axum::{body::{Body,to_bytes},http::Request};
        use tower::ServiceExt;
        let (f,voice,service,raw)=setup("voice_internal_proxy_binding").await;
        let a=auth(&f,&raw).await;
        let identity=super::execution::resolve(&f.state.db,&a,&service).await.unwrap();
        let primary=crate::services::assistant_nyxagent::get(&f.state.db,&f.owner,&f.nyxbot_thread).await.unwrap();
        let other_agent=crate::services::assistant_team_service::agent_for_conversation(&f.state.db,&primary).await.unwrap();
        let other=super::binding::publish(&f.state.db,&f.owner,&other_agent.id, &f.state.config.base_url).await.unwrap();
        f.state.db.collection::<bson::Document>(KEYS).update_one(doc! {"_id":a.api_key_id.unwrap()},
            doc! {"$set":{"allowed_service_ids":[&service,&other.id]}}).await.unwrap();
        let (_,private)=crate::routes::build_router_with_state(f.state.clone());let app=private.with_state(f.state.clone());
        // Publication records a real, exact connection URL on this deployment.
        let endpoint=f.state.db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
            .find_one(doc! {"_id":&identity.service.endpoint_id}).await.unwrap().unwrap();
        assert_eq!(endpoint.url,super::execution::service_url(&f.state.config.base_url,&service).unwrap());
        let response=app.clone().oneshot(Request::builder()
            .uri(format!("/api/v1/proxy/nyxbot-voice/{}/voice/agents",service))
            .header("authorization",format!("Bearer {raw}")).body(Body::empty()).unwrap()).await.unwrap();
        let status=response.status();let bytes=to_bytes(response.into_body(),128*1024).await.unwrap();
        assert_eq!(status,200,"{}",String::from_utf8_lossy(&bytes));
        let value:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["agents"].as_array().unwrap().len(),1);
        assert_eq!(value["agents"][0]["agent_id"],identity.agent.id);
        for uri in [format!("/api/v1/proxy/nyxbot-voice/{}/voice/sessions",service),
            format!("/api/v1/assistant/nyxagent/conversations/{}/voice-sessions", f.row.id)] {
            let response=app.clone().oneshot(Request::builder().method("POST").uri(uri)
                .header("authorization",format!("Bearer {raw}")).header("content-type","application/json")
                .body(Body::from(serde_json::json!({"service_id":service,"agent_id":other_agent.id,
                    "client_request_id":Uuid::new_v4(),"preferences":platform(&voice),"sdp_offer":"fixture"}).to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(),403,"The body cannot retarget the authorized connection");
        }
        let response = app.clone().oneshot(Request::builder().method("POST")
            .uri("/api/v1/assistant/nyxagent/voice/sessions")
            .header("authorization", format!("Bearer {raw}"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), 404, "The retired device alias must not be mounted");
        for body in ["[]", "null", "true", "42", "\"invalid\"", "{}"] {
            let response = app.clone().oneshot(Request::builder().method("POST")
                .uri(format!("/api/v1/proxy/nyxbot-voice/{}/voice/sessions", service))
                .header("authorization", format!("Bearer {raw}"))
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()).await.unwrap();
            assert_eq!(response.status(), 400, "Invalid start body must be rejected: {body}");
        }
        f.state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn voice_device_context_and_concurrency_retain_the_browser_contract() {
    Box::pin(async {
        let (f, voice, service, raw) = setup("voice_context_concurrency").await;
        let a = auth(&f, &raw).await;
        let agent = super::execution::resolve(&f.state.db, &a, &service)
            .await
            .unwrap()
            .agent;
        let first = crate::services::assistant_team_service::home_thread(
            &f.state.db,
            &f.state.encryption_keys,
            &agent,
        )
        .await
        .unwrap();
        let second = crate::services::assistant_team_service::home_thread(
            &f.state.db,
            &f.state.encryption_keys,
            &agent,
        )
        .await
        .unwrap();
        assert_eq!(
            first.id, second.id,
            "Devices and browser explicitly share the Bot home history"
        );
        let proof = keys::authorize(&f.state.db, &a, &first.id).await.unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.owner,
            &first.id,
            &Uuid::new_v4().to_string(),
            platform(&voice),
            "fixture".into(),
            "worker",
            proof,
        )
        .await
        .unwrap();
        assert!(
            session::admit(
                &f.state.db,
                &f.owner,
                &f.row.id,
                &Uuid::new_v4().to_string(),
                platform(&voice),
                "fixture".into(),
                "browser"
            )
            .await
            .is_err(),
            "The live slot is per person across devices and browser"
        );
        let options = crate::services::assistant_voice::options(
            &f.state.db,
            &f.state.encryption_keys,
            &first,
        )
        .await
        .unwrap();
        let options = serde_json::to_value(options).unwrap();
        assert!(
            options
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["announcements"] == true)
        );
        let dictation = crate::services::assistant_voice::dictation_options(
            &f.state.db,
            &f.state.encryption_keys,
            &first,
        )
        .await
        .unwrap();
        assert!(
            serde_json::to_value(dictation)
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["announcements"] == false)
        );
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(
                doc! {"_id":a.api_key_id.as_deref().unwrap()},
                doc! {"$set":{"is_active":false}},
            )
            .await
            .unwrap();
        keys::own_call(
            &f.state.db,
            &crate::test_utils::test_auth_user(&f.owner),
            &call,
        )
        .await
        .unwrap();
        let ended = session::control(
            &f.state.db,
            &f.owner,
            &first.id,
            &call.id,
            &Uuid::new_v4().to_string(),
            call.control_revision,
            "end",
        )
        .await
        .unwrap();
        assert!(
            ended.end_requested,
            "Human owner retains control after device revocation"
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_concurrent_publication_retains_one_live_binding_and_fresh_identity_after_delete() {
    Box::pin(async {
        let (f, _, service, raw) = setup("voice_publication_races").await;
        let agent =
            crate::services::assistant_team_service::agent_for_conversation(&f.state.db, &f.row)
                .await
                .unwrap();
        crate::services::user_service_service::deactivate_user_service(
            &f.state.db,
            &f.owner,
            &f.owner,
            &service,
        )
        .await
        .unwrap();
        let (left, right) = tokio::join!(
            super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url),
            super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url)
        );
        let old = left.unwrap();
        assert_eq!(old.id, service);
        assert_eq!(right.unwrap().id, service);
        crate::services::user_endpoint_service::delete_service_endpoint(
            &f.state.db,
            &f.owner,
            &old.endpoint_id,
            std::slice::from_ref(&service),
        )
        .await
        .unwrap();
        let (left, right) = tokio::join!(
            super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url),
            super::binding::publish(&f.state.db, &f.owner, &agent.id, &f.state.config.base_url)
        );
        let new = left.unwrap();
        assert_ne!(new.id, service);
        assert_eq!(new.id, right.unwrap().id);
        assert!(
            keys::resolve(&f.state.db, &auth(&f, &raw).await, &f.row.id)
                .await
                .is_err(),
            "Selected-service grants never transfer"
        );
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>("user_services")
                .count_documents(doc! {"source":super::access::BOUND_SOURCE,"source_id":&agent.id})
                .await
                .unwrap(),
            1
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[path = "deep_audit_tests.rs"]
mod deep_audit_tests;

#[path = "service_ingress_tests.rs"]
mod service_ingress_tests;

#[tokio::test]
async fn device_voice_cannot_inherit_enrolled_owner_permissions_or_task_root() {
    for global in [false, true] {
        use crate::services::{
            assistant_nyxagent as engine, assistant_voice as voice,
            execution_permission_dispatch::phase_b_tests,
        };
        Box::pin(async {
            for login_child in [false, true] {
                let (f, provider, _, raw) = setup("voice_device_execution_excluded").await;
                phase_b_tests::enroll_cohort(&f.state.db, &f.owner, global).await;
                let parent = auth(&f, &raw).await.api_key_id.unwrap();
                let secret = if login_child {
                    child(&f, &parent).await.1
                } else {
                    raw
                };
                let a = auth(&f, &secret).await;
                assert!(a.execution_enrolled, "The owner's enrollment is live");
                let caller = keys::caller(&f.state.db, &a).await.unwrap();
                assert!(!caller.auth.execution_enrolled);
                assert!(caller.auth.execution_permission.is_none());
                let binding = keys::authorize(&f.state.db, &a, &f.row.id).await.unwrap();
                assert!(binding.is_some());
                // Even an incorrectly propagated hint cannot enroll device audio.
                // Grok preparation creates the durable session without provider I/O.
                let mut call = super::grok_runtime::prepare(
                    &f.state,
                    &f.owner,
                    &f.row.id,
                    &Uuid::new_v4().to_string(),
                    platform(&provider),
                    "fixture".into(),
                    true,
                    binding,
                )
                .await
                .unwrap()
                .session;
                assert!(!call.execution_enrolled);
                let stored = session::get(&f.state.db, &f.owner, &f.row.id, &call.id)
                    .await
                    .unwrap();
                assert!(!stored.execution_enrolled);
                assert!(stored.client_key == call.client_key);
                // Task admission separately refuses to transfer enrollment, even
                // if the in-memory session hint is stale or incorrectly set.
                call.execution_enrolled = true;
                let segment = super::transcript::Segment {
                    fragments: Vec::new(),
                    id: Uuid::new_v4().to_string(),
                    speaker: super::transcript::Speaker::User,
                    text: "Check the calendar".into(),
                    start_ms: 0,
                    end_ms: 1000,
                    sealed: true,
                    complete: true,
                };
                super::transcript::persist(&f.state.db, &call, &segment)
                    .await
                    .unwrap();
                let request =
                    super::transcript::delegate(&f.state.db, &call, "device-request", &segment)
                        .await
                        .unwrap();
                assert!(!request.execution_enrolled);
                let request = voice::ensure_task_conversation(
                    &f.state.db,
                    &f.state.encryption_keys,
                    &request,
                )
                .await
                .unwrap();
                let mut start = engine::TurnStart::from(&engine::TurnRequest {
                    conversation_id: request.task_conversation_id.clone(),
                    text: segment.text,
                    model: None,
                    agent_id: None,
                    attachment_ids: Vec::new(),
                    access_mode: None,
                    acknowledgement_id: None,
                    retry_of: None,
                });
                start.execution_enrollment = request.execution_enrolled;
                let hidden = Box::pin(engine::begin_turn_with_voice(
                    &f.state.db,
                    &f.owner,
                    start,
                    &f.state.encryption_keys,
                    Some(&request.id),
                ))
                .await
                .unwrap();
                assert!(
                    hidden
                        .active_turn
                        .as_ref()
                        .unwrap()
                        .execution_task
                        .is_none()
                );
                let key =
                    key_service::get_api_key(&f.state.db, &f.owner, &hidden.credential_api_key_id)
                        .await
                        .unwrap();
                let task_auth =
                    crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
                        .await
                        .unwrap();
                assert!(task_auth.execution_permission.is_none());
                voice::cancel(&f.state.db, &f.owner, &f.row.id, &request.id)
                    .await
                    .unwrap();
                f.state.db.drop().await.unwrap();
            }
        })
        .await;
    }
}
