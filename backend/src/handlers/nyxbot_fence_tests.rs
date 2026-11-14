use super::*;
use crate::services::nyxbot_thread_mutation::{
    self as mutation,
    pause::{HOOK, Hook},
};

const SECOND: &str = "conv_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const THIRD: &str = "conv_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Fixture {
    state: AppState,
    row: NyxbotChannel,
    key: String,
    server: tokio_util::task::AbortOnDropHandle<()>,
    source: NyxbotThread,
    history: String,
    relay: String,
}

async fn fixture(org: bool, source_partition: &str) -> Fixture {
    let (state, _, server) = setup("thread_row_fences").await;
    let (row, key) = if org {
        let (row, key, _, _) = org_channel_tests::org_channel(&state, false).await;
        (row, key)
    } else {
        channel(&state, "gateway").await
    };
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_turns".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    for partition in [PARTITION, SECOND, THIRD] {
        assert_eq!(
            put_conversation(
                State(state.clone()),
                Path(("bnd_turns".into(), partition.into())),
                bearer(&key)
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
    let relay = chats::direct_partition("fenced-private-chat", "7", None);
    let source_partition = if source_partition == "direct" {
        &relay
    } else {
        source_partition
    };
    let source = chats::record_chat(&state, &row, source_partition, &private_facts(), None)
        .await
        .unwrap();
    let history = uuid::Uuid::new_v4().to_string();
    let mut origin = doc! {"nyxbot_channel_id":&row.id,"partition":source_partition};
    if let Some(binding) = &row.org_specialist {
        origin.insert(
            "org_execution",
            doc! {"org_id":&binding.org_id,"revision":&binding.revision},
        );
        state.db.collection::<bson::Document>(crate::models::org_channel::LANES).insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"channel_id":&row.id,"partition":source_partition,
            "org_id":&binding.org_id,"binding_revision":&binding.revision,"conversation_id":&history
        }).await.unwrap();
    }
    state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .insert_one(doc! {
            "_id":&history,"user_id":OWNER,"channel":&origin,"reply_channel":origin
        })
        .await
        .unwrap();
    state.db.collection::<bson::Document>(THREADS).update_one(doc! {"_id":&source.id},doc! {"$set":{
        "relay_partition":&relay,"conversation_id":&history,"members":"owner","allow_posts":true
    }}).await.unwrap();
    let source = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id":&source.id})
        .await
        .unwrap()
        .unwrap();
    Fixture {
        state,
        row,
        key,
        source,
        history,
        relay,
        server: tokio_util::task::AbortOnDropHandle::new(server),
    }
}

fn private_facts() -> chats::ChatFacts {
    chats::ChatFacts {
        kind: "private",
        chat_id: "fenced-private-chat".into(),
        thread_id: None,
        owner: true,
        title: None,
    }
}

async fn adopt(f: &Fixture, destination: &str) -> AppResult<()> {
    chats::adopt_relay_chat(
        &f.state,
        &f.row,
        destination,
        "fenced-private-chat",
        "7",
        None,
    )
    .await
}

async fn finish(f: Fixture) {
    drop(f.server);
    f.state.db.drop().await.unwrap();
}

async fn assert_history(f: &Fixture, partition: &str) {
    let rows = f.state.db.collection::<bson::Document>(THREADS);
    let actual = rows
        .find_one(doc! {"_id":&f.source.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual.get_str("partition"), Ok(partition));
    assert_eq!(actual.get_str("relay_partition"), Ok(f.relay.as_str()));
    assert_eq!(actual.get_str("conversation_id"), Ok(f.history.as_str()));
    assert_eq!(
        rows.count_documents(doc! {"channel_id":&f.row.id,"relay_partition":&f.relay})
            .await
            .unwrap(),
        1
    );
    let history = f
        .state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .find_one(doc! {"_id":&f.history})
        .await
        .unwrap()
        .unwrap();
    for field in ["channel", "reply_channel"] {
        assert_eq!(
            history.get_document(field).unwrap().get_str("partition"),
            Ok(partition)
        );
    }
    if f.row.org_specialist.is_some() {
        let lanes: Vec<bson::Document> = f
            .state
            .db
            .collection(crate::models::org_channel::LANES)
            .find(doc! {"channel_id":&f.row.id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].get_str("partition"), Ok(partition));
        assert_eq!(lanes[0].get_str("conversation_id"), Ok(f.history.as_str()));
    }
}

#[tokio::test]
async fn exact_personal_and_org_adoption_preserve_serial_history_and_remove_source() {
    for org in [false, true] {
        for source in ["direct", PARTITION] {
            let f = fixture(org, source).await;
            adopt(&f, SECOND).await.unwrap();
            assert_history(&f, SECOND).await;
            assert!(
                f.state
                    .db
                    .collection::<NyxbotThread>(THREADS)
                    .find_one(doc! {
                        "channel_id":&f.row.id,"partition":&f.source.partition
                    })
                    .await
                    .unwrap()
                    .is_none()
            );
            adopt(&f, SECOND).await.unwrap();
            assert_history(&f, SECOND).await;
            finish(f).await;
        }
    }
}

// The future retention release's storage effect, entirely test-only. It must
// commit while the real release-1 transaction is paused after its first read.
async fn retaining_move(f: &Fixture, destination: &str) {
    let context = (
        f.state.db.clone(),
        f.row.clone(),
        f.source.clone(),
        f.history.clone(),
        destination.to_owned(),
    );
    crate::services::transaction_service::run_db_only(&f.state.db,&context,|tx,(db,channel,source,history,destination)|Box::pin(async move {
        let rows=db.collection::<bson::Document>(THREADS);
        rows.delete_one(doc! {"channel_id":&channel.id,"partition":destination,"kind":null}).session(&mut *tx).await?;
        assert_eq!(rows.update_one(mutation::exact(source),doc! {"$set":{"partition":destination}}).session(&mut *tx).await?.matched_count,1);
        rows.insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"channel_id":&channel.id,"user_id":&channel.user_id,
            "partition":&source.partition,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"allow_posts":false
        }).session(&mut *tx).await?;
        for field in ["channel","reply_channel"] {
            db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":history},
                doc! {"$set":{format!("{field}.partition"):destination}}).session(&mut *tx).await?;
        }
        db.collection::<bson::Document>(crate::models::org_channel::LANES).update_many(doc! {
            "channel_id":&channel.id,"partition":&source.partition
        },doc! {"$set":{"partition":destination}}).session(tx).await?;
        Ok(())
    })).await.unwrap();
}

#[tokio::test]
async fn paused_personal_and_org_adoption_cannot_move_a_retained_replacement() {
    for org in [false, true] {
        let f = fixture(org, PARTITION).await;
        let hook = Hook::new(if org {
            "org_adoption"
        } else {
            "personal_adoption"
        });
        let writer = HOOK.scope(hook.clone(), adopt(&f, THIRD));
        let competitor = async {
            tokio::time::timeout(Duration::from_secs(10), hook.reached.notified())
                .await
                .unwrap();
            retaining_move(&f, SECOND).await;
            let stale = hook.observations.lock().await[0].clone();
            assert_eq!(
                f.state
                    .db
                    .collection::<bson::Document>(THREADS)
                    .update_one(stale, doc! {"$set":{"must_not_be_written":true}})
                    .await
                    .unwrap()
                    .matched_count,
                0
            );
            let replacement = f
                .state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {
                    "channel_id":&f.row.id,"partition":PARTITION
                })
                .await
                .unwrap()
                .unwrap();
            assert_ne!(replacement.id, f.source.id);
            assert!(replacement.kind.is_none() && replacement.relay_partition.is_none());
            hook.resume.notify_one();
        };
        let (result, ()) = tokio::join!(writer, competitor);
        result.unwrap();
        assert_history(&f, THIRD).await;
        let placeholder = f
            .state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {
                "channel_id":&f.row.id,"partition":PARTITION
            })
            .await
            .unwrap()
            .unwrap();
        assert!(
            placeholder.kind.is_none()
                && placeholder.relay_partition.is_none()
                && placeholder.conversation_id.is_none()
        );
        assert!(!placeholder.allow_posts);
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(THREADS)
                .count_documents(doc! {"must_not_be_written":true})
                .await
                .unwrap(),
            0
        );
        finish(f).await;
    }
}

#[tokio::test]
async fn personal_destination_replacement_is_atomic_to_provider_responses() {
    let f = fixture(false, "direct").await;
    let hook = Hook::new("personal_destination_deleted");
    let writer = HOOK.scope(hook.clone(), adopt(&f, SECOND));
    let probes = async {
        tokio::time::timeout(Duration::from_secs(10), hook.reached.notified())
            .await
            .unwrap();
        for i in 0..4 {
            let response = respond(
                &f.state,
                &f.key,
                &json!({"conversation":SECOND}),
                &format!("during-delete-{i}"),
            )
            .await;
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "{}",
                body_text(response).await
            );
        }
        hook.resume.notify_one();
    };
    let (result, ()) = tokio::join!(writer, probes);
    result.unwrap();
    assert_history(&f, SECOND).await;
    let response = respond(
        &f.state,
        &f.key,
        &json!({"conversation":SECOND}),
        "after-commit",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    finish(f).await;
}

#[tokio::test]
async fn group_rediscovery_deletes_observed_private_rows_without_retention() {
    for org in [false, true] {
        let f = fixture(org, PARTITION).await;
        chats::record_chat(&f.state, &f.row, &f.relay, &private_facts(), None)
            .await
            .unwrap();
        let partition = chats::group_partition("fenced-private-chat", None);
        let group = chats::record_chat(
            &f.state,
            &f.row,
            &partition,
            &chats::ChatFacts {
                kind: "group",
                ..private_facts()
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(group.kind.as_deref(), Some("group"));
        assert!(group.conversation_id.is_none());
        let rows = f.state.db.collection::<NyxbotThread>(THREADS);
        assert!(
            rows.find_one(doc! {"channel_id":&f.row.id,"partition":PARTITION})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            rows.find_one(doc! {"channel_id":&f.row.id,"partition":&f.relay})
                .await
                .unwrap()
                .is_none()
        );
        let response = respond(
            &f.state,
            &f.key,
            &json!({"conversation":PARTITION}),
            "still-dropped",
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(body_text(response).await.contains("conversation_not_found"));
        finish(f).await;
    }
}

#[tokio::test]
async fn group_rediscovery_cannot_delete_a_replacement_from_its_stale_snapshot() {
    let f = fixture(false, PARTITION).await;
    let hook = Hook::new("group_rediscovery");
    let partition = chats::group_partition("fenced-private-chat", None);
    let facts = chats::ChatFacts {
        kind: "group",
        ..private_facts()
    };
    let writer = HOOK.scope(
        hook.clone(),
        chats::record_chat(&f.state, &f.row, &partition, &facts, None),
    );
    let competitor = async {
        tokio::time::timeout(Duration::from_secs(10), hook.reached.notified())
            .await
            .unwrap();
        retaining_move(&f, SECOND).await;
        let stale = hook.observations.lock().await[0].clone();
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(THREADS)
                .delete_one(stale)
                .await
                .unwrap()
                .deleted_count,
            0
        );
        hook.resume.notify_one();
    };
    let (result, ()) = tokio::join!(writer, competitor);
    result.unwrap();
    let placeholder = f
        .state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {
            "channel_id":&f.row.id,"partition":PARTITION
        })
        .await
        .unwrap()
        .unwrap();
    assert!(placeholder.kind.is_none() && placeholder.relay_partition.is_none());
    // The retry reads and removes the actual stale private row at SECOND.
    assert!(
        f.state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"_id":&f.source.id})
            .await
            .unwrap()
            .is_none()
    );
    finish(f).await;
}

#[tokio::test]
async fn observed_chat_identity_refuses_kind_or_alias_changes() {
    let f = fixture(false, PARTITION).await;
    let rows = f.state.db.collection::<bson::Document>(THREADS);
    let original = rows
        .find_one(doc! {"_id":&f.source.id})
        .await
        .unwrap()
        .unwrap();
    for change in [
        doc! {"kind":"group"},
        doc! {"relay_partition":"other-alias"},
    ] {
        rows.update_one(doc! {"_id":&f.source.id}, doc! {"$set":change})
            .await
            .unwrap();
        assert_eq!(
            rows.delete_one(mutation::observed(&original).unwrap())
                .await
                .unwrap()
                .deleted_count,
            0
        );
        rows.replace_one(doc! {"_id":&f.source.id}, original.clone())
            .await
            .unwrap();
    }
    finish(f).await;
}

// Main's personal adoption sequence, with the already-read source supplied so
// tests can reproduce either reader ordering without running an old binary.
async fn main_personal_move(f: &Fixture, source: &NyxbotThread, destination: &str) -> u64 {
    let rows = f.state.db.collection::<bson::Document>(THREADS);
    rows.delete_one(doc! {"channel_id":&f.row.id,"partition":destination,"kind":null})
        .await
        .unwrap();
    let moved=rows.update_one(doc! {"channel_id":&f.row.id,"partition":&source.partition},
        doc! {"$set":{"partition":destination,"relay_partition":&f.relay,"updated_at":bson::DateTime::now()}}).await.unwrap();
    if moved.matched_count == 1 {
        for field in ["channel", "reply_channel"] {
            f.state.db.collection::<bson::Document>(CONVERSATIONS).update_many(doc! {"user_id":OWNER,
                format!("{field}.nyxbot_channel_id"):&f.row.id,format!("{field}.partition"):&source.partition
            },doc! {"$set":{format!("{field}.partition"):destination}}).await.unwrap();
        }
    }
    moved.matched_count
}

#[tokio::test]
async fn main_and_fenced_personal_writers_never_create_a_replacement_to_mistake() {
    // Main reads A; the new writer removes A; main's stale write matches zero.
    let f = fixture(false, PARTITION).await;
    let old_read = f.source.clone();
    adopt(&f, SECOND).await.unwrap();
    assert_eq!(main_personal_move(&f, &old_read, THIRD).await, 0);
    assert_history(&f, SECOND).await;
    finish(f).await;

    // The new writer reads A; main removes A; the transaction retries and
    // reads the actual private row at B before moving it to C.
    let f = fixture(false, PARTITION).await;
    let hook = Hook::new("personal_adoption");
    let writer = HOOK.scope(hook.clone(), adopt(&f, THIRD));
    let old_writer = async {
        tokio::time::timeout(Duration::from_secs(10), hook.reached.notified())
            .await
            .unwrap();
        assert_eq!(main_personal_move(&f, &f.source, SECOND).await, 1);
        hook.resume.notify_one();
    };
    let (result, ()) = tokio::join!(writer, old_writer);
    result.unwrap();
    assert_history(&f, THIRD).await;
    assert!(
        f.state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {
                "channel_id":&f.row.id,"partition":PARTITION
            })
            .await
            .unwrap()
            .is_none()
    );
    finish(f).await;
}

#[tokio::test]
async fn provider_refusal_history_distinguishes_retained_evidence_without_claiming_first_contact() {
    let (state, calls, server) = setup("partition_refusal_evidence").await;
    let (row, key) = channel(&state, "gateway").await;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"owner_sender_ids":["7"]}},
        )
        .await
        .unwrap();
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_turns".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    crate::services::coordination_service::TEST_RATE_WINDOW_NOW
        .scope(std::cell::Cell::new(bson::DateTime::now()), async {
            let body = event("Hello", "7", "evidence");
            let response = respond(&state, &key, &body, "evidence").await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(body_text(response).await.contains("conversation_not_found"));
            assert!(calls.lock().await.is_empty());
            let audit = state
                .db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
            let filter =
                doc! {"event_type":"nyxbot_provider_refused","event_data.channel_agent_id":&row.id};
            let first = audit.find_one(filter.clone()).await.unwrap().unwrap();
            let first = first.get_document("event_data").unwrap();
            assert_eq!(first.get_str("partition_history"), Ok("no_retained_event"));
            for _ in 0..2 {
                assert_eq!(
                    put_conversation(
                        State(state.clone()),
                        Path(("bnd_turns".into(), PARTITION.into())),
                        bearer(&key)
                    )
                    .await
                    .status(),
                    StatusCode::OK
                );
            }
            let response = respond(&state, &key, &body, "evidence").await;
            assert_eq!(response.status(), StatusCode::OK);
            assert!(body_text(response).await.contains("Here is your answer"));
            assert_eq!(calls.lock().await.len(), 1);
            assert_eq!(
                delete_conversation(
                    State(state.clone()),
                    Path(("bnd_turns".into(), PARTITION.into())),
                    bearer(&key)
                )
                .await
                .status(),
                StatusCode::NO_CONTENT
            );
            crate::services::coordination_service::TEST_RATE_WINDOW_NOW.with(|clock| {
                clock.set(bson::DateTime::from_millis(
                    clock.get().timestamp_millis() + 61_000,
                ))
            });
            let response = respond(
                &state,
                &key,
                &event("After delete", "7", "after-delete"),
                "after-delete",
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(body_text(response).await.contains("conversation_not_found"));
            let after = audit
                .find_one(filter)
                .sort(doc! {"seq":-1})
                .await
                .unwrap()
                .unwrap();
            let after = after.get_document("event_data").unwrap();
            assert_eq!(after.get_str("partition_history"), Ok("retained_event"));
            assert_eq!(
                after.get_str("partition_correlation_hash"),
                first.get_str("partition_correlation_hash")
            );
            assert_eq!(
                after.get_str("partition_correlation_hash").unwrap().len(),
                64
            );
            assert!(!after.to_string().contains(PARTITION));
            assert_eq!(calls.lock().await.len(), 1);
        })
        .await;
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn main_and_fenced_group_rediscovery_keep_the_same_destructive_semantics() {
    // Main has read A for adoption; new group discovery removes A as before.
    let f = fixture(false, PARTITION).await;
    let group = chats::group_partition("fenced-private-chat", None);
    let facts = chats::ChatFacts {
        kind: "group",
        ..private_facts()
    };
    chats::record_chat(&f.state, &f.row, &group, &facts, None)
        .await
        .unwrap();
    assert_eq!(main_personal_move(&f, &f.source, THIRD).await, 0);
    assert_eq!(
        f.state
            .db
            .collection::<NyxbotThread>(THREADS)
            .count_documents(doc! {
                "channel_id":&f.row.id,"kind":"private"
            })
            .await
            .unwrap(),
        0
    );
    finish(f).await;

    // New discovery reads A; main adoption moves it to B; retry reads B.
    let f = fixture(false, PARTITION).await;
    let hook = Hook::new("group_rediscovery");
    let writer = HOOK.scope(
        hook.clone(),
        chats::record_chat(&f.state, &f.row, &group, &facts, None),
    );
    let old_writer = async {
        tokio::time::timeout(Duration::from_secs(10), hook.reached.notified())
            .await
            .unwrap();
        assert_eq!(main_personal_move(&f, &f.source, SECOND).await, 1);
        hook.resume.notify_one();
    };
    let (result, ()) = tokio::join!(writer, old_writer);
    result.unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<NyxbotThread>(THREADS)
            .count_documents(doc! {
                "channel_id":&f.row.id,"kind":"private"
            })
            .await
            .unwrap(),
        0
    );
    assert!(
        f.state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .find_one(doc! {"_id":&f.history})
            .await
            .unwrap()
            .is_some()
    );
    finish(f).await;
}

#[tokio::test]
async fn group_rediscovery_removes_all_selected_rows_and_preserves_other_chats() {
    let f = fixture(false, PARTITION).await;
    let rows = f.state.db.collection::<bson::Document>(THREADS);
    let now = bson::DateTime::now();
    rows.insert_many((0..300).map(|i| doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"channel_id":&f.row.id,"user_id":OWNER,
        "partition":format!("conv_{i:032x}"),"relay_partition":format!("direct_{i:032x}"),
        "kind":"private","platform_chat_id":"fenced-private-chat","created_at":now,"updated_at":now
    })).await.unwrap();
    let other = chats::record_chat(
        &f.state,
        &f.row,
        "direct_other",
        &chats::ChatFacts {
            chat_id: "other-chat".into(),
            ..private_facts()
        },
        None,
    )
    .await
    .unwrap();
    let before = rows
        .find_one(doc! {"_id":&other.id})
        .await
        .unwrap()
        .unwrap();
    let group = chats::group_partition("fenced-private-chat", None);
    chats::record_chat(
        &f.state,
        &f.row,
        &group,
        &chats::ChatFacts {
            kind: "group",
            ..private_facts()
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        rows.count_documents(doc! {"channel_id":&f.row.id,"kind":"private"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        rows.find_one(doc! {"_id":&other.id})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert!(
        rows.find_one(doc! {"_id":&f.source.id})
            .await
            .unwrap()
            .is_none()
    );
    finish(f).await;
}

// These requests deliberately stop at the existing member policy after chat
// reconciliation. They exercise real provider admission without making model
// scheduling (or a turn already running) part of the row-contention assertion.
async fn concurrent_messages(f: &Fixture, kind: &str) -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let transactions = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(tokio::sync::Barrier::new(20));
    let requests = (0..20).map(|i| {
        let barrier = barrier.clone();
        let transactions = transactions.clone();
        async move {
            let id = format!("concurrent-{kind}-{i}");
            let mut body = event("Hello", "7", &id);
            body["conversation"] = json!(SECOND);
            body["event_context"]["conversation_id"] = json!(SECOND);
            body["event_context"]["activity"]["conversation"] =
                json!({"id":"fenced-private-chat","kind":kind});
            barrier.wait().await;
            mutation::pause::TRANSACTIONS
                .scope(transactions, async {
                    let response = Box::pin(respond(&f.state, &f.key, &body, &id)).await;
                    assert_eq!(response.status(), StatusCode::OK);
                    let text = body_text(response).await;
                    assert!(text.contains("response.completed"), "{text}");
                    assert!(!text.contains("response.failed"), "{text}");
                })
                .await;
        }
    });
    futures::future::join_all(requests).await;
    assert_eq!(
        f.state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .count_documents(doc! {"channel_id":&f.row.id,"status":"refused"})
            .await
            .unwrap(),
        20
    );
    transactions.load(Ordering::SeqCst)
}

#[tokio::test]
async fn twenty_concurrent_group_messages_use_plain_upserts_without_mutation_transactions() {
    let f = fixture(false, "direct").await;
    let rows = f.state.db.collection::<NyxbotThread>(THREADS);
    rows.delete_one(doc! {"_id":&f.source.id}).await.unwrap();
    let partition = chats::group_partition("fenced-private-chat", None);
    let group = chats::record_chat(
        &f.state,
        &f.row,
        &partition,
        &chats::ChatFacts {
            kind: "group",
            ..private_facts()
        },
        None,
    )
    .await
    .unwrap();
    rows.update_one(
        doc! {"_id":&group.id},
        doc! {"$set":{"reply_mode":"all","members":"owner"}},
    )
    .await
    .unwrap();
    assert_eq!(concurrent_messages(&f, "group").await, 0);
    let after = rows
        .find_one(doc! {"_id":&group.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.partition, partition);
    assert_eq!(after.kind.as_deref(), Some("group"));
    assert_eq!(after.members.as_deref(), Some("owner"));
    assert_eq!(after.reply_mode.as_deref(), Some("all"));
    assert!(after.conversation_id.is_none());
    // As on main: three untouched provider registrations and one group row.
    assert_eq!(
        rows.count_documents(doc! {"channel_id":&f.row.id})
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        rows.count_documents(doc! {"channel_id":&f.row.id,"kind":null})
            .await
            .unwrap(),
        3
    );
    finish(f).await;
}

#[tokio::test]
async fn twenty_concurrent_first_private_messages_adopt_once_without_caller_errors() {
    let f = fixture(false, "direct").await;
    assert!(concurrent_messages(&f, "private").await > 0);
    assert_history(&f, SECOND).await;
    let rows = f.state.db.collection::<NyxbotThread>(THREADS);
    assert!(
        rows.find_one(doc! {"channel_id":&f.row.id,"partition":&f.relay})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        rows.count_documents(doc! {"channel_id":&f.row.id})
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        rows.count_documents(doc! {"channel_id":&f.row.id,"kind":"private"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        rows.count_documents(doc! {"channel_id":&f.row.id,"kind":null})
            .await
            .unwrap(),
        2
    );
    finish(f).await;
}

#[tokio::test]
async fn personal_adoption_no_source_or_classified_destination_opens_no_transaction() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = fixture(false, "direct").await;
    let transactions = Arc::new(AtomicUsize::new(0));
    mutation::pause::TRANSACTIONS
        .scope(transactions.clone(), async {
            chats::adopt_relay_chat(&f.state, &f.row, THIRD, "no-source", "7", None)
                .await
                .unwrap();
            assert_eq!(transactions.load(Ordering::SeqCst), 0);
            adopt(&f, SECOND).await.unwrap();
            assert_eq!(transactions.load(Ordering::SeqCst), 1);
            adopt(&f, SECOND).await.unwrap();
            assert_eq!(transactions.load(Ordering::SeqCst), 1);
        })
        .await;
    assert_history(&f, SECOND).await;
    finish(f).await;
}
