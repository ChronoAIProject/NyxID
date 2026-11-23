use super::*;
use crate::{
    models::{assistant_conversation::TurnAttachment, downstream_service},
    services::assistant_upload_service::image_plan,
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};
use axum::{body::Body, http::StatusCode, response::IntoResponse};
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::Barrier;

struct ScriptedEgress {
    owners: Mutex<Vec<String>>,
    replies: Mutex<VecDeque<AppResult<axum::response::Response>>>,
    rendezvous: Option<Arc<Barrier>>,
}

#[async_trait::async_trait]
impl contract::NyxAgentEgress for ScriptedEgress {
    async fn request(
        &self,
        authority: &contract::NyxAgentExecutionContext,
        _credential: &contract::RuntimeCredential,
        request: contract::NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        let contract::NyxAgentExecutionContext::Owner { user_id, .. } = authority else {
            panic!("expected owner metadata request");
        };
        assert_eq!(request.method, axum::http::Method::GET);
        assert_eq!(request.relative_path, "v1/capabilities");
        self.owners.lock().await.push(user_id.clone());
        if let Some(rendezvous) = &self.rendezvous {
            rendezvous.wait().await;
        }
        self.replies
            .lock()
            .await
            .pop_front()
            .expect("unexpected upstream lookup")
    }
}

fn capabilities() -> Value {
    json!({"protocol":"nyxagent-input-image-v1", "input_image": {
        "version":1, "sources":["nyxid_attachment_url"], "max_images":8,
        "max_image_bytes":5_000_000, "max_total_bytes":20_000_000,
        "content_types":["image/png"]
    }})
}

fn success() -> AppResult<axum::response::Response> {
    Ok(axum::Json(capabilities()).into_response())
}

fn egress(replies: Vec<AppResult<axum::response::Response>>) -> ScriptedEgress {
    ScriptedEgress {
        owners: Mutex::new(Vec::new()),
        replies: Mutex::new(replies.into()),
        rendezvous: None,
    }
}

async fn fixture() -> (AppState, CapabilityKey) {
    let db = connect_transaction_test_database("nyxa_capability_cache").await;
    engine::ensure_indexes(&db).await.unwrap();
    let mut service = downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = engine::SERVICE_SLUG.into();
    service.is_active = true;
    service.requires_user_credential = false;
    let cache_key = (
        db.name().to_owned(),
        service.id.clone(),
        service.base_url.clone(),
    );
    db.collection(downstream_service::COLLECTION_NAME)
        .insert_one(service)
        .await
        .unwrap();
    (test_app_state(db), cache_key)
}

struct Owner {
    id: String,
    context: contract::NyxAgentExecutionContext,
    credential: AssistantCredential,
}

async fn owner(state: &AppState) -> Owner {
    let id = Uuid::new_v4().to_string();
    state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&id, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let row = engine::begin_turn(
        &state.db,
        &id,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: Vec::new(),
            agent_id: None,
            conversation_id: None,
            text: "hello".into(),
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let credential =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, &id, &row.id)
            .await
            .unwrap()
            .unwrap();
    Owner {
        id,
        context: owner_context(&row).unwrap(),
        credential,
    }
}

async fn lookup(state: &AppState, egress: &ScriptedEgress, owner: &Owner) -> Value {
    upstream_capabilities(state, egress, &owner.context, &owner.credential, None).await
}

#[tokio::test]
async fn reattach_admission_reuses_authenticated_limits_and_rejects_ambiguous_destinations() {
    async fn deployment(
        db: &mongodb::Database,
    ) -> Option<crate::models::assistant_conversation::ReattachDeployment> {
        let (destination, value) = unique_cached_destination(&*CAPABILITIES.lock().await, db)?;
        reattach_deployment(destination, &value)
    }
    let (state, key) = fixture().await;
    let actor = owner(&state).await;
    let supported = json!({"reattach": {"protocol":"nyxagent-reattach-v1", "credential_watch":{"interval_seconds":30,"model_request_max_age_seconds":15}, "max_events":453439,"recent_events":4096,"max_event_bytes":33554432,"absence_retention_seconds":604800,"retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32}});
    let egress = egress(vec![Ok(axum::Json(&supported).into_response())]);
    assert!(deployment(&state.db).await.is_none());
    assert_eq!(lookup(&state, &egress, &actor).await, supported);
    let selected = deployment(&state.db).await.unwrap();
    assert_eq!(selected.limits.max_subscribers_per_response, 8);
    let watch = selected.limits.credential_watch.unwrap();
    assert_eq!(
        (watch.interval_seconds, watch.model_request_max_age_seconds),
        (30, 15)
    );
    let mut previous_protocol = supported.clone();
    previous_protocol["reattach"]
        .as_object_mut()
        .unwrap()
        .remove("credential_watch");
    CAPABILITIES
        .lock()
        .await
        .insert(key.clone(), (Instant::now(), previous_protocol));
    assert!(deployment(&state.db).await.is_none());
    let mut implicit_horizon = supported.clone();
    implicit_horizon["reattach"]
        .as_object_mut()
        .unwrap()
        .remove("absence_retention_seconds");
    CAPABILITIES
        .lock()
        .await
        .insert(key.clone(), (Instant::now(), implicit_horizon));
    assert_eq!(
        deployment(&state.db)
            .await
            .unwrap()
            .limits
            .absence_retention_seconds,
        604800
    );
    let destination = contract::NyxAgentDestination(selected.destination_hash);
    destination.verify(&key.1, &key.2).unwrap();
    assert!(destination.verify("replaced-service", &key.2).is_err());
    assert!(destination.verify(&key.1, "https://moved.invalid").is_err());
    let changed = (key.0.clone(), key.1.clone(), "https://other.invalid".into());
    CAPABILITIES
        .lock()
        .await
        .insert(changed.clone(), (Instant::now(), supported.clone()));
    assert!(deployment(&state.db).await.is_none());
    CAPABILITIES.lock().await.remove(&changed);
    CAPABILITIES.lock().await.insert(
        key.clone(),
        (Instant::now() - CAPABILITY_TTL, supported.clone()),
    );
    assert!(deployment(&state.db).await.is_none());
    for (field, invalid) in [
        ("protocol", json!("future")),
        ("credential_watch", json!({})),
        (
            "credential_watch",
            json!({"interval_seconds":31,"model_request_max_age_seconds":15}),
        ),
        (
            "credential_watch",
            json!({"interval_seconds":30,"model_request_max_age_seconds":16}),
        ),
        ("max_subscribers_per_owner", json!(1)),
        ("retention_seconds", Value::Null),
    ] {
        let mut value = supported.clone();
        value["reattach"][field] = invalid;
        CAPABILITIES
            .lock()
            .await
            .insert(key.clone(), (Instant::now(), value));
        assert!(deployment(&state.db).await.is_none());
    }
    assert_eq!(egress.owners.lock().await.len(), 1);
    CAPABILITIES.lock().await.remove(&key);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn contended_capability_read_waits_and_reuses_pinned_metadata() {
    let (state, key) = fixture().await;
    let supported = json!({"reattach":{"protocol":"nyxagent-reattach-v1","credential_watch":{"interval_seconds":30,"model_request_max_age_seconds":15},"recent_events":32,"max_events":64,"max_event_bytes":65536,"retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32}});
    for mode in [
        CapabilityLookup::CurrentCatalog,
        CapabilityLookup::Admission,
    ] {
        let mut held = CAPABILITIES.lock().await;
        let read = fetch_capabilities(&state, mode, |_| async {
            panic!("unexpected upstream GET")
        });
        tokio::pin!(read);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut read)
                .await
                .is_err()
        );
        held.insert(key.clone(), (Instant::now(), supported.clone()));
        drop(held);
        let (destination, value) = read.await.unwrap();
        destination.verify(&key.1, &key.2).unwrap();
        assert!(reattach_deployment(destination, &value).is_some());
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn failed_owner_lookup_does_not_suppress_another_owners_image_and_success_is_shared() {
    let (state, key) = fixture().await;
    let a = owner(&state).await;
    let b = owner(&state).await;
    let egress = egress(vec![Ok(StatusCode::BAD_GATEWAY.into_response()), success()]);
    let image = TurnAttachment {
        artifact: Default::default(),
        id: Uuid::new_v4().to_string(),
        content_type: "image/png".into(),
        label: "pasted-image.png".into(),
        origin: "user_upload".into(),
        size: 20,
        pages: None,
        chat_delivery: None,
        image_input: None,
        image_input_reason: None,
        image_inspection: None,
    };
    let failed = lookup(&state, &egress, &a).await;
    assert!(failed.is_null());
    assert!(!CAPABILITIES.lock().await.contains_key(&key));
    assert_eq!(
        image_plan(std::slice::from_ref(&image), &failed, "https://nyx.example").1,
        vec![image.id.clone()]
    );

    let recovered = lookup(&state, &egress, &b).await;
    let (parts, omitted) = image_plan(&[image], &recovered, "https://nyx.example");
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0]["type"], "input_image");
    assert!(omitted.is_empty());
    // Deployment capabilities are static: even the first owner now reuses them.
    assert_eq!(lookup(&state, &egress, &a).await, recovered);
    assert_eq!(*egress.owners.lock().await, [a.id, b.id]);
    CAPABILITIES.lock().await.remove(&key);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn failed_and_invalid_capability_responses_are_not_cached() {
    let (state, key) = fixture().await;
    let a = owner(&state).await;
    let b = owner(&state).await;
    let egress = egress(vec![
        Err(AppError::Forbidden("private request failure".into())),
        Ok(StatusCode::BAD_GATEWAY.into_response()),
        Ok(axum::response::Response::new(Body::from("not JSON"))),
        Ok(axum::Json(Value::Null).into_response()),
        success(),
    ]);
    for _ in 0..4 {
        assert!(lookup(&state, &egress, &a).await.is_null());
        assert!(!CAPABILITIES.lock().await.contains_key(&key));
    }
    assert_eq!(lookup(&state, &egress, &a).await, capabilities());
    assert_eq!(lookup(&state, &egress, &b).await, capabilities());
    assert_eq!(egress.owners.lock().await.len(), 5);
    CAPABILITIES.lock().await.remove(&key);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_capability_misses_do_not_hold_the_cache_lock_during_egress() {
    let (state, key) = fixture().await;
    let a = owner(&state).await;
    let b = owner(&state).await;
    let mut egress = egress(vec![success(), success()]);
    // Neither response can finish until both requests enter upstream. Holding
    // the cache mutex across a request deadlocks this rendezvous. No sleep-based
    // comparison or assumption about scheduler timing is needed.
    egress.rendezvous = Some(Arc::new(Barrier::new(2)));
    let (first, second) = tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(lookup(&state, &egress, &a), lookup(&state, &egress, &b))
    })
    .await
    .expect("concurrent requests must reach upstream together");
    assert_eq!(first, capabilities());
    assert_eq!(second, capabilities());
    assert_eq!(egress.owners.lock().await.len(), 2);
    assert_eq!(lookup(&state, &egress, &a).await, capabilities());
    assert_eq!(egress.owners.lock().await.len(), 2);
    CAPABILITIES.lock().await.remove(&key);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn image_planning_reuses_owner_model_metadata_on_transports_without_discovery() {
    use crate::services::assistant_nyxagent_model_cache::{MODELS, ModelInfo};
    let (state, _) = fixture().await;
    let a = owner(&state).await;
    let b = owner(&state).await;
    let cache_key = format!("{}:{}", state.db.name(), a.id);
    MODELS.lock().await.insert(
        cache_key.clone(),
        (
            Instant::now(),
            Some(vec![ModelInfo {
                id: engine::DEFAULT_MODEL.into(),
                label: "Text model".into(),
                inference_credential: None,
                available: true,
                description: None,
                input_modalities: Some(vec!["text".into()]),
            }]),
        ),
    );
    let egress = egress(Vec::new());
    assert_eq!(
        images::model_support(&state, &egress, &a.context, engine::DEFAULT_MODEL, None).await,
        Some(false)
    );
    assert_eq!(
        images::model_support(&state, &egress, &b.context, engine::DEFAULT_MODEL, None).await,
        None
    );
    assert!(egress.owners.lock().await.is_empty());
    MODELS.lock().await.get_mut(&cache_key).unwrap().0 = Instant::now() - Duration::from_secs(61);
    assert_eq!(
        images::model_support(&state, &egress, &a.context, engine::DEFAULT_MODEL, None).await,
        None
    );
    MODELS.lock().await.remove(&cache_key);
    state.db.drop().await.unwrap();
}
