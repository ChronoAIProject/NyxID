//! Ordinary price components do not confer ownership on the voice protocol.
use super::*;
use crate::billing_integration_tests::usage_checkpoints::Fixture;
use crate::models::{
    service_billing::{BillingMetric, ResaleSpec},
    usage_meter::UsageMeterRow,
};
use chrono::Utc;
async fn fixture(live: bool, malformed: bool) -> (Fixture, String) {
    use crate::services::billing::grants::{IssueCreditGrantInput, issue_grants};
    let f = Fixture::new().await;
    let seed = f.open(false).await;
    f.db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    issue_grants(
        &f.db,
        IssueCreditGrantInput {
            amount_credits: 1,
            target_kind: crate::models::billing_target::BillingTargetKind::SelectedUsers,
            target_user_ids: vec![f.owner.clone()],
            target_org_ids: vec![],
            target_group_ids: vec![],
            all_services: true,
            service_refs: vec![],
            expires_at: None,
            reason: None,
            granted_by: f.owner.clone(),
            amount: None,
            service_limits: vec![],
            request_id: None,
        },
    )
    .await
    .unwrap();
    let id = if malformed {
        "voice:malformed:1".to_owned()
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    let mut route = seed.route.unwrap();
    route.platform_billable = true;
    route.service_platform_billable = true;
    let metrics = [
        BillingMetric::Tokens,
        BillingMetric::InputTokens,
        BillingMetric::OutputTokens,
        BillingMetric::CacheReadTokens,
        BillingMetric::CacheWriteTokens,
        BillingMetric::VoiceSeconds,
    ];
    let specs: Vec<_> = metrics
        .iter()
        .enumerate()
        .map(|(i, m)| ResaleSpec {
            metric: *m,
            lago_metric_code: format!("voice-test-{i}"),
        })
        .collect();
    for spec in &specs {
        let mut rate = crate::billing_integration_tests::rate(&spec.lago_metric_code, Utc::now());
        rate.credits_per_unit_micros = 0;
        rate.credits_per_unit_pico = Some(800_000);
        f.db.collection("billing_rate_cache")
            .insert_one(rate)
            .await
            .unwrap();
    }
    route.platform_metric = specs[0].metric;
    route.platform_lago_metric_code = specs[0].lago_metric_code.clone();
    route.platform_components = specs[1..].to_vec();
    route.billing_request_id = id.clone();
    let route = route.with_native_text_bound(2, 1, 32);
    let m = f.service.open(&route).await.unwrap();
    f.service.mark_forwarded(&m).await.unwrap();
    f.db.collection::<Document>(METERS).update_many(doc!{},doc!{"$set":{"created_at":bson::DateTime::from_millis(0),"forwarded_owner":{"generation_id":"gone","ingress":"llm_provider","marked_at":bson::DateTime::from_millis(0)}}}).await.unwrap();
    if live {
        f.db.collection::<Document>(METERS)
            .update_many(
                doc! {},
                doc! {"$set":{"forwarded_owner.generation_id":"survivor"}},
            )
            .await
            .unwrap();
    }
    f.db.collection::<Document>("security_replicas").insert_one(doc!{"_id":"survivor","generation_id":"survivor","started_at":bson::DateTime::now(),"observed_at":bson::DateTime::now()}).await.unwrap();
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        6
    );
    (f, id)
}

async fn rows(f: &Fixture) -> Vec<UsageMeterRow> {
    f.db.collection(METERS)
        .find(doc! {})
        .sort(doc! {"lago_metric_code":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn sweep(f: &Fixture) {
    // Cycle all three hold-bearing sources, then prove replay is harmless.
    for _ in 0..6 {
        recover(&f.db).await.unwrap();
    }
}
async fn clean(f: &Fixture) {
    f.clean().await;
    f.clean().await;
    assert_eq!(
        f.db.collection::<Document>("billing_hold_observations")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn voice_component_dead_request_returns_five_grant_holds_once() {
    let (f, id) = fixture(false, false).await;
    let before = rows(&f).await;
    assert_eq!(
        before
            .iter()
            .filter(|r| r
                .funding
                .as_ref()
                .is_some_and(|f| !f.grant_reservations.is_empty()))
            .count(),
        5
    );
    let voice = before
        .iter()
        .find(|r| r.metric == BillingMetric::VoiceSeconds)
        .unwrap();
    assert_eq!(voice.reserved_credits, Credits::ZERO);
    assert!(
        voice
            .funding
            .as_ref()
            .unwrap()
            .grant_reservations
            .is_empty()
    );
    clean(&f).await;
    let detail = diagnostics(&f.db)
        .await
        .unwrap()
        .remove(0)
        .technical_details
        .unwrap();
    assert!(
        detail.contains("class=eligible_now eligible=true age=over_7d rows=5 requests=1"),
        "{detail}"
    );
    assert!(!detail.contains("excluded_voice"), "{detail}");
    sweep(&f).await;
    let ended = rows(&f).await;
    assert!(
        ended.iter().all(|r| r.released && r.quantity == Some(0)),
        "{id}"
    );
    let grants =
        f.db.collection::<crate::models::credit_grant::CreditGrant>("credit_grants");
    let grant = grants.find_one(doc! {}).await.unwrap().unwrap();
    assert_eq!(grant.reserved, Credits::ZERO);
    assert_eq!(grant.remaining, Credits::from_whole(1));
    let audit = f.db.collection::<Document>("audit_log");
    assert_eq!(
        audit
            .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap(),
        1
    );
    let event = audit
        .find_one(doc! {"event_type":"billing_unknown_request_ended"})
        .await
        .unwrap()
        .unwrap();
    assert!(event.get("seq").is_some() && event.get_str("entry_hash").is_ok());
    clean(&f).await;
    sweep(&f).await;
    assert_eq!(
        audit
            .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        grants.find_one(doc! {}).await.unwrap().unwrap().remaining,
        grant.remaining
    );
    clean(&f).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn voice_component_live_owner_is_untouched() {
    let (f, _) = fixture(true, false).await;
    let before: Vec<Document> =
        f.db.collection(METERS)
            .find(doc! {})
            .sort(doc! {"_id":1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    let detail = diagnostics(&f.db)
        .await
        .unwrap()
        .remove(0)
        .technical_details
        .unwrap();
    assert!(
        detail.contains("class=owned_by_live_generation eligible=false"),
        "{detail}"
    );
    sweep(&f).await;
    let after: Vec<Document> =
        f.db.collection(METERS)
            .find(doc! {})
            .sort(doc! {"_id":1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap(),
        0
    );
    clean(&f).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn voice_component_malformed_voice_identity_stays_excluded() {
    let (f, id) = fixture(false, true).await;
    let detail = diagnostics(&f.db)
        .await
        .unwrap()
        .remove(0)
        .technical_details
        .unwrap();
    assert!(
        detail.contains("class=excluded_voice_unknown_shape eligible=false"),
        "{detail}"
    );
    assert!(terminal::publish(&f.db, &id).await.unwrap().is_empty());
    sweep(&f).await;
    assert!(
        rows(&f)
            .await
            .iter()
            .all(|r| !r.released && r.quantity.is_none())
    );
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap(),
        0
    );
    clean(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn voice_component_legacy_carrier_is_not_an_ordinary_request() {
    let (f, _) = fixture(false, false).await;
    let id = format!("adapter-call:{}", uuid::Uuid::new_v4());
    f.db.collection::<Document>(METERS)
        .update_many(
            doc! {},
            doc! {"$set":{"billing_request_id":&id},"$unset":{"forwarded_owner":""}},
        )
        .await
        .unwrap();
    let detail = diagnostics(&f.db)
        .await
        .unwrap()
        .remove(0)
        .technical_details
        .unwrap();
    assert!(
        detail.contains("class=excluded_channel_outbound eligible=false"),
        "{detail}"
    );
    assert!(terminal::publish(&f.db, &id).await.unwrap().is_empty());
    sweep(&f).await;
    assert!(
        rows(&f)
            .await
            .iter()
            .all(|r| !r.released && r.quantity.is_none())
    );
    clean(&f).await;
    f.db.drop().await.unwrap();
}
