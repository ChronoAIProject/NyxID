//! Verified controls use durable bounded replies; proof bytes never enter them.
use super::*;

async fn assert_fixed(state: &AppState, event: &EventIdentity, reason: UnavailableReason) {
    let ingress = state
        .db
        .collection::<PlatformManagerIngress>(INGRESS)
        .find_one(crate::services::platform_entry_scope::event_fields(event))
        .await
        .unwrap()
        .expect("exact control event receipt");
    let row = state
        .db
        .collection::<PlatformManagerDelivery>(
            crate::models::platform_manager_delivery::COLLECTION_NAME,
        )
        .find_one(doc! {"ingress_id":&ingress.id,"ingress_claim_id":&ingress.claim_id,"reference.kind":"fixed"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, DeliveryState::Sent);
    assert!(matches!(row.reference,
        crate::models::platform_manager_delivery::DeliveryReference::Fixed {reason: stored}
        if stored == format!("{reason:?}")));
}

#[test]
fn disabled_native_and_telegram_login_reports_paused_without_intent_or_execution() {
    crate::test_utils::run_native_journey_test("disabled_login_feedback", || async {
        let native = NativeJourney::new().await;
        let telegram = Journey::new().await;
        for state in [&native.state, &telegram.state] {
            state
                .db
                .collection::<Document>(
                    crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
                )
                .update_many(doc! {}, doc! {"$set":{"personal_enabled":false}})
                .await
                .unwrap();
        }
        // These fresh sender fixtures have never used an intent rate bucket.
        let admission = native.admission(720, "login", None).await;
        let native_event = admission.verified_update().event;
        native.dispatch(admission).await;
        telegram
            .webhook(&telegram.update(721, "/login", false))
            .await;
        let native_calls = native.server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&native_calls.last().unwrap().body).unwrap();
        let telegram_sends = telegram.sends().await;
        for text in [
            body["content"].as_str().unwrap(),
            telegram_sends.last().unwrap()["text"].as_str().unwrap(),
        ] {
            assert!(text.contains("Personal NyxBot is paused"));
            assert!(!text.contains("fresh account review") && !text.contains("nyx_pli_"));
        }
        let telegram_event = EventIdentity::TelegramV1 {
            update_id: 721,
            message_id: 721,
        };
        for (state, event) in [
            (&native.state, &native_event),
            (&telegram.state, &telegram_event),
        ] {
            assert_fixed(state, event, UnavailableReason::PersonalDisabled).await;
            assert_eq!(
                state
                    .db
                    .collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        assert_eq!(native.egress.0.load(Ordering::SeqCst), 0);
        assert_eq!(telegram.egress.0.load(Ordering::SeqCst), 0);
        native.state.db.drop().await.unwrap();
        telegram.state.db.drop().await.unwrap();
    });
}

#[test]
fn rejected_native_confirmation_has_actionable_provider_correct_durable_feedback() {
    crate::test_utils::run_native_journey_test("native_control_feedback", || async {
        let j = NativeJourney::new().await;
        let admission = j
            .admission(701, "confirm", Some(("code", "P-AAAA-BBBB")))
            .await;
        let event = admission.verified_update().event;
        j.dispatch(admission).await;
        let calls = j.server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&calls.last().unwrap().body).unwrap();
        let text = body["content"].as_str().unwrap();
        assert!(text.contains("confirmation could not be accepted"));
        assert!(text.contains("/nyx login") && text.contains("same private chat"));
        assert!(!text.contains("P-AAAA-BBBB") && !text.contains("unavailable request"));
        assert_fixed(&j.state, &event, UnavailableReason::ConfirmationRejected).await;
        assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
        j.state.db.drop().await.unwrap();
    });
}

#[tokio::test]
async fn malformed_and_rate_limited_telegram_controls_get_help_without_execution() {
    let j = Journey::new().await;
    j.webhook(&j.update(710, "/login unexpected-argument", false))
        .await;
    assert!(
        j.sends().await.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("command could not be completed")
    );
    assert_fixed(
        &j.state,
        &EventIdentity::TelegramV1 {
            update_id: 710,
            message_id: 710,
        },
        UnavailableReason::ControlInvalid,
    )
    .await;
    for _ in 0..10 {
        assert!(
            crate::services::coordination_service::RateWindowStore::admit(
                &j.state.db,
                "personal_intent_sender",
                &format!("{}:700", j.profile.id),
                10,
                std::time::Duration::from_secs(3600)
            )
            .await
            .unwrap()
            .allowed
        );
    }
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    j.webhook(&j.update(711, "/login", false)).await;
    let sends = j.sends().await;
    let text = sends.last().unwrap()["text"].as_str().unwrap();
    assert!(text.contains("Wait a moment") && text.contains("/help"));
    assert!(!text.contains("NyxBot is busy") && !text.contains("nyx_pli_"));
    assert_fixed(
        &j.state,
        &EventIdentity::TelegramV1 {
            update_id: 711,
            message_id: 711,
        },
        UnavailableReason::ControlRateLimited,
    )
    .await;
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
    assert_eq!(
        j.state
            .db
            .collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    // Runtime context overrides bad-code guidance while linking is paused.
    j.state
        .db
        .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":&j.profile.id},
            doc! {"$set":{"personal_enabled":false}},
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    j.webhook(&j.update(712, "/confirm P-AAAA-BBBB", false))
        .await;
    assert_fixed(
        &j.state,
        &EventIdentity::TelegramV1 {
            update_id: 712,
            message_id: 712,
        },
        UnavailableReason::PersonalDisabled,
    )
    .await;
    assert!(
        j.sends().await.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("Personal NyxBot is paused")
    );
    j.state.db.drop().await.unwrap();
}
