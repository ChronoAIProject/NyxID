//! Persisted v1 rows use the pre-native HMAC field order and BSON layout.
use super::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

// Frozen v1 byte framing, intentionally independent of the new identity encoder.
fn legacy_hash(key: &[u8], domain: &str, parts: &[&str]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    for part in std::iter::once(domain).chain(parts.iter().copied()) {
        mac.update(&(part.len() as u64).to_be_bytes());
        mac.update(part.as_bytes());
    }
    hex::encode(mac.finalize().into_bytes())
}

#[tokio::test]
async fn persisted_v1_intent_and_approved_challenge_redeem_through_new_service_and_saved_reply_delivery()
 {
    let j = Journey::new().await;
    let intent_id = uuid::Uuid::new_v4().to_string();
    let challenge_id = uuid::Uuid::new_v4().to_string();
    let token = Zeroizing::new(format!("nyx_pli_{}", "42".repeat(32)));
    let code = Zeroizing::new("P-ABCD-EFGH".to_owned());
    let purpose = "personal_nyxbot_v1";
    let key = j.state.auth_device_hmac_key.as_slice();
    let review = legacy_hash(
        key,
        "platform-personal-consent:v1",
        &[
            purpose,
            &challenge_id,
            &intent_id,
            &j.profile.id,
            "100",
            "1",
            "700",
            "700",
            &j.owner,
            "nyxmode",
            "1",
            "null",
        ],
    );
    let intent_hmac = legacy_hash(
        key,
        "platform-personal-intent:v1",
        &[
            purpose,
            &intent_id,
            &j.profile.id,
            "100",
            "1",
            "700",
            "700",
            token.as_str(),
        ],
    );
    let challenge_hmac = legacy_hash(
        key,
        "platform-personal-challenge:v1",
        &[
            purpose,
            &challenge_id,
            &intent_id,
            &j.profile.id,
            "100",
            "1",
            "700",
            "700",
            &j.owner,
            &review,
            code.as_str(),
        ],
    );
    let now = Utc::now();
    let expires = bson::DateTime::from_chrono(now + Duration::minutes(10));
    let purge = bson::DateTime::from_chrono(now + Duration::days(1));
    let intent = doc! {"_id":&intent_id,"purpose":purpose,"platform_config_id":&j.profile.id,"manager_bot_id":"100",
    "telegram_bot_id":100_i64,"manager_authority_epoch":1_i64,"telegram_sender_id":700_i64,"telegram_chat_id":700_i64,
    "initiation_ref":"pre-native-persisted-proof","token_lookup_hmac":legacy_hash(key,"platform-personal-intent-lookup:v1",&[token.as_str()]),
    "token_hmac":&intent_hmac,"state":"pending","claimant_user_id":&j.owner,"current_challenge_id":&challenge_id,"completion":bson::Bson::Null,
    "created_at":bson::DateTime::from_chrono(now),"expires_at":expires,"purge_after":purge};
    let challenge = doc! {"_id":&challenge_id,"intent_id":&intent_id,"purpose":purpose,"platform_config_id":&j.profile.id,"manager_bot_id":"100",
    "telegram_bot_id":100_i64,"manager_authority_epoch":1_i64,"telegram_sender_id":700_i64,"telegram_chat_id":700_i64,
    "reviewed_user_id":&j.owner,"reviewed_access_mode":"nyxmode","consent_version":1_i64,"review_digest":&review,
    "previous_binding":bson::Bson::Null,"state":"awaiting_chat_confirmation",
    "code_lookup_hmac":legacy_hash(key,"platform-personal-challenge-lookup:v1",&[code.as_str()]),"code_hmac":&challenge_hmac,
    "completion":bson::Bson::Null,"created_at":bson::DateTime::from_chrono(now),"expires_at":expires,"purge_after":purge};
    let intents = j
        .state
        .db
        .collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME);
    let challenges = j
        .state
        .db
        .collection::<Document>(crate::models::platform_link_intent::CHALLENGES);
    intents.insert_one(&intent).await.unwrap();
    challenges.insert_one(&challenge).await.unwrap();
    let preview =
        crate::services::platform_link_intent_service::preview(&j.state.db, key, token.as_str())
            .await
            .unwrap();
    assert!(matches!(
        preview.entry,
        EntryIdentity::TelegramV1 {
            telegram_bot_id: 100
        }
    ));
    let observed = crate::services::platform_link_intent_service::approve(
        &j.state.db,
        key,
        &j.owner,
        &challenge_id,
        &review,
        false,
    )
    .await
    .unwrap();
    assert!(
        observed.confirmation_command.is_none(),
        "old approved code is retained"
    );
    j.webhook(&j.update(7001, &format!("/confirm {}", code.as_str()), false))
        .await;
    let binding = j
        .state
        .db
        .collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .find_one(doc! {"platform_config_id":&j.profile.id,"state":"active"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.user_id, j.owner);
    assert!(matches!(
        binding.entry,
        EntryIdentity::TelegramV1 {
            telegram_bot_id: 100
        }
    ));
    let intent = intents
        .find_one(doc! {"_id":&intent_id})
        .await
        .unwrap()
        .unwrap();
    let challenge = challenges
        .find_one(doc! {"_id":&challenge_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(intent.get_str("state").unwrap(), "consumed");
    assert_eq!(challenge.get_str("state").unwrap(), "confirmed");
    assert!(intent.get_str("token_hmac").unwrap() == intent_hmac);
    assert!(challenge.get_str("code_hmac").unwrap() == challenge_hmac);
    for row in [&intent, &challenge] {
        assert!(
            !row.contains_key("identity_version")
                && !row.contains_key("platform_entry")
                && !row.contains_key("platform_sender")
        );
    }
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    j.webhook(&j.update(7002, "Continue from the persisted legacy proof", false))
        .await;
    let (destination, reply, saved) = j.saved().await;
    assert!(matches!(
        destination.manager.entry,
        EntryIdentity::TelegramV1 {
            telegram_bot_id: 100
        }
    ));
    assert_eq!(saved.user_id, j.owner);
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if j.state
                .db
                .collection::<PlatformManagerDelivery>(
                    crate::models::platform_manager_delivery::COLLECTION_NAME,
                )
                .find_one(doc! {"reference.reply_id":&reply.reply_id,"state":"sent"})
                .await
                .unwrap()
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let sent = j
        .state
        .db
        .collection::<PlatformManagerDelivery>(
            crate::models::platform_manager_delivery::COLLECTION_NAME,
        )
        .find_one(doc! {"reference.reply_id":&reply.reply_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!sent.telegram_message_ids.is_empty());
    assert!(sent.platform_message_ids.is_empty());
    assert!(matches!(
        sent.entry,
        EntryIdentity::TelegramV1 {
            telegram_bot_id: 100
        }
    ));
    assert_eq!(
        j.sends()
            .await
            .iter()
            .filter(|send| send["text"]
                .as_str()
                .unwrap()
                .contains("Retained personal answer"))
            .count(),
        1
    );
    assert!(
        engine::messages(&j.state.db, &j.owner, &reply.turn.conversation_id, 50, None)
            .await
            .unwrap()
            .iter()
            .any(|message| message.id == saved.id)
    );
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn persisted_v1_saved_origin_and_pending_delivery_decode_and_send_once_through_current_service()
 {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, saved) = j.prepared_reply(7101).await;
    let origin = saved.platform_origin.as_ref().unwrap();
    let crate::models::assistant_conversation::PlatformOriginSource::Ingress {
        receipt_id,
        claim_id,
    } = &origin.source
    else {
        panic!("saved original ingress required");
    };
    let Some(crate::models::assistant_conversation::PlatformReplyRouting::Captured {
        turn_id,
        expires_at,
        ..
    }) = &saved.platform_reply
    else {
        panic!("retained original delivery required");
    };
    // Import the old BSON projection independently of the current serializers.
    // The surrounding live binding/receipt is established by real producers.
    let old_thread = doc! {"platform":"telegram","profile_id":&j.profile.id,"manager_bot_id":"100","telegram_bot_id":100_i64,
    "binding_id":&origin.thread.binding_id,"binding_generation":origin.thread.binding_generation,"user_id":&j.owner,
    "nyxbot_agent_id":&origin.thread.nyxbot_agent_id};
    let old_origin = doc! {"thread":old_thread.clone(),"manager_authority_epoch":origin.manager_authority_epoch,
    "personal_control_revision":origin.personal_control_revision,"sender_id":700_i64,"chat_id":700_i64,
    "source":{"kind":"ingress","receipt_id":receipt_id,"claim_id":claim_id}};
    let old_reply = doc! {"kind":"captured","origin":old_origin.clone(),"turn_id":turn_id,
    "expires_at":bson::DateTime::from_chrono(*expires_at)};
    let old_message = doc! {"_id":&saved.id,"conversation_id":&saved.conversation_id,"user_id":&j.owner,
    "seq":saved.seq,"turn_id":&saved.turn_id,"role":"assistant","text":&saved.text,"status":"completed",
    "error_code":bson::Bson::Null,"platform_origin":old_origin.clone(),"platform_reply":old_reply,
    "platform_delivery_recovery_queued":false,"created_at":bson::DateTime::from_chrono(saved.created_at)};
    let messages = j.state.db.collection::<Document>(MESSAGES);
    messages
        .replace_one(doc! {"_id":&saved.id}, &old_message)
        .await
        .unwrap();
    j.state
        .db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&saved.conversation_id},
            doc! {"$set":{"platform_thread":old_thread}},
        )
        .await
        .unwrap();
    let delivery_id = uuid::Uuid::new_v4().to_string();
    use sha2::Digest;
    let dedupe = hex::encode(Sha256::digest(
        format!(
            "manager-send:v1:{}:100:{receipt_id}:reply:{}",
            j.profile.id, saved.id
        )
        .as_bytes(),
    ));
    let old_delivery = doc! {"_id":&delivery_id,"dedupe_key":dedupe,"profile_id":&j.profile.id,"manager_bot_id":"100",
    "telegram_bot_id":100_i64,"manager_authority_epoch":origin.manager_authority_epoch,"sender_id":700_i64,"chat_id":700_i64,
    "ingress_id":receipt_id,"ingress_claim_id":claim_id,"origin":old_origin.clone(),"origin_turn_id":turn_id,
    "reference":{"kind":"reply","conversation_id":&saved.conversation_id,"turn_id":&saved.turn_id,"reply_id":&saved.id},
    "state":"pending","claim_id":bson::Bson::Null,"telegram_message_ids":[],
    "expires_at":bson::DateTime::from_chrono(*expires_at),"retry_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()};
    let deliveries = j
        .state
        .db
        .collection::<Document>(crate::models::platform_manager_delivery::COLLECTION_NAME);
    deliveries.insert_one(&old_delivery).await.unwrap();
    let history = engine::messages(&j.state.db, &j.owner, &saved.conversation_id, 50, None)
        .await
        .unwrap();
    let imported = history
        .iter()
        .find(|message| message.id == saved.id)
        .unwrap();
    let decoded_origin = imported.platform_origin.as_ref().unwrap();
    assert!(decoded_origin == origin);
    assert!(bson::to_document(decoded_origin).unwrap() == old_origin);
    let body = delivery::render_personal(&j.state, imported, None)
        .await
        .unwrap();
    let before = j.sends().await.len();
    let outcome = j.deliver(&destination, &reply, body.clone()).await.unwrap();
    assert!(matches!(outcome, DeliveryOutcome::Sent { .. }));
    let sent = deliveries
        .find_one(doc! {"_id":&delivery_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sent.get_str("state").unwrap(), "sent");
    assert!(!sent.get_array("telegram_message_ids").unwrap().is_empty());
    assert!(sent.get_document("origin").unwrap() == &old_origin);
    assert!(
        !sent.contains_key("identity_version")
            && !sent.contains_key("platform_entry")
            && !sent.contains_key("platform_sender")
    );
    assert!(matches!(
        j.deliver(&destination, &reply, body.clone()).await.unwrap(),
        DeliveryOutcome::Duplicate
    ));
    assert_eq!(
        deliveries
            .count_documents(doc! {"reference.reply_id":&saved.id})
            .await
            .unwrap(),
        1
    );
    let sends = j.sends().await;
    assert_eq!(sends.len(), before + 1);
    assert_eq!(sends.last().unwrap()["text"], body);
    let mut persisted_message = messages
        .find_one(doc! {"_id":&saved.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        persisted_message.remove("platform_delivery_recovery_queued"),
        Some(bson::Bson::Boolean(true))
    );
    let mut old_content = old_message;
    old_content.remove("platform_delivery_recovery_queued");
    assert_eq!(persisted_message, old_content);
    j.state.db.drop().await.unwrap();
}
