//! Real MongoDB contention and cancellation at the PDF preparation boundary.
use super::super::{
    assistant_acknowledgement_service as acks, assistant_group_service as groups,
    assistant_team_service as team,
};
use super::{pdf_renderer as renderer, pdf_vision as pdf, *};
use crate::models::assistant_conversation::AssistantConversation;
use mongodb::event::{EventHandler, command::CommandEvent};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration as WallDuration,
};

fn capabilities() -> Value {
    json!({"protocol":"nyxagent-input-image-v1","input_image":{"version":1,"sources":["nyxid_attachment_url"],"max_images":10,"max_image_bytes":8*1024*1024,"max_total_bytes":16*1024*1024,"content_types":["image/jpeg"]}})
}

#[derive(Clone, Copy)]
enum Statement {
    ParentWrite,
    ParentRead,
    SourceRead,
}

type ParentWatch = Arc<Mutex<Option<(String, tokio::sync::oneshot::Sender<()>)>>>;

/// Pause an actual driver statement before dispatch, without a failpoint or
/// fabricated Mongo error. A separate client can change its snapshot's parent.
struct Gate {
    target: Arc<Mutex<Option<String>>>,
    next_parent: ParentWatch,
    hit: Option<tokio::sync::oneshot::Receiver<()>>,
    resume: std::sync::mpsc::Sender<()>,
    conflicts: Arc<AtomicUsize>,
}

impl Gate {
    async fn database(kind: Statement, ordinal: usize) -> (Database, Self) {
        let target = Arc::new(Mutex::new(None::<String>));
        let next_parent: ParentWatch = Arc::new(Mutex::new(None));
        let (notify, hit) = tokio::sync::oneshot::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let pause = Mutex::new(Some((notify, wait)));
        let count = AtomicUsize::new(0);
        let conflicts = Arc::new(AtomicUsize::new(0));
        let observer = EventHandler::callback({
            let target = target.clone();
            let next_parent = next_parent.clone();
            let conflicts = conflicts.clone();
            move |event| match event {
                CommandEvent::Started(event) => {
                    let filter = match event.command_name.as_str() {
                        "find" if event.command.get_str("find") == Ok(COLLECTION_NAME) => {
                            event.command.get_document("filter").ok()
                        }
                        "update"
                            if event.command.get_str("update") == Ok(COLLECTION_NAME)
                                && event.command.get_bool("autocommit") == Ok(false) =>
                        {
                            event.command.get_array("updates").ok().and_then(|updates| {
                                updates.first()?.as_document()?.get_document("q").ok()
                            })
                        }
                        _ => None,
                    };
                    let Some(filter) = filter else {
                        return;
                    };
                    {
                        let mut next = next_parent.lock().unwrap();
                        if event.command_name == "find"
                            && next
                                .as_ref()
                                .is_some_and(|(id, _)| filter.get_str("_id") == Ok(id.as_str()))
                        {
                            let (_, notify) = next.take().unwrap();
                            let _ = notify.send(());
                        }
                    }
                    let target = target.lock().unwrap().clone();
                    let Some(target) = target else {
                        return;
                    };
                    let matches = match kind {
                        Statement::ParentWrite => {
                            event.command_name == "update"
                                && filter.get_str("_id") == Ok(target.as_str())
                        }
                        Statement::ParentRead => {
                            event.command_name == "find"
                                && filter.get_str("_id") == Ok(target.as_str())
                        }
                        Statement::SourceRead => {
                            event.command_name == "find"
                                && filter.get_str("parent_attachment_id") == Ok(target.as_str())
                                && filter.contains_key("chunk_index")
                        }
                    };
                    if matches && count.fetch_add(1, Ordering::SeqCst) + 1 == ordinal {
                        let paused = pause.lock().unwrap().take();
                        if let Some((notify, wait)) = paused {
                            let _ = notify.send(());
                            wait.recv_timeout(WallDuration::from_secs(15))
                                .expect("release PDF statement gate");
                        }
                    }
                }
                CommandEvent::Failed(event)
                    if event.failure.contains_label("TransientTransactionError") =>
                {
                    let code = match event.failure.kind.as_ref() {
                        mongodb::error::ErrorKind::Command(error) => Some(error.code),
                        mongodb::error::ErrorKind::Write(
                            mongodb::error::WriteFailure::WriteError(error),
                        ) => Some(error.code),
                        _ => None,
                    };
                    // An aborted transaction may also report NoSuchTransaction
                    // on cleanup. Only a real write conflict proves the race.
                    if code == Some(112) {
                        conflicts.fetch_add(1, Ordering::SeqCst);
                    }
                }
                _ => {}
            }
        });
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "pdf_contention",
            observer,
        )
        .await;
        (
            db,
            Self {
                target,
                next_parent,
                hit: Some(hit),
                resume,
                conflicts,
            },
        )
    }

    fn arm(&self, parent: &str) {
        *self.target.lock().unwrap() = Some(parent.into());
    }

    async fn reached(&mut self) {
        tokio::time::timeout(WallDuration::from_secs(10), self.hit.take().unwrap())
            .await
            .unwrap()
            .unwrap();
    }

    fn watch_next_parent(&self, parent: &str) -> tokio::sync::oneshot::Receiver<()> {
        let (notify, observed) = tokio::sync::oneshot::channel();
        *self.next_parent.lock().unwrap() = Some((parent.into(), notify));
        observed
    }
}

async fn independent(db: &Database) -> Database {
    mongodb::Client::with_uri_str(crate::test_utils::selected_test_database_uri())
        .await
        .unwrap()
        .database(db.name())
}

async fn fixture(
    db: Database,
    pages: &[usize],
) -> (
    crate::AppState,
    AssistantConversation,
    Vec<TurnAttachment>,
    ChatAuthority,
) {
    let (state, user, id) = super::integration_tests::fixture_with_database(db).await;
    let mut items = Vec::new();
    for &count in pages {
        items.push(
            upload(
                &state.db,
                &state.encryption_keys,
                &user,
                &id,
                "scan.pdf",
                attachment_extraction::pdf_fixture(&vec![None; count], 90),
            )
            .await
            .unwrap(),
        );
    }
    let row = engine::begin_turn(
        &state.db,
        &user,
        &super::integration_tests::request(&id, items.iter().map(|a| a.id.clone()).collect()),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let chat = acks::for_key(&state.db, &user, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "flag_key": "assistant:pdf-vision", "enabled": true,
        "target_kind": "global", "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(), "updated_by": "fixture",
    }).await.unwrap();
    (state, row, items, chat)
}

fn prepare(
    state: crate::AppState,
    row: AssistantConversation,
    items: Vec<TurnAttachment>,
    cap: Value,
) -> tokio::task::JoinHandle<AppResult<pdf::Prepared>> {
    tokio::spawn(async move {
        pdf::prepare_with_timeout(
            &state,
            &row,
            &items,
            &cap,
            false,
            WallDuration::from_secs(1),
        )
        .await
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pdf_shared_parent_real_write_conflict_retries_and_group_specialists_share_one_page() {
    let _budget_guard = byte_budget::test_guard().await;
    let (db, mut gate) = Gate::database(Statement::ParentWrite, 1).await;
    let (state, user, _) = super::integration_tests::fixture_with_database(db).await;
    let nyxbot = team::ensure_nyxbot(&state.db, &user).await.unwrap();
    let (specialist, _) = team::create_specialist(
        &state.db,
        &state.encryption_keys,
        &user,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: "pdf-review".into(),
            description: "Review scanned PDFs".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let group = groups::create(
        &state.db,
        &user,
        "PDF reviewers",
        &[nyxbot.id.clone(), specialist.id.clone()],
        "user",
    )
    .await
    .unwrap();
    let item = upload(
        &state.db,
        &state.encryption_keys,
        &user,
        &group.id,
        "shared.pdf",
        attachment_extraction::pdf_fixture(&[None], 90),
    )
    .await
    .unwrap();
    groups::append_with_uploads(
        &state.db,
        &user,
        &group.id,
        "user",
        None,
        "Review this",
        std::slice::from_ref(&item.id),
    )
    .await
    .unwrap();
    let mut chats = Vec::new();
    for agent in [nyxbot, specialist] {
        let mut start = engine::TurnStart::from(&super::integration_tests::request("", vec![]));
        start.conversation_id = None;
        start.agent_id = Some(agent.id);
        start.group_id = Some(group.id.clone());
        start.origin = crate::models::assistant_conversation::TurnOrigin::Group;
        start.group_attachments = group_attachments(&state.db, &user, &group.id, 0)
            .await
            .unwrap();
        let row = engine::begin_turn(&state.db, &user, &start, &state.encryption_keys)
            .await
            .unwrap();
        chats.push(
            acks::for_key(&state.db, &user, Some(&row.credential_api_key_id))
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let parent = for_chat(&state.db, &chats[0], &item.id).await.unwrap();
    let bytes = renderer::render(&attachment_extraction::pdf_fixture(&[None], 90), 1)
        .await
        .unwrap();
    gate.arm(&item.id);
    let first = tokio::spawn({
        let (state, chat, parent, bytes) = (
            state.clone(),
            chats[0].clone(),
            parent.clone(),
            bytes.clone(),
        );
        async move { pdf::store(&state.db, &state.encryption_keys, &chat, &parent, 1, &bytes).await }
    });
    gate.reached().await;
    // The first transaction has already established a snapshot through the
    // owner/user fences. This real write makes its parent UPDATE fail with 112.
    let competing = independent(&state.db).await;
    competing
        .collection::<bson::Document>(COLLECTION_NAME)
        .update_one(doc! {"_id": &item.id}, doc! {"$inc": {"pdf_generation": 1}})
        .await
        .unwrap();
    let second = pdf::store(
        &state.db,
        &state.encryption_keys,
        &chats[1],
        &parent,
        1,
        &bytes,
    );
    let _ = gate.resume.send(());
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap().unwrap();
    let second = second.unwrap();
    assert!(
        gate.conflicts.load(Ordering::SeqCst) >= 1,
        "must exercise a real MongoDB112"
    );
    assert_eq!(first.id, second.id);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .count_documents(doc! {"origin":pdf::ORIGIN,"parent_attachment_id":&item.id})
            .await
            .unwrap(),
        1
    );
    for chat in &chats {
        assert_eq!(
            pdf::bytes(&state.db, &state.encryption_keys, chat, &first.id)
                .await
                .unwrap()
                .1,
            bytes
        );
    }
    state.db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pdf_conflict_retry_rechecks_stop_parent_deletion_and_retention() {
    let _budget_guard = byte_budget::test_guard().await;
    for revoke in ["stop", "delete", "expire"] {
        let (db, mut gate) = Gate::database(Statement::ParentWrite, 1).await;
        let (state, row, items, chat) = fixture(db, &[1]).await;
        let parent = for_chat(&state.db, &chat, &items[0].id).await.unwrap();
        gate.arm(&parent.id);
        let pending = tokio::spawn({
            let (state, parent) = (state.clone(), parent.clone());
            async move {
                pdf::store(
                    &state.db,
                    &state.encryption_keys,
                    &chat,
                    &parent,
                    1,
                    b"bounded JPEG fixture",
                )
                .await
            }
        });
        gate.reached().await;
        let competing = independent(&state.db).await;
        let roots = competing.collection::<bson::Document>(COLLECTION_NAME);
        roots
            .update_one(
                doc! {"_id": &parent.id},
                doc! {"$inc": {"pdf_generation": 1}},
            )
            .await
            .unwrap();
        match revoke {
            "stop" => {
                competing
                    .collection::<bson::Document>(
                        crate::models::assistant_conversation::COLLECTION_NAME,
                    )
                    .update_one(
                        doc! {"_id":&row.id},
                        doc! {"$set":{"active_turn.stop_requested":true}},
                    )
                    .await
                    .unwrap();
            }
            "delete" => {
                roots.delete_one(doc! {"_id":&parent.id}).await.unwrap();
            }
            "expire" => {
                roots.update_one(doc! {"_id":&parent.id},doc! {"$set":{"bound_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(31))}}).await.unwrap();
            }
            _ => unreachable!(),
        }
        let _ = gate.resume.send(());
        let outcome = pending.await.unwrap();
        assert!(outcome.is_err(), "revoked {revoke} must fail closed");
        if revoke == "expire" {
            assert!(matches!(outcome, Err(AppError::AssistantAttachmentExpired)));
        }
        assert!(gate.conflicts.load(Ordering::SeqCst) >= 1);
        assert_eq!(
            roots
                .count_documents(doc! {"origin":pdf::ORIGIN})
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pdf_memory_contention_obeys_whole_deadline_and_releases_waiter() {
    let _budget_guard = byte_budget::test_guard().await;
    let db = crate::test_utils::connect_transaction_test_database("pdf_memory_deadline").await;
    let (state, row, items, _) = fixture(db, &[2]).await;
    let full = byte_budget::reserve(byte_budget::CAPACITY / 2)
        .await
        .unwrap();
    let input = tokio::time::timeout(
        WallDuration::from_secs(3),
        pdf::prepare_with_timeout(
            &state,
            &row,
            &items,
            &capabilities(),
            false,
            WallDuration::from_secs(1),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(input.images.is_empty());
    assert!(input.sources.is_empty());
    assert_eq!(input.reports[0].1.unread_pages, vec![1, 2]);
    assert_eq!(input.reports[0].1.reason.as_deref(), Some("render_timeout"));
    assert_eq!(byte_budget::available_for_tests(), 0);
    drop(full);
    assert_eq!(byte_budget::available_for_tests(), byte_budget::CAPACITY);
    let prepared = pdf::prepare(&state, &row, &items, &capabilities(), false)
        .await
        .unwrap();
    assert_eq!(prepared.images.len(), 2);
    assert_eq!(byte_budget::available_for_tests(), byte_budget::CAPACITY);
    engine::request_stop(&state.db, &row.user_id, &row.id)
        .await
        .unwrap();
    let full = byte_budget::reserve(byte_budget::CAPACITY / 2)
        .await
        .unwrap();
    assert!(
        pdf::prepare_with_timeout(
            &state,
            &row,
            &items,
            &capabilities(),
            false,
            WallDuration::from_secs(1)
        )
        .await
        .is_err()
    );
    drop(full);
    state.db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pdf_source_lookup_deadline_fails_closed_on_authority_and_releases_loaded_source_permits() {
    let _budget_guard = byte_budget::test_guard().await;
    for kind in [Statement::ParentRead, Statement::SourceRead] {
        let (db, mut gate) = Gate::database(kind, 1).await;
        let (state, row, mut items, _) = fixture(db, &[2]).await;
        // Incoming metadata must never turn a failed authorization/read into a
        // claim that these pages were included in the current model input.
        let cached = items[0].pdf.as_mut().unwrap();
        cached.image_pages = vec![1, 2];
        cached.unread_pages.clear();
        cached.image_input_confirmed = true;
        gate.arm(&items[0].id);
        let pending = prepare(state.clone(), row, items, capabilities());
        gate.reached().await;
        if matches!(kind, Statement::SourceRead) {
            assert_eq!(
                byte_budget::available_for_tests(),
                byte_budget::CAPACITY - 2 * renderer::RESERVATION
            );
        }
        // Synchronous command handlers hold the polling thread. Let the
        // original PDF budget elapse, then release the statement so its next
        // asynchronous database wait observes that same expired deadline.
        tokio::time::sleep(WallDuration::from_secs(1)).await;
        let _ = gate.resume.send(());
        let result = tokio::time::timeout(WallDuration::from_secs(3), pending)
            .await
            .unwrap()
            .unwrap();
        match kind {
            Statement::ParentRead => assert!(matches!(result, Err(AppError::Internal(_)))),
            Statement::SourceRead => {
                let input = result.unwrap();
                assert!(input.images.is_empty());
                assert!(input.reports[0].1.image_pages.is_empty());
                assert!(!input.reports[0].1.image_input_confirmed);
                assert_eq!(input.reports[0].1.unread_pages, vec![1, 2]);
                assert_eq!(input.reports[0].1.reason.as_deref(), Some("render_timeout"));
            }
            _ => unreachable!(),
        }
        assert_eq!(byte_budget::available_for_tests(), byte_budget::CAPACITY);
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pdf_legacy_coverage_decryption_deadline_uses_authorized_parent_page_count() {
    use crate::crypto::{
        key_provider::{KeyProvider, WrappedKey},
        local_key_provider::LocalKeyProvider,
    };
    use zeroize::Zeroizing;

    struct PausedCoverageKey {
        local: LocalKeyProvider,
        target: Mutex<Option<Zeroizing<Vec<u8>>>>,
        reached: tokio::sync::Notify,
    }
    impl std::fmt::Debug for PausedCoverageKey {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("PausedCoverageKey([REDACTED])")
        }
    }
    #[async_trait::async_trait]
    impl KeyProvider for PausedCoverageKey {
        async fn wrap_dek(&self, dek: &[u8]) -> AppResult<WrappedKey> {
            let wrapped = self.local.wrap_dek(dek).await?;
            *self.target.lock().unwrap() = Some(wrapped.ciphertext.clone());
            Ok(wrapped)
        }
        async fn unwrap_dek(&self, wrapped: &WrappedKey) -> AppResult<Zeroizing<Vec<u8>>> {
            let targeted = self
                .target
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|target| **target == *wrapped.ciphertext);
            if targeted {
                self.reached.notify_one();
                // Model a KMS read that remains pending until the PDF deadline
                // cancels it. Other real encrypted record reads still succeed.
                std::future::pending::<()>().await;
            }
            self.local.unwrap_dek(wrapped).await
        }
        fn current_key_id(&self) -> u8 {
            self.local.current_key_id()
        }
        fn has_key_id(&self, id: u8) -> bool {
            self.local.has_key_id(id)
        }
        fn has_previous_key(&self) -> bool {
            self.local.has_previous_key()
        }
    }

    let _budget_guard = byte_budget::test_guard().await;
    let db = crate::test_utils::connect_transaction_test_database("pdf_legacy_deadline").await;
    let (mut state, row, mut items, chat) = fixture(db, &[2]).await;
    let parent = for_chat(&state.db, &chat, &items[0].id).await.unwrap();
    let plaintext = super::super::record_field_service::decrypt(
        &state.encryption_keys,
        COLLECTION_NAME,
        &parent.id,
        &parent.user_id,
        "text_encrypted",
        parent
            .secret_context_versions
            .get("text_encrypted")
            .copied(),
        &parent.text_encrypted,
    )
    .await
    .unwrap();
    let provider = Arc::new(PausedCoverageKey {
        local: LocalKeyProvider::from_config(&state.config),
        target: Mutex::new(None),
        reached: tokio::sync::Notify::new(),
    });
    let keys = EncryptionKeys::with_provider(provider.clone());
    let encrypted = keys.encrypt(&plaintext).await.unwrap();
    state.db.collection::<bson::Document>(COLLECTION_NAME).update_one(doc! {"_id":&parent.id}, doc! {
        "$set":{"text_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}},
        "$unset":{"pdf":"","secret_context_versions.text_encrypted":""},
    }).await.unwrap();
    state.encryption_keys = Arc::new(keys);
    items[0].pages = Some(1); // Untrusted/stale turn metadata is not the fallback source.
    let cached = items[0].pdf.as_mut().unwrap();
    cached.image_pages = vec![1, 2];
    cached.unread_pages.clear();
    cached.image_input_confirmed = true;
    let pending = prepare(state.clone(), row, items, capabilities());
    tokio::time::timeout(WallDuration::from_secs(3), provider.reached.notified())
        .await
        .unwrap();
    let input = pending.await.unwrap().unwrap();
    assert!(input.images.is_empty());
    let report = &input.reports[0].1;
    assert_eq!(report.classification, "unknown");
    assert_eq!(report.unread_pages, vec![1, 2]);
    assert!(report.image_pages.is_empty());
    assert!(!report.image_input_confirmed);
    assert_eq!(report.reason.as_deref(), Some("render_timeout"));
    assert_eq!(byte_budget::available_for_tests(), byte_budget::CAPACITY);
    state.db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pdf_storage_deadline_retains_staged_progress_without_charging_later_pdf_budget() {
    let _budget_guard = byte_budget::test_guard().await;
    let (db, mut gate) = Gate::database(Statement::ParentWrite, 2).await;
    let (state, row, items, _) = fixture(db, &[2, 1]).await;
    gate.arm(&items[0].id);
    let next = gate.watch_next_parent(&items[1].id);
    let mut cap = capabilities();
    cap["input_image"]["max_images"] = json!(2);
    let pending = prepare(state.clone(), row, items.clone(), cap);
    gate.reached().await;
    // Hold the second storage statement past the first PDF's deadline. Once
    // released, its next async wait must cancel it before the next PDF starts.
    tokio::time::sleep(WallDuration::from_secs(1)).await;
    let _ = gate.resume.send(());
    tokio::time::timeout(WallDuration::from_secs(3), next)
        .await
        .unwrap()
        .unwrap();
    let input = pending.await.unwrap().unwrap();
    assert_eq!(input.images.len(), 2);
    assert_eq!(
        input
            .sources
            .iter()
            .map(|(_, id, page)| (id.as_str(), *page))
            .collect::<Vec<_>>(),
        vec![(items[0].id.as_str(), 1), (items[1].id.as_str(), 1)]
    );
    assert_eq!(input.reports[0].1.image_pages, vec![1]);
    assert_eq!(input.reports[0].1.unread_pages, vec![2]);
    assert_eq!(input.reports[0].1.reason.as_deref(), Some("render_timeout"));
    assert_eq!(input.delivery().reports[0].1.unread_pages, vec![1, 2]);
    assert_eq!(input.reports[1].1.image_pages, vec![1]);
    assert_eq!(byte_budget::available_for_tests(), byte_budget::CAPACITY);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .count_documents(doc! {"origin":pdf::ORIGIN})
            .await
            .unwrap(),
        2
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pdf_image_budget_uses_canonical_cached_size_in_both_directions() {
    let _budget_guard = byte_budget::test_guard().await;
    let db = crate::test_utils::connect_transaction_test_database("pdf_canonical_budget").await;
    let (state, row, items, chat) = fixture(db, &[1, 1]).await;
    let parent = for_chat(&state.db, &chat, &items[0].id).await.unwrap();
    let source = attachment_extraction::pdf_fixture(&[None], 90);
    let rendered = renderer::render(&source, 1).await.unwrap();
    let mut larger = rendered.clone();
    larger.resize(rendered.len() + 100_000, 0); // Valid JPEG with trailing padding.
    let canonical = pdf::store(
        &state.db,
        &state.encryption_keys,
        &chat,
        &parent,
        1,
        &larger,
    )
    .await
    .unwrap();
    for limit in ["max_image_bytes", "max_total_bytes"] {
        let mut cap = capabilities();
        cap["input_image"]["max_images"] = json!(1);
        cap["input_image"][limit] = json!(rendered.len() + 1);
        let input = pdf::prepare(&state, &row, &items, &cap, false)
            .await
            .unwrap();
        assert_eq!(input.images.len(), 1);
        assert_eq!(input.sources[0].1, items[1].id);
        assert_eq!(input.reports[0].1.unread_pages, vec![1]);
        assert_eq!(input.reports[0].1.reason.as_deref(), Some("image_limit"));
    }
    // A smaller canonical cache entry fits even when this replica's rendering
    // would exceed the request's byte limit. Candidate size cannot reject it.
    let mut small = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(2, 2)
        .write_to(&mut small, image::ImageFormat::Jpeg)
        .unwrap();
    state
        .db
        .collection::<bson::Document>(COLLECTION_NAME)
        .delete_one(doc! {"_id":&canonical.id})
        .await
        .unwrap();
    let small = small.into_inner();
    let canonical = pdf::store(&state.db, &state.encryption_keys, &chat, &parent, 1, &small)
        .await
        .unwrap();
    let mut cap = capabilities();
    cap["input_image"]["max_image_bytes"] = json!(small.len());
    let input = pdf::prepare(&state, &row, &items[..1], &cap, false)
        .await
        .unwrap();
    assert_eq!(input.images[0].id, canonical.id);
    assert_eq!(input.images[0].size, small.len() as i64);
    assert_eq!(input.reports[0].1.image_pages, vec![1]);
    state.db.drop().await.unwrap();
}
