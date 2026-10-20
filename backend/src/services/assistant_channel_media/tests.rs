use super::*;
use crate::models::{assistant_conversation::AgentEvent, assistant_upload::AssistantUpload};

#[test]
fn channel_media_old_documents_roundtrip_and_new_fields_survive_rewrites_with_redacted_debug() {
    let now = bson::DateTime::now();
    let mut event: AgentEvent =
        bson::from_document(doc! {"id":"event","kind":"message","text":"private","created_at":now})
            .unwrap();
    assert!(event.media_event_ids.is_empty());
    event.media_event_ids = vec!["event-source".into()];
    event.media_declared = Some(11);
    event.media_sent_at = Some(now.to_chrono());
    let rewritten: AgentEvent = bson::from_document(bson::to_document(&event).unwrap()).unwrap();
    assert_eq!(rewritten.media_event_ids, event.media_event_ids);
    assert_eq!(rewritten.media_sent_at, event.media_sent_at);
    let mut message:AssistantMessage=bson::from_document(doc! {"_id":"message","conversation_id":"conversation","user_id":"owner",
        "seq":1i64,"turn_id":"turn","role":"user","text":"private","status":"completed","created_at":now}).unwrap();
    assert!(message.media_pending.is_none());
    assert!(message.media_bound.is_none());
    message.media_event_ids = event.media_event_ids.clone();
    message.media_pending = Some(11);
    message.media_queue_notice_id = Some("notice".into());
    message.media_bound = Some(MediaBound {
        event_ids: event.media_event_ids.clone(),
        ok: Vec::new(),
        failed: Vec::new(),
        context: "private".into(),
    });
    let rewritten: AssistantMessage =
        bson::from_document(bson::to_document(&message).unwrap()).unwrap();
    assert_eq!(rewritten.media_pending, Some(11));
    assert_eq!(
        rewritten.media_queue_notice_id,
        message.media_queue_notice_id
    );
    assert_eq!(rewritten.media_bound.unwrap().context, "private");
    assert!(!format!("{message:?}").contains("private"));
    let mut upload:AssistantUpload=bson::from_document(doc! {"_id":"upload","origin":"user_upload","user_id":"owner","conversation_id":"conversation",
        "label":"secret-label","content_type":"image/png","size":4i64,"pages":0i64,"chunks":1i64,
        "text_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![42]},"created_at":now}).unwrap();
    assert!(upload.source.is_none());
    assert!(upload.content_sha256.is_none());
    upload.source = Some(UploadSource {
        turn_id: "turn".into(),
        event_key: "secret-key".into(),
        index: 0,
        embedded: true,
        declared: 11,
    });
    upload.content_sha256 = Some("digest".into());
    let rewritten: AssistantUpload =
        bson::from_document(bson::to_document(&upload).unwrap()).unwrap();
    assert_eq!(rewritten.source.unwrap().event_key, "secret-key");
    assert_eq!(rewritten.content_sha256.as_deref(), Some("digest"));
    assert_eq!(
        format!("{:?}", upload.source),
        "Some(UploadSource([REDACTED]))"
    );
    assert_eq!(
        format!("{:?}", message.media_bound),
        "Some(MediaBound([REDACTED]))"
    );
}

use crate::{
    services::{
        assistant_nyxagent as engine, assistant_team_service as team,
        assistant_upload_retention as retention,
    },
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};
use chrono::{Duration as ChronoDuration, Utc};
use futures::TryStreamExt;
use uuid::Uuid;

async fn fixture() -> (
    AppState,
    AssistantConversation,
    AssistantMessage,
    NyxbotEvent,
) {
    let db = connect_transaction_test_database("channel_media_binding").await;
    let owner = Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db);
    ensure_indexes(&state.db).await.unwrap();
    retention::ensure_indexes(&state.db).await.unwrap();
    let agent = team::ensure_nyxbot(&state.db, &owner).await.unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let thread = team::create_thread_for(
        &state.db,
        &state.encryption_keys,
        &owner,
        &agent,
        "media",
        &mut session,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    let row = Box::pin(engine::begin_turn(
        &state.db,
        &owner,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            conversation_id: Some(thread.id.clone()),
            attachment_ids: vec![],
            text: "receipt".into(),
            agent_id: None,
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    let turn = &row.active_turn.as_ref().unwrap().turn_id;
    let event_id = Uuid::new_v4().to_string();
    let event:NyxbotEvent=bson::from_document(doc! {
        "_id":&event_id,"channel_id":"channel","user_id":&owner,"partition":"private","event_id":"native",
        "status":"running","conversation_id":&row.id,"turn_id":turn,"created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::days(7)),
        "media":{"key":Uuid::new_v4().to_string(),"declared":1i64,"items":[{"index":0i64,"embedded":false,"kind":"image"}],"channel_bot_id":"bot","platform":"lark","platform_chat_id":"chat"}
    }).unwrap();
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&event)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(MESSAGES)
        .update_one(
            doc! {"conversation_id":&row.id,"turn_id":turn,"role":"user"},
            doc! {"$set":{"media_pending":1i64,"media_event_ids":[&event_id]}},
        )
        .await
        .unwrap();
    let message = message(&state.db, &row, turn).await.unwrap().unwrap();
    (state, row, message, event)
}

fn png() -> Vec<u8> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}
async fn upload(state: &AppState, row: &AssistantConversation, event: &NyxbotEvent) -> String {
    uploads::upload_channel_authorized(
        &state.db,
        &state.encryption_keys,
        &row.user_id,
        &row.id,
        "private-label.png",
        png(),
        UploadSource {
            turn_id: row.active_turn.as_ref().unwrap().turn_id.clone(),
            event_key: event.media.as_ref().unwrap().key.clone(),
            index: 0,
            embedded: false,
            declared: 1,
        },
    )
    .await
    .unwrap()
    .id
}
fn proposed(event: &NyxbotEvent, id: String) -> MediaBound {
    MediaBound {
        event_ids: vec![event.id.clone()],
        ok: vec![outcome(event, Some(0), false, "bound", Some(id))],
        failed: vec![],
        context: "bounded failure context".into(),
    }
}
async fn saved(state: &AppState, message: &AssistantMessage) -> AssistantMessage {
    state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {"_id":&message.id})
        .await
        .unwrap()
        .unwrap()
}
async fn settle(state: &AppState, row: &AssistantConversation, code: Option<&str>) {
    Box::pin(engine::finish_turn(
        &state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "done".into(),
            session_id: None,
            response_id: None,
            error: code.map(engine::TurnError::new),
        },
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn channel_media_bind_receipt_replays_after_lost_ack_stop_new_turn_and_reconciles_crash() {
    let (state, row, message, event) = fixture().await;
    let id = upload(&state, &row, &event).await;
    let proposed = proposed(&event, id.clone());
    // Drop the successful bind result, representing a lost commit acknowledgement
    // and process death before the separate event-outcome projection.
    binding::bind(&state.db, &message, &proposed).await.unwrap();
    let first = saved(&state, &message).await;
    let root = state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(root.get_str("first_used_turn_id").unwrap(), message.turn_id);
    assert!(
        root.get_binary_generic("text_encrypted")
            .unwrap()
            .windows(7)
            .all(|w| w != b"private")
    );
    assert_eq!(first.attachments.len(), 1);
    assert!(first.media_pending.is_none());
    let old = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    assert!(old.media.unwrap().items[0].outcome.is_none());
    settle(&state, &row, Some("cancelled")).await;
    let successor = Box::pin(engine::begin_turn(
        &state.db,
        &row.user_id,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            conversation_id: Some(row.id.clone()),
            attachment_ids: vec![],
            text: "successor".into(),
            agent_id: None,
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    assert_ne!(
        successor.active_turn.as_ref().unwrap().turn_id,
        message.turn_id
    );
    let replay = binding::bind(&state.db, &message, &proposed).await.unwrap();
    binding::reconcile(&state.db, &replay).await.unwrap();
    let second = saved(&state, &message).await;
    assert_eq!(first.text, second.text);
    assert_eq!(first.attachments.len(), second.attachments.len());
    let root2 = state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(root.get("bound_at"), root2.get("bound_at"));
    let projected = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        projected.media.unwrap().items[0]
            .outcome
            .as_ref()
            .unwrap()
            .upload_id
            .as_deref(),
        Some(id.as_str())
    );
    let model: AssistantUpload = bson::from_document(root2).unwrap();
    assert_eq!(format!("{model:?}"), "AssistantUpload([REDACTED])");
}

#[tokio::test]
async fn channel_media_stop_before_bind_cleans_known_roots_chunks_and_late_commit_expires() {
    let (state, row, message, event) = fixture().await;
    let id = upload(&state, &row, &event).await;
    state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    assert!(
        binding::bind(&state.db, &message, &proposed(&event, id.clone()))
            .await
            .is_err()
    );
    settle(&state, &row, Some("cancelled")).await;
    let receipt = saved(&state, &message).await;
    assert!(receipt.media_pending.is_none());
    assert_eq!(
        receipt.media_bound.as_ref().unwrap().failed[0].status,
        "not_processed: stopped"
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .count_documents(doc! {"$or":[{"_id":&id},{"parent_attachment_id":&id}]})
            .await
            .unwrap(),
        0
    );
    // An unknown upload commit landing after finalization is never adopted.
    let late = upload(&state, &row, &event).await;
    let replay = binding::bind(&state.db, &message, &proposed(&event, late.clone()))
        .await
        .unwrap();
    assert!(replay.ok.is_empty());
    let roots = state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
    let late_row = roots.find_one(doc! {"_id":&late}).await.unwrap().unwrap();
    assert!(
        late_row
            .get("message_id")
            .is_some_and(|v| v == &bson::Bson::Null)
    );
    roots.update_one(doc! {"_id":&late},doc! {"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(2))}}).await.unwrap();
    retention::sweep(&state.db).await.unwrap();
    assert_eq!(
        roots
            .count_documents(doc! {"$or":[{"_id":&late},{"parent_attachment_id":&late}]})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn channel_media_failure_context_counts_successful_and_standalone_items_before_embedded() {
    let (_, _, _, mut event) = fixture().await;
    let media = event.media.as_mut().unwrap();
    let mut second = media.items[0].clone();
    second.index = 1;
    let mut embedded = media.items[0].clone();
    embedded.embedded = true;
    media.items.extend([second, embedded]);
    media.declared = 4;
    let receipt = MediaBound {
        event_ids: vec![event.id.clone()],
        ok: vec![
            outcome(&event, Some(0), false, "bound", Some("first".into())),
            outcome(&event, Some(1), false, "bound", Some("second".into())),
        ],
        failed: vec![
            outcome(&event, Some(0), true, "unsupported_or_invalid", None),
            outcome(&event, None, true, "1 more embedded media omitted", None),
        ],
        context: String::new(),
    };
    let mut order = ItemOrder::new();
    remember_order(&mut order, &event);
    let text = context(&receipt, &order);
    assert!(text.contains("Attachment 3 of 4 could not be read (unsupported or invalid file)."));
    assert!(text.contains("1 more embedded media omitted"));
    assert!(!text.contains("unsupported_or_invalid"));
    assert!(!text.contains("Attachment 1"));
}

#[tokio::test]
async fn channel_media_timeout_receipt_excludes_unknown_upload_and_cleans_late_commit() {
    let (state, row, message, event) = fixture().await;
    let known = upload(&state, &row, &event).await;
    let mut receipt = proposed(&event, known.clone());
    let mut timed_out = receipt.ok.remove(0);
    timed_out.upload_id = None;
    timed_out.status = "timeout".into();
    receipt.failed.push(timed_out);
    receipt.context = "Attachment timed out".into();
    let bound = binding::bind(&state.db, &message, &receipt).await.unwrap();
    assert!(bound.ok.is_empty());
    assert!(saved(&state, &message).await.attachments.is_empty());
    settle(&state, &row, None).await;
    let roots = state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
    assert_eq!(
        roots
            .count_documents(doc! {"$or":[{"_id":&known},{"parent_attachment_id":&known}]})
            .await
            .unwrap(),
        0
    );
    // Model a store acknowledgement that arrives only after timeout settlement.
    let late = upload(&state, &row, &event).await;
    let replay = binding::bind(&state.db, &message, &proposed(&event, late.clone()))
        .await
        .unwrap();
    assert!(replay.ok.is_empty());
    assert_eq!(replay.failed[0].status, "timeout");
    let late_row = roots.find_one(doc! {"_id": &late}).await.unwrap().unwrap();
    assert_eq!(late_row.get("message_id"), Some(&bson::Bson::Null));
    roots.update_one(doc! {"_id": &late}, doc! {"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(2))}}).await.unwrap();
    retention::sweep(&state.db).await.unwrap();
    assert_eq!(
        roots
            .count_documents(doc! {"$or":[{"_id":&late},{"parent_attachment_id":&late}]})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn channel_media_late_bind_immediate_image_deletion_success_failure_stop() {
    for code in [None, Some("assistant_unavailable"), Some("cancelled")] {
        let (state, row, message, event) = fixture().await;
        let id = upload(&state, &row, &event).await;
        binding::bind(&state.db, &message, &proposed(&event, id.clone()))
            .await
            .unwrap();
        // Use the same settings collection/id as the production retention service.
        let settings = bson::to_document(&crate::models::assistant_upload_retention::Policy {
            images_delete_after_turn: true,
            ..Default::default()
        })
        .unwrap();
        state
            .db
            .collection::<bson::Document>(crate::models::platform_settings::COLLECTION_NAME)
            .update_one(
                doc! {"_id":crate::models::assistant_upload_retention::SETTINGS_ID},
                doc! {"$set":{"policy":settings}},
            )
            .await
            .unwrap();
        settle(&state, &row, code).await;
        assert!(
            state
                .db
                .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .is_none(),
            "{code:?}"
        );
        let before = saved(&state, &message).await;
        let receipt = binding::bind(&state.db, &message, &proposed(&event, id))
            .await
            .unwrap();
        binding::reconcile(&state.db, &receipt).await.unwrap();
        let after = saved(&state, &message).await;
        assert_eq!(before.text, after.text);
        assert_eq!(before.attachments.len(), after.attachments.len());
    }
}

#[tokio::test]
async fn channel_media_unique_source_refuses_double_ingestion_without_duplicate_roots_or_chunks() {
    let (state, row, _, event) = fixture().await;
    let first = upload(&state, &row, &event).await;
    let duplicate = uploads::upload_channel_authorized(
        &state.db,
        &state.encryption_keys,
        &row.user_id,
        &row.id,
        "duplicate.png",
        png(),
        UploadSource {
            turn_id: row.active_turn.as_ref().unwrap().turn_id.clone(),
            event_key: event.media.as_ref().unwrap().key.clone(),
            index: 0,
            embedded: false,
            declared: 1,
        },
    )
    .await;
    assert!(duplicate.is_err());
    let rows = state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
    assert_eq!(
        rows.count_documents(doc! {"source.event_key":&event.media.unwrap().key})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        rows.count_documents(
            doc! {"user_id":&row.user_id,"parent_attachment_id":{"$ne":bson::Bson::Null}}
        )
        .await
        .unwrap(),
        1
    );
    assert!(rows.find_one(doc! {"_id":first}).await.unwrap().is_some());
}

#[tokio::test]
async fn channel_media_cancel_during_upload_and_bind_transactions_finalizes_without_orphans() {
    for phase in ["upload_before_commit", "bind_before_commit"] {
        let (state, row, message, event) = fixture().await;
        let gate = test_hooks::install(&state.db, &message.turn_id, phase, false);
        let state2 = state.clone();
        let row2 = row.clone();
        let event2 = event.clone();
        let message2 = message.clone();
        let work = tokio::spawn(async move {
            let id = upload(&state2, &row2, &event2).await;
            if phase == "bind_before_commit" {
                binding::bind(&state2.db, &message2, &proposed(&event2, id))
                    .await
                    .unwrap();
            }
        });
        tokio::time::timeout(Duration::from_secs(20), gate.entered.notified())
            .await
            .unwrap();
        work.abort();
        assert!(work.await.unwrap_err().is_cancelled());
        settle(&state, &row, Some("cancelled")).await;
        assert!(saved(&state, &message).await.media_pending.is_none());
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                .count_documents(doc! {"user_id":&row.user_id})
                .await
                .unwrap(),
            0,
            "{phase}"
        );
    }
}

#[tokio::test]
async fn channel_media_unknown_upload_ack_is_failed_and_known_committed_root_is_removed() {
    let (state, row, message, event) = fixture().await;
    let gate = test_hooks::install(&state.db, &message.turn_id, "upload_after_commit", true);
    let state2 = state.clone();
    let row2 = row.clone();
    let event2 = event.clone();
    let work = tokio::spawn(async move {
        uploads::upload_channel_authorized(
            &state2.db,
            &state2.encryption_keys,
            &row2.user_id,
            &row2.id,
            "image.png",
            png(),
            UploadSource {
                turn_id: row2.active_turn.unwrap().turn_id,
                event_key: event2.media.unwrap().key,
                index: 0,
                embedded: false,
                declared: 1,
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(20), gate.entered.notified())
        .await
        .unwrap();
    gate.release.notify_one();
    assert!(work.await.unwrap().is_err());
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .count_documents(doc! {"source.turn_id":&message.turn_id})
            .await
            .unwrap(),
        1
    );
    settle(&state, &row, Some("assistant_unavailable")).await;
    let final_message = saved(&state, &message).await;
    assert!(final_message.attachments.is_empty());
    assert!(final_message.media_bound.unwrap().ok.is_empty());
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&row.user_id})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn channel_media_bind_commit_writes_parent_fence_against_concurrent_stop() {
    let (state, row, message, event) = fixture().await;
    let id = upload(&state, &row, &event).await;
    let gate = test_hooks::install(&state.db, &message.turn_id, "bind_before_commit", false);
    let db = state.db.clone();
    let m = message.clone();
    let p = proposed(&event, id.clone());
    let bind = tokio::spawn(async move { binding::bind(&db, &m, &p).await });
    tokio::time::timeout(Duration::from_secs(20), gate.entered.notified())
        .await
        .unwrap();
    let db = state.db.clone();
    let conversation = row.id.clone();
    let stop = tokio::spawn(async move {
        db.collection::<bson::Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id":conversation},
                doc! {"$set":{"active_turn.stop_requested":true}},
            )
            .await
    });
    tokio::task::yield_now().await;
    gate.release.notify_one();
    bind.await.unwrap().unwrap();
    stop.await.unwrap().unwrap();
    settle(&state, &row, Some("cancelled")).await;
    let final_message = saved(&state, &message).await;
    assert_eq!(final_message.attachments.len(), 1);
    assert_eq!(final_message.media_bound.unwrap().ok[0].upload_id, Some(id));
}

#[tokio::test]
async fn channel_media_orphan_finalization_progresses_across_multiple_sweep_batches_with_flags_off()
{
    let (state, row, message, event) = fixture().await;
    state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&row.id}, doc! {"$unset":{"active_turn":""}})
        .await
        .unwrap();
    let mut rows = Vec::new();
    for index in 0..55 {
        let mut m = message.clone();
        m.id = Uuid::new_v4().to_string();
        m.turn_id = format!("orphan-{index}");
        m.seq = 100 + index;
        rows.push(m);
    }
    state
        .db
        .collection(MESSAGES)
        .insert_many(rows)
        .await
        .unwrap();
    assert_eq!(queue::sweep(&state.db).await.unwrap(), 50);
    assert_eq!(queue::sweep(&state.db).await.unwrap(), 6);
    assert_eq!(queue::sweep(&state.db).await.unwrap(), 0);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(MESSAGES)
            .count_documents(doc! {"media_pending":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        0
    );
    assert!(
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":&event.id})
            .await
            .unwrap()
            .unwrap()
            .media
            .unwrap()
            .items[0]
            .outcome
            .is_some()
    );
}

#[tokio::test]
async fn channel_media_six_day_conversion_survives_seven_day_source_deletion_and_is_visible_once() {
    let (state, row, message, event) = fixture().await;
    let queued:AgentEvent=bson::from_document(doc! {"id":Uuid::new_v4().to_string(),"kind":"message","text":"transfer everything",
        "created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(7)),"media_event_ids":[&event.id],"source_event_ids":[&event.id],
        "media_declared":1i64,"media_sent_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(7))}).unwrap();
    state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"pending_events":[bson::to_bson(&queued).unwrap()]}},
        )
        .await
        .unwrap();
    assert_eq!(
        queue::convert_queued(&state.db, Utc::now() - ChronoDuration::days(6))
            .await
            .unwrap(),
        1
    );
    let converted = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    for item in converted.media.unwrap().items {
        assert_eq!(item.outcome.unwrap().status, "not_processed: expired");
    }
    state
        .db
        .collection::<bson::Document>(EVENTS)
        .delete_one(doc! {"_id":&event.id})
        .await
        .unwrap();
    assert_eq!(
        queue::convert_queued(&state.db, Utc::now()).await.unwrap(),
        0
    );
    let notices: Vec<AssistantMessage> = state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .find(doc! {"media_queue_notice_id":&queued.id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0]
            .text
            .contains("expired before it could be processed; please resend")
    );
    assert!(
        notices[0]
            .text
            .contains("Quoted history (not a new request):\n> transfer everything")
    );
    let fresh = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(fresh.pending_events.is_empty());
    assert_ne!(notices[0].id, message.id);
    // Read the owner's transcript only after deleting the seven-day source.
    let visible = engine::messages(&state.db, &row.user_id, &row.id, 100, None)
        .await
        .unwrap();
    let visible_notices: Vec<_> = visible
        .iter()
        .filter(|m| m.media_queue_notice_id.as_deref() == Some(queued.id.as_str()))
        .collect();
    assert_eq!(visible_notices.len(), 1);
    assert_eq!(visible_notices[0].id, notices[0].id);
    assert_eq!(visible_notices[0].role, "assistant");
    assert!(visible_notices[0].text.contains("please resend"));
    assert!(visible_notices[0].text.contains("> transfer everything"));
}

#[tokio::test]
async fn channel_media_browser_single_read_accepts_above_initial_channel_limit_and_encrypts() {
    use axum::{
        body::Body,
        extract::{Path, State},
        http::Request,
    };
    let (state, row, _, _) = fixture().await;
    let mut payload = png();
    payload.resize(5 * 1024 * 1024, 0);
    let request = Request::builder()
        .header("x-attachment-name", "private-label.png")
        .body(Body::from(payload.clone()))
        .unwrap();
    let response = crate::handlers::assistant_uploads::upload(
        State(state.clone()),
        crate::test_utils::test_auth_user(&row.user_id),
        Path(row.id.clone()),
        request,
    )
    .await
    .unwrap();
    assert_eq!(response.0, axum::http::StatusCode::CREATED);
    let roots = state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
    let root = roots
        .find_one(doc! {"user_id":&row.user_id,"origin":"user_upload"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(root.get_i64("size").unwrap(), payload.len() as i64);
    use sha2::{Digest, Sha256};
    assert_eq!(
        root.get_str("content_sha256").unwrap(),
        format!("{:x}", Sha256::digest(&payload))
    );
    assert!(
        root.get("source")
            .is_none_or(|source| matches!(source, bson::Bson::Null))
    );
    let id = root.get_str("_id").unwrap();
    let chunks: Vec<bson::Document> = roots
        .find(doc! {"parent_attachment_id":id})
        .sort(doc! {"chunk_index":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(chunks.len(), 2);
    let mut clear = Vec::new();
    for chunk in chunks {
        let encrypted = chunk.get_binary_generic("data_encrypted").unwrap();
        assert!(!encrypted.starts_with(&payload[..8]));
        clear.extend_from_slice(
            &crate::services::assistant_record_service::decrypt_chunk(
                &state.encryption_keys,
                chunk.get_str("_id").unwrap(),
                root.get_str("user_id").unwrap(),
                id,
                chunk.get_i64("chunk_index").unwrap(),
                crate::services::record_field_service::stored_versions(&chunk)
                    .unwrap()
                    .get("data_encrypted")
                    .copied(),
                encrypted,
            )
            .await
            .unwrap(),
        );
    }
    assert_eq!(clear, payload);
}

#[tokio::test]
async fn channel_media_browser_waits_for_shared_budget_before_body_read_and_cancels() {
    use axum::{
        body::{Body, Bytes},
        extract::{Path, State},
        http::Request,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (state, row, _, _) = fixture().await;
    let reservation = uploads::byte_budget::reserve(uploads::byte_budget::CAPACITY / 2)
        .await
        .unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let counted = reads.clone();
    let body = Body::from_stream(futures::stream::once(async move {
        counted.fetch_add(1, Ordering::SeqCst);
        Ok::<_, std::io::Error>(Bytes::from(png()))
    }));
    let request = Request::builder()
        .header("x-attachment-name", "image.png")
        .body(body)
        .unwrap();
    let run_state = state.clone();
    let user = row.user_id.clone();
    let scope = row.id.clone();
    let task = tokio::spawn(async move {
        crate::handlers::assistant_uploads::upload(
            State(run_state),
            crate::test_utils::test_auth_user(&user),
            Path(scope),
            request,
        )
        .await
    });
    for _ in 0..100 {
        if state
            .db
            .collection::<bson::Document>("assistant_upload_limits")
            .count_documents(doc! {})
            .await
            .unwrap()
            > 0
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert!(!task.is_finished());
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    drop(reservation);
    let all = tokio::time::timeout(
        Duration::from_secs(1),
        uploads::byte_budget::reserve(uploads::byte_budget::CAPACITY / 2),
    )
    .await
    .unwrap()
    .unwrap();
    drop(all);
}

#[tokio::test]
async fn channel_media_guest_and_trigger_admission_leave_queued_owner_media_untouched() {
    for guest in [true, false] {
        let (state, row, _, event) = fixture().await;
        settle(&state, &row, None).await;
        let queued:AgentEvent=bson::from_document(doc! {"id":"queued-media","kind":"message","text":"owner work","created_at":bson::DateTime::now(),
            "media_event_ids":[&event.id],"media_declared":1i64,"media_sent_at":bson::DateTime::now()}).unwrap();
        state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"pending_events":[bson::to_bson(&queued).unwrap()]}},
            )
            .await
            .unwrap();
        let request = engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            conversation_id: Some(row.id.clone()),
            attachment_ids: vec![],
            text: "new request".into(),
            agent_id: None,
            model: None,
            access_mode: None,
        };
        let mut start: engine::TurnStart = (&request).into();
        start.guest = guest;
        if !guest {
            start.origin = crate::models::assistant_conversation::TurnOrigin::Trigger;
        }
        let admitted = Box::pin(engine::begin_turn(
            &state.db,
            &row.user_id,
            start,
            &state.encryption_keys,
        ))
        .await
        .unwrap();
        assert!(admitted.active_turn.as_ref().unwrap().events.is_empty());
        assert_eq!(admitted.pending_events[0].media_event_ids, vec![event.id]);
        settle(&state, &admitted, None).await;
    }
}

#[tokio::test]
async fn channel_media_missing_seven_day_source_is_not_executed_and_notice_is_visible() {
    let (state, row, _, event) = fixture().await;
    state
        .db
        .collection::<bson::Document>(EVENTS)
        .delete_one(doc! {"_id":&event.id})
        .await
        .unwrap();
    let queued:AgentEvent=bson::from_document(doc! {"id":"seven-day-source","kind":"message","text":"old request","created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(7)),
        "source_event_ids":[&event.id],"media_event_ids":[&event.id],"media_declared":1i64,"media_sent_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(7))}).unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        crate::services::channel_thread_follow_service::filter_events(
            &state.db,
            &row.user_id,
            &row.id,
            vec![queued.clone()],
            &mut session
        )
        .await
        .unwrap()
        .is_empty()
    );
    session.commit_transaction().await.unwrap();
    let visible = engine::messages(&state.db, &row.user_id, &row.id, 100, None)
        .await
        .unwrap();
    let notice = visible
        .iter()
        .find(|m| m.media_queue_notice_id.as_deref() == Some("seven-day-source"))
        .unwrap();
    assert!(
        notice
            .text
            .contains("expired before it could be processed; please resend")
    );
    assert!(notice.text.contains("> old request"));
    assert_eq!(notice.role, "assistant");
}

#[tokio::test]
async fn channel_media_early_admin_conversion_requires_first_party_human_admin() {
    use axum::extract::State;
    let (state, row, _, event) = fixture().await;
    let queued: AgentEvent = bson::from_document(doc! {
        "id":Uuid::new_v4().to_string(),"kind":"message","text":"queued photo",
        "created_at":bson::DateTime::now(),"media_event_ids":[&event.id],"source_event_ids":[&event.id],
        "media_declared":1i64,"media_sent_at":bson::DateTime::now(),
    }).unwrap();
    state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"pending_events":[bson::to_bson(&queued).unwrap()]}},
        )
        .await
        .unwrap();
    let auth = crate::test_utils::test_auth_user(&row.user_id);
    assert!(
        crate::handlers::admin_assistant::drain_channel_media(State(state.clone()), auth.clone())
            .await
            .is_err()
    );
    let unchanged = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        unchanged
            .media
            .unwrap()
            .items
            .iter()
            .all(|i| i.outcome.is_none())
    );
    crate::services::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&state.db)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.user_id},
            doc! {"$set":{"role_ids":[roles.admin]}},
        )
        .await
        .unwrap();
    let result =
        crate::handlers::admin_assistant::drain_channel_media(State(state.clone()), auth.clone())
            .await;
    assert!(result.is_ok());
    let converted = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    for item in converted.media.unwrap().items {
        assert_eq!(item.outcome.unwrap().status, "not_processed: expired");
    }
    let drained = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(drained.pending_events.is_empty());
    for method in [
        crate::mw::auth::AuthMethod::ApiKey,
        crate::mw::auth::AuthMethod::Delegated,
    ] {
        let mut other = auth.clone();
        other.auth_method = method;
        assert!(
            crate::handlers::admin_assistant::drain_channel_media(State(state.clone()), other)
                .await
                .is_err()
        );
    }
    let mut third_party = auth;
    third_party.oauth_client_id = Some("third-party".into());
    assert!(
        crate::handlers::admin_assistant::drain_channel_media(State(state.clone()), third_party)
            .await
            .is_err()
    );
}

#[derive(Clone, Default)]
struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for LogCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[tokio::test]
async fn channel_media_ingestion_and_settlement_emit_no_bytes_keys_or_labels_to_logs_or_audit() {
    let logs = LogCapture::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_env_filter("nyxid_server=trace,nyxid=trace")
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (state, row, message, event) = fixture().await;
    let id = upload(&state, &row, &event).await;
    binding::bind(&state.db, &message, &proposed(&event, id))
        .await
        .unwrap();
    settle(&state, &row, None).await;
    let audit: Vec<bson::Document> = state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let log = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for sensitive in ["private-label.png", &event.media.unwrap().key] {
        assert!(!log.contains(sensitive));
        assert!(!format!("{audit:?}").contains(sensitive));
    }
}

// Pre-media model field sets exercise old readers without importing the new fields.
mod pre_media_shapes {
    pub mod agent_event {
        use crate::models::assistant_conversation::*;
        use chrono::{DateTime, Utc};
        use serde::{Deserialize, Serialize};
        #[derive(Serialize, Deserialize)]
        pub struct Old {
            pub id: String,
            pub kind: String,
            pub text: String,
            #[serde(default)]
            pub agent_id: Option<String>,
            #[serde(default)]
            pub question_key: Option<String>,
            #[serde(default)]
            pub reply_to: Vec<ChannelOrigin>,
            #[serde(default)]
            pub source_event_ids: Vec<String>,
            #[serde(default)]
            pub bound: bool,
            #[serde(default)]
            pub report_to: Option<String>,
            #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
            pub created_at: DateTime<Utc>,
        }
    }
    pub mod nyxbot_event {
        use crate::models::nyxbot_channel::*;
        use chrono::{DateTime, Utc};
        use serde::{Deserialize, Serialize};
        #[derive(Serialize, Deserialize)]
        pub struct Old {
            #[serde(default)]
            pub native_bot_budget_claimed: bool,
            #[serde(default)]
            pub retry_request_sha256: Option<String>,
            #[serde(default)]
            pub delivery: Option<ChannelTurnDelivery>,
            #[serde(default)]
            pub resolved_thread_id: Option<String>,
            #[serde(default)]
            pub resolved_conversation_id: Option<String>,
            #[serde(rename = "_id")]
            pub id: String,
            pub channel_id: String,
            pub user_id: String,
            pub partition: String,
            pub event_id: String,
            #[serde(default)]
            pub relay_message_id: Option<String>,
            #[serde(with = "crate::models::bson_bytes::optional", default)]
            pub event_context_ciphertext: Option<Vec<u8>>,
            pub status: String,
            #[serde(default)]
            pub conversation_id: Option<String>,
            #[serde(default)]
            pub turn_id: Option<String>,
            #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
            pub created_at: DateTime<Utc>,
            #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
            pub expires_at: DateTime<Utc>,
        }
    }
    pub mod channel_message {
        use crate::models::channel_message::StoredAttachment;
        use chrono::{DateTime, Utc};
        use serde::{Deserialize, Serialize};
        #[derive(Serialize, Deserialize)]
        pub struct Old {
            #[serde(default)]
            pub thread_context: Option<crate::models::channel_thread::ChannelThreadFacts>,
            #[serde(default)]
            pub activity: Option<crate::models::channel_activity::ActivityMetadata>,
            #[serde(default)]
            pub platform_send: Option<crate::models::channel_delivery::PlatformSendRecord>,
            #[serde(rename = "_id")]
            pub id: String,
            #[serde(default)]
            pub channel_bot_id: Option<String>,
            pub conversation_id: String,
            #[serde(default)]
            pub platform_conversation_id: Option<String>,
            pub user_id: String,
            pub direction: String,
            pub platform: String,
            #[serde(default)]
            pub platform_message_id: Option<String>,
            #[serde(default)]
            pub native_event_id: Option<String>,
            #[serde(default)]
            pub sender_platform_id: Option<String>,
            #[serde(default)]
            pub sender_display_name: Option<String>,
            pub content_type: String,
            #[serde(default)]
            pub attachments: Vec<StoredAttachment>,
            #[serde(default)]
            pub thread_id: Option<String>,
            #[serde(default)]
            pub agent_api_key_id: Option<String>,
            #[serde(default)]
            pub callback_status: Option<String>,
            #[serde(default)]
            pub callback_http_status: Option<u16>,
            #[serde(default)]
            pub reply_to_message_id: Option<String>,
            #[serde(default)]
            pub platform_reply_message_id: Option<String>,
            #[serde(default)]
            pub reply_to_platform_message_id: Option<String>,
            #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
            pub created_at: DateTime<Utc>,
            #[serde(default, with = "crate::models::bson_datetime::optional")]
            pub updated_at: Option<DateTime<Utc>>,
        }
    }
    pub mod upload {
        use crate::models::assistant_upload::*;
        use chrono::{DateTime, Utc};
        use serde::{Deserialize, Serialize};
        #[derive(Serialize, Deserialize)]
        pub struct Old {
            #[serde(rename = "_id")]
            pub id: String,
            pub origin: String,
            pub user_id: String,
            pub conversation_id: String,
            pub group_id: Option<String>,
            pub message_id: Option<String>,
            pub label: String,
            pub content_type: String,
            pub size: i64,
            pub pages: usize,
            #[serde(default)]
            pub extracted_chars: Option<i64>,
            pub chunks: usize,
            #[serde(with = "crate::models::bson_bytes::required")]
            pub text_encrypted: Vec<u8>,
            #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
            pub created_at: DateTime<Utc>,
            #[serde(default, with = "crate::models::bson_datetime::optional")]
            pub expires_at: Option<DateTime<Utc>>,
            #[serde(default, with = "crate::models::bson_datetime::optional")]
            pub bound_at: Option<DateTime<Utc>>,
            #[serde(default)]
            pub reads: i64,
            #[serde(default)]
            pub coverage: Vec<ReadInterval>,
        }
    }
    pub mod message {
        use crate::models::assistant_message::*;
        use chrono::{DateTime, Utc};
        use serde::{Deserialize, Serialize};
        #[derive(Serialize, Deserialize)]
        pub struct Old {
            #[serde(default)]
            pub platform_delivery_recovery_queued: bool,
            #[serde(default)]
            pub platform_reply: Option<crate::models::assistant_conversation::PlatformReplyRouting>,
            #[serde(default)]
            pub platform_origin: Option<crate::models::assistant_conversation::PlatformTurnOrigin>,
            #[serde(default)]
            pub author: Option<MessageAuthor>,
            #[serde(default)]
            pub steering: Option<Box<Steering>>,
            #[serde(default)]
            pub voice: Option<VoiceTranscript>,
            #[serde(default)]
            pub execution_pending: bool,
            #[serde(rename = "_id")]
            pub id: String,
            pub conversation_id: String,
            pub user_id: String,
            pub seq: i64,
            pub turn_id: String,
            pub role: String,
            pub text: String,
            pub status: String,
            pub error_code: Option<String>,
            #[serde(default)]
            pub upstream_error_code: Option<String>,
            #[serde(default)]
            pub error_detail: Option<AssistantErrorDetail>,
            #[serde(default)]
            pub turn_metadata: Option<TurnMetadata>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub specialist_id: Option<String>,
            #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
            pub created_at: DateTime<Utc>,
            #[serde(default)]
            pub activities: Vec<crate::models::assistant_conversation::TurnActivity>,
            #[serde(default)]
            pub activity_total: i64,
            #[serde(default)]
            pub attachments: Vec<crate::models::assistant_conversation::TurnAttachment>,
            #[serde(default)]
            pub origin: Option<crate::models::assistant_conversation::TurnOrigin>,
            #[serde(default)]
            pub via: Option<String>,
            #[serde(default)]
            pub retry_of: Option<String>,
            #[serde(default)]
            pub retry_claimed_by: Option<String>,
            #[serde(default)]
            pub activity_count: Option<i64>,
            #[serde(default)]
            pub continuation_of: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub report_to: Option<String>,
        }
    }
}

fn pre_media_projection<T: serde::de::DeserializeOwned + serde::Serialize>(
    document: &bson::Document,
) -> bson::Document {
    let old: T = bson::from_document(document.clone()).unwrap();
    bson::to_document(&old).unwrap()
}

#[tokio::test]
async fn channel_media_mixed_version_models_preserve_old_fields_through_services_and_new_writes() {
    use crate::models::channel_message::{CallbackDisposition, ChannelMessage, StoredAttachment};
    use crate::services::channel_relay_service as relay;
    use pre_media_shapes::{
        agent_event, channel_message, message as old_message, nyxbot_event, upload as old_upload,
    };

    let (state, row, message, event) = fixture().await;
    let now = bson::DateTime::now();

    // A pre-media queue item survives the real append service alongside new media work.
    let old_agent = doc! {
        "id":Uuid::new_v4().to_string(), "kind":"subagent_settled", "text":"legacy queued result",
        "agent_id":"specialist", "question_key":"old-question", "bound":true,
        "report_to":"manager", "source_event_ids":[], "reply_to":[], "created_at":now,
    };
    let old_agent_expected = pre_media_projection::<agent_event::Old>(&old_agent);
    state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"pending_events":[&old_agent]}},
        )
        .await
        .unwrap();
    let mut queued_source = event.clone();
    queued_source.id = Uuid::new_v4().to_string();
    queued_source.event_id = queued_source.id.clone();
    queued_source.status = "queued".into();
    queued_source.turn_id = None;
    queued_source.media.as_mut().unwrap().key = Uuid::new_v4().to_string();
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&queued_source)
        .await
        .unwrap();
    let queued: AgentEvent = bson::from_document(doc! {
        "id":Uuid::new_v4().to_string(), "kind":"message", "text":"new queued photo", "created_at":now,
        "source_event_ids":[&queued_source.id], "media_event_ids":[&queued_source.id], "media_declared":1i64, "media_sent_at":now,
    }).unwrap();
    let appended = engine::push_events(&state.db, &row.user_id, &row.id, vec![queued.clone()])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(appended.pending_events.len(), 2);
    assert_eq!(
        pre_media_projection::<agent_event::Old>(
            &bson::to_document(&appended.pending_events[0]).unwrap()
        ),
        old_agent_expected
    );
    let queued_new = bson::to_document(&appended.pending_events[1]).unwrap();
    assert_eq!(
        pre_media_projection::<agent_event::Old>(&queued_new)
            .get_str("text")
            .unwrap(),
        "new queued photo"
    );
    assert_eq!(
        appended.pending_events[1].media_event_ids,
        queued.media_event_ids
    );

    // A pre-media source event can be loaded, upgraded, and reconciled without losing routing or ciphertext.
    let mut original_event = bson::to_document(&event).unwrap();
    original_event.insert("relay_message_id", "relay-source");
    original_event.insert(
        "event_context_ciphertext",
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: vec![17, 23, 42],
        },
    );
    original_event.insert("resolved_thread_id", "legacy-thread");
    original_event.insert("retry_request_sha256", "legacy-request-digest");
    let old_event = pre_media_projection::<nyxbot_event::Old>(&original_event);
    state
        .db
        .collection::<bson::Document>(EVENTS)
        .replace_one(doc! {"_id":&event.id}, &old_event)
        .await
        .unwrap();
    let mut loaded = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    assert!(loaded.media.is_none());
    assert!(loaded.source_media_declared.is_none());
    loaded.media = event.media.clone();
    loaded.source_media_declared = Some(1);
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .replace_one(doc! {"_id":&event.id}, &loaded)
        .await
        .unwrap();
    assert_eq!(
        pre_media_projection::<nyxbot_event::Old>(&bson::to_document(&loaded).unwrap()),
        old_event
    );

    // Callback updates on a pre-media relay row preserve every old field except the requested status change.
    let old_channel = doc! {
        "_id":Uuid::new_v4().to_string(), "channel_bot_id":"bot", "conversation_id":"route",
        "platform_conversation_id":"chat", "user_id":&row.user_id, "direction":"inbound", "platform":"lark",
        "platform_message_id":"native", "native_event_id":"native-event", "sender_platform_id":"sender",
        "sender_display_name":"sender name", "content_type":"post", "thread_id":"thread", "agent_api_key_id":"key",
        "attachments":[{"content_type":"image","provider_ref":"legacy-handle","image_key":"legacy-image","size_bytes":42i64}],
        "callback_status":"pending", "callback_http_status":202, "reply_to_platform_message_id":"parent", "created_at":now,
    };
    let channel_id = old_channel.get_str("_id").unwrap();
    let mut channel_expected = pre_media_projection::<channel_message::Old>(&old_channel);
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(&old_channel)
        .await
        .unwrap();
    let mut channel = relay::get_message(&state.db, channel_id).await.unwrap();
    assert!(channel.embedded_attachments.is_empty());
    assert_eq!(channel.embedded_attachments_omitted, 0);
    assert!(channel.duplicate_of_media_key.is_none());
    assert!(channel.callback_disposition.is_none());
    channel.embedded_attachments.push(StoredAttachment {
        content_type: "image".into(),
        provider_ref: "embedded-handle".into(),
        platform_message_id: None,
        file_key: None,
        image_key: Some("embedded-image".into()),
        filename: None,
        mime_type: None,
        size_bytes: Some(4),
    });
    channel.embedded_attachments_omitted = 2;
    channel.duplicate_of_media_key = Some("a".repeat(64));
    state
        .db
        .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
        .replace_one(doc! {"_id":channel_id}, &channel)
        .await
        .unwrap();
    relay::record_callback_disposition(&state.db, channel_id, &CallbackDisposition::Accepted)
        .await
        .unwrap();
    relay::update_callback_status(&state.db, channel_id, "delivered")
        .await
        .unwrap();
    channel_expected.insert("callback_status", "delivered");
    let channel = relay::get_message(&state.db, channel_id).await.unwrap();
    assert_eq!(
        pre_media_projection::<channel_message::Old>(&bson::to_document(&channel).unwrap()),
        channel_expected
    );
    assert_eq!(channel.embedded_attachments.len(), 1);
    assert_eq!(channel.embedded_attachments_omitted, 2);
    assert_eq!(
        channel.duplicate_of_media_key.as_deref(),
        Some("a".repeat(64).as_str())
    );
    assert_eq!(
        channel.callback_disposition,
        Some(CallbackDisposition::Accepted)
    );

    // Bind an actual browser upload after stripping the two media additions, as an old writer would.
    let attachment = uploads::upload_authorized(
        &state.db,
        &state.encryption_keys,
        &row.user_id,
        &row.id,
        "legacy.png",
        png(),
        None,
    )
    .await
    .unwrap();
    let upload_rows = state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
    let original_upload = upload_rows
        .find_one(doc! {"_id":&attachment.id})
        .await
        .unwrap()
        .unwrap();
    let mut upload_expected = pre_media_projection::<old_upload::Old>(&original_upload);
    upload_rows
        .replace_one(doc! {"_id":&attachment.id}, &upload_expected)
        .await
        .unwrap();
    let old_root: AssistantUpload = bson::from_document(upload_expected.clone()).unwrap();
    assert!(old_root.source.is_none());
    assert!(old_root.content_sha256.is_none());
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let bound = uploads::bind(
        &state.db,
        &row.user_id,
        &row.id,
        &message.id,
        std::slice::from_ref(&attachment.id),
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(bound.len(), 1);
    session.commit_transaction().await.unwrap();
    let bound_root = upload_rows
        .find_one(doc! {"_id":&attachment.id})
        .await
        .unwrap()
        .unwrap();
    upload_expected.insert("message_id", &message.id);
    upload_expected.insert("bound_at", bound_root.get("bound_at").unwrap().clone());
    upload_expected.insert("expires_at", bson::Bson::Null);
    assert_eq!(
        pre_media_projection::<old_upload::Old>(&bound_root),
        upload_expected
    );

    // A pre-media user message is a no-op for finalization, then accepts a new bind receipt.
    let mut message_expected =
        pre_media_projection::<old_message::Old>(&bson::to_document(&message).unwrap());
    message_expected.insert("via", "lark");
    message_expected.insert("retry_of", "legacy-retry");
    message_expected.insert("report_to", "manager");
    state
        .db
        .collection::<bson::Document>(MESSAGES)
        .replace_one(doc! {"_id":&message.id}, &message_expected)
        .await
        .unwrap();
    let mut upgraded = saved(&state, &message).await;
    assert!(upgraded.media_event_ids.is_empty());
    assert!(upgraded.media_pending.is_none());
    assert!(upgraded.media_bound.is_none());
    assert!(upgraded.media_queue_notice_id.is_none());
    media_finalize(&state.db, &row.user_id, &row.id, &message.turn_id)
        .await
        .unwrap();
    assert_eq!(
        pre_media_projection::<old_message::Old>(
            &bson::to_document(&saved(&state, &message).await).unwrap()
        ),
        message_expected
    );
    upgraded.media_event_ids = vec![event.id.clone()];
    upgraded.media_pending = Some(1);
    state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .replace_one(doc! {"_id":&message.id}, &upgraded)
        .await
        .unwrap();
    let channel_upload = upload(&state, &row, &event).await;
    let new_root = upload_rows
        .find_one(doc! {"_id":&channel_upload})
        .await
        .unwrap()
        .unwrap();
    let old_reader: old_upload::Old = bson::from_document(new_root.clone()).unwrap();
    assert_eq!(old_reader.id, channel_upload);
    assert_eq!(
        old_reader.text_encrypted,
        bson::from_document::<AssistantUpload>(new_root)
            .unwrap()
            .text_encrypted
    );
    let mut receipt = proposed(&event, channel_upload);
    receipt.context.clear();
    binding::bind(&state.db, &upgraded, &receipt).await.unwrap();
    let committed = saved(&state, &message).await;
    message_expected.insert(
        "attachments",
        bson::to_bson(&committed.attachments).unwrap(),
    );
    assert_eq!(
        pre_media_projection::<old_message::Old>(&bson::to_document(&committed).unwrap()),
        message_expected
    );
    assert!(ingest_and_bind(&state, &row).await.unwrap().is_empty());
    let reconciled = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        reconciled.media.as_ref().unwrap().items[0]
            .outcome
            .is_some()
    );
    assert_eq!(
        pre_media_projection::<nyxbot_event::Old>(&bson::to_document(&reconciled).unwrap()),
        old_event
    );
    // Rollback conversion leaves ordinary old queue items intact; the next turn drains them normally.
    assert_eq!(
        queue::convert_queued(&state.db, Utc::now()).await.unwrap(),
        1
    );
    settle(&state, &row, None).await;
    let successor = Box::pin(engine::begin_turn(
        &state.db,
        &row.user_id,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            conversation_id: Some(row.id.clone()),
            attachment_ids: vec![],
            text: "next owner turn".into(),
            agent_id: None,
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    assert!(successor.pending_events.is_empty());
    let drained = &successor.active_turn.as_ref().unwrap().events;
    assert_eq!(drained.len(), 1);
    assert_eq!(
        pre_media_projection::<agent_event::Old>(&bson::to_document(&drained[0]).unwrap()),
        old_agent_expected
    );
}

#[tokio::test]
async fn channel_media_rollback_rehearsal_flags_off_sweeps_then_admin_drain_reach_zero_gates() {
    use crate::services::{channel_media_events, feature_flag_service as flags};
    use axum::extract::State;

    let (state, row, crashed_message, event) = fixture().await;
    for key in [
        flags::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
        flags::ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY,
    ] {
        assert!(
            !flags::personal_flag_enabled(&state.db, &row.user_id, key)
                .await
                .unwrap()
        );
    }
    let old_time = Utc::now() - ChronoDuration::days(6) - ChronoDuration::hours(1);
    let fresh_time = Utc::now() - ChronoDuration::minutes(1);
    let mut sources = Vec::new();
    let mut queued = Vec::new();
    for (label, time) in [
        ("old photo request", old_time),
        ("fresh photo request", fresh_time),
    ] {
        let mut source = event.clone();
        source.id = Uuid::new_v4().to_string();
        source.event_id = source.id.clone();
        source.status = "queued".into();
        source.turn_id = None;
        source.created_at = time;
        source.expires_at = time + ChronoDuration::days(7);
        source.media.as_mut().unwrap().key = Uuid::new_v4().to_string();
        queued.push(bson::from_document::<AgentEvent>(doc! {
            "id":Uuid::new_v4().to_string(),"kind":"message","text":label,
            "source_event_ids":[&source.id],"media_event_ids":[&source.id],"media_declared":1i64,
            "created_at":bson::DateTime::from_chrono(time),"media_sent_at":bson::DateTime::from_chrono(time),
        }).unwrap());
        sources.push(source);
    }
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_many(&sources)
        .await
        .unwrap();
    // Also exercise the real event sweeper's admission-crash classification.
    let mut interrupted = event.clone();
    interrupted.id = Uuid::new_v4().to_string();
    interrupted.event_id = interrupted.id.clone();
    interrupted.conversation_id = None;
    interrupted.turn_id = None;
    interrupted.created_at = Utc::now() - ChronoDuration::minutes(10);
    interrupted.media.as_mut().unwrap().key = Uuid::new_v4().to_string();
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&interrupted)
        .await
        .unwrap();
    state.db.collection::<bson::Document>(CONVERSATIONS).update_one(
        doc! {"_id":&row.id},
        doc! {"$unset":{"active_turn":""},"$set":{"pending_events":bson::to_bson(&queued).unwrap()}},
    ).await.unwrap();

    let mut sweep_totals = (0, 0);
    let mut stopped = false;
    for _ in 0..10 {
        let messages_and_queue = queue::sweep(&state.db).await.unwrap();
        let admissions = channel_media_events::sweep(&state.db).await.unwrap();
        sweep_totals.0 += messages_and_queue;
        sweep_totals.1 += admissions;
        if messages_and_queue + admissions == 0 {
            stopped = true;
            break;
        }
    }
    assert!(stopped, "bounded maintenance must stop making progress");
    assert_eq!(sweep_totals, (2, 1));
    let current = engine::get(&state.db, &row.user_id, &row.id).await.unwrap();
    assert!(current.active_turn.is_none());
    assert_eq!(current.pending_events.len(), 1);
    assert_eq!(current.pending_events[0].id, queued[1].id);
    assert_eq!(current.pending_events[0].text, "fresh photo request");
    let fresh = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&sources[1].id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        fresh
            .media
            .unwrap()
            .items
            .iter()
            .all(|i| i.outcome.is_none())
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(MESSAGES)
            .count_documents(doc! {"media_queue_notice_id":&queued[1].id})
            .await
            .unwrap(),
        0
    );
    let finalized = saved(&state, &crashed_message).await;
    assert!(finalized.media_pending.is_none());
    assert_eq!(
        finalized.media_bound.unwrap().failed[0].status,
        "not_processed: failed"
    );
    let interrupted = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&interrupted.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(interrupted.status, "interrupted");
    assert!(interrupted.media.unwrap().interrupted);

    crate::services::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&state.db)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.user_id},
            doc! {"$set":{"role_ids":[roles.admin]}},
        )
        .await
        .unwrap();
    let auth = crate::test_utils::test_auth_user(&row.user_id);
    let mut admin_converted = 0;
    let mut drained = false;
    for _ in 0..10 {
        let result = crate::handlers::admin_assistant::drain_channel_media(
            State(state.clone()),
            auth.clone(),
        )
        .await
        .unwrap()
        .0;
        admin_converted += result.converted;
        if result.converted == 0 {
            drained = true;
            break;
        }
    }
    assert!(drained, "bounded admin drain must reach zero");
    assert_eq!(
        admin_converted, 1,
        "only the fresh item remains for admin conversion"
    );
    assert_eq!(queue::sweep(&state.db).await.unwrap(), 0);
    assert_eq!(channel_media_events::sweep(&state.db).await.unwrap(), 0);

    // These are the two media rollback queries documented in chat/10-uploads.md.
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .count_documents(doc! {
                "pending_events":{"$elemMatch":{"media_event_ids.0":{"$exists":true}}}
            })
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(MESSAGES)
            .count_documents(doc! {
                "media_pending":{"$ne":bson::Bson::Null}
            })
            .await
            .unwrap(),
        0
    );
    for (source, request) in sources.iter().zip(&queued) {
        let saved = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":&source.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            saved
                .media
                .unwrap()
                .items
                .iter()
                .all(|i| i.outcome.as_ref().unwrap().status == "not_processed: expired")
        );
        let notices: Vec<AssistantMessage> = state
            .db
            .collection::<AssistantMessage>(MESSAGES)
            .find(doc! {"media_queue_notice_id":&request.id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(notices.len(), 1);
        assert!(
            notices[0]
                .text
                .contains("expired before it could be processed; please resend")
        );
        assert!(notices[0].text.contains(&format!("> {}", request.text)));
        assert!(notices[0].activities.is_empty());
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    for key in [
        flags::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
        flags::ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY,
    ] {
        assert!(
            !flags::personal_flag_enabled(&state.db, &row.user_id, key)
                .await
                .unwrap()
        );
    }
}

#[tokio::test]
async fn channel_media_uploads_share_browser_record_context_and_strict_base_egress() {
    let (mut state, row, message, event) = fixture().await;
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(true);
    keys.set_allow_legacy_record_reads(false);
    state.encryption_keys = std::sync::Arc::new(keys);
    let id = upload(&state, &row, &event).await;
    binding::bind(&state.db, &message, &proposed(&event, id.clone()))
        .await
        .unwrap();
    let chat = crate::services::assistant_acknowledgement_service::for_key(
        &state.db,
        &row.user_id,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    let attachment = uploads::for_egress(&state.db, &chat, &id).await.unwrap();
    assert_eq!(
        uploads::egress_bytes(&state.db, &state.encryption_keys, &chat, &attachment)
            .await
            .unwrap(),
        png()
    );
    let roots = state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
    let root = roots.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
    assert_eq!(
        root.get_document("secret_context_versions")
            .unwrap()
            .get_i32("text_encrypted")
            .unwrap(),
        3
    );
    crate::services::assistant_record_service::verify_current_attachment(
        &state.db,
        &state.encryption_keys,
        &root,
    )
    .await
    .unwrap();
    let chunk = roots
        .find_one(doc! {"parent_attachment_id":&id})
        .await
        .unwrap()
        .unwrap();
    crate::services::assistant_record_service::verify_current_attachment(
        &state.db,
        &state.encryption_keys,
        &chunk,
    )
    .await
    .unwrap();
    // A browser-upload chunk has the same context schema but a different root.
    let browser = uploads::upload(
        &state.db,
        &state.encryption_keys,
        &row.user_id,
        &row.id,
        "browser.png",
        png(),
    )
    .await
    .unwrap();
    let browser_chunk = roots
        .find_one(doc! {"parent_attachment_id":&browser.id})
        .await
        .unwrap()
        .unwrap();
    roots
        .update_one(
            doc! {"_id":chunk.get("_id").unwrap()},
            doc! {"$set":{"data_encrypted":browser_chunk.get("data_encrypted").unwrap()}},
        )
        .await
        .unwrap();
    assert!(
        uploads::egress_bytes(&state.db, &state.encryption_keys, &chat, &attachment)
            .await
            .is_err()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn channel_media_erasure_fence_rejects_held_event_and_upload() {
    let (state, row, _, event) = fixture().await;
    let mut tx = state.db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    crate::services::erasure_admission::close_in_session(&state.db, &row.user_id, &mut tx)
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .delete_many(doc! {"user_id":&row.user_id})
        .await
        .unwrap();
    assert!(
        crate::services::channel_media_events::insert_event(&state.db, &event)
            .await
            .is_err()
    );
    assert!(matches!(
        uploads::upload_channel_authorized(
            &state.db,
            &state.encryption_keys,
            &row.user_id,
            &row.id,
            "receipt.png",
            png(),
            UploadSource {
                turn_id: row.active_turn.as_ref().unwrap().turn_id.clone(),
                event_key: event.media.as_ref().unwrap().key.clone(),
                index: 0,
                embedded: false,
                declared: 1,
            }
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    for collection in [EVENTS, crate::models::assistant_upload::COLLECTION_NAME] {
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {"user_id":&row.user_id})
                .await
                .unwrap(),
            0
        );
    }
    state.db.drop().await.unwrap();
}
