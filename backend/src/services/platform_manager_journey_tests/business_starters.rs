//! Loopback Telegram, real linking transactions, and recorded owner-turn input.
use super::*;
use crate::models::platform_use_case_selection::{COLLECTION_NAME as SELECTIONS, UseCaseSelection};

#[tokio::test]
async fn business_starter_deep_link_carries_published_context_across_login_once() {
    let j = Journey::new().await;
    super::super::super::platform_help_service::ensure(&j.state.db)
        .await
        .unwrap();
    j.state
        .db
        .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id": &j.profile.id},
            doc! {"$set": {"public_enabled": true}},
        )
        .await
        .unwrap();
    j.webhook(&j.update(5001, "/start uc_personal-assistant", false))
        .await;
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
    let sends = j.sends().await;
    let summary = sends.last().unwrap()["text"].as_str().unwrap();
    assert!(summary.contains("Personal assistant"));
    assert!(summary.contains("Summarize this forwarded message."));
    assert!(summary.contains("Send /login"));
    let row = j
        .state
        .db
        .collection::<UseCaseSelection>(SELECTIONS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(row.binding_id.is_none());
    j.link().await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let request = management::next_personal_request(&j, &j.owner, 5002)
        .await
        .to_string();
    assert!(
        request.contains("Before signing in, the owner chose the use case 'Personal assistant'")
    );
    assert!(request.contains("Summarize this forwarded message."));
    assert!(request.contains("wait for the owner's go-ahead"));
    assert_eq!(
        j.state
            .db
            .collection::<UseCaseSelection>(SELECTIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    j.webhook(&j.update(5003, "/start uc_restaurant-guide", true))
        .await;
    assert_eq!(
        j.state
            .db
            .collection::<UseCaseSelection>(SELECTIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    j.webhook(&j.update(5004, "/usecase restaurant-guide", false))
        .await;
    let row = j
        .state
        .db
        .collection::<UseCaseSelection>(SELECTIONS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(row.binding_id.is_some());
    let request = management::next_personal_request(&j, &j.owner, 5005)
        .await
        .to_string();
    assert!(request.contains("In this chat, the owner chose the use case 'Restaurant guide'"));
    assert!(request.contains("Help me prepare a restaurant information bot."));
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    j.control(5006, "/usecase personal-assistant")
        .await
        .unwrap();
    assert_eq!(
        j.state
            .db
            .collection::<UseCaseSelection>(SELECTIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    j.control(5007, "/logout").await.unwrap();
    assert_eq!(
        j.state
            .db
            .collection::<UseCaseSelection>(SELECTIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    j.state.db.drop().await.unwrap();
}
