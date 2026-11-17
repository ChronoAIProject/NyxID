//! Organization-owned device services retain personal call identity and live ACLs.
use super::{binding, client_key, credential_tests, execution, session};
use crate::{
    models::{assistant_agent::AssistantAgent, user_service::UserService},
    mw::auth::AuthUser,
    services::{feature_flag_service as flags, key_service, org_agent_tests::Fixture},
};
use axum::{
    body::{Body, to_bytes},
    extract::{Path, State},
    http::Request,
};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn fixture(
    name: &str,
) -> (
    Fixture,
    AssistantAgent,
    UserService,
    crate::models::downstream_service::DownstreamService,
) {
    let mut f = Fixture::new(name).await;
    let (agent, _) = f.create().await;
    let voice = credential_tests::configure(&mut f.state, &f.admin).await;
    flags::set_platform_override(
        &f.state.db,
        flags::VOICE_AGENT_KEY_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.admin,
    )
    .await
    .unwrap();
    super::catalog::seed(&f.state.db, &f.state.config.base_url)
        .await
        .unwrap();
    let published = crate::handlers::assistant_voice::publish_voice_service(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.admin),
        Path(agent.id.clone()),
    )
    .await
    .unwrap()
    .0;
    let service = binding::get(&f.state.db, &f.org, &agent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(published["service"]["id"], service.id);
    assert_eq!(service.user_id, f.org);
    (f, agent, service, voice)
}

async fn device(f: &Fixture, actor: &str, service: &str) -> (String, AuthUser) {
    let created = key_service::create_api_key_with_scope_authorization(
        &f.state.db,
        actor,
        Some(actor),
        "org-panel",
        "read proxy",
        None,
        None,
        Some(&[service.to_owned()]),
        None,
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let (_, key, child) = key_service::validate_api_key(&f.state.db, &created.full_key)
        .await
        .unwrap();
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, child, None, None)
        .await
        .unwrap();
    (created.full_key, auth)
}

async fn get(f: &Fixture, raw: &str, path: &str) -> (u16, Value) {
    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let response = private
        .with_state(f.state.clone())
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {raw}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"body":String::from_utf8_lossy(&bytes)})),
    )
}

#[tokio::test]
async fn org_device_voice_publishes_discovers_and_admits_actor_private_calls() {
    Box::pin(async {
        let (f, agent, service, voice) = fixture("org_device_voice_private").await;
        let (admin_raw, admin) = device(&f, &f.admin, &service.id).await;
        let (member_raw, member) = device(&f, &f.member, &service.id).await;
        let (status, inventory) = get(&f, &member_raw, "/api/v1/keys").await;
        assert_eq!(status, 200, "{inventory}");
        assert_eq!(inventory["keys"].as_array().unwrap().len(), 1);
        assert_eq!(inventory["keys"][0]["id"], service.id);
        let identity = execution::resolve(&f.state.db, &admin, &service.id)
            .await
            .unwrap();
        assert_eq!(identity.actor, f.admin);
        assert_eq!(identity.agent.user_id, f.org);
        let mut threads = Vec::new();
        for (raw, actor) in [(&admin_raw, &f.admin), (&member_raw, &f.member)] {
            let (status, body) = get(
                &f,
                raw,
                &format!("/api/v1/proxy/nyxbot-voice/{}/voice/agents", service.id),
            )
            .await;
            assert_eq!(status, 200, "{body}");
            assert_eq!(body["agents"].as_array().unwrap().len(), 1);
            assert_eq!(body["agents"][0]["agent_id"], agent.id);
            let id = body["agents"][0]["conversation_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let thread = crate::services::assistant_voice::thread(&f.state.db, actor, &id)
                .await
                .unwrap();
            assert_eq!(thread.agent_owner_id.as_deref(), Some(f.org.as_str()));
            let backend = f.key(&thread).await;
            assert_eq!(&backend.user_id, actor);
            assert_eq!(
                backend.assistant_agent_owner_id.as_deref(),
                Some(f.org.as_str())
            );
            let (status, body) = get(
                &f,
                raw,
                &format!("/api/v1/proxy/nyxbot-voice/{}/voice/options", service.id),
            )
            .await;
            assert_eq!(status, 200, "{body}");
            assert!(!body["options"].as_array().unwrap().is_empty(), "{body}");
            threads.push(id);
        }
        assert_ne!(threads[0], threads[1]);
        assert!(
            execution::conversation(&f.state.db, &identity, &threads[1])
                .await
                .is_err()
        );
        let proof = client_key::authorize(&f.state.db, &member, &threads[1])
            .await
            .unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.member,
            &threads[1],
            &Uuid::new_v4().to_string(),
            credential_tests::platform(&voice),
            "fixture".into(),
            "worker",
            proof,
        )
        .await
        .unwrap();
        assert_eq!(call.user_id, f.member);
        client_key::revalidate(&f.state.db, &call).await.unwrap();
        let identity = execution::resolve(&f.state.db, &member, &service.id)
            .await
            .unwrap();
        execution::call(&f.state.db, &identity, &call.id)
            .await
            .unwrap();
        let muted = session::control_for_caller(
            &f.state.db,
            &f.member,
            &call.conversation_id,
            &call.id,
            &Uuid::new_v4().to_string(),
            call.control_revision,
            "mute",
            true,
        )
        .await
        .unwrap();
        f.revoke(&f.member).await;
        assert!(
            execution::resolve(&f.state.db, &member, &service.id)
                .await
                .is_err()
        );
        assert!(client_key::revalidate(&f.state.db, &call).await.is_err());
        assert!(
            session::control_for_caller(
                &f.state.db,
                &f.member,
                &call.conversation_id,
                &call.id,
                &Uuid::new_v4().to_string(),
                muted.control_revision,
                "unmute",
                true
            )
            .await
            .is_err()
        );
        let (status, _) = get(
            &f,
            &member_raw,
            &format!("/api/v1/proxy/nyxbot-voice/{}/voice/agents", service.id),
        )
        .await;
        assert!(status == 403 || status == 404);
        // A different member's revocation does not remove the admin's binding.
        execution::resolve(&f.state.db, &admin, &service.id)
            .await
            .unwrap();
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn org_device_voice_publication_and_use_obey_roles_scopes_and_lifecycle() {
    Box::pin(async {
        let (f, agent, service, _) = fixture("org_device_voice_acl").await;
        for actor in [&f.member, &f.viewer, &f.outsider] {
            assert!(
                binding::publish(&f.state.db, actor, &agent.id, &f.state.config.base_url)
                    .await
                    .is_err()
            );
        }
        let (_, auth) = device(&f, &f.member, &service.id).await;
        let other = f.service(&f.org, "other").await;
        let (_, wrong) = device(&f, &f.member, &other).await;
        assert!(
            execution::resolve(&f.state.db, &wrong, &service.id)
                .await
                .is_err()
        );
        let memberships = f
            .state
            .db
            .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME);
        for role in ["viewer", "member"] {
            memberships
                .update_one(
                    doc! {"org_user_id":&f.org,"member_user_id":&f.member},
                    doc! {"$set":{"role":role,"allowed_service_ids":[]}},
                )
                .await
                .unwrap();
            assert!(
                execution::resolve(&f.state.db, &auth, &service.id)
                    .await
                    .is_err()
            );
        }
        memberships
            .update_one(
                doc! {"org_user_id":&f.org,"member_user_id":&f.admin},
                doc! {"$set":{"allowed_service_ids":[]}},
            )
            .await
            .unwrap();
        assert!(
            binding::publish(&f.state.db, &f.admin, &agent.id, &f.state.config.base_url)
                .await
                .is_err()
        );
        memberships
            .update_many(
                doc! {"org_user_id":&f.org},
                doc! {"$set":{"allowed_service_ids":bson::Bson::Null}},
            )
            .await
            .unwrap();
        execution::resolve(&f.state.db, &auth, &service.id)
            .await
            .unwrap();
        memberships
            .update_one(
                doc! {"org_user_id":&f.org,"member_user_id":&f.member},
                doc! {"$set":{"scope_source":"inherit"}},
            )
            .await
            .unwrap();
        crate::services::org_role_scope_service::set_scope(
            &f.state.db,
            &f.org,
            crate::models::org_membership::OrgRole::Member,
            Some(vec![]),
            &f.admin,
        )
        .await
        .unwrap();
        assert!(
            execution::resolve(&f.state.db, &auth, &service.id)
                .await
                .is_err()
        );
        crate::services::org_role_scope_service::clear_scope(
            &f.state.db,
            &f.org,
            crate::models::org_membership::OrgRole::Member,
        )
        .await
        .unwrap();
        execution::resolve(&f.state.db, &auth, &service.id)
            .await
            .unwrap();
        let services = f
            .state
            .db
            .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME);
        services
            .update_one(doc! {"_id":&service.id}, doc! {"$set":{"admin_only":true}})
            .await
            .unwrap();
        assert!(
            execution::resolve(&f.state.db, &auth, &service.id)
                .await
                .is_err()
        );
        let (_, admin) = device(&f, &f.admin, &service.id).await;
        execution::resolve(&f.state.db, &admin, &service.id)
            .await
            .unwrap();
        services
            .update_one(doc! {"_id":&service.id}, doc! {"$set":{"admin_only":false}})
            .await
            .unwrap();
        let agents = f
            .state
            .db
            .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME);
        agents
            .update_one(
                doc! {"_id":&agent.id},
                doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
            )
            .await
            .unwrap();
        assert!(
            execution::resolve(&f.state.db, &auth, &service.id)
                .await
                .is_err()
        );
        assert!(
            binding::publish(&f.state.db, &f.admin, &agent.id, &f.state.config.base_url)
                .await
                .is_err()
        );
        agents
            .update_one(
                doc! {"_id":&agent.id},
                doc! {"$set":{"destroyed_at":bson::Bson::Null}},
            )
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id":&f.org}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        assert!(
            execution::resolve(&f.state.db, &auth, &service.id)
                .await
                .is_err()
        );
        assert!(
            binding::publish(&f.state.db, &f.admin, &agent.id, &f.state.config.base_url)
                .await
                .is_err()
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}
