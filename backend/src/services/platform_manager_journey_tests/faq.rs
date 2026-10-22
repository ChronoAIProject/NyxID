//! Real admission transactions and the production send pipeline against loopback Telegram.
use super::*;

#[tokio::test]
async fn faq_admission_survives_crash_and_preserves_canonical_modes_without_raw_questions() {
    let j = Journey::new().await;
    j.state
        .db
        .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":&j.profile.id},
            doc! {"$set":{"public_enabled":true,"public_policy":bson::Bson::Null}},
        )
        .await
        .unwrap();
    for (number, text) in [
        (201, "/faq connect-services"),
        (202, "/help"),
        (203, "/start"),
    ] {
        let payload = j.update(number, text, false);
        let update =
            dispatch::verified_message(&j.profile.id, "100", 100, "test-observation", &payload)
                .unwrap();
        let admission = dispatch::admit_text_update(
            &j.state.db,
            j.producers.identity.as_ref(),
            &update,
            Some(text),
        )
        .await
        .unwrap();
        let IngressAdmission::Claimed {
            ingress,
            audience: ManagerAudience::Public(_),
        } = admission
        else {
            panic!("FAQ must use public authority");
        };
        // Simulated process crash: no delivery call occurred after committing admission.
        let row = j
            .state
            .db
            .collection::<PlatformManagerDelivery>(
                crate::models::platform_manager_delivery::COLLECTION_NAME,
            )
            .find_one(doc! {"ingress_id":&ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, DeliveryState::Pending);
        let stored = bson::to_document(&row).unwrap().to_string();
        assert!(
            !stored.contains(text),
            "No raw question in recoverable metadata"
        );
        assert!(matches!(
            dispatch::admit_text_update(
                &j.state.db,
                j.producers.identity.as_ref(),
                &update,
                Some("different retry text")
            )
            .await
            .unwrap(),
            IngressAdmission::Duplicate { .. }
        ));
        let api = crate::services::telegram_new_api::TelegramApi {
            http: &j.state.http_client,
            base_url: &j.server.uri(),
        };
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        delivery::deliver_faq(&j.state, j.producers.identity.clone(), &ingress, &api)
            .await
            .unwrap();
        delivery::deliver_faq(&j.state, j.producers.identity.clone(), &ingress, &api)
            .await
            .unwrap();
    }
    assert_eq!(j.sends().await.len(), 3);
    j.link().await;
    for (number, personal) in [(204, true), (205, false)] {
        j.state.db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME).update_one(doc!{"platform_config_id":&j.profile.id},doc!{"$set":{"personal_enabled":personal},"$inc":{"personal_control_revision":1_i64}}).await.unwrap();
        let payload = j.update(number, "/faq connect-services", false);
        let update =
            dispatch::verified_message(&j.profile.id, "100", 100, "test-observation", &payload)
                .unwrap();
        let admission = dispatch::admit_text_update(
            &j.state.db,
            j.producers.identity.as_ref(),
            &update,
            Some("/faq connect-services"),
        )
        .await
        .unwrap();
        let IngressAdmission::Claimed {
            ingress,
            audience: ManagerAudience::Public(_),
        } = admission
        else {
            panic!("Explicit FAQ is public even for a linked sender in FAQ-only mode");
        };
        let api = crate::services::telegram_new_api::TelegramApi {
            http: &j.state.http_client,
            base_url: &j.server.uri(),
        };
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        delivery::deliver_faq(&j.state, j.producers.identity.clone(), &ingress, &api)
            .await
            .unwrap();
        let sends = j.sends().await;
        let body = sends.last().unwrap()["text"].as_str().unwrap();
        assert!(body.contains("Connecting services"));
        assert!(body.contains(if personal {
            "Already linked?"
        } else {
            "Personal work and linking are paused"
        }));
    }
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn faq_limits_reset_and_admission_abort_or_mode_change_cannot_send() {
    // Start at the last millisecond of a minute and advance explicitly. Future
    // windows also keep Mongo's real TTL monitor out of this clocked fixture.
    let minute = Utc::now().timestamp_millis() / 60_000 + 1440;
    let now = bson::DateTime::from_millis(minute * 60_000 + 59_999);
    crate::services::coordination_service::TEST_RATE_WINDOW_NOW
        .scope(std::cell::Cell::new(now), faq_limits_at_controlled_time())
        .await;
}

async fn faq_limits_at_controlled_time() {
    use crate::services::coordination_service::TEST_RATE_WINDOW_NOW;
    let j = Journey::new().await;
    j.state
        .db
        .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":&j.profile.id},
            doc! {"$set":{"public_enabled":true}},
        )
        .await
        .unwrap();
    let make_update = |number| {
        dispatch::verified_message(
            &j.profile.id,
            "100",
            100,
            "test-observation",
            &j.update(number, "/faq", false),
        )
        .unwrap()
    };
    let mut session = j.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let claim = IngressClaim {
        receipt_id: uuid::Uuid::new_v4().to_string(),
        claim_id: uuid::Uuid::new_v4().to_string(),
    };
    dispatch::admit_in_session(
        &j.state.db,
        &mut session,
        j.producers.identity.as_ref(),
        &make_update(300),
        &claim,
        Some("/faq"),
    )
    .await
    .unwrap();
    session.abort_transaction().await.unwrap();
    assert_eq!(
        j.state
            .db
            .collection::<Document>(INGRESS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        j.state
            .db
            .collection::<Document>(crate::models::platform_manager_delivery::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    for number in 301..321 {
        dispatch::admit_text_update(
            &j.state.db,
            j.producers.identity.as_ref(),
            &make_update(number),
            Some("/faq"),
        )
        .await
        .unwrap();
    }
    assert!(matches!(
        dispatch::admit_text_update(
            &j.state.db,
            j.producers.identity.as_ref(),
            &make_update(321),
            Some("/faq")
        )
        .await,
        Err(AppError::RateLimited)
    ));
    assert_eq!(
        j.state
            .db
            .collection::<Document>(INGRESS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        20
    );
    let windows = j
        .state
        .db
        .collection::<Document>(crate::models::coordination::RATE_WINDOW_COLLECTION_NAME);
    TEST_RATE_WINDOW_NOW.with(|clock| {
        clock.set(bson::DateTime::from_millis(
            clock.get().timestamp_millis() + 1,
        ));
    });
    let admitted = dispatch::admit_text_update(
        &j.state.db,
        j.producers.identity.as_ref(),
        &make_update(321),
        Some("/faq"),
    )
    .await
    .unwrap();
    let IngressAdmission::Claimed { ingress, .. } = admitted else {
        panic!("Window recovery admits the same event");
    };
    j.state
        .db
        .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":&j.profile.id},
            doc! {"$set":{"public_enabled":false},"$inc":{"public_policy_revision":1_i64}},
        )
        .await
        .unwrap();
    let api = crate::services::telegram_new_api::TelegramApi {
        http: &j.state.http_client,
        base_url: &j.server.uri(),
    };
    delivery::deliver_faq(&j.state, j.producers.identity.clone(), &ingress, &api)
        .await
        .unwrap();
    assert!(j.sends().await.is_empty());
    assert_eq!(
        j.state
            .db
            .collection::<PlatformManagerDelivery>(
                crate::models::platform_manager_delivery::COLLECTION_NAME
            )
            .find_one(doc! {"ingress_id":&ingress.receipt_id})
            .await
            .unwrap()
            .unwrap()
            .state,
        DeliveryState::Refused
    );
    assert!(matches!(
        dispatch::admit_text_update(
            &j.state.db,
            j.producers.identity.as_ref(),
            &make_update(322),
            Some("/faq")
        )
        .await
        .unwrap(),
        IngressAdmission::Claimed {
            audience: ManagerAudience::Fixed(UnavailableReason::PublicDisabled),
            ..
        }
    ));
    // A different sender cannot evade the finite profile window. Rejected
    // admissions leave no receipt; resetting the server-clock window recovers.
    j.state
        .db
        .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":&j.profile.id},
            doc! {"$set":{"public_enabled":true}},
        )
        .await
        .unwrap();
    windows
        .update_many(
            doc! {"namespace":"manager_faq_profile"},
            doc! {"$set":{"count":200_i64}},
        )
        .await
        .unwrap();
    let mut other = make_update(323);
    other.sender = SenderIdentity::TelegramV1 {
        telegram_sender_id: 999,
        telegram_chat_id: 999,
    };
    assert!(matches!(
        dispatch::admit_text_update(
            &j.state.db,
            j.producers.identity.as_ref(),
            &other,
            Some("/faq")
        )
        .await,
        Err(AppError::RateLimited)
    ));
    let mut reread = j.state.db.client().start_session().await.unwrap();
    assert!(
        crate::services::platform_manager_ingress_service::find_in_session(
            &j.state.db,
            &mut reread,
            &other
        )
        .await
        .unwrap()
        .is_none()
    );
    TEST_RATE_WINDOW_NOW.with(|clock| {
        clock.set(bson::DateTime::from_millis(
            clock.get().timestamp_millis() + 60_000,
        ));
    });
    assert!(matches!(
        dispatch::admit_text_update(
            &j.state.db,
            j.producers.identity.as_ref(),
            &other,
            Some("/faq")
        )
        .await
        .unwrap(),
        IngressAdmission::Claimed {
            audience: ManagerAudience::Public(_),
            ..
        }
    ));
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn canonical_unsupported_input_never_queues_faq_or_admits_personal_or_public_model_work() {
    let j = Journey::new().await;
    let runtime = j
        .state
        .db
        .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME);
    for (index, personal, public) in [(0, false, true), (1, true, true), (2, true, false)] {
        runtime
            .update_one(
                doc! {"platform_config_id":&j.profile.id},
                doc! {"$set":{"personal_enabled":personal,"public_enabled":public}},
            )
            .await
            .unwrap();
        let update = dispatch::verified_message(
            &j.profile.id,
            "100",
            100,
            "test-observation",
            &j.update(401 + index, "/unsupported", false),
        )
        .unwrap();
        assert!(matches!(
            dispatch::admit_text_update(
                &j.state.db,
                j.producers.identity.as_ref(),
                &update,
                Some("/unsupported")
            )
            .await
            .unwrap(),
            IngressAdmission::Claimed {
                audience: ManagerAudience::Fixed(UnavailableReason::UnsupportedInput),
                ..
            }
        ));
    }
    j.link().await;
    let update = dispatch::verified_message(
        &j.profile.id,
        "100",
        100,
        "test-observation",
        &j.update(410, "/unsupported", false),
    )
    .unwrap();
    assert!(matches!(
        dispatch::admit_text_update(
            &j.state.db,
            j.producers.identity.as_ref(),
            &update,
            Some("/unsupported")
        )
        .await
        .unwrap(),
        IngressAdmission::Claimed {
            audience: ManagerAudience::Fixed(UnavailableReason::UnsupportedInput),
            ..
        }
    ));
    assert_eq!(
        j.state
            .db
            .collection::<Document>(crate::models::platform_manager_delivery::COLLECTION_NAME)
            .count_documents(doc! {"reference.kind":"faq"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
    j.state.db.drop().await.unwrap();
}
