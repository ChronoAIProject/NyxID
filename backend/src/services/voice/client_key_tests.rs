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
    let agent =
        crate::services::assistant_team_service::agent_for_conversation(&f.state.db, &f.row)
            .await
            .unwrap();
    let id = super::binding::publish(&f.state.db, &f.owner, &agent.id)
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
        let other = super::binding::publish(&f.state.db, &f.owner, &primary_agent.id)
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
        keys::fence_request(&f.state.db, &f.owner, &request.id, &mut tx)
            .await
            .unwrap();
        tx.commit_transaction().await.unwrap();
        // Once action admission fences the child, a concurrent revocation must
        // retry after that admission commits rather than silently passing it.
        let mut admission = f.state.db.client().start_session().await.unwrap();
        admission.start_transaction().await.unwrap();
        keys::fence_request(&f.state.db, &f.owner, &request.id, &mut admission)
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
            keys::fence_request(&f.state.db, &f.owner, &request.id, &mut after_revocation)
                .await
                .is_err()
        );
        after_revocation.abort_transaction().await.unwrap();
        keys::end(&f.state.db, &call).await.unwrap();
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
            .uri("/api/v1/assistant/nyxagent/voice/agents")
            .header("authorization", format!("Bearer {secret}"))
            .body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 128*1024).await.unwrap();
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["agents"].as_array().unwrap().len(), 1);
        assert_eq!(body["agents"][0]["agent_id"], agent.id);
        let response = app.clone().oneshot(Request::builder().method("POST")
            .uri(format!("/api/v1/assistant/nyxagent/voice/sessions/{}/control", Uuid::new_v4()))
            .header("authorization", format!("Bearer {secret}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"command_id":Uuid::new_v4(),
                "expected_revision":0,"action":"end"}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), 404, "Proxy-only credential must reach the voice handler");
        let root_agent = crate::services::assistant_team_service::agent_for_conversation(&f.state.db,
            &crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.nyxbot_thread).await.unwrap()).await.unwrap();
        for (method, path, data) in [
            ("GET", format!("voice/options?conversation_id={}", f.nyxbot_thread), serde_json::Value::Null),
            ("POST", "voice/sessions".into(), serde_json::json!({"agent_id":root_agent.id,
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
        let other = super::binding::publish(&f.state.db, &f.owner, &root_agent.id).await.unwrap();
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
        let same = super::binding::publish(&f.state.db, &f.owner, &agent.id)
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
        let replacement = super::binding::publish(&f.state.db, &f.owner, &agent.id)
            .await
            .unwrap();
        assert_ne!(replacement.id, service_id);
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
        super::binding::publish(&f.state.db, &f.owner, &agent.id).await.unwrap();
        let key = auth(&f, &raw).await.api_key_id.unwrap();
        f.state.db.collection::<bson::Document>(KEYS).update_one(doc! {"_id":key}, doc! {"$set":{
            "allowed_service_ids":[],"allow_all_services":false,"allow_auto_connected_services":true}}).await.unwrap();
        assert!(keys::authorize(&f.state.db, &auth(&f,&raw).await, &f.row.id).await.is_err());
        f.state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn voice_connection_preserves_catalog_operations_and_existing_approval() {
    Box::pin(async {
        let (f, _, service, raw) = setup("voice_catalog_and_approval").await;
        let connection = f
            .state
            .db
            .collection::<crate::models::user_service::UserService>(
                crate::models::user_service::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&service})
            .await
            .unwrap()
            .unwrap();
        // The public approval-config API normalizes catalog-backed connections
        // to this ID. A connection scope and its catalog approval are distinct.
        let catalog_id = connection.catalog_service_id.as_deref().unwrap();
        assert_ne!(catalog_id, service);
        assert!(
            keys::authorize(&f.state.db, &auth(&f, &raw).await, &f.row.id)
                .await
                .is_ok()
        );
        crate::services::approval_service::set_service_approval_config(
            &f.state.db,
            &f.owner,
            catalog_id,
            "Bot voice",
            Some(true),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            keys::authorize(&f.state.db, &auth(&f, &raw).await, &f.row.id)
                .await
                .is_err()
        );
        crate::services::approval_service::set_service_approval_config(
            &f.state.db,
            &f.owner,
            catalog_id,
            "Bot voice",
            Some(false),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            keys::authorize(&f.state.db, &auth(&f, &raw).await, &f.row.id)
                .await
                .is_ok()
        );
        f.state
            .db
            .collection::<bson::Document>("downstream_services")
            .update_one(
                doc! {"slug":super::access::SERVICE_SLUG},
                doc! {"$set":{"proxy_operation_policy":{"rules":[
                {"method":"GET","path_template":"/voice/agents"}]}}},
            )
            .await
            .unwrap();
        assert!(
            keys::authorize(&f.state.db, &auth(&f, &raw).await, &f.row.id)
                .await
                .is_err()
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_notifications_keep_exact_child_and_live_grant_authority() {
    Box::pin(async {
        let (f, voice, _, raw) = setup("voice_notice_child").await;
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
                            "/api/v1/assistant/nyxagent/voice/sessions/{}/announcements",
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
