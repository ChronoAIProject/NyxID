//! #1897: real turn admission/settlement, both result consumers and Telegram claims.
use super::*;
use crate::models::assistant_conversation::{AssistantConversation, TurnOrigin};
use crate::services::{assistant_delegated_images as delegated, assistant_team_service as team};

fn request(id: &str) -> engine::TurnStart {
    engine::TurnStart::from(&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: Vec::new(),
        agent_id: None,
        conversation_id: Some(id.into()),
        text: "Get the result".into(),
        model: None,
        access_mode: None,
    })
}
async fn receiving(f: &mut Fixture) -> AssistantConversation {
    // The image fixture starts with a synthetic reply at seq 1. These journeys
    // use real turn admission, whose first input needs that sequence number.
    f.state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .delete_one(doc! {"conversation_id":&f.event.conversation_id,"turn_id":&f.event.turn_id,"role":"assistant"})
        .await.unwrap();
    let mut start = request(f.event.conversation_id.as_deref().unwrap());
    start.origin = TurnOrigin::Channel;
    start.reply_channel = Some(f.event.delivery.as_ref().unwrap().origin.clone());
    let row = engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
        .await
        .unwrap_or_else(|error| panic!("Receiving admission failed: {error}"));
    f.event.turn_id = Some(row.active_turn.as_ref().unwrap().turn_id.clone());
    let d = f.event.delivery.as_mut().unwrap();
    d.state = "pending".into();
    d.stream_deadline = Utc::now() - ChronoDuration::seconds(1);
    f.state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .replace_one(doc! {"_id":&f.event.id}, &f.event)
        .await
        .unwrap();
    row
}
async fn source(
    f: &Fixture,
    recipient: &AssistantConversation,
    name: &str,
) -> AssistantConversation {
    let (_, home) = team::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        OWNER,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: name.into(),
            description: "Image specialist".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "nyxbot",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let mut start = request(&home.id);
    start.origin = TurnOrigin::Orchestrator;
    start.report_to = Some(recipient.id.clone());
    let row = engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
        .await
        .unwrap_or_else(|error| panic!("Source admission failed: {error}"));
    let edge = row
        .active_turn
        .as_ref()
        .unwrap()
        .delegated_images
        .assignment
        .as_ref()
        .unwrap();
    assert_eq!(edge.recipient_conversation, recipient.id);
    assert_eq!(
        edge.assigning_turn,
        recipient.active_turn.as_ref().unwrap().turn_id
    );
    row
}
async fn attach(f: &Fixture, source: &AssistantConversation, machine: bool) -> TurnAttachment {
    let attachment = AssistantAttachment {
        secret_context_versions: Default::default(),
        id: Uuid::new_v4().to_string(),
        origin: "tool".into(),
        user_id: OWNER.into(),
        conversation_id: source.id.clone(),
        turn_id: source.active_turn.as_ref().unwrap().turn_id.clone(),
        content_type: "image/png".into(),
        size: 8,
        data_encrypted: f.state.encryption_keys.encrypt(b"pixels-0").await.unwrap(),
        created_at: Utc::now(),
    };
    f.state
        .db
        .collection::<AssistantAttachment>(crate::models::assistant_attachment::COLLECTION_NAME)
        .insert_one(&attachment)
        .await
        .unwrap();
    let metadata = TurnAttachment {
        id: attachment.id,
        content_type: attachment.content_type,
        size: 8,
        label: if machine {
            "Machine screenshot"
        } else {
            "camera"
        }
        .into(),
        origin: "tool".into(),
        pages: None,
        chat_delivery: Some(!machine),
        image_input: None,
        image_inspection: None,
    };
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&source.id},
            doc! {"$push":{"active_turn.attachments":bson::to_bson(&metadata).unwrap()}},
        )
        .await
        .unwrap();
    metadata
}
async fn finish(f: &Fixture, row: &AssistantConversation) {
    engine::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Finished".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap_or_else(|error| panic!("Source/recipient settlement failed: {error}"));
}
async fn wait(
    f: &Fixture,
    receiver: &AssistantConversation,
    agents: Vec<String>,
) -> serde_json::Value {
    let chat = acks::for_key(&f.state.db, OWNER, Some(&receiver.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let (value, error) = crate::handlers::assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__wait_for_subagents",
        &json!({"subagents":agents,"timeout_secs":1}),
    )
    .await;
    assert!(!error, "{value}");
    value
}
async fn refs(
    f: &Fixture,
    row: &AssistantConversation,
) -> Vec<crate::models::assistant_delegated_image::DeliveryReference> {
    engine::get(&f.state.db, OWNER, &row.id)
        .await
        .unwrap()
        .active_turn
        .unwrap()
        .delegated_images
        .references
}
async fn send_refs(
    f: &Fixture,
    row: &AssistantConversation,
    refs: Vec<crate::models::assistant_delegated_image::DeliveryReference>,
) -> images::ImageReport {
    let thread = thread(f).await;
    images::send_turn_images(
        &f.state,
        ImageSendContext {
            owner: OWNER,
            conversation_id: &row.id,
            turn_id: &row.active_turn.as_ref().unwrap().turn_id,
            channel: &f.channel,
            thread: &thread,
            anchor: ImageAnchor::Inbound {
                message_id: f.event.relay_message_id.clone().unwrap(),
            },
            audience: ImageAudience {
                owner_private: true,
            },
            deadline: None,
            initiated: None,
            fence: None,
        },
        &ImageSendPlan {
            attachments: Vec::new(),
            delegated: refs,
        },
    )
    .await
    .unwrap()
}
async fn photo_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/sendPhoto"))
        .count()
}

#[tokio::test]
async fn delegated_images_assign_wait_telegram_and_late_restart_share_source_claims() {
    let (mut f, server) = image_fixture("telegram", "gateway").await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    let image = attach(&f, &sub, false).await;
    attach(&f, &sub, true).await;
    finish(&f, &sub).await;
    let value = wait(&f, &receiver, vec![sub.agent_id.clone().unwrap()]).await;
    assert_eq!(value["delegated_images"].as_array().unwrap().len(), 1);
    let adopted = refs(&f, &receiver).await;
    assert_eq!(adopted[0].attachment.id, image.id);
    assert!(
        engine::get(&f.state.db, OWNER, &receiver.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    finish(&f, &receiver).await;
    let persisted = delegated::settled_references(
        &f.state.db,
        OWNER,
        &receiver.id,
        receiver.active_turn.as_ref().unwrap().turn_id.as_str(),
    )
    .await
    .unwrap();
    assert_eq!(persisted.len(), 1);
    // No in-memory plan: the delayed sender reloads the settled reply after restart.
    crate::handlers::nyxbot::late_delivery::process(&f.state, &f.event.id).await;
    crate::handlers::nyxbot::late_delivery::process(&f.state, &f.event.id).await;
    assert_eq!(photo_count(&server).await, 1);
    let repeated = send_refs(&f, &receiver, persisted).await;
    assert_eq!(repeated.outcomes[0].status, "previously_delivered");
    let claim = f
        .state
        .db
        .collection::<NyxbotImageDelivery>(crate::models::nyxbot_image_delivery::COLLECTION_NAME)
        .find_one(doc! {"attachment_id":&image.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim.conversation_id, sub.id);
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn delegated_images_async_event_adopts_several_exact_results_and_sends_once() {
    let (mut f, server) = image_fixture("telegram", "direct").await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let receiver = receiving(&mut f).await;
    let a = source(&f, &receiver, "camera-a").await;
    let b = source(&f, &receiver, "camera-b").await;
    attach(&f, &a, false).await;
    attach(&f, &b, false).await;
    finish(&f, &receiver).await;
    finish(&f, &a).await;
    finish(&f, &b).await;
    let mut start = request(&receiver.id);
    start.origin = TurnOrigin::Event;
    let event = engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
        .await
        .unwrap();
    let adopted = refs(&f, &event).await;
    assert_eq!(adopted.len(), 2);
    assert!(
        adopted
            .iter()
            .all(|r| r.recipient_turn == event.active_turn.as_ref().unwrap().turn_id)
    );
    finish(&f, &event).await;
    // The same settled projection drives event and explicit delivery consumers.
    let plan = ImageSendPlan {
        attachments: Vec::new(),
        delegated: delegated::settled_references(
            &f.state.db,
            OWNER,
            &event.id,
            &event.active_turn.as_ref().unwrap().turn_id,
        )
        .await
        .unwrap(),
    };
    crate::handlers::nyxbot::deliver_to(
        &f.state,
        &event,
        &f.event.delivery.as_ref().unwrap().origin,
        "Finished",
        Some(&plan),
    )
    .await;
    assert_eq!(photo_count(&server).await, 2);
    assert_eq!(send_refs(&f, &event, adopted).await.sent, 0);
}

#[tokio::test]
async fn delegated_images_wait_racing_event_admission_has_one_consumer() {
    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    attach(&f, &sub, false).await;
    finish(&f, &sub).await;
    let mut event = request(&receiver.id);
    event.origin = TurnOrigin::Event;
    let agents = vec![sub.agent_id.clone().unwrap()];
    let turn = &receiver.active_turn.as_ref().unwrap().turn_id;
    let (adopted, drain) = tokio::join!(
        delegated::consume_wait(&f.state.db, OWNER, &receiver.id, turn, &agents),
        engine::begin_turn(&f.state.db, OWNER, event, &f.state.encryption_keys)
    );
    assert_eq!(adopted.unwrap().references.len(), 1);
    assert!(matches!(drain, Err(AppError::AssistantTurnActive)));
    finish(&f, &receiver).await;
    let mut event = request(&receiver.id);
    event.origin = TurnOrigin::Event;
    assert!(
        engine::begin_turn(&f.state.db, OWNER, event, &f.state.encryption_keys)
            .await
            .is_err()
    );
    // Opposite winner: once the assigning turn settles, a stale wait must not
    // pull the result out from under the freshly admitted Event turn.
    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    attach(&f, &sub, false).await;
    finish(&f, &sub).await;
    finish(&f, &receiver).await;
    let mut start = request(&receiver.id);
    start.origin = TurnOrigin::Event;
    let agents = vec![sub.agent_id.clone().unwrap()];
    let (waited, event) = tokio::join!(
        delegated::consume_wait(
            &f.state.db,
            OWNER,
            &receiver.id,
            &receiver.active_turn.as_ref().unwrap().turn_id,
            &agents
        ),
        engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
    );
    assert!(matches!(waited, Err(AppError::AssistantTurnRequired)));
    let event = event.unwrap();
    assert_eq!(
        event.active_turn.unwrap().delegated_images.references.len(),
        1
    );
    assert!(
        engine::get(&f.state.db, OWNER, &receiver.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
}

#[tokio::test]
async fn delegated_images_reassignment_and_direct_source_messages_do_not_rebind_completed_result() {
    let (mut f, server) = image_fixture("telegram", "direct").await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    let first = attach(&f, &sub, false).await;
    finish(&f, &sub).await;
    f.state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME).update_one(doc! {"conversation_id":&sub.id,"turn_id":&sub.active_turn.as_ref().unwrap().turn_id,"role":"assistant"},doc! {"$set":{"text":"Original assignment answer"}}).await.unwrap();
    // Another assignment/result for the same agent must not be consumed by agent alone.
    let other =
        crate::services::assistant_authority_tests::new_orchestrator(&f.state, OWNER, "Other work")
            .await;
    let mut start = request(&sub.id);
    start.origin = TurnOrigin::Orchestrator;
    start.report_to = Some(other.id.clone());
    let reassigned = engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
        .await
        .unwrap();
    attach(&f, &reassigned, false).await;
    finish(&f, &reassigned).await;
    let direct = engine::begin_turn(
        &f.state.db,
        OWNER,
        request(&sub.id),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    attach(&f, &direct, false).await;
    finish(&f, &direct).await;
    let waited = wait(&f, &receiver, vec![sub.agent_id.clone().unwrap()]).await;
    assert_eq!(
        waited["settled"][0]["reply"]["text"],
        "Original assignment answer"
    );
    let adopted = refs(&f, &receiver).await;
    assert_eq!(adopted.len(), 1);
    assert_eq!(adopted[0].attachment.id, first.id);
    assert_eq!(
        engine::get(&f.state.db, OWNER, &other.id)
            .await
            .unwrap()
            .pending_events
            .len(),
        1
    );
    assert_eq!(send_refs(&f, &receiver, adopted).await.sent, 1);
}

#[tokio::test]
async fn delegated_images_stop_reset_deletion_wrong_turn_and_guest_fail_closed() {
    for reason in [
        "stop",
        "reset",
        "source_deleted",
        "wrong_turn",
        "guest",
        "retention",
    ] {
        let (mut f, server) = image_fixture("telegram", "direct").await;
        mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
        let receiver = receiving(&mut f).await;
        let sub = source(&f, &receiver, "camera").await;
        let image = attach(&f, &sub, false).await;
        finish(&f, &sub).await;
        wait(&f, &receiver, vec![sub.agent_id.clone().unwrap()]).await;
        let mut adopted = refs(&f, &receiver).await;
        assert_eq!(adopted.len(), 1);
        match reason {
            "stop" => {
                engine::request_stop(&f.state.db, OWNER, &receiver.id)
                    .await
                    .unwrap();
            }
            "reset" => {
                f.state
                    .db
                    .collection::<bson::Document>(CONVERSATIONS)
                    .update_one(
                        doc! {"_id":&receiver.id},
                        doc! {"$set":{"context_reset_at":bson::DateTime::now()}},
                    )
                    .await
                    .unwrap();
            }
            "source_deleted" => {
                engine::delete(&f.state.db, OWNER, &sub.id).await.unwrap();
            }
            "wrong_turn" => {
                adopted[0].assignment.source_turn = "foreign-turn".into();
            }
            "guest" => {
                f.state
                    .db
                    .collection::<bson::Document>(CONVERSATIONS)
                    .update_one(doc! {"_id":&receiver.id}, doc! {"$set":{"guest_turn":true}})
                    .await
                    .unwrap();
            }
            "retention" => {
                f.state.db.collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME).update_one(doc! {"_id":&image.id},doc! {"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(400))}}).await.unwrap();
                f.state.db.collection::<bson::Document>(crate::models::platform_settings::COLLECTION_NAME).insert_one(doc! {"_id":crate::models::assistant_upload_retention::SETTINGS_ID,"revision":1_i64,"policy":bson::to_bson(&crate::models::assistant_upload_retention::Policy {tool_image_days:Some(1),..Default::default()}).unwrap()}).await.unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(send_refs(&f, &receiver, adopted).await.sent, 0, "{reason}");
        assert_eq!(photo_count(&server).await, 0, "{reason}");
    }
}

#[tokio::test]
async fn delegated_images_guest_and_wrong_source_never_adopt() {
    for wrong in [false, true] {
        let (mut f, _) = image_fixture("telegram", "direct").await;
        let receiver = receiving(&mut f).await;
        let sub = source(&f, &receiver, "camera").await;
        attach(&f, &sub, false).await;
        finish(&f, &sub).await;
        if wrong {
            f.state
                .db
                .collection::<bson::Document>(CONVERSATIONS)
                .update_one(
                    doc! {"_id":&receiver.id},
                    doc! {"$set":{"pending_events.0.delegated_result.source_turn":"wrong"}},
                )
                .await
                .unwrap();
        } else {
            f.state
                .db
                .collection::<bson::Document>(CONVERSATIONS)
                .update_one(doc! {"_id":&receiver.id}, doc! {"$set":{"guest_turn":true}})
                .await
                .unwrap();
        }
        let result = delegated::consume_wait(
            &f.state.db,
            OWNER,
            &receiver.id,
            &receiver.active_turn.as_ref().unwrap().turn_id,
            &[sub.agent_id.unwrap()],
        )
        .await;
        if wrong {
            assert!(result.unwrap().references.is_empty())
        } else {
            assert!(result.is_err())
        }
        assert!(refs(&f, &receiver).await.is_empty());
    }
}

#[tokio::test]
async fn delegated_images_org_membership_is_rechecked_at_adoption_and_send() {
    for revoke_before in [false, true] {
        let (mut f, server) = image_fixture("telegram", "direct").await;
        mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
        let receiver = receiving(&mut f).await;
        let sub = source(&f, &receiver, "camera").await;
        attach(&f, &sub, false).await;
        finish(&f, &sub).await;
        let org = Uuid::new_v4().to_string();
        f.state
            .db
            .collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &org,
                crate::models::user::UserType::Org,
            ))
            .await
            .unwrap();
        f.state
            .db
            .collection(crate::models::org_membership::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_membership(
                &org,
                OWNER,
                crate::models::org_membership::OrgRole::Member,
                None,
            ))
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(doc! {"_id":&sub.agent_id}, doc! {"$set":{"user_id":&org}})
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .update_one(doc! {"_id":&sub.id}, doc! {"$set":{"agent_owner_id":&org}})
            .await
            .unwrap();
        if revoke_before {
            f.state
                .db
                .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
                .update_one(
                    doc! {"org_user_id":&org,"member_user_id":OWNER},
                    doc! {"$set":{"revoked_at":bson::DateTime::now()}},
                )
                .await
                .unwrap();
        }
        let adopted = delegated::consume_wait(
            &f.state.db,
            OWNER,
            &receiver.id,
            &receiver.active_turn.as_ref().unwrap().turn_id,
            &[sub.agent_id.clone().unwrap()],
        )
        .await
        .unwrap()
        .references;
        assert_eq!(adopted.len(), usize::from(!revoke_before));
        if !revoke_before {
            f.state
                .db
                .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
                .update_one(
                    doc! {"org_user_id":&org,"member_user_id":OWNER},
                    doc! {"$set":{"revoked_at":bson::DateTime::now()}},
                )
                .await
                .unwrap();
            assert_eq!(send_refs(&f, &receiver, adopted).await.sent, 0);
        }
        assert_eq!(photo_count(&server).await, 0);
    }
}

#[tokio::test]
async fn delegated_images_clearing_home_dm_does_not_redirect_old_result() {
    let (mut f, server) = image_fixture("telegram", "direct").await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    // Real owner-home routing (as opposed to a dedicated non-owner chat thread).
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.event.conversation_id},
            doc! {"$set":{"channel":bson::Bson::Null}},
        )
        .await
        .unwrap();
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    attach(&f, &sub, false).await;
    finish(&f, &sub).await;
    wait(&f, &receiver, vec![sub.agent_id.clone().unwrap()]).await;
    let adopted = refs(&f, &receiver).await;
    finish(&f, &receiver).await;
    let app = engine::begin_turn(
        &f.state.db,
        OWNER,
        request(&receiver.id),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert!(app.reply_channel.is_none());
    assert_eq!(send_refs(&f, &receiver, adopted).await.sent, 0);
    assert_eq!(photo_count(&server).await, 0);
}

#[tokio::test]
async fn delegated_images_v1_barrier_uses_source_identity_and_guest_cannot_assign() {
    let (mut f, server) = image_fixture("telegram", "direct").await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    let image = attach(&f, &sub, false).await;
    finish(&f, &sub).await;
    wait(&f, &receiver, vec![sub.agent_id.clone().unwrap()]).await;
    f.state.db.collection::<bson::Document>(crate::models::nyxbot_image_delivery::LEGACY_COLLECTION_NAME).insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"user_id":OWNER,"conversation_id":&sub.id,"turn_id":&sub.active_turn.as_ref().unwrap().turn_id,
        "channel_id":&f.channel.id,"state":"done","items":[{"id":&image.id,"state":"sent"}],
    }).await.unwrap();
    let report = send_refs(&f, &receiver, refs(&f, &receiver).await).await;
    assert_eq!(report.outcomes[0].status, "previously_delivered");
    assert_eq!(photo_count(&server).await, 0);
    let mut chat = acks::for_key(&f.state.db, OWNER, Some(&receiver.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    chat.guest = true;
    let (_, error) = crate::handlers::assistant_team::execute_tool(
        &f.state,
        &chat,
        "nyxid__wait_for_subagents",
        &json!({"subagents":[sub.agent_id]}),
    )
    .await;
    assert!(error);
}

#[test]
fn delegated_images_legacy_projection_defaults_empty() {
    let value =
        serde_json::from_value::<crate::models::assistant_delegated_image::DelegatedImages>(json!(
            {}
        ))
        .unwrap();
    assert!(value.assignment.is_none());
    assert!(value.publication_refusal.is_none());
    assert!(value.references.is_empty());
}

async fn replace_pending(
    f: &Fixture,
    receiver: &AssistantConversation,
    events: &[crate::models::assistant_conversation::AgentEvent],
) {
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&receiver.id},
            doc! {"$set":{"pending_events":bson::to_bson(events).unwrap()}},
        )
        .await
        .unwrap();
}

fn delegated_queue(
    sub: &AssistantConversation,
) -> Vec<crate::models::assistant_conversation::AgentEvent> {
    (0..engine::MAX_PENDING_EVENTS)
        .map(|_| {
            let mut event = team::event(
                "subagent_settled",
                "Earlier result".into(),
                sub.agent_id.as_deref(),
            );
            let mut edge = sub
                .active_turn
                .as_ref()
                .unwrap()
                .delegated_images
                .assignment
                .clone()
                .unwrap();
            edge.id = event.id.clone();
            edge.source_turn = event.id.clone();
            event.delegated_result = Some(edge);
            event
        })
        .collect()
}

async fn refused_publication_settles(queue_kind: &str) {
    use crate::models::assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES};
    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let mut sub = source(&f, &receiver, "camera").await;
    attach(&f, &sub, false).await;
    let mut events = if queue_kind == "delegated" {
        // An invocation-bound producer still refuses a full queue, including
        // delegated results that an ordinary producer is now allowed to evict.
        let invocation = Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_invocation::COLLECTION_NAME)
            .insert_one(
                doc! {"_id":&invocation,"user_id":OWNER,"cancel_requested":false,
                "terminal_status":null,"conversation_deleted":false,"generation":0_i64},
            )
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id":&sub.id},
                doc! {"$set":{"active_turn.invocation_id":&invocation}},
            )
            .await
            .unwrap();
        sub.active_turn.as_mut().unwrap().invocation_id = Some(invocation);
        delegated_queue(&sub)
    } else {
        (0..engine::MAX_PENDING_EVENTS)
            .map(|_| team::event("notice", "Existing work".into(), None))
            .collect()
    };
    if queue_kind == "invocation" {
        events[0].invocation_id = Some(Uuid::new_v4().to_string());
    } else if queue_kind == "source" {
        for event in &mut events {
            event.source_event_ids = vec![Uuid::new_v4().to_string()];
        }
    }
    replace_pending(&f, &receiver, &events).await;
    finish(&f, &sub).await;
    let saved = f.state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
        "conversation_id":&sub.id,"turn_id":&sub.active_turn.as_ref().unwrap().turn_id,"role":"assistant",
    }).await.unwrap().unwrap();
    assert_eq!(saved.status, "completed");
    assert_eq!(saved.text, "Finished");
    assert_eq!(saved.attachments.len(), 1);
    assert_eq!(
        saved.delegated_images.publication_refusal.as_deref(),
        Some("event_queue_full")
    );
    assert!(
        engine::get(&f.state.db, OWNER, &sub.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    let pending = engine::get(&f.state.db, OWNER, &receiver.id)
        .await
        .unwrap()
        .pending_events;
    assert_eq!(
        bson::to_bson(&pending).unwrap(),
        bson::to_bson(&events).unwrap()
    );

    // The legacy after-settlement path remains best effort. While capacity is
    // unavailable it cannot replace protected work or undo the stored reply.
    Box::pin(crate::handlers::assistant_team::after_turn(
        &f.state, &sub, "Finished", None,
    ))
    .await;
    let pending = engine::get(&f.state.db, OWNER, &receiver.id)
        .await
        .unwrap()
        .pending_events;
    assert_eq!(
        bson::to_bson(&pending).unwrap(),
        bson::to_bson(&events).unwrap()
    );
    events.pop();
    replace_pending(&f, &receiver, &events).await;
    Box::pin(crate::handlers::assistant_team::after_turn(
        &f.state, &sub, "Finished", None,
    ))
    .await;
    let pending = engine::get(&f.state.db, OWNER, &receiver.id)
        .await
        .unwrap()
        .pending_events;
    let notice = pending.last().unwrap();
    assert_eq!(notice.kind, "subagent_settled");
    assert!(notice.text.contains("Finished"));
    assert!(notice.delegated_result.is_none());
    assert_eq!(
        notice.invocation_id,
        sub.active_turn.as_ref().unwrap().invocation_id
    );
    let result = wait(&f, &receiver, vec![sub.agent_id.clone().unwrap()]).await;
    assert!(result["delegated_images"].as_array().unwrap().is_empty());
    assert!(refs(&f, &receiver).await.is_empty());
}

#[tokio::test]
async fn delegated_images_invocation_full_queue_keeps_settlement_and_text_fallback() {
    refused_publication_settles("invocation").await;
}

#[tokio::test]
async fn delegated_images_all_delegated_full_queue_keeps_invocation_settlement_and_text_fallback() {
    refused_publication_settles("delegated").await;
}

#[tokio::test]
async fn delegated_images_source_full_queue_keeps_settlement_and_text_fallback() {
    refused_publication_settles("source").await;
}

#[tokio::test]
async fn delegated_images_full_queue_evicts_oldest_result_and_still_settles() {
    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    attach(&f, &sub, false).await;
    let events = delegated_queue(&sub);
    replace_pending(&f, &receiver, &events).await;
    finish(&f, &sub).await;
    let pending = engine::get(&f.state.db, OWNER, &receiver.id)
        .await
        .unwrap()
        .pending_events;
    assert_eq!(pending.len(), engine::MAX_PENDING_EVENTS);
    assert_eq!(pending[0].id, events[1].id);
    assert_eq!(
        pending.last().unwrap().id,
        sub.active_turn.as_ref().unwrap().turn_id
    );
    let result = wait(&f, &receiver, vec![sub.agent_id.clone().unwrap()]).await;
    assert_eq!(result["delegated_images"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn delegated_images_do_not_starve_ordinary_producers_or_change_legacy_fifo() {
    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    let delegated = delegated_queue(&sub);
    for scenario in ["ordinary", "mixed", "delegated"] {
        let mut events = if scenario == "delegated" {
            delegated.clone()
        } else {
            (0..engine::MAX_PENDING_EVENTS)
                .map(|_| team::event("notice", "Existing notice".into(), None))
                .collect::<Vec<_>>()
        };
        let evicted = if scenario == "mixed" {
            events[0].source_event_ids = vec![Uuid::new_v4().to_string()];
            events[1] = delegated[0].clone();
            1
        } else {
            0
        };
        replace_pending(&f, &receiver, &events).await;
        let incoming = team::event("notice", "New notice".into(), None);
        engine::push_events(&f.state.db, OWNER, &receiver.id, vec![incoming.clone()])
            .await
            .unwrap();
        events.remove(evicted);
        events.push(incoming);
        let pending = engine::get(&f.state.db, OWNER, &receiver.id)
            .await
            .unwrap()
            .pending_events;
        assert_eq!(
            bson::to_bson(&pending).unwrap(),
            bson::to_bson(&events).unwrap(),
            "{scenario}"
        );
    }
}

#[tokio::test]
async fn delegated_images_cancelled_source_has_no_deliverable_result() {
    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    attach(&f, &sub, false).await;
    engine::request_stop(&f.state.db, OWNER, &sub.id)
        .await
        .unwrap();
    finish(&f, &sub).await;
    let value = wait(&f, &receiver, vec![sub.agent_id.clone().unwrap()]).await;
    assert!(value["delegated_images"].as_array().unwrap().is_empty());
    assert!(refs(&f, &receiver).await.is_empty());
}

#[tokio::test]
async fn delegated_images_cancelled_invocation_settles_without_publishing() {
    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    attach(&f, &sub, false).await;
    let invocation = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_invocation::COLLECTION_NAME)
        .insert_one(doc! {"_id":&invocation,"user_id":OWNER,"cancel_requested":true})
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&sub.id},doc! {"$set":{"active_turn.invocation_id":invocation,"active_turn.stop_requested":true}})
        .await.unwrap();
    finish(&f, &sub).await;
    assert!(
        engine::get(&f.state.db, OWNER, &sub.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    assert!(
        engine::get(&f.state.db, OWNER, &receiver.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
}

#[tokio::test]
async fn delegated_images_trigger_projection_keeps_source_and_original_destination() {
    let (mut f, server) = image_fixture("telegram", "direct").await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let dispatches = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = dispatches.clone();
    Mock::given(path("/bottoken/sendPhoto"))
        .respond_with(move |_: &wiremock::Request| {
            observed.lock().unwrap().push(std::time::Instant::now());
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":42}}))
        })
        .with_priority(1)
        .expect(2)
        .mount(&server)
        .await;
    let mut receiver = receiving(&mut f).await;
    let run = Uuid::new_v4().to_string();
    let target = thread(&f).await;
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"_id":&target.id},
            doc! {"$set":{
                "kind":"group","owner_chat":false,"platform_chat_id":"-100123",
            }},
        )
        .await
        .unwrap();
    receiver.active_turn.as_mut().unwrap().trigger_run_id = Some(run.clone());
    receiver.active_turn.as_mut().unwrap().origin = TurnOrigin::Trigger;
    f.state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .replace_one(doc! {"_id":&receiver.id}, &receiver)
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME).insert_one(doc! {
        "_id":&run,"user_id":OWNER,"thread_id":&receiver.id,"turn_id":&receiver.active_turn.as_ref().unwrap().turn_id,
        "trigger_id":Uuid::new_v4().to_string(),"scheduled_at":bson::DateTime::now(),
        "deadline":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(1)),
        "outcome":"started","fence":Uuid::new_v4().to_string(),
        "lease_until":bson::DateTime::now(),
        "expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::days(7)),
        "deliver_to":{"type":"chat","chat_id":&target.id},"image_refs":[],
    }).await.unwrap();
    let sub = source(&f, &receiver, "camera").await;
    let image = attach(&f, &sub, false).await;
    attach(&f, &sub, false).await;
    finish(&f, &sub).await;
    wait(&f, &receiver, vec![sub.agent_id.clone().unwrap()]).await;
    let adopted = refs(&f, &receiver).await;
    assert_eq!(adopted.len(), 2);
    assert_eq!(
        adopted[0].assignment.trigger_chat.as_deref(),
        Some(target.id.as_str())
    );
    finish(&f, &receiver).await;
    // Recover only the durable run, as a restarted automation sender does.
    let saved = f
        .state
        .db
        .collection::<crate::models::trigger_run::TriggerRun>(
            crate::models::trigger_run::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&run})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.delegated_image_refs[0].attachment.id, image.id);
    assert_eq!(
        saved.delegated_image_refs[0].assignment.source_conversation,
        sub.id
    );
    assert!(saved.image_refs.is_empty());
    for _ in 0..2 {
        crate::handlers::nyxbot::chats::post_automation_images(
            &f.state,
            &saved,
            &target.id,
            None,
            &receiver.id,
            "Finished",
        )
        .await
        .unwrap();
    }
    let sent = dispatches.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert!(sent[1].duration_since(sent[0]) >= std::time::Duration::from_millis(900));
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(doc! {"_id":&target.id}, doc! {"$set":{"allow_posts":false}})
        .await
        .unwrap();
    assert!(
        crate::handlers::nyxbot::chats::post_automation_images(
            &f.state,
            &saved,
            &target.id,
            None,
            &receiver.id,
            "Finished",
        )
        .await
        .is_err()
    );
    assert_eq!(photo_count(&server).await, 2);
}

#[tokio::test]
async fn delegated_images_projection_is_bounded_and_never_adopts_foreign_turn_bytes() {
    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let mut agents = Vec::new();
    for name in ["camera-a", "camera-b", "camera-c"] {
        let sub = source(&f, &receiver, name).await;
        for _ in 0..engine::MAX_TURN_ATTACHMENTS {
            attach(&f, &sub, false).await;
        }
        finish(&f, &sub).await;
        agents.push(sub.agent_id.unwrap());
    }
    let value = wait(&f, &receiver, agents).await;
    assert_eq!(
        value["delegated_images"].as_array().unwrap().len(),
        delegated::MAX_REFERENCES
    );
    finish(&f, &receiver).await;
    assert_eq!(
        delegated::settled_references(
            &f.state.db,
            OWNER,
            &receiver.id,
            &receiver.active_turn.as_ref().unwrap().turn_id
        )
        .await
        .unwrap()
        .len(),
        delegated::MAX_REFERENCES
    );

    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    let image = attach(&f, &sub, false).await;
    finish(&f, &sub).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&image.id},
            doc! {"$set":{"turn_id":"different-source-turn"}},
        )
        .await
        .unwrap();
    let value = wait(&f, &receiver, vec![sub.agent_id.unwrap()]).await;
    assert!(value["delegated_images"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn delegated_images_async_revoked_source_is_not_rendered_or_adopted() {
    let (mut f, _) = image_fixture("telegram", "direct").await;
    let receiver = receiving(&mut f).await;
    let sub = source(&f, &receiver, "camera").await;
    attach(&f, &sub, false).await;
    finish(&f, &receiver).await;
    finish(&f, &sub).await;
    engine::delete(&f.state.db, OWNER, &sub.id).await.unwrap();
    let mut start = request(&receiver.id);
    start.origin = TurnOrigin::Event;
    assert!(
        engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
            .await
            .is_err()
    );
    let saved = engine::get(&f.state.db, OWNER, &receiver.id).await.unwrap();
    assert!(saved.pending_events.is_empty());
    assert!(saved.active_turn.is_none());
}
