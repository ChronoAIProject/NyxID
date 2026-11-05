use super::*;
use crate::services::{
    assistant_oneshot_inference::tests::{ACTOR, utility_fixture},
    crm::{objects, records},
};
use mongodb::bson::{Document, doc};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

fn envelope() -> Value {
    json!({"formatVersion":1,"renderer":"json-render","rendererSpecVersion":"nyxid-crm-v1","queries":{},"spec":{"root":"text","elements":{"text":{"type":"Text","props":{"content":"Current records","variant":"body"},"children":[]}}}})
}
fn input() -> GenerateInput {
    GenerateInput {
        object: "company".into(),
        prompt: "Create a directory".into(),
        draft: None,
    }
}
fn receipt(key: &str) -> records::ReceiptContext {
    records::receipt_context(
        ACTOR,
        ACTOR,
        Some(key),
        "POST",
        "/views/generate",
        &[],
        &input(),
    )
    .unwrap()
    .unwrap()
}
async fn prepared() -> (AppState, wiremock::MockServer) {
    let (state, _, mock) = utility_fixture().await;
    crate::services::crm::ensure_indexes(&state.db)
        .await
        .unwrap();
    objects::ensure_seeded(&state.db, ACTOR).await.unwrap();
    (state, mock)
}
async fn completion(
    mock: &wiremock::MockServer,
    invalid_first: bool,
    delay: Duration,
) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    Mock::given(method("POST")).and(path("/chat/completions")).respond_with(move|_:&wiremock::Request|{let n=count.fetch_add(1,Ordering::SeqCst);let text=if invalid_first&&n==0{"not json".into()}else{envelope().to_string()};ResponseTemplate::new(200).set_body_json(json!({"choices":[{"finish_reason":"stop","message":{"content":text}}],"usage":{"prompt_tokens":8,"completion_tokens":6,"total_tokens":14}})).set_delay(delay)}).mount(mock).await;
    calls
}

#[tokio::test]
async fn crm_generation_claim_is_at_most_once_and_replays_before_new_metadata_preflight() {
    let (state, mock) = prepared().await;
    let calls = completion(&mock, false, Duration::from_millis(150)).await;
    let (a, b) = tokio::join!(
        Box::pin(generate_once(
            &state,
            ACTOR,
            ACTOR,
            input(),
            receipt("same")
        )),
        Box::pin(generate_once(
            &state,
            ACTOR,
            ACTOR,
            input(),
            receipt("same")
        ))
    );
    assert!(a.is_ok() || b.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let successful = a.or(b).unwrap();
    // Exact successful replay survives metadata that cannot support a new attempt.
    state
        .db
        .collection::<Document>(crate::models::crm_object::COLLECTION_NAME)
        .delete_many(doc! {"owner_id":ACTOR})
        .await
        .unwrap();
    let replay = Box::pin(generate_once(
        &state,
        ACTOR,
        ACTOR,
        input(),
        receipt("same"),
    ))
    .await
    .unwrap();
    assert_eq!(json!(replay), json!(successful));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut changed = receipt("same");
    changed.request_digest = "changed".into();
    assert!(matches!(
        Box::pin(generate_once(&state, ACTOR, ACTOR, input(), changed)).await,
        Err(AppError::CrmIdempotencyMismatch)
    ));
    use futures::TryStreamExt;
    let indexes: Vec<_> = state
        .db
        .collection::<Document>("crm_generation_jobs")
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(indexes.iter().any(|i| i.keys.get_i32("expires_at") == Ok(1)
        && i.options.as_ref().and_then(|o| o.expire_after) == Some(Duration::ZERO)));
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_generation_repairs_once_and_keeps_generation_namespace_separate() {
    let (state, mock) = prepared().await;
    let calls = completion(&mock, true, Duration::ZERO).await;
    let generated = Box::pin(generate_once(
        &state,
        ACTOR,
        ACTOR,
        input(),
        receipt("shared-key"),
    ))
    .await
    .unwrap();
    assert!(generated.valid);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    // Generation has its own claim/result namespace; ordinary writes use mutation receipts.
    let create = records::CreateRecordInput {
        object: "company".into(),
        values: serde_json::from_value(json!({"name":"Created"})).unwrap(),
        protect: vec![],
    };
    let mutation = records::receipt_context(
        ACTOR,
        ACTOR,
        Some("shared-key"),
        "POST",
        "/records",
        &[],
        &create,
    )
    .unwrap();
    records::create(
        &state.db,
        ACTOR,
        crate::models::crm_record::CrmActor::human(ACTOR),
        create,
        mutation,
    )
    .await
    .unwrap();
    assert!(
        Box::pin(generate_once(
            &state,
            ACTOR,
            ACTOR,
            input(),
            receipt("shared-key")
        ))
        .await
        .unwrap()
        .valid
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_generation_pre_dispatch_can_retry_but_dispatched_ambiguity_cannot_rebill() {
    let (state, mock) = prepared().await;
    crate::services::utility_inference_service::set(&state.db, None)
        .await
        .unwrap();
    assert!(
        Box::pin(generate_once(
            &state,
            ACTOR,
            ACTOR,
            input(),
            receipt("retry")
        ))
        .await
        .is_err()
    );
    let job = state
        .db
        .collection::<Document>("crm_generation_jobs")
        .find_one(doc! {"_id":receipt("retry").id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.get_str("state").unwrap(), "retryable");
    crate::services::utility_inference_service::set(
        &state.db,
        Some(crate::models::platform_settings::UtilityInference {
            service_slug: "chrono-llm-public".into(),
            model: "gpt-6-luna".into(),
        }),
    )
    .await
    .unwrap();
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    assert!(
        Box::pin(generate_once(
            &state,
            ACTOR,
            ACTOR,
            input(),
            receipt("retry")
        ))
        .await
        .is_err()
    );
    let count = mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method == "POST")
        .count();
    assert_eq!(count, 1);
    assert!(
        Box::pin(generate_once(
            &state,
            ACTOR,
            ACTOR,
            input(),
            receipt("retry")
        ))
        .await
        .is_err()
    );
    assert_eq!(
        mock.received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        count
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("crm_generation_jobs")
            .find_one(doc! {"_id":receipt("retry").id})
            .await
            .unwrap()
            .unwrap()
            .get_str("state")
            .unwrap(),
        "uncertain"
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_generation_never_substitutes_for_the_admin_pinned_model() {
    let (state, mock) = prepared().await;
    crate::services::utility_inference_service::set_route(
        &state.db,
        Some(crate::models::platform_settings::UtilityInference {
            service_slug: "chrono-llm-public".into(),
            model: "admin-pinned-crm-model".into(),
        }),
        crate::services::utility_inference_service::InferenceRoute::Crm,
    )
    .await
    .unwrap();

    let result = Box::pin(generate_once(
        &state,
        ACTOR,
        ACTOR,
        input(),
        receipt("pinned-model-missing"),
    ))
    .await;
    assert!(matches!(result, Err(AppError::CrmGenerationUnavailable)));
    let requests = mock.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.url.path())
            .collect::<Vec<_>>(),
        ["/models"]
    );
    assert!(requests.iter().all(|request| request.method == "GET"));
    state.db.drop().await.unwrap();
}
