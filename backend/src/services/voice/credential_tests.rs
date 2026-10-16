//! Real replica-set regressions for specialist audio authority and billing.
use super::credentials;
use crate::{
    billing_integration_tests::{FakeLago, billing_service, create_usage_index, rate},
    models::{
        assistant_agent::AgentGrants,
        assistant_voice::{VoiceInputMode, VoiceKeySource, VoicePreferences},
        downstream_service::{
            COLLECTION_NAME as SERVICES, DownstreamService, PlatformKeyAudience, PlatformKeyConfig,
            test_helpers::dummy_service,
        },
        usage_meter::CredentialClass,
    },
    services::{
        assistant_authority_tests::{Fixture, direct_fixture},
        assistant_team_service as team, assistant_voice, feature_flag_service as flags,
        key_service,
    },
};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub(super) async fn fixture(name: &str) -> (Fixture, DownstreamService) {
    let mut f = direct_fixture(name).await;
    let service = configure(&mut f.state, &f.owner).await;
    (f, service)
}

async fn configure(state: &mut crate::AppState, owner: &str) -> DownstreamService {
    let db = &state.db;
    assistant_voice::ensure_indexes(db).await.unwrap();
    super::session::ensure_indexes(db).await.unwrap();
    create_usage_index(db).await;
    for flag in [
        flags::ASSISTANT_VOICE_FLAG_KEY,
        flags::VOICE_OPENAI_PLATFORM_FLAG_KEY,
        flags::BILLING_FLAG_KEY,
    ] {
        flags::set_platform_override(db, flag, &flags::FlagTarget::Global, true, owner)
            .await
            .unwrap();
    }
    crate::services::billing::exact_migration::run(db)
        .await
        .unwrap();
    state.billing = Arc::new(billing_service(db, Arc::new(FakeLago::default()), 0));
    db.collection::<crate::models::billing_rate_cache::BillingRateCache>(
        crate::models::billing_rate_cache::COLLECTION_NAME,
    )
    .insert_many([
        rate("voice-test-input", chrono::Utc::now()),
        rate("voice-test-seconds", chrono::Utc::now()),
    ])
    .await
    .unwrap();
    let mut service = dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "llm-openai".into();
    service.name = "Platform OpenAI".into();
    service.visibility = "public".into();
    service.is_active = true;
    service.service_category = "internal".into();
    service.requires_user_credential = false;
    service.auth_method = "bearer".into();
    service.base_url = "https://api.openai.com/v1".into();
    service.platform_key = Some(PlatformKeyConfig {
        enabled: true,
        audience: PlatformKeyAudience::Public,
        allowed_owner_ids: vec![],
    });
    service.credential_encrypted = state
        .encryption_keys
        .encrypt(b"fixture-platform-key")
        .await
        .unwrap();
    service.inference = crate::services::inference_service::default_inference("llm-openai");
    service.billing = Some(
        serde_json::from_value(json!({
            "platform_billable": true,
            "platform_key_pricing": {
                "metric":"input_tokens", "credits_per_unit":"1",
                "lago_metric_code":"voice-test-input", "sync_status":"synced",
                "components":[{
                    "metric":"voice_seconds", "credits_per_unit":"1",
                    "lago_metric_code":"voice-test-seconds", "sync_status":"synced"
                }]
            }
        }))
        .unwrap(),
    );
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
    service
}

pub(super) fn platform(service: &DownstreamService) -> VoicePreferences {
    VoicePreferences {
        service_id: service.id.clone(),
        connection_id: None,
        key_source: VoiceKeySource::Platform,
        model: service
            .inference
            .as_ref()
            .unwrap()
            .voice
            .as_ref()
            .unwrap()
            .models[0]
            .id
            .clone(),
        voice: None,
        input_mode: VoiceInputMode::PushToTalk,
        language: None,
        notify_on_completion: false,
    }
}

async fn options(f: &Fixture) -> Value {
    serde_json::to_value(
        assistant_voice::options(&f.state.db, &f.state.encryption_keys, &f.row)
            .await
            .unwrap(),
    )
    .unwrap()
}

pub(super) async fn connection(f: &Fixture, service: &DownstreamService, slug: &str) -> String {
    let endpoint = Uuid::new_v4().to_string();
    let id = Uuid::new_v4().to_string();
    let key_id = Uuid::new_v4().to_string();
    let owner = f.row.agent_owner_id.as_deref().unwrap_or(&f.owner);
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .insert_one(doc! {
            "_id":&key_id, "user_id":owner, "label":slug, "credential_type":"bearer",
            "credential_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,
                bytes:f.state.encryption_keys.encrypt(b"fixture-own-key").await.unwrap()},
            "status":"active", "created_at":now, "updated_at":now
        })
        .await
        .unwrap();
    f.state
        .db
        .collection(crate::models::user_endpoint::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user_endpoint(
            &endpoint,
            owner,
            slug,
            &service.base_url,
            None,
            Some(&service.id),
        ))
        .await
        .unwrap();
    let mut row =
        crate::test_utils::test_user_service(&id, owner, slug, &endpoint, Some(&service.id), None);
    row.api_key_id = Some(key_id);
    row.auth_method = "bearer".into();
    f.state
        .db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(row)
        .await
        .unwrap();
    id
}

async fn grant(f: &Fixture, ids: Vec<String>) {
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: ids,
            ..Default::default()
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn specialist_platform_voice_resolves_without_connection_or_tool_grant() {
    Box::pin(async {
        let (f, service) = fixture("specialist_voice_platform").await;
        let before = key_service::get_api_key(&f.state.db, &f.owner, &f.row.credential_api_key_id)
            .await
            .unwrap();
        assert!(
            !before.allow_all_services
                && before.allowed_service_ids.is_empty()
                && before.allowed_platform_service_ids.is_empty()
        );
        assert!(f.state.billing.billing_enabled());
        let advertised = options(&f).await;
        assert_eq!(advertised[0]["key_source"], "platform");
        assert_eq!(advertised[0]["available"], true);
        let resolved = credentials::resolve(&f.state, &f.owner, &f.row.id, &platform(&service))
            .await
            .expect("grantless specialist must use platform voice as model transport");
        assert_eq!(
            resolved.billing.credential_class,
            CredentialClass::NyxidManagedMaster
        );
        assert_eq!(resolved.billing.billing_owner_id, f.owner);
        assert!(resolved.billing.user_service_id.is_none());
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "voice must not provision an OpenAI connection"
        );
        let after = key_service::get_api_key(&f.state.db, &f.owner, &f.row.credential_api_key_id)
            .await
            .unwrap();
        assert_eq!(
            bson::to_document(&before).unwrap(),
            bson::to_document(&after).unwrap(),
            "audio must not mutate tool or delegated authority"
        );
        assert!(
            crate::services::assistant_authority_tests::service_gate(
                &f.state.db,
                &f.chat,
                &service.id,
                &service.slug,
                &service.name,
                true
            )
            .await
            .unwrap()
            .is_some(),
            "the platform model must still require a tool grant"
        );
    })
    .await;
}

#[tokio::test]
async fn specialist_byok_voice_uses_selected_connection_grant_and_hides_ungranted_sibling() {
    Box::pin(async {
        let (f, service) = fixture("specialist_voice_byok").await;
        let allowed = connection(&f, &service, "openai-allowed").await;
        let denied = connection(&f, &service, "openai-denied").await;
        grant(&f, vec![allowed.clone()]).await;
        let mut p = platform(&service);
        p.key_source = VoiceKeySource::Own;
        p.connection_id = Some(allowed.clone());
        let resolved = credentials::resolve(&f.state, &f.owner, &f.row.id, &p)
            .await
            .expect("BYOK authorization must use the granted connection UUID");
        assert_eq!(
            resolved.billing.credential_class,
            CredentialClass::UserOwned
        );
        assert_eq!(
            resolved.billing.user_service_id.as_deref(),
            Some(allowed.as_str())
        );
        let advertised = options(&f).await;
        assert!(
            advertised
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["connection_id"] == allowed && o["available"] == true)
        );
        assert!(
            !advertised
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["connection_id"] == denied && o["available"] == true)
        );
        p.connection_id = Some(denied);
        assert!(
            credentials::resolve(&f.state, &f.owner, &f.row.id, &p)
                .await
                .is_err()
        );
        team::set_grants(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            team::GrantChange::Remove(AgentGrants {
                service_ids: vec![allowed.clone()],
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        p.connection_id = Some(allowed);
        assert!(
            credentials::resolve(&f.state, &f.owner, &f.row.id, &p)
                .await
                .is_err(),
            "revalidation must see grant removal"
        );
    })
    .await;
}

#[tokio::test]
async fn specialist_voice_discovery_loads_bound_authority_once_across_sources_and_models() {
    Box::pin(async {
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (f, mut service) = fixture("specialist_voice_discovery_reads").await;
        let a = connection(&f, &service, "own-a").await;
        let b = connection(&f, &service, "own-b").await;
        grant(&f, vec![a, b]).await;
        let voice = service.inference.as_mut().unwrap().voice.as_mut().unwrap();
        let mut second = voice.models[0].clone();
        second.id = "another-catalog-model".into();
        second.default = false;
        voice.models.push(second);
        f.state
            .db
            .collection::<DownstreamService>(SERVICES)
            .replace_one(doc! {"_id":&service.id}, &service)
            .await
            .unwrap();
        let key_reads = Arc::new(AtomicUsize::new(0));
        let credential_reads = Arc::new(AtomicUsize::new(0));
        let (k, c) = (key_reads.clone(), credential_reads.clone());
        let handler = EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && let Ok(collection) = event.command.get_str("find")
            {
                if collection == crate::models::api_key::COLLECTION_NAME {
                    k.fetch_add(1, Ordering::SeqCst);
                }
                if collection == crate::models::assistant_agent_credential::COLLECTION_NAME {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        let observer = crate::test_utils::connect_test_database_with_command_handler(
            "voice_options_observer",
            handler,
        )
        .await
        .expect("MongoDB required");
        let db = observer.client().database(f.state.db.name());
        let advertised = assistant_voice::options(&db, &f.state.encryption_keys, &f.row)
            .await
            .unwrap();
        assert_eq!(
            advertised.len(),
            6,
            "two models for platform and each own connection"
        );
        assert_eq!(
            key_reads.load(Ordering::SeqCst),
            1,
            "authenticate once per options request"
        );
        assert_eq!(
            credential_reads.load(Ordering::SeqCst),
            1,
            "load the exact conversation binding once"
        );
    })
    .await;
}

#[tokio::test]
async fn specialist_voice_discovery_enforces_operation_denial_and_openai_origin() {
    Box::pin(async {
        let (f, service) = fixture("specialist_voice_discovery").await;
        let denied = connection(&f, &service, "openai-denied").await;
        let allowed = connection(&f, &service, "openai-allowed").await;
        grant(&f, vec![denied.clone(), allowed.clone()]).await;
        crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
        crate::services::agent_operation_scope_service::set(&f.state.db, &f.owner, &f.chat.agent_id, &denied,
            &crate::models::agent_operation_scope::OperationSelection {
                expected_revision:0, all_operations:false, endpoint_ids:vec![], rules:vec![], contract_digest:None, inputs:Default::default()
            }, false).await.unwrap();
        let advertised = options(&f).await;
        assert!(!advertised.as_array().unwrap().iter().any(|o|o["available"] == true && o["connection_id"] != allowed),
            "OpenAI discovery must honor the authored denial, including the platform inference alias");
        assert!(advertised.as_array().unwrap().iter().any(|o|o["available"] == true && o["connection_id"] == allowed),
            "the allowed sibling must retain its independent selected operation identity");
        assert!(credentials::resolve(&f.state, &f.owner, &f.row.id, &platform(&service)).await.is_err());
        let mut selected = platform(&service);
        selected.key_source = VoiceKeySource::Own;
        selected.connection_id = Some(allowed.clone());
        credentials::resolve(&f.state, &f.owner, &f.row.id, &selected).await
            .expect("an unscoped explicitly granted sibling remains usable despite another sibling's deny scope");
        f.state.db.collection::<bson::Document>(SERVICES).update_one(doc! {"_id":&service.id},
            doc! {"$set":{"base_url":"https://attacker.invalid/v1"}}).await.unwrap();
        assert!(options(&f).await.as_array().unwrap().is_empty(), "OpenAI catalog origin must be vetted during discovery");
        f.state.db.collection::<bson::Document>(SERVICES).update_one(doc! {"_id":&service.id},
            doc! {"$set":{"base_url":&service.base_url}}).await.unwrap();
        // Grok previously checked only the conservative catalog alias during
        // discovery. It must retain this selected connection's independence too.
        for flag in [flags::VOICE_GROK_FLAG_KEY, flags::VOICE_GROK_PLATFORM_FLAG_KEY] {
            flags::set_platform_override(&f.state.db, flag, &flags::FlagTarget::Global, true, &f.owner).await.unwrap();
        }
        f.state.db.collection::<bson::Document>(SERVICES).update_one(doc! {"_id":&service.id},
            doc! {"$set":{"base_url":"https://api.x.ai/v1","inference.voice.protocol":"xai_realtime","inference.voice.usage_source":"server_measured"}}).await.unwrap();
        f.state.db.collection::<bson::Document>(crate::models::user_endpoint::COLLECTION_NAME)
            .update_many(doc! {"catalog_service_id":&service.id},doc! {"$set":{"url":"https://api.x.ai/v1"}}).await.unwrap();
        let grok = options(&f).await;
        assert!(grok.as_array().unwrap().iter().any(|o|o["connection_id"]==allowed && o["available"]==true));
        assert!(!grok.as_array().unwrap().iter().any(|o|o["connection_id"]!=allowed && o["available"]==true));
        credentials::resolve(&f.state, &f.owner, &f.row.id, &selected).await
            .expect("Grok uses the same independent selected BYOK identity");
        f.state.db.collection::<bson::Document>(SERVICES).update_one(doc!{"_id":&service.id}, doc!{"$set":{"base_url":"https://attacker.invalid/v1"}}).await.unwrap();
        assert!(options(&f).await.as_array().unwrap().is_empty(), "Grok discovery must reject unsupported credential destinations");
    }).await;
}

#[tokio::test]
async fn specialist_platform_voice_keeps_draft_binding_key_and_agent_fences() {
    Box::pin(async {
        use crate::handlers::assistant_uploads::{Draft, draft};
        use axum::{Json, extract::State};
        let (f, service) = fixture("specialist_voice_binding").await;
        let Json(created) = Box::pin(draft(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            Json(Draft {
                agent_id: Some(f.chat.agent_id.clone()),
            }),
        ))
        .await
        .unwrap();
        let draft_id = created["id"].as_str().unwrap();
        let draft = assistant_voice::thread(&f.state.db, &f.owner, draft_id)
            .await
            .unwrap();
        assert!(draft.active_turn.is_none());
        assert!(
            !draft.credential_api_key_id.is_empty(),
            "new drafts already have their exact conversation key"
        );
        let p = platform(&service);
        credentials::resolve(&f.state, &f.owner, draft_id, &p)
            .await
            .expect("voice must work before the first text turn");
        let rows = f
            .state
            .db
            .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME);
        // Another valid conversation key, even belonging to this same agent,
        // does not authorize audio in the draft's conversation.
        rows.update_one(
            doc! {"_id":draft_id},
            doc! {"$set":{"credential_api_key_id":&f.row.credential_api_key_id}},
        )
        .await
        .unwrap();
        assert!(
            credentials::resolve(&f.state, &f.owner, draft_id, &p)
                .await
                .is_err()
        );
        rows.update_one(
            doc! {"_id":draft_id},
            doc! {"$set":{"credential_api_key_id":&draft.credential_api_key_id}},
        )
        .await
        .unwrap();
        let keys = f
            .state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME);
        // Broad ordinary keys and a conversation key whose durable credential
        // was removed do not acquire this model-transport exception.
        let mut ordinary = keys
            .find_one(doc! {"_id":&draft.credential_api_key_id})
            .await
            .unwrap()
            .unwrap();
        let ordinary_id = Uuid::new_v4().to_string();
        ordinary.insert("_id", &ordinary_id);
        ordinary.insert("key_hash", Uuid::new_v4().to_string());
        ordinary.insert("platform", bson::Bson::Null);
        ordinary.insert("assistant_agent_owner_id", bson::Bson::Null);
        ordinary.insert("allow_all_services", true);
        keys.insert_one(ordinary).await.unwrap();
        rows.update_one(
            doc! {"_id":draft_id},
            doc! {"$set":{"credential_api_key_id":&ordinary_id}},
        )
        .await
        .unwrap();
        assert!(
            credentials::resolve(&f.state, &f.owner, draft_id, &p)
                .await
                .is_err()
        );
        rows.update_one(
            doc! {"_id":draft_id},
            doc! {"$set":{"credential_api_key_id":&draft.credential_api_key_id}},
        )
        .await
        .unwrap();
        let credential_rows = f.state.db.collection::<bson::Document>(
            crate::models::assistant_agent_credential::COLLECTION_NAME,
        );
        let credential = credential_rows
            .find_one_and_delete(doc! {"api_key_id":&draft.credential_api_key_id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            credentials::resolve(&f.state, &f.owner, draft_id, &p)
                .await
                .is_err()
        );
        credential_rows.insert_one(credential).await.unwrap();
        rows.update_one(doc! {"_id":draft_id}, doc! {"$set":{"guest_turn":true}})
            .await
            .unwrap();
        assert!(
            credentials::resolve(&f.state, &f.owner, draft_id, &p)
                .await
                .is_err()
        );
        rows.update_one(doc! {"_id":draft_id}, doc! {"$set":{"guest_turn":false}})
            .await
            .unwrap();
        let other_agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
        let mut rebound = other_agent.clone();
        rebound.id = Uuid::new_v4().to_string();
        rebound.name = "rebound-agent".into();
        f.state
            .db
            .collection::<crate::models::assistant_agent::AssistantAgent>(
                crate::models::assistant_agent::COLLECTION_NAME,
            )
            .insert_one(&rebound)
            .await
            .unwrap();
        rows.update_one(
            doc! {"_id":draft_id},
            doc! {"$set":{"agent_id":&rebound.id}},
        )
        .await
        .unwrap();
        // The captured thread and the credential's live thread must also agree
        // on agent identity (discovery can receive an earlier thread snapshot).
        assert!(
            credentials::inference_authority(&f.state.db, &draft)
                .await
                .is_err()
        );
        rows.update_one(
            doc! {"_id":draft_id},
            doc! {"$set":{"agent_id":&draft.agent_id}},
        )
        .await
        .unwrap();
        for expired in [false, true] {
            let change = if expired {
                doc! {"expires_at":bson::DateTime::from_millis(1)}
            } else {
                doc! {"is_active":false}
            };
            keys.update_one(
                doc! {"_id":&draft.credential_api_key_id},
                doc! {"$set":change},
            )
            .await
            .unwrap();
            assert!(
                credentials::resolve(&f.state, &f.owner, draft_id, &p)
                    .await
                    .is_err()
            );
            keys.update_one(
                doc! {"_id":&draft.credential_api_key_id},
                doc! {"$set":{"is_active":true,"expires_at":bson::Bson::Null}},
            )
            .await
            .unwrap();
        }
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.agent_id},
                doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
            )
            .await
            .unwrap();
        assert!(
            credentials::resolve(&f.state, &f.owner, draft_id, &p)
                .await
                .is_err()
        );
    })
    .await;
}

#[tokio::test]
async fn specialist_platform_voice_keeps_rollout_acl_tariff_and_wallet_gates() {
    Box::pin(async {
        let (f, service) = fixture("specialist_voice_funding_fences").await;
        let p = platform(&service);
        credentials::resolve(&f.state, &f.owner, &f.row.id, &p).await.unwrap();
        flags::set_platform_override(&f.state.db, flags::VOICE_OPENAI_PLATFORM_FLAG_KEY, &flags::FlagTarget::Global, false, &f.owner).await.unwrap();
        assert!(credentials::resolve(&f.state, &f.owner, &f.row.id, &p).await.is_err());
        assert_eq!(options(&f).await[0]["unavailable_reason"], "provider_rollout_pending");
        flags::set_platform_override(&f.state.db, flags::VOICE_OPENAI_PLATFORM_FLAG_KEY, &flags::FlagTarget::Global, true, &f.owner).await.unwrap();
        let services = f.state.db.collection::<bson::Document>(SERVICES);
        services.update_one(doc!{"_id":&service.id}, doc!{"$set":{"platform_key.audience":"restricted","platform_key.allowed_owner_ids":[]}}).await.unwrap();
        assert!(credentials::resolve(&f.state, &f.owner, &f.row.id, &p).await.is_err());
        assert!(options(&f).await.as_array().unwrap().is_empty());
        services.update_one(doc!{"_id":&service.id}, doc!{"$set":{"platform_key.audience":"public","billing.platform_key_pricing.components.0.sync_status":"pending"}}).await.unwrap();
        assert!(credentials::resolve(&f.state, &f.owner, &f.row.id, &p).await.is_err());
        assert_eq!(options(&f).await[0]["unavailable_reason"], "duration_tariff_unavailable");
        services.update_one(doc!{"_id":&service.id}, doc!{"$set":{"billing.platform_key_pricing.components.0.sync_status":"synced"}}).await.unwrap();
        let resolved = credentials::resolve(&f.state, &f.owner, &f.row.id, &p).await.unwrap();
        // Empty prepaid funding is a hard refusal with billing enabled.
        let billing = billing_service(&f.state.db, Arc::new(FakeLago::with_wallet_balance(crate::models::credits::Credits::ZERO)), 0);
        let error = crate::services::billing::voice::reserve(&f.state.db, &billing, resolved.billing, &Uuid::new_v4().to_string(), 0).await.err().unwrap();
        assert!(matches!(error, crate::errors::AppError::InsufficientCredits), "{error:?}");
    }).await;
}

#[tokio::test]
async fn org_specialist_platform_voice_bills_actor_and_revalidates_member_access() {
    Box::pin(async {
        use crate::handlers::assistant_uploads::{Draft, draft};
        use axum::{Json, extract::State};
        let mut f = crate::services::org_agent_tests::Fixture::new("org_specialist_voice").await;
        let (agent, _) = f.create().await;
        let service = configure(&mut f.state, &f.member).await;
        let Json(created) = Box::pin(draft(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.member),
            Json(Draft {
                agent_id: Some(agent.id),
            }),
        ))
        .await
        .unwrap();
        let id = created["id"].as_str().unwrap();
        let thread = assistant_voice::thread(&f.state.db, &f.member, id)
            .await
            .unwrap();
        assert_eq!(thread.agent_owner_id.as_deref(), Some(f.org.as_str()));
        assert!(thread.active_turn.is_none());
        let p = platform(&service);
        let resolved = credentials::resolve(&f.state, &f.member, id, &p)
            .await
            .unwrap();
        assert_eq!(resolved.billing.billing_owner_id, f.member);
        assert_eq!(
            resolved.billing.credential_class,
            CredentialClass::NyxidManagedMaster
        );
        assert!(
            credentials::resolve(&f.state, &f.admin, id, &p)
                .await
                .is_err(),
            "org admins cannot use another member's private thread"
        );
        f.revoke(&f.member).await;
        assert!(
            credentials::resolve(&f.state, &f.member, id, &p)
                .await
                .is_err()
        );
        assert!(
            assistant_voice::options(&f.state.db, &f.state.encryption_keys, &thread)
                .await
                .unwrap()
                .is_empty(),
            "even discovery with a previously loaded thread rechecks live key/org authority"
        );
    })
    .await;
}

#[tokio::test]
async fn specialist_own_voice_discovery_checks_effective_origin_and_usable_credentials() {
    Box::pin(async {
        let (f, service) = fixture("specialist_voice_own_eligibility").await;
        let id = connection(&f, &service, "openai-own").await;
        grant(&f, vec![id.clone()]).await;
        let row = f.state.db.collection::<crate::models::user_service::UserService>(crate::models::user_service::COLLECTION_NAME)
            .find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        let endpoints = f.state.db.collection::<bson::Document>(crate::models::user_endpoint::COLLECTION_NAME);
        let keys = f.state.db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME);
        let own_available = |o:&Value|o.as_array().unwrap().iter().any(|v|v["connection_id"]==id && v["available"]==true);
        assert!(own_available(&options(&f).await));
        let mut p = platform(&service); p.key_source=VoiceKeySource::Own; p.connection_id=Some(id.clone());
        endpoints.update_one(doc! {"_id":&row.endpoint_id}, doc! {"$set":{"url":"https://attacker.invalid/v1"}}).await.unwrap();
        assert!(!own_available(&options(&f).await));
        assert!(credentials::resolve(&f.state,&f.owner,&f.row.id,&p).await.is_err());
        endpoints.update_one(doc! {"_id":&row.endpoint_id}, doc! {"$set":{"url":&service.base_url}}).await.unwrap();
        keys.update_one(doc! {"_id":&row.api_key_id}, doc! {"$set":{"status":"revoked"}}).await.unwrap();
        assert!(!own_available(&options(&f).await));
        assert!(credentials::resolve(&f.state,&f.owner,&f.row.id,&p).await.is_err());
        keys.update_one(doc! {"_id":&row.api_key_id}, doc! {"$set":{"status":"active","credential_encrypted":bson::Bson::Null}}).await.unwrap();
        assert!(!own_available(&options(&f).await));
        assert!(credentials::resolve(&f.state,&f.owner,&f.row.id,&p).await.is_err());
        keys.update_one(doc! {"_id":&row.api_key_id}, doc! {"$set":{"credential_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic, bytes:f.state.encryption_keys.encrypt(b"fixture-own-key").await.unwrap()}}}).await.unwrap();
        assert!(own_available(&options(&f).await));
        endpoints.delete_one(doc! {"_id":&row.endpoint_id}).await.unwrap();
        assert!(matches!(assistant_voice::options(&f.state.db,&f.state.encryption_keys,&f.row).await, Err(crate::errors::AppError::Internal(_))),
            "read-only snapshot infrastructure/data-integrity errors must propagate");
    }).await;
}

#[tokio::test]
async fn specialist_openai_http_operation_grant_cannot_authorize_voice_sideband() {
    Box::pin(async {
        let (f, service) = fixture("specialist_voice_http_scope").await;
        let id = connection(&f, &service, "openai-http-only").await;
        grant(&f, vec![id.clone()]).await;
        crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
        crate::services::agent_operation_scope_service::set(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &id,
            &crate::models::agent_operation_scope::OperationSelection {
                expected_revision: 0,
                all_operations: false,
                endpoint_ids: vec![],
                rules: vec![crate::models::downstream_service::ProxyOperationRule {
                    method: "POST".into(),
                    path_template: "/live/sessions".into(),
                    ..Default::default()
                }],
                contract_digest: None,
                inputs: Default::default(),
            },
            false,
        )
        .await
        .unwrap();
        let mut p = platform(&service);
        p.key_source = VoiceKeySource::Own;
        p.connection_id = Some(id.clone());
        assert!(
            credentials::resolve(&f.state, &f.owner, &f.row.id, &p)
                .await
                .is_err()
        );
        assert!(
            !options(&f)
                .await
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["connection_id"] == id && o["available"] == true),
            "a scoped HTTP create cannot grant unrestricted WebSocket attachment"
        );
    })
    .await;
}

#[tokio::test]
async fn specialist_platform_voice_live_revalidation_closes_revoked_key_and_bills_seconds() {
    Box::pin(async {
        use axum::{Json, Router, extract::{WebSocketUpgrade, ws::Message}, routing::{get, post}};
        use crate::models::{assistant_voice_session::{VoiceSession, COLLECTION_NAME as CALLS}, usage_meter::{UsageMeterRow, COLLECTION_NAME as USAGE}};
        let (f, service) = fixture("specialist_voice_live_billing").await;
        let p = platform(&service);
        let resolved = credentials::resolve(&f.state, &f.owner, &f.row.id, &p).await.unwrap();
        let provider_closed = Arc::new(tokio::sync::Notify::new());
        let closed = provider_closed.clone();
        let app = Router::new()
            .route("/v1/live/sessions", post(|headers:axum::http::HeaderMap, Json(body):Json<Value>| async move {
                assert_eq!(headers["authorization"], "Bearer fixture-platform-key");
                assert_eq!(body["session"]["store"], false);
                assert!(body["session"].get("tools").is_none());
                Json(json!({"session":{"id":"live_specialist"},"transport":{"type":"webrtc","sdp":"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"}}))
            }))
            .route("/v1/live/sessions/live_specialist/attach", get(move |ws:WebSocketUpgrade| {
                let closed=closed.clone();
                async move {ws.on_upgrade(move |mut socket| async move {
                    socket.send(Message::Text(json!({"type":"session.usage.updated","usage":{"seconds":18.2}}).to_string().into())).await.unwrap();
                    while let Some(Ok(message)) = socket.recv().await {
                        if let Message::Text(text)=message {
                            let event:Value=serde_json::from_str(&text).unwrap();
                            if event["type"]=="session.close" {
                                socket.send(Message::Text(json!({"type":"session.closed","usage":{"seconds":18.2}}).to_string().into())).await.unwrap();
                                closed.notify_one();
                                break;
                            }
                        }
                    }
                })}
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {axum::serve(listener, app).await.unwrap()});
        let started = super::runtime::start_with_provider(&f.state,
            super::runtime::StartInput { user:&f.owner, conversation:&f.row.id, client_request_id:&Uuid::new_v4().to_string(), preferences:p,
                sdp:"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n" },
            resolved.identity, resolved.billing,
            super::openai::OpenAi::fixture(resolved.key,address),None).await.unwrap();
        assert_eq!(started.session.state, crate::models::assistant_voice_session::SessionState::Active);
        let initial = f.state.db.collection::<UsageMeterRow>(USAGE)
            .find_one(doc!{"billing_request_id":format!("voice:{}:0",started.session.id),"metric":"voice_seconds"}).await.unwrap().unwrap();
        assert_eq!(initial.billing_owner_id, f.owner);
        assert!(initial.wallet_id.is_some(), "billing must reserve real funds before provider creation");
        f.state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .update_one(doc!{"_id":&f.row.credential_api_key_id},doc!{"$set":{"is_active":false}}).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15),provider_closed.notified()).await.expect("live revalidation must close the provider after key revocation");
        let closed = tokio::time::timeout(std::time::Duration::from_secs(10),async {
            loop {
                let call = f.state.db.collection::<VoiceSession>(CALLS).find_one(doc!{"_id":&started.session.id}).await.unwrap().unwrap();
                if !call.live_slot {break call;}
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        }).await.unwrap();
        assert!(closed.billing_finalized && closed.final_usage_confirmed);
        assert_eq!(closed.observed_seconds, 18);
        let usage = f.state.db.collection::<UsageMeterRow>(USAGE).find_one(doc!{"_id":&initial.id}).await.unwrap().unwrap();
        assert_eq!(usage.quantity, Some(18));
        assert_eq!(usage.credential_class, CredentialClass::NyxidManagedMaster);
        assert!(usage.user_service_id.is_none());
        assert_eq!(usage.funding.unwrap().wallet_funded, Some(crate::models::credits::Credits::from_whole(18)));
        server.abort();
    }).await;
}

/// Real duration/request reservations and grant issuance; the provider is a
/// loopback server, never an OpenAI endpoint.
#[tokio::test]
async fn specialist_platform_voice_funding_new_personal_grant_immediately_allows_retry() {
    Box::pin(async {
        use axum::{Json, Router, extract::{WebSocketUpgrade, ws::Message}, routing::{get, post}};
        use axum::response::IntoResponse;
        use crate::models::{
            assistant_voice_session::{VoiceSession, COLLECTION_NAME as CALLS},
            billing_target::BillingTargetKind,
            billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
            credit_grant::{CreditGrant, COLLECTION_NAME as GRANTS},
            credits::Credits,
            usage_meter::{UsageMeterRow, COLLECTION_NAME as USAGE},
        };
        use crate::services::billing::{grants, voice};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (mut f, service) = fixture("voice_grant_immediate_retry").await;
        f.state.billing = Arc::new(billing_service(&f.state.db,
            Arc::new(FakeLago::with_wallet_balance(Credits::ZERO)), 0));
        f.state.db.collection::<bson::Document>(SERVICES).update_one(doc! {"_id":&service.id},
            doc! {"$push":{"billing.platform_key_pricing.components":{
                "metric":"requests","credits_per_unit":"2","lago_metric_code":"voice-test-start","sync_status":"synced"
            }}}).await.unwrap();
        let mut start_rate = rate("voice-test-start", chrono::Utc::now());
        start_rate.credits_per_unit_micros = 2_000_000;
        f.state.db.collection::<crate::models::billing_rate_cache::BillingRateCache>(
            crate::models::billing_rate_cache::COLLECTION_NAME).insert_one(start_rate).await.unwrap();
        let p = platform(&service);
        // Exercise the upfront owner/credential resolver with an actual empty
        // wallet too. Platform use still selects the acting person's funding.
        f.state.billing.ensure_wallet(&f.owner).await.unwrap();
        let resolved = credentials::resolve(&f.state, &f.owner, &f.row.id, &p).await.unwrap();
        assert_eq!(resolved.billing.billing_owner_id, f.owner);
        let creates = Arc::new(AtomicUsize::new(0));
        let attempts = creates.clone();
        let provider_db = f.state.db.clone();
        let provider_owner = f.owner.clone();
        let app = Router::new().route("/v1/live/sessions", post(move || {
            let attempts = attempts.clone(); let db = provider_db.clone(); let owner = provider_owner.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                let grant = db.collection::<CreditGrant>(GRANTS).find_one(doc! {"recipient_user_id":owner}).await.unwrap().unwrap();
                assert_eq!(grant.reserved, Credits::from_whole(32), "30 seconds plus the configured start fee must be held before provider creation");
                assert_eq!(grant.remaining, Credits::from_whole(100));
                Json(json!({"session":{"id":"live_funding"},"transport":{"sdp":"v=0\r\nm=audio"}}))
            }
        })).route("/v1/live/sessions/live_funding/attach", get(|ws:WebSocketUpgrade| async move {
            ws.on_upgrade(|mut socket| async move {
                socket.send(Message::Text(json!({"type":"session.closed","usage":{"seconds":0}}).to_string().into())).await.unwrap();
            })
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let rejected_request = Uuid::new_v4().to_string();
        let rejected = super::runtime::start_with_provider(&f.state, super::runtime::StartInput {
            user:&f.owner, conversation:&f.row.id, client_request_id:&rejected_request,
            preferences:p.clone(), sdp:"v=0\r\nm=audio",
        }, resolved.identity, resolved.billing, super::openai::OpenAi::fixture(resolved.key,address), None).await;
        let error = rejected.err().expect("zero wallet without benefits must refuse startup");
        assert_eq!(error.error_key(), "insufficient_credits");
        assert_eq!(error.into_response().status(), axum::http::StatusCode::PAYMENT_REQUIRED);
        assert_eq!(creates.load(Ordering::SeqCst), 0, "funding refusal must precede every provider invocation");
        let wallet = f.state.db.collection::<BillingWallet>(WALLETS).find_one(doc! {"owner_id":&f.owner}).await.unwrap().unwrap();
        assert_eq!(wallet.balance_credits, Credits::ZERO);
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
        assert_eq!(f.state.db.collection::<UsageMeterRow>(USAGE).count_documents(doc! {}).await.unwrap(), 0);
        let failed = f.state.db.collection::<VoiceSession>(CALLS).find_one(doc! {"client_request_id":&rejected_request}).await.unwrap().unwrap();
        assert!(!failed.live_slot && failed.billing_finalized);
        assert!(failed.provider_session_id.is_none());

        let grant = grants::issue_grants(&f.state.db, grants::IssueCreditGrantInput {
            amount_credits:100, target_kind:BillingTargetKind::SelectedUsers,
            target_user_ids:vec![f.owner.clone()], target_org_ids:vec![], target_group_ids:vec![],
            all_services:false, service_refs:vec![service.id.clone()], expires_at:None,
            reason:Some("local voice funding regression".into()), granted_by:f.owner.clone(),
        }).await.unwrap().remove(0);
        assert!(grant.issued_ledgered_at.is_some(), "issuance must be durably journaled before spend");
        assert_eq!(f.state.db.collection::<bson::Document>("billing_ledger").count_documents(
            doc! {"dedupe_key":format!("grant-issued:{}",grant.id)}).await.unwrap(), 1);
        // No reconcile, TTL wait, cache invalidation or BillingService rebuild.
        let resolved = credentials::resolve(&f.state, &f.owner, &f.row.id, &p).await.unwrap();
        let started = super::runtime::start_with_provider(&f.state, super::runtime::StartInput {
            user:&f.owner, conversation:&f.row.id, client_request_id:&Uuid::new_v4().to_string(),
            preferences:p, sdp:"v=0\r\nm=audio",
        }, resolved.identity, resolved.billing, super::openai::OpenAi::fixture(resolved.key,address), None).await.unwrap();
        assert_eq!(creates.load(Ordering::SeqCst), 1);
        let closed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let row = super::session::get(&f.state.db,&f.owner,&f.row.id,&started.session.id).await.unwrap();
                if !row.live_slot { break row; }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.expect("provider-confirmed zero-usage close must settle locally");
        assert!(closed.billing_finalized && closed.final_usage_confirmed);
        let request_id = voice::window_id(&closed.id, 0).unwrap();
        voice::finalize(&f.state.db,&request_id,false).await.unwrap();
        let grant = f.state.db.collection::<CreditGrant>(GRANTS).find_one(doc! {"_id":&grant.id}).await.unwrap().unwrap();
        assert_eq!(grant.remaining, Credits::from_whole(100));
        assert_eq!(grant.reserved, Credits::ZERO);
        assert_eq!(f.state.db.collection::<bson::Document>("billing_ledger").count_documents(
            doc! {"event_type":"accounting_v2","movement":{"$in":["grant_consumed","usage_settled"]}}).await.unwrap(), 0,
            "zero observed usage must not consume grants or create a wallet charge");
        for metric in ["voice_seconds", "requests"] {
            let row = f.state.db.collection::<UsageMeterRow>(USAGE).find_one(doc! {"billing_request_id":&request_id,"metric":metric}).await.unwrap().unwrap();
            assert_eq!(row.quantity, Some(0));
            assert!(row.released);
        }
        let wallet = f.state.db.collection::<BillingWallet>(WALLETS).find_one(doc! {"owner_id":&f.owner}).await.unwrap().unwrap();
        assert_eq!(wallet.balance_credits, Credits::ZERO);
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
        server.abort();
        f.state.db.drop().await.unwrap();
    }).await;
}
