use super::*;
use crate::services::{
    node_owner_service::ReplicaIdentity, security_readiness_service as readiness,
};

async fn observe(f: &Fixture) {
    readiness::observe(
        &f.db,
        &ReplicaIdentity {
            instance_name: "test-backend".into(),
            generation_id: "test-process".into(),
            internal_base_url: "http://127.0.0.1:3002".into(),
        },
        &readiness::RolloutConfig::parse("", false).unwrap(),
        false,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn stuck_request_forwarding_stamps_generation_without_an_extra_command() {
    let f = Fixture::new().await;
    readiness::ready_test_replica(&f.db).await;
    readiness::stop_test_replica(&f.db).await;
    observe(&f).await;
    f.count();
    let _metered = f.open(true).await;
    let commands = f.count();
    let raw =
        f.db.collection::<bson::Document>(USAGE_METER)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    f.db.drop().await.unwrap();
    assert_eq!(commands, 19);
    let owner = raw
        .get_document("forwarded_owner")
        .expect("forwarding must record its process generation");
    assert_eq!(owner.get_str("generation_id").unwrap(), "test-process");
    assert_eq!(owner.get_str("ingress").unwrap(), "llm_gateway");
    assert!(owner.get_datetime("marked_at").is_ok());
}

#[tokio::test]
async fn stuck_request_registry_registration_time_is_durable_and_not_a_heartbeat_time() {
    let f = Fixture::new().await;
    observe(&f).await;
    let rows = f.db.collection::<bson::Document>("security_replicas");
    let first = rows.find_one(doc! {}).await.unwrap().unwrap();
    observe(&f).await;
    let second = rows.find_one(doc! {}).await.unwrap().unwrap();
    f.db.drop().await.unwrap();
    let started = first
        .get_datetime("started_at")
        .expect("registry must preserve registration before serving");
    assert_eq!(started, second.get_datetime("started_at").unwrap());
    assert!(second.get_datetime("observed_at").unwrap() >= started);
}

#[tokio::test]
async fn stuck_request_classification_reports_legacy_eligibility_without_mutation() {
    let f = Fixture::new().await;
    observe(&f).await;
    let _metered = f.open(true).await;
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_many(
            doc! {},
            doc! {
                "$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-Duration::hours(2))},
                "$unset":{"forwarded_owner":""}
            },
        )
        .await
        .unwrap();
    let before =
        f.db.collection::<bson::Document>(USAGE_METER)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    let after =
        f.db.collection::<bson::Document>(USAGE_METER)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    f.db.drop().await.unwrap();
    assert_eq!(before, after);
    let diagnostic = diagnostics
        .iter()
        .find(|d| d.code == "billing_stuck_requests")
        .expect("classification must be visible before any write-off is enabled");
    assert!(
        diagnostic
            .technical_details
            .as_ref()
            .unwrap()
            .contains("class=eligible_now eligible=true age=1h_to_24h rows=1 requests=1"),
        "{}",
        diagnostic.technical_details.as_ref().unwrap()
    );
    assert!(
        diagnostic
            .technical_details
            .as_ref()
            .unwrap()
            .contains("kind=wallet unit=credits"),
        "{}",
        diagnostic.technical_details.as_ref().unwrap()
    );
}
