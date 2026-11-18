//! Device services retain the Key owner's identity and existing live ACLs.
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
    device_for(f, actor, actor, service).await
}

async fn device_for(f: &Fixture, owner: &str, actor: &str, service: &str) -> (String, AuthUser) {
    let created = key_service::create_api_key_with_scope_authorization(
        &f.state.db,
        owner,
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
    send(f, raw, "GET", path, Value::Null).await
}

async fn send(f: &Fixture, raw: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let response = private
        .with_state(f.state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {raw}"))
                .header("content-type", "application/json")
                .body(if body.is_null() {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
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

async fn publication(f: &Fixture, actor: &str, agent: &str) -> Value {
    crate::handlers::assistant_voice::voice_service(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(actor),
        Path(agent.to_owned()),
    )
    .await
    .unwrap()
    .0
}

#[tokio::test]
async fn org_device_voice_publication_capability_tracks_live_resource_authority() {
    Box::pin(async {
        let (f, agent, service, _) = fixture("org_device_voice_capability").await;
        let allowed = publication(&f, &f.admin, &agent.id).await;
        assert_eq!(allowed["can_publish"], true);
        assert_eq!(allowed["service"]["id"], service.id);
        for actor in [&f.member, &f.viewer] {
            let refused = publication(&f, actor, &agent.id).await;
            assert_eq!(refused, json!({"can_publish":false,"service":null}));
            assert!(matches!(
                crate::handlers::assistant_voice::publish_voice_service(
                    State(f.state.clone()),
                    crate::test_utils::test_auth_user(actor),
                    Path(agent.id.clone()),
                )
                .await,
                Err(crate::errors::AppError::Forbidden(_))
            ));
        }
        assert!(matches!(
            crate::handlers::assistant_voice::voice_service(
                State(f.state.clone()),
                crate::test_utils::test_auth_user(&f.outsider),
                Path(agent.id.clone()),
            )
            .await,
            Err(crate::errors::AppError::NotFound(_))
        ));
        // The admin label alone is insufficient: the effective resource scope
        // must include the connection, and revocation must affect the next read.
        let memberships = f
            .state
            .db
            .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME);
        let admin = doc! {"org_user_id":&f.org,"member_user_id":&f.admin};
        memberships
            .update_one(admin.clone(), doc! {"$set":{"allowed_service_ids":[]}})
            .await
            .unwrap();
        assert_eq!(
            publication(&f, &f.admin, &agent.id).await,
            json!({"can_publish":false,"service":null})
        );
        assert!(
            binding::publish(&f.state.db, &f.admin, &agent.id, &f.state.config.base_url)
                .await
                .is_err()
        );
        memberships
            .update_one(admin, doc! {"$set":{"allowed_service_ids":[&service.id]}})
            .await
            .unwrap();
        assert_eq!(
            publication(&f, &f.admin, &agent.id).await["can_publish"],
            true
        );
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&agent.id},
                doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
            )
            .await
            .unwrap();
        assert_eq!(
            publication(&f, &f.admin, &agent.id).await,
            json!({"can_publish":false,"service":null})
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
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
async fn org_device_voice_serializes_inherited_role_scope_changes() {
    use crate::{
        errors::AppError,
        models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole},
        services::org_role_scope_service::{clear_scope, get_scope, set_scope},
    };

    Box::pin(async {
        let (f, _, service, voice) = fixture("org_device_voice_scope_fence").await;
        let db = &f.state.db;
        let role = OrgRole::Member;
        db.collection::<bson::Document>(MEMBERSHIPS)
            .update_one(
                doc! {"org_user_id": &f.org, "member_user_id": &f.member},
                doc! {"$set": {"scope_source": "inherit"}},
            )
            .await
            .unwrap();
        let (_, key) = device(&f, &f.member, &service.id).await;
        let rows = super::application::agents(&f.state, &key, None)
            .await
            .unwrap();
        let conversation = rows["agents"][0]["conversation_id"].as_str().unwrap();
        let proof = client_key::authorize(db, &key, conversation).await.unwrap();
        let call = session::admit_for_client_key(
            db,
            &f.member,
            conversation,
            &Uuid::new_v4().to_string(),
            credential_tests::platform(&voice),
            "fixture".into(),
            "worker",
            proof,
        )
        .await
        .unwrap();

        // Cover the implicit full-access default and an existing explicit grant.
        for existing in [false, true] {
            if existing {
                set_scope(db, &f.org, role, Some(vec![service.id.clone()]), &f.admin)
                    .await
                    .unwrap();
            }
            let original = get_scope(db, &f.org, role)
                .await
                .unwrap()
                .map(|row| row.allowed_service_ids);
            assert_eq!(original.is_some(), existing);
            let mut control = db.client().start_session().await.unwrap();
            control.start_transaction().await.unwrap();
            client_key::fence_call(db, &call, &mut control)
                .await
                .unwrap();
            assert!(matches!(
                set_scope(db, &f.org, role, Some(vec![]), &f.admin).await,
                Err(AppError::Conflict(_))
            ));
            assert!(matches!(
                clear_scope(db, &f.org, role).await,
                Err(AppError::Conflict(_))
            ));
            control.commit_transaction().await.unwrap();
            assert_eq!(
                get_scope(db, &f.org, role)
                    .await
                    .unwrap()
                    .map(|row| row.allowed_service_ids),
                original
            );

            set_scope(db, &f.org, role, Some(vec![]), &f.admin)
                .await
                .unwrap();
            let mut control = db.client().start_session().await.unwrap();
            control.start_transaction().await.unwrap();
            assert!(
                client_key::fence_call(db, &call, &mut control)
                    .await
                    .is_err()
            );
            control.abort_transaction().await.unwrap();
            clear_scope(db, &f.org, role).await.unwrap();
        }
        db.drop().await.unwrap();
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

#[tokio::test]
async fn org_device_voice_org_keys_reuse_owner_identity_history_and_call_contract() {
    Box::pin(async {
        let (f, agent, service, voice) = fixture("org_device_voice_org_owner").await;
        let (raw, auth) = device_for(&f, &f.org, &f.admin, &service.id).await;
        let (other_raw, other) = device_for(&f, &f.org, &f.admin, &service.id).await;
        let (personal_raw, _) = device(&f, &f.admin, &service.id).await;
        let base = format!("/api/v1/proxy/nyxbot-voice/{}", service.id);
        let (status, inventory) = get(&f, &raw, "/api/v1/keys").await;
        assert_eq!(status, 200, "{inventory}");
        assert_eq!(inventory["keys"].as_array().unwrap().len(), 1);
        assert_eq!(inventory["keys"][0]["id"], service.id);
        let identity = execution::resolve(&f.state.db, &auth, &service.id)
            .await
            .unwrap();
        assert_eq!(identity.actor, f.org);
        assert_eq!(identity.agent.user_id, f.org);
        let home = crate::services::assistant_team_service::home_thread_for(
            &f.state.db, &f.state.encryption_keys, &f.org, &agent,
        ).await.unwrap();
        for key in [&raw, &other_raw] {
            let (status, body) = get(&f, key, &format!("{base}/voice/agents")).await;
            assert_eq!(status, 200, "{body}");
            assert_eq!(body["agents"].as_array().unwrap().len(), 1);
            assert_eq!(body["agents"][0]["conversation_id"], home.id);
        }
        let (status, personal) = get(&f, &personal_raw, &format!("{base}/voice/agents")).await;
        assert_eq!(status, 200, "{personal}");
        let personal_thread = personal["agents"][0]["conversation_id"].as_str().unwrap();
        assert_ne!(personal_thread, home.id);
        assert!(execution::conversation(&f.state.db, &identity, personal_thread).await.is_err());
        assert_eq!(home.user_id, f.org);
        let backend_key = f.key(&home).await;
        assert_eq!(backend_key.user_id, f.org);
        assert_eq!(backend_key.allowed_service_ids, agent.grants.service_ids);
        let (status, options) = get(&f, &raw, &format!("{base}/voice/options")).await;
        assert_eq!(status, 200, "{options}");
        assert!(!options["options"].as_array().unwrap().is_empty());
        let resolved = super::credentials::resolve(
            &f.state, &f.org, &home.id, &credential_tests::platform(&voice),
        ).await.unwrap();
        assert_eq!(resolved.billing.actor_user_id, f.org);
        assert_eq!(resolved.billing.billing_owner_id, f.org);
        // Production HTTP start reaches the existing offer validator, rather
        // than rejecting an organization principal or contacting a provider.
        let (status, body) = send(&f, &raw, "POST", &format!("{base}/voice/sessions"), json!({
            "client_request_id":Uuid::new_v4(),"preferences":credential_tests::platform(&voice),
            "sdp_offer":"invalid"
        })).await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(body["error"], "validation_error", "{body}");
        assert_eq!(body["details"]["stage"], "transport", "{body}");
        let proof = client_key::authorize(&f.state.db, &auth, &home.id).await.unwrap();
        let call = session::admit_for_client_key(
            &f.state.db, &f.org, &home.id, &Uuid::new_v4().to_string(),
            credential_tests::platform(&voice), "fixture".into(), "worker", proof,
        ).await.unwrap();
        assert_eq!(call.user_id, f.org);
        client_key::own_call(&f.state.db, &auth, &call).await.unwrap();
        assert!(client_key::own_call(&f.state.db, &other, &call).await.is_err());
        let (status, body) = get(&f, &raw, &format!("{base}/voice/sessions/{}", call.id)).await;
        assert_eq!(status, 200, "{body}");
        let (status, _) = get(&f, &other_raw, &format!("{base}/voice/sessions/{}", call.id)).await;
        assert_eq!(status, 403);
        let second = client_key::authorize(&f.state.db, &other, &home.id).await.unwrap();
        assert!(matches!(session::admit_for_client_key(
            &f.state.db, &f.org, &home.id, &Uuid::new_v4().to_string(),
            credential_tests::platform(&voice), "fixture".into(), "worker", second,
        ).await, Err(crate::errors::AppError::Conflict(_))));
        // A creator's membership is not an organization Key's authority.
        f.revoke(&f.admin).await;
        execution::resolve(&f.state.db, &auth, &service.id).await.unwrap();
        client_key::revalidate(&f.state.db, &call).await.unwrap();
        let (status, body) = send(&f, &raw, "POST", &format!("{base}/voice/sessions/{}/control", call.id), json!({
            "command_id":Uuid::new_v4(),"expected_revision":call.control_revision,"action":"mute"
        })).await;
        assert_eq!(status, 200, "{body}");
        let muted = session::get(&f.state.db, &f.org, &home.id, &call.id).await.unwrap();
        assert!(muted.desired_muted);
        let (status, _) = send(&f, &other_raw, "POST", &format!("{base}/voice/sessions/{}/control", call.id), json!({
            "command_id":Uuid::new_v4(),"expected_revision":muted.control_revision,"action":"end"
        })).await;
        assert_eq!(status, 403);
        session::close(&f.state.db, &muted, "fixture", true).await.unwrap();
        f.state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn org_device_voice_org_key_children_and_live_revocation_reuse_existing_fences() {
    Box::pin(async {
        let (f, _, service, voice) = fixture("org_device_voice_org_revocation").await;
        let (_, parent) = device_for(&f, &f.org, &f.admin, &service.id).await;
        let key_id = parent.api_key_id.as_deref().unwrap();
        let other_service = f.service(&f.org, "ungranted").await;
        let (_, wrong) = device_for(&f, &f.org, &f.admin, &other_service).await;
        assert!(execution::resolve(&f.state.db, &wrong, &service.id).await.is_err());
        let mut children = Vec::new();
        for _ in 0..2 {
            let secret = crate::services::api_key_credential_service::generate_secret();
            let id = Uuid::new_v4().to_string();
            f.state.db.collection::<bson::Document>("api_key_credentials").insert_one(doc! {
                "_id":&id,"api_key_id":key_id,"user_id":&f.org,
                "secret_hash":crate::crypto::token::hash_token(&secret),
                "secret_prefix":"nyxid_ag_test","label":"board","login_request_id":Uuid::new_v4().to_string(),
                "is_active":true,"created_at":bson::DateTime::now(),
                "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::hours(1)),
            }).await.unwrap();
            let (_, key, child) = key_service::validate_api_key(&f.state.db, &secret).await.unwrap();
            let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, child, None, None).await.unwrap();
            children.push((id, auth));
        }
        let auth = &children[0].1;
        let rows = super::application::agents(&f.state, auth, None).await.unwrap();
        let conversation = rows["agents"][0]["conversation_id"].as_str().unwrap();
        let proof = client_key::authorize(&f.state.db, auth, conversation).await.unwrap();
        let call = session::admit_for_client_key(
            &f.state.db, &f.org, conversation, &Uuid::new_v4().to_string(),
            credential_tests::platform(&voice), "fixture".into(), "worker", proof,
        ).await.unwrap();
        client_key::own_call(&f.state.db, auth, &call).await.unwrap();
        assert!(client_key::own_call(&f.state.db, &children[1].1, &call).await.is_err());
        assert!(client_key::own_call(&f.state.db, &parent, &call).await.is_err());
        // Both ordinary live authorization and transactional mutations must
        // observe the same revocations, without a person-type predicate.
        for (collection, id, withdrawn, restored) in [
            ("api_keys", key_id, doc!{"is_active":false}, doc!{"is_active":true}),
            ("api_keys", key_id, doc!{"allowed_service_ids":[]}, doc!{"allowed_service_ids":[&service.id]}),
            ("user_services", service.id.as_str(), doc!{"is_active":false}, doc!{"is_active":true}),
            ("assistant_agents", service.source_id.as_deref().unwrap(), doc!{"destroyed_at":bson::DateTime::now()}, doc!{"destroyed_at":bson::Bson::Null}),
            ("users", f.org.as_str(), doc!{"is_active":false}, doc!{"is_active":true}),
            ("api_key_credentials", children[0].0.as_str(), doc!{"is_active":false}, doc!{"is_active":true}),
        ] {
            let rows = f.state.db.collection::<bson::Document>(collection);
            rows.update_one(doc!{"_id":id}, doc!{"$set":withdrawn}).await.unwrap();
            assert!(client_key::revalidate(&f.state.db, &call).await.is_err(), "{collection}");
            assert!(session::control_for_caller(
                &f.state.db, &f.org, conversation, &call.id, &Uuid::new_v4().to_string(),
                call.control_revision, "mute", true,
            ).await.is_err(), "{collection}");
            rows.update_one(doc!{"_id":id}, doc!{"$set":restored}).await.unwrap();
            client_key::revalidate(&f.state.db, &call).await.unwrap();
        }
        session::close(&f.state.db, &call, "fixture", true).await.unwrap();
        f.state.db.drop().await.unwrap();
    }).await;
}
