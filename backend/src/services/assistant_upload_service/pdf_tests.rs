use super::super::{assistant_acknowledgement_service as acks, attachment_extraction::pdf_fixture};
use super::pdf_vision as pdf;
use super::*;

fn capabilities() -> Value {
    json!({"protocol":"nyxagent-input-image-v1","input_image":{"version":1,"sources":["nyxid_attachment_url"],"max_images":10,"max_image_bytes":8*1024*1024,"max_total_bytes":16*1024*1024,"content_types":["image/jpeg","image/png"]}})
}

#[tokio::test]
async fn pdf_index_upgrade_preserves_chunk_and_page_uniqueness() {
    use mongodb::{IndexModel, options::IndexOptions};
    let db = crate::test_utils::connect_transaction_test_database("pdf_indexes").await;
    let collection = db.collection::<bson::Document>(COLLECTION_NAME);
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"parent_attachment_id":1,"chunk_index":1})
                .options(IndexOptions::builder().unique(true).sparse(true).build())
                .build(),
        )
        .await
        .unwrap();
    collection
        .insert_one(doc! {"_id":"chunk","parent_attachment_id":"pdf","chunk_index":0_i64})
        .await
        .unwrap();
    collection.insert_one(doc! {"_id":"page1","origin":pdf::ORIGIN,"parent_attachment_id":"pdf","page":1_i64,"render_version":"v1"}).await.unwrap();
    let page2 = doc! {"_id":"page2","origin":pdf::ORIGIN,"parent_attachment_id":"pdf","page":2_i64,"render_version":"v1"};
    assert!(collection.insert_one(&page2).await.is_err());
    for _ in 0..2 {
        pdf::ensure_indexes(&db).await.unwrap();
    }
    collection.insert_one(page2).await.unwrap();
    assert!(
        collection
            .insert_one(
                doc! {"_id":"duplicate_chunk","parent_attachment_id":"pdf","chunk_index":0_i64}
            )
            .await
            .is_err()
    );
    assert!(collection.insert_one(doc! {"_id":"duplicate_page","origin":pdf::ORIGIN,"parent_attachment_id":"pdf","page":2_i64,"render_version":"v1"}).await.is_err());
    db.drop().await.unwrap();
}
async fn enable(db: &Database) {
    db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {"_id":Uuid::new_v4().to_string(),"flag_key":"assistant:pdf-vision","enabled":true,"target_kind":"global","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"updated_by":"fixture"}).await.unwrap();
}

async fn bound(
    pages: &[Option<&str>],
    channel: bool,
) -> (
    crate::AppState,
    crate::models::assistant_conversation::AssistantConversation,
    TurnAttachment,
    ChatAuthority,
) {
    let (state, user, id) = super::integration_tests::fixture().await;
    let original = pdf_fixture(pages, 90);
    let item = if channel {
        upload_channel_authorized(
            &state.db,
            &state.encryption_keys,
            &user,
            &id,
            "scan.pdf",
            original,
            crate::models::assistant_channel_media::UploadSource {
                guest: None,
                turn_id: String::new(),
                event_key: "fixture-lark".into(),
                index: 0,
                embedded: false,
                declared: 1,
            },
        )
        .await
        .unwrap()
    } else {
        upload(
            &state.db,
            &state.encryption_keys,
            &user,
            &id,
            "scan.pdf",
            original,
        )
        .await
        .unwrap()
    };
    let row = engine::begin_turn(
        &state.db,
        &user,
        &super::integration_tests::request(&id, vec![item.id.clone()]),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let chat = acks::for_key(&state.db, &user, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    (state, row, item, chat)
}

#[tokio::test]
async fn scanned_pdf_web_and_channel_share_images_authority_proofs_and_retention() {
    use sha2::{Digest, Sha256};
    for channel in [false, true] {
        let (state, row, item, mut chat) = bound(&[None, Some("Available text")], channel).await;
        enable(&state.db).await;
        let input = pdf::prepare(
            &state,
            &row,
            std::slice::from_ref(&item),
            &capabilities(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(input.images.len(), 1);
        assert_eq!(input.reports[0].1.text_pages, vec![2]);
        assert_eq!(input.reports[0].1.image_pages, vec![1]);
        assert!(input.reports[0].1.unread_pages.is_empty());
        let (parts, omitted) = image_plan(&input.images, &capabilities(), "https://nyx.example");
        assert_eq!(parts.len(), 1);
        assert!(omitted.is_empty());
        assert!(
            parts[0]["image_url"]
                .as_str()
                .unwrap()
                .ends_with(&format!("/{}/content", input.images[0].id))
        );
        chat.platform_model_context = Some("fixture-admitted-handle".into());
        let image_id = &input.images[0].id;
        let (mime, bytes) = chat_bytes(&state.db, &state.encryption_keys, &chat, image_id, true)
            .await
            .unwrap();
        assert_eq!(mime, "image/jpeg");
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 2048); // rotation is applied
        let digests = std::collections::HashSet::from([format!("{:x}", Sha256::digest(&bytes))]);
        super::super::assistant_platform_model::check_images(
            &state.db,
            &chat,
            "fixture-admitted-handle",
            &digests,
        )
        .await
        .unwrap();
        assert!(for_egress(&state.db, &chat, image_id).await.is_ok());
        let file = for_file_egress(&state.db, &chat, image_id).await.unwrap();
        assert_eq!(
            file_egress_bytes(&state.db, &state.encryption_keys, &chat, &file)
                .await
                .unwrap(),
            bytes
        );
        let identity = super::super::raw_attachment_tool::metadata_identity(&state.db, image_id)
            .await
            .unwrap();
        assert_eq!(
            turn_attachments(
                &state.db,
                &row.user_id,
                &row.id,
                &row.active_turn.as_ref().unwrap().turn_id
            )
            .await
            .unwrap()
            .len(),
            1
        );
        let mut stranger = chat.clone();
        stranger.api_key_id = Uuid::new_v4().to_string();
        assert!(
            chat_bytes(&state.db, &state.encryption_keys, &stranger, image_id, true)
                .await
                .is_err()
        );
        for collaboration in [false, true] {
            let mut guest = chat.clone();
            guest.guest = true;
            guest.collaboration_thread = collaboration;
            assert!(
                chat_bytes(&state.db, &state.encryption_keys, &guest, image_id, true)
                    .await
                    .is_err()
            );
        }
        // Encrypt/migrate by typed parent+page identity, never by chunk_index.
        let stored = state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":image_id})
            .await
            .unwrap()
            .unwrap();
        let mut strict = crate::test_utils::test_encryption_keys();
        strict.set_contextual_record_writes(true);
        super::super::assistant_record_service::migrate_attachment_snapshot(
            &state.db, &strict, &stored,
        )
        .await
        .unwrap();
        strict.set_allow_legacy_record_reads(false);
        let migrated = state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":image_id})
            .await
            .unwrap()
            .unwrap();
        super::super::assistant_record_service::verify_current_attachment(
            &state.db, &strict, &migrated,
        )
        .await
        .unwrap();
        assert_eq!(
            identity,
            super::super::raw_attachment_tool::metadata_identity(&state.db, image_id)
                .await
                .unwrap()
        );
        assert_eq!(
            chat_bytes(&state.db, &strict, &chat, image_id, true)
                .await
                .unwrap()
                .1,
            bytes
        );
        state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(doc! {"_id":image_id}, doc! {"$set":{"page":2}})
            .await
            .unwrap();
        assert_ne!(
            identity,
            super::super::raw_attachment_tool::metadata_identity(&state.db, image_id)
                .await
                .unwrap()
        );
        assert!(
            chat_bytes(&state.db, &strict, &chat, image_id, true)
                .await
                .is_err()
        );
        state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(doc! {"_id":image_id}, doc! {"$set":{"page":1}})
            .await
            .unwrap();
        state.db.collection::<bson::Document>(COLLECTION_NAME).update_one(doc!{"_id":&item.id},doc!{"$set":{"bound_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(31))}}).await.unwrap();
        assert!(matches!(
            chat_bytes(&state.db, &strict, &chat, image_id, true).await,
            Err(AppError::AssistantAttachmentExpired)
        ));
        assert!(
            super::super::assistant_platform_model::check_images(
                &state.db,
                &chat,
                "fixture-admitted-handle",
                &digests
            )
            .await
            .is_err()
        );
        assert!(matches!(
            super::super::assistant_upload_retention::require_available(
                &state.db,
                doc! {"_id":image_id}
            )
            .await,
            Err(AppError::AssistantAttachmentExpired)
        ));
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pdf_fallbacks_and_original_images_share_one_budget_and_report_unread_pages() {
    let (state, row, item, chat) = bound(&[None; 10], false).await;
    let no_text = read(
        &state.db,
        &state.encryption_keys,
        &chat,
        &json!({"attachment_id":item.id}),
    )
    .await
    .unwrap();
    assert_eq!(no_text["error"], "attachment_has_no_extracted_text");
    let off = pdf::prepare(
        &state,
        &row,
        std::slice::from_ref(&item),
        &capabilities(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(off.reports[0].1.reason.as_deref(), Some("disabled"));
    assert_eq!(off.reports[0].1.unread_pages.len(), 10);
    assert!(off.images.is_empty());
    enable(&state.db).await;
    for (cap, text_only, reason) in [
        (capabilities(), true, "model_text_only"),
        (Value::Null, false, "agent_unsupported"),
    ] {
        let input = pdf::prepare(&state, &row, std::slice::from_ref(&item), &cap, text_only)
            .await
            .unwrap();
        assert_eq!(input.reports[0].1.reason.as_deref(), Some(reason));
        assert!(input.images.is_empty());
    }
    let limited = pdf::prepare(
        &state,
        &row,
        std::slice::from_ref(&item),
        &capabilities(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(limited.images.len(), 8);
    assert_eq!(limited.reports[0].1.unread_pages, vec![9, 10]);
    let mut original = limited.images[0].clone();
    original.origin = "user_upload".into();
    original.id = Uuid::new_v4().to_string();
    let mut cap = capabilities();
    cap["input_image"]["max_images"] = json!(2);
    let mut input = pdf::prepare(&state, &row, &[original.clone(), item.clone()], &cap, false)
        .await
        .unwrap();
    assert_eq!(input.images.len(), 1);
    assert_eq!(input.images[0].id, limited.images[0].id);
    assert_eq!(
        Uuid::parse_str(&input.images[0].id)
            .unwrap()
            .get_version_num(),
        4
    );
    let mut all = vec![original];
    all.extend(input.images.clone());
    let (parts, omitted) = image_plan(&all, &cap, "https://nyx.example");
    assert_eq!(parts.len(), 2);
    assert!(omitted.is_empty());
    input.finalize(
        &input
            .images
            .iter()
            .map(|a| a.id.clone())
            .collect::<Vec<_>>(),
        "image_input_unsupported",
    );
    assert!(input.reports[0].1.image_pages.is_empty());
    assert_eq!(input.reports[0].1.unread_pages.len(), 10);
    assert!(input.notice().contains("NOT read"));
    // Settlement downgrades staged/sent pages on Stop, outside render cancellation.
    state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id":&row.id,"attachments.id":&item.id},
            doc! {"$set":{"attachments.$.pdf":bson::to_bson(&limited.reports[0].1).unwrap()}},
        )
        .await
        .unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    pdf::failed_in_session(&state.db, &mut session, &row, "stopped")
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let saved = turn_attachments(
        &state.db,
        &row.user_id,
        &row.id,
        &row.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap();
    let coverage = saved[0].pdf.as_ref().unwrap();
    assert!(coverage.image_pages.is_empty());
    assert_eq!(coverage.unread_pages.len(), 10);
    assert_eq!(coverage.reason.as_deref(), Some("stopped"));
    engine::request_stop(&state.db, &row.user_id, &row.id)
        .await
        .unwrap();
    assert!(
        pdf::for_chat(&state.db, &chat, &limited.images[0].id)
            .await
            .is_err()
    );
    state.db.drop().await.unwrap();
}

#[test]
fn legacy_pdf_classification_ignores_separator_newlines_and_read_ranges() {
    let extracted = Extracted {
        pdf: None,
        content_type: "application/pdf".into(),
        text: " \n\ntext\n".into(),
        sections: vec![
            attachment_extraction::Section {
                offset: 0,
                label: "Page 1".into(),
            },
            attachment_extraction::Section {
                offset: 3,
                label: "Page 2".into(),
            },
        ],
    };
    let result = pdf::coverage(&extracted);
    assert_eq!(result.classification, "mixed");
    assert_eq!(result.unread_pages, vec![1]);
    assert_eq!(result.text_pages, vec![2]);
}

#[tokio::test]
async fn pdf_staged_pages_are_unread_when_begin_turn_reclaims_an_expired_lease() {
    let (state, row, item, _) = bound(&[None, None], false).await;
    enable(&state.db).await;
    let input = pdf::prepare(
        &state,
        &row,
        std::slice::from_ref(&item),
        &capabilities(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(input.reports[0].1.image_pages, vec![1, 2]);
    let old_turn = row.active_turn.as_ref().unwrap().turn_id.clone();
    state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id":&row.id,"turn_id":&old_turn,"attachments.id":&item.id},
            doc! {"$set":{"attachments.$.pdf":bson::to_bson(&input.reports[0].1).unwrap()}},
        )
        .await
        .unwrap();
    state.db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{
            "active_turn.started_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(engine::ACTIVE_TURN_TTL_SECS+1)),
            "active_turn.heartbeat_at":bson::Bson::Null,
            "active_turn.lease_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))
        }}).await.unwrap();
    let next = engine::begin_turn(
        &state.db,
        &row.user_id,
        &super::integration_tests::request(&row.id, vec![]),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    assert_ne!(next.active_turn.as_ref().unwrap().turn_id, old_turn);
    let saved = turn_attachments(&state.db, &row.user_id, &row.id, &old_turn)
        .await
        .unwrap();
    let coverage = saved[0].pdf.as_ref().unwrap();
    assert!(coverage.image_pages.is_empty());
    assert_eq!(coverage.unread_pages, vec![1, 2]);
    assert_eq!(coverage.reason.as_deref(), Some("turn_failed"));
    let messages = engine::messages(&state.db, &row.user_id, &row.id, 100, None)
        .await
        .unwrap();
    assert!(
        messages.iter().any(|message| message.turn_id == old_turn
            && message.error_code.as_deref() == Some("turn_lost"))
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn stale_worker_cannot_write_pdf_coverage_after_reclaim_transaction() {
    let (state, row, item, _) = bound(&[None, None], false).await;
    enable(&state.db).await;
    let prepared = pdf::prepare(
        &state,
        &row,
        std::slice::from_ref(&item),
        &capabilities(),
        false,
    )
    .await
    .unwrap();
    let old_turn = row.active_turn.as_ref().unwrap().turn_id.clone();
    state.db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{
            "active_turn.started_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(engine::ACTIVE_TURN_TTL_SECS+1)),
            "active_turn.heartbeat_at":bson::Bson::Null,
            "active_turn.lease_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))
        }}).await.unwrap();
    engine::begin_turn(
        &state.db,
        &row.user_id,
        &super::integration_tests::request(&row.id, vec![]),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let stale = crate::services::assistant_nyxagent_runtime::images::record_pdf_outcomes(
        &state,
        &row,
        &prepared.reports,
    )
    .await;
    assert!(stale.is_err());
    let saved = turn_attachments(&state.db, &row.user_id, &row.id, &old_turn)
        .await
        .unwrap();
    let coverage = saved[0].pdf.as_ref().unwrap();
    assert!(coverage.image_pages.is_empty());
    assert_eq!(coverage.unread_pages, vec![1, 2]);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn blank_non_pdf_keeps_generic_read_guidance() {
    let (state, user, id) = super::integration_tests::fixture().await;
    let item = upload(
        &state.db,
        &state.encryption_keys,
        &user,
        &id,
        "empty.txt",
        b" \n\t".to_vec(),
    )
    .await
    .unwrap();
    let row = engine::begin_turn(
        &state.db,
        &user,
        &super::integration_tests::request(&id, vec![item.id.clone()]),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let chat = acks::for_key(&state.db, &user, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let value = read(
        &state.db,
        &state.encryption_keys,
        &chat,
        &json!({"attachment_id":item.id}),
    )
    .await
    .unwrap();
    assert_eq!(value["error"], "attachment_has_no_extracted_text");
    assert!(value["pdf"].is_null());
    let instructions = value["instructions"].as_str().unwrap();
    assert!(instructions.contains("nyx__machine_save_attachment"));
    assert!(!instructions.contains("PDF"));
    assert!(!instructions.contains("page images"));
    state.db.drop().await.unwrap();
}

#[tokio::test]
#[ignore = "local private sample only; NYXID_PDF_SAMPLE points outside repository"]
async fn local_private_pdf_regression() {
    let path = std::env::var_os("NYXID_PDF_SAMPLE").expect("local sample path");
    let bytes = std::fs::read(path).unwrap();
    let extracted = attachment_extraction::extract(bytes.clone(), "pdf".into())
        .await
        .unwrap();
    let report = pdf::coverage(&extracted);
    assert_eq!(report.classification, "no_text");
    assert_eq!(report.unread_pages, vec![1, 2]);
    let _reservation = byte_budget::reserve(super::pdf_renderer::RESERVATION)
        .await
        .unwrap();
    for page in [1, 2] {
        let image = super::pdf_renderer::render(&bytes, page).await.unwrap();
        let decoded = image::load_from_memory(&image).unwrap();
        eprintln!(
            "local page {page}: {}x{}, {} encoded bytes",
            decoded.width(),
            decoded.height(),
            image.len()
        );
    }
}
