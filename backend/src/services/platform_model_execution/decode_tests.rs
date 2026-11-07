use super::*;
use crate::services::platform_model_execution::images::DecodeBudget;
use std::sync::Arc;

fn body() -> Value {
    serde_json::from_str::<Value>(include_str!("fixtures/responses-images-v1.json")).unwrap()
        ["request"]
        .clone()
}

#[tokio::test]
async fn two_people_parse_images_concurrently_without_retaining_decode_slots() {
    let budget = Arc::new(DecodeBudget::new());
    let (a, b) = tokio::join!(
        Request::parse_with_budget(
            http::Method::POST,
            "chat/completions".into(),
            body(),
            "person-a",
            &budget,
        ),
        Request::parse_with_budget(
            http::Method::POST,
            "chat/completions".into(),
            body(),
            "person-b",
            &budget,
        ),
    );
    let requests = [a.unwrap(), b.unwrap()];
    assert!(requests.iter().all(Request::has_images));
    // Request values (and their clones) may wait for approvals/streams. They
    // must not retain either the weighted memory or per-person decode slot.
    let cloned = requests.clone();
    let a = budget.reserve("person-a", 0).unwrap();
    let b = budget.reserve("person-b", 0).unwrap();
    assert_eq!(cloned[0].body, requests[0].body);
    drop((a, b));
}

#[tokio::test]
async fn invalid_raster_releases_both_decode_limits() {
    let budget = Arc::new(DecodeBudget::new());
    let mut invalid = body();
    invalid["messages"][0]["content"][1]["image_url"]["url"] = json!("data:image/png;base64,AAAA");
    assert!(matches!(
        Request::parse_with_budget(
            http::Method::POST,
            "chat/completions".into(),
            invalid,
            "person",
            &budget,
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    let _permit = budget.reserve("person", 0).unwrap();
}
