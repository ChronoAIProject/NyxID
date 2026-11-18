use super::*;
use crate::test_utils::connect_transaction_test_database;
fn at(value: &str) -> DateTime<Utc> {
    value.parse().unwrap()
}
fn work(
    owner: &str,
    turn: &str,
    time: DateTime<Utc>,
    success: bool,
    status: &str,
    source: &str,
) -> Document {
    doc! {"_id":turn,"turn_id":turn,"conversation_id":"execution","role":"assistant","status":status,"created_at":bson::DateTime::from_chrono(time),
    "turn_metadata":{"work":{"owner_id":owner,"settled_at":bson::DateTime::from_chrono(time),"source":source,"business_system_success":success}},"text":"PRIVATE CONTENT MUST NOT ENTER ANALYTICS"}
}
#[test]
fn singapore_weeks_and_threshold_bounds() {
    let boundary = at("2026-11-01T16:00:00Z");
    assert_eq!(week_start(boundary), boundary);
    assert_eq!(
        week_start(boundary - Duration::milliseconds(1)),
        boundary - Duration::weeks(1)
    );
    assert_eq!(week_start(boundary + Duration::days(6)), boundary);
    assert_eq!(threshold(None).unwrap(), 3);
    assert_eq!(threshold(Some(1)).unwrap(), 1);
    assert!(threshold(Some(0)).is_err());
    assert!(threshold(Some(1001)).is_err());
}
#[tokio::test]
async fn weekly_work_separates_internal_counts_thresholds_rates_and_voice_mirrors() {
    let db = connect_transaction_test_database("weekly_work").await;
    ensure_indexes(&db).await.unwrap();
    let from = at("2026-11-01T16:00:00Z");
    let to = from + Duration::weeks(1);
    let messages = db.collection::<Document>("assistant_messages");
    for (owner, total) in [("merchant", 3), ("internal", 4), ("below", 2)] {
        for i in 0..total {
            messages
                .insert_one(work(
                    owner,
                    &format!("{owner}-{i}"),
                    from + Duration::hours(1),
                    true,
                    "completed",
                    "chat",
                ))
                .await
                .unwrap();
        }
    }
    messages
        .insert_one(work("merchant", "failed", from, true, "failed", "chat"))
        .await
        .unwrap();
    messages
        .insert_one(work(
            "merchant",
            "model-only",
            from,
            false,
            "completed",
            "chat",
        ))
        .await
        .unwrap();
    let mut legacy = work("merchant", "legacy", from, true, "completed", "chat");
    legacy.remove("turn_metadata");
    messages.insert_one(legacy).await.unwrap();
    let mut mirror = work(
        "merchant",
        "merchant-0",
        from + Duration::days(1),
        true,
        "completed",
        "chat",
    );
    mirror
        .get_document_mut("turn_metadata")
        .unwrap()
        .get_document_mut("work")
        .unwrap()
        .insert(
            "settled_at",
            bson::DateTime::from_chrono(from + Duration::hours(1)),
        );
    mirror.insert("_id", "mirror");
    mirror.insert("conversation_id", "visible");
    messages.insert_one(mirror).await.unwrap();
    // A delayed mirror in the next reporting week is not another task.
    let mut delayed = work("merchant", "merchant-0", to, true, "completed", "chat");
    delayed
        .get_document_mut("turn_metadata")
        .unwrap()
        .get_document_mut("work")
        .unwrap()
        .insert(
            "settled_at",
            bson::DateTime::from_chrono(from + Duration::hours(1)),
        );
    delayed.insert("_id", "delayed");
    delayed.insert("conversation_id", "visible");
    messages.insert_one(delayed).await.unwrap();
    db.collection::<Document>(SETTINGS)
        .insert_one(doc! {"_id":"internal_owners","owner_ids":["internal"]})
        .await
        .unwrap();
    db.collection::<Document>("assistant_message_feedback")
        .insert_many([
            doc! {"turn_id":"merchant-0","vote":"up","comment":"PRIVATE"},
            doc! {"turn_id":"merchant-1","vote":"down"},
            doc! {"turn_id":"failed","vote":"up"},
        ])
        .await
        .unwrap();
    let report = weekly(&db, from, to, 3).await.unwrap();
    let w = &report.weeks[0];
    assert_eq!(
        (w.merchants, w.internal_owners, w.completed, w.attempted),
        (1, 1, 5, 6)
    );
    assert_eq!((w.internal_completed, w.helpful, w.votes), (4, 1, 2));
    assert!(!w.partial);
    assert_eq!(w.owners.len(), 3);
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("PRIVATE"));
    assert!(!json.contains("comment"));
    let lower = weekly(&db, from, to, 2).await.unwrap();
    assert_eq!(lower.weeks[0].merchants, 2);
    messages
        .update_one(
            doc! {"_id":"delayed"},
            doc! {"$set":{"status":"failed","error_code":"cancelled"}},
        )
        .await
        .unwrap();
    let revised = weekly(&db, from, to, 3).await.unwrap();
    assert_eq!(
        (revised.weeks[0].completed, revised.weeks[0].merchants),
        (4, 0)
    );
    let next = weekly(&db, to, to + Duration::weeks(1), 1).await.unwrap();
    assert_eq!(next.weeks[0].completed, 0);
    db.drop().await.unwrap();
}
#[tokio::test]
async fn triggered_runs_count_once_and_week_cutoffs_are_exclusive() {
    let db = connect_transaction_test_database("weekly_triggered_work").await;
    let boundary = at("2026-11-01T16:00:00Z");
    let from = boundary - Duration::hours(1);
    let to = boundary + Duration::hours(1);
    let messages = db.collection::<Document>("assistant_messages");
    messages
        .insert_one(work(
            "owner",
            "sunday",
            boundary - Duration::milliseconds(1),
            true,
            "completed",
            "channel",
        ))
        .await
        .unwrap();
    messages
        .insert_one(work("owner", "monday", boundary, true, "completed", "chat"))
        .await
        .unwrap();
    messages
        .insert_one(work("owner", "excluded-end", to, true, "completed", "chat"))
        .await
        .unwrap();
    for i in 0..2 {
        let mut turn = work(
            "owner",
            &format!("run-turn-{i}"),
            boundary,
            true,
            "completed",
            "scheduled",
        );
        turn.get_document_mut("turn_metadata")
            .unwrap()
            .get_document_mut("work")
            .unwrap()
            .insert("trigger_run_id", "schedule");
        messages.insert_one(turn).await.unwrap();
    }
    for (id, source, outcome) in [
        ("schedule", "scheduled", "completed"),
        ("webhook", "event", "completed"),
        ("failed", "event", "failed"),
        ("waiting", "event", "waiting"),
    ] {
        db.collection::<Document>("trigger_runs").insert_one(doc!{"_id":id,"turn_id":format!("{id}-turn"),"finished_at":bson::DateTime::from_chrono(boundary),"outcome":outcome,"work":{"owner_id":"owner","business_system_success":true,"source":source}}).await.unwrap();
    }
    let report = weekly(&db, from, to, 3).await.unwrap();
    assert_eq!(report.weeks.len(), 2);
    assert_eq!(
        (report.weeks[0].completed, report.weeks[1].completed),
        (1, 3)
    );
    assert_eq!(
        (
            report.weeks[1].triggered,
            report.weeks[1].attempted,
            report.weeks[1].merchants
        ),
        (2, 4, 1)
    );
    assert!(report.weeks.iter().all(|w| w.partial));
    db.drop().await.unwrap();
}
#[tokio::test]
async fn activation_funnel_is_owner_scoped_cumulative_and_measured_through_end() {
    let db = connect_transaction_test_database("real_work_funnel").await;
    ensure_indexes(&db).await.unwrap();
    let from = at("2026-11-01T16:00:00Z");
    let created = from + Duration::hours(1);
    let to = from + Duration::weeks(1);
    for (id, kind) in [
        ("activated", "person"),
        ("chat", "person"),
        ("platform", "person"),
        ("no-bot", "person"),
        ("internal", "org"),
    ] {
        db.collection::<Document>("users")
            .insert_one(
                doc! {"_id":id,"user_type":kind,"created_at":bson::DateTime::from_chrono(created)},
            )
            .await
            .unwrap();
        if id != "no-bot" {
            db.collection::<Document>("assistant_agents").insert_one(doc!{"user_id":id,"kind":if kind=="org"{"specialist"}else{"nyxbot"},"created_at":bson::DateTime::from_chrono(created)}).await.unwrap();
        }
        let mut connection = doc! {"user_id":id,"slug":"business","created_at":bson::DateTime::from_chrono(created),"api_key_id":"key"};
        if id == "platform" {
            connection.insert("credential_binding", "platform");
        }
        db.collection::<Document>("user_services")
            .insert_one(connection)
            .await
            .unwrap();
        let mut channel = work(
            id,
            &format!("{id}-channel"),
            created,
            false,
            "completed",
            "channel",
        );
        channel.insert("role", "user");
        if id != "chat" {
            db.collection::<Document>("assistant_messages")
                .insert_one(channel)
                .await
                .unwrap();
        }
        db.collection::<Document>("assistant_messages")
            .insert_one(work(
                id,
                &format!("{id}-done"),
                created + Duration::hours(2),
                true,
                "completed",
                "chat",
            ))
            .await
            .unwrap();
    }
    // A model connection is never a business system, even with an owner's key.
    db.collection::<Document>("user_services").insert_one(doc!{"user_id":"platform","slug":"custom-model","catalog_service_id":"model","created_at":bson::DateTime::from_chrono(created),"api_key_id":"key"}).await.unwrap();
    db.collection::<Document>("downstream_services").insert_one(doc!{"_id":"model","service_category":"connection","inference":{},"slug":"custom-model"}).await.unwrap();
    db.collection::<Document>(SETTINGS)
        .insert_one(doc! {"_id":"internal_owners","owner_ids":["internal"]})
        .await
        .unwrap();
    let report = weekly(&db, from, to, 3).await.unwrap();
    let f = report.weeks[0].funnel.iter().find(|f| !f.internal).unwrap();
    assert_eq!(
        (
            f.owners,
            f.has_nyxbot,
            f.connected_business,
            f.assigned_channel_work,
            f.first_real_work
        ),
        (4, 3, 2, 1, 1)
    );
    assert_eq!(f.median_seconds_to_first_work, Some(7200.));
    let i = report.weeks[0].funnel.iter().find(|f| f.internal).unwrap();
    assert_eq!(i.first_real_work, 1);
    let early = weekly(&db, from, created + Duration::hours(1), 3)
        .await
        .unwrap();
    assert!(early.weeks[0].funnel.iter().all(|f| f.first_real_work == 0));
    db.drop().await.unwrap();
}
#[tokio::test]
async fn internal_owner_updates_validate_ids_dedupe_and_audit_metadata() {
    let db = connect_transaction_test_database("real_work_internal").await;
    assert!(internal(&db).await.unwrap().owner_ids.is_empty());
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>("users")
        .insert_one(doc! {"_id":&id,"user_type":"org"})
        .await
        .unwrap();
    let actor = AuditActor {
        user_id: uuid::Uuid::new_v4().to_string(),
        api_key_id: None,
        api_key_name: None,
        ip_address: None,
        user_agent: None,
    };
    let result = set_internal(
        &db,
        &actor,
        InternalOwners {
            owner_ids: vec![id.clone(), id.clone()],
        },
    )
    .await
    .unwrap();
    assert_eq!(result.owner_ids, vec![id.clone()]);
    assert!(
        set_internal(
            &db,
            &actor,
            InternalOwners {
                owner_ids: vec!["invalid".into()]
            }
        )
        .await
        .is_err()
    );
    assert!(
        set_internal(
            &db,
            &actor,
            InternalOwners {
                owner_ids: vec![uuid::Uuid::new_v4().to_string()]
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"assistant_internal_owners_updated"})
            .await
            .unwrap(),
        1
    );
    set_internal(&db, &actor, InternalOwners::default())
        .await
        .unwrap();
    assert!(internal(&db).await.unwrap().owner_ids.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn real_work_indexes_only_include_rows_with_work_owner_evidence() {
    let db = connect_transaction_test_database("real_work_partial_indexes").await;
    ensure_indexes(&db).await.unwrap();
    let mut seen = 0;
    for (collection, field, expected) in [
        ("assistant_messages", "turn_metadata.work.owner_id", 3),
        ("trigger_runs", "work.owner_id", 2),
    ] {
        let rows = db.collection::<Document>(collection);
        let indexes: Vec<_> = rows
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let work_indexes: Vec<_> = indexes
            .into_iter()
            .filter(|index| {
                index
                    .options
                    .as_ref()
                    .and_then(|o| o.name.as_ref())
                    .is_some_and(|name| name.contains("_work_"))
            })
            .collect();
        assert_eq!(work_indexes.len(), expected);
        for index in work_indexes {
            assert_eq!(
                index.options.unwrap().partial_filter_expression,
                Some(doc! {field:{"$exists":true}})
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 5);
    db.drop().await.unwrap();
}
