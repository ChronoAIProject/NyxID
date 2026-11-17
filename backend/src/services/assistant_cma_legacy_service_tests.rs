use super::*;

fn at(value: &str) -> bson::DateTime {
    bson::DateTime::parse_rfc3339_str(value).unwrap()
}

async fn marker(db: &Database) -> bson::Document {
    durable_database(db)
        .collection::<bson::Document>(COLLECTION)
        .find_one(doc! {"_id": MARKER})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn earliest_retained_intent_wins_and_restart_deletion_and_catalog_changes_do_not_move_it() {
    let db =
        crate::test_utils::connect_transaction_test_database("cma_legacy_marker_earliest").await;
    let rows =
        db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME);
    let earliest = at("2026-01-02T00:00:00Z");
    rows.insert_many([
        doc! {"_id":"external","source":"discovered","created_at":at("2020-01-01T00:00:00Z")},
        doc! {"_id":"later","source":"nyxbot","created_at":at("2026-01-03T00:00:00Z")},
        doc! {"_id":"intent","source":"nyxbot","cma_sandbox_id":null,"admission_state":"retired","created_at":earliest},
    ]).await.unwrap();
    initialize(&db).await.unwrap();
    let original = marker(&db).await;
    assert_eq!(*original.get_datetime("cutoff").unwrap(), earliest);
    assert_eq!(original.get_str("rule").unwrap(), "earliest_creation");
    let original = bson::to_vec(&original).unwrap();
    rows.delete_many(doc! {}).await.unwrap();
    initialize(&db).await.unwrap();
    assert_eq!(bson::to_vec(&marker(&db).await).unwrap(), original);
    rows.insert_one(doc! {"_id":"older","source":"nyxbot","created_at":at("2025-01-01T00:00:00Z")})
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(doc! {"_id":"catalog","slug":"cma","base_url":"https://example.invalid"})
        .await
        .unwrap();
    initialize(&db).await.unwrap();
    assert_eq!(bson::to_vec(&marker(&db).await).unwrap(), original);
    let changed = insert_cutoff(&db, Some(at("2025-01-01T00:00:00Z")))
        .await
        .unwrap();
    assert_eq!(changed.modified_count, 0);
    assert!(changed.upserted_id.is_none());
    assert_eq!(bson::to_vec(&marker(&db).await).unwrap(), original);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn empty_deployment_uses_the_same_write_database_time_and_majority_concerns() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let db = crate::test_utils::connect_transaction_test_database("cma_legacy_marker_empty").await;
    let commands = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = commands.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            captured.lock().unwrap().push(event.command);
        }
    }));
    let client = mongodb::Client::with_options(options).unwrap();
    let db = client.database(db.name());
    initialize(&db).await.unwrap();
    let saved = marker(&db).await;
    assert_eq!(saved.get_str("rule").unwrap(), "database_time");
    assert_eq!(
        saved.get_datetime("cutoff").unwrap(),
        saved.get_datetime("written_at").unwrap()
    );
    assert!(cutoff(&db).await.is_some());
    let recorded = commands.lock().unwrap().clone();
    let write = recorded.iter().find(|c| c.contains_key("update")).unwrap();
    assert_eq!(
        write
            .get_document("writeConcern")
            .unwrap()
            .get_str("w")
            .unwrap(),
        "majority"
    );
    let update = write.get_array("updates").unwrap()[0]
        .as_document()
        .unwrap();
    let set = update.get_array("u").unwrap()[0]
        .as_document()
        .unwrap()
        .get_document("$set")
        .unwrap();
    assert_eq!(
        set.get_document("cutoff")
            .unwrap()
            .get_array("$cond")
            .unwrap()[1]
            .as_str(),
        Some("$$NOW")
    );
    for read in recorded.iter().filter(|c| c.contains_key("find")) {
        assert_eq!(
            read.get_document("readConcern")
                .unwrap()
                .get_str("level")
                .unwrap(),
            "majority"
        );
    }
    let before = bson::to_vec(&saved).unwrap();
    initialize(&db).await.unwrap();
    assert_eq!(bson::to_vec(&marker(&db).await).unwrap(), before);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_replicas_with_different_candidates_never_modify_the_winner() {
    let db = crate::test_utils::connect_transaction_test_database("cma_legacy_marker_race").await;
    let client = mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let peer = client.database(db.name());
    let candidates = [
        None,
        Some(at("2026-01-01T00:00:00Z")),
        Some(at("2026-01-02T00:00:00Z")),
    ];
    let mut writes = tokio::task::JoinSet::new();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(candidates.len()));
    for (index, candidate) in candidates.iter().copied().enumerate() {
        let db = if index % 2 == 0 {
            db.clone()
        } else {
            peer.clone()
        };
        let barrier = barrier.clone();
        writes.spawn(async move {
            barrier.wait().await;
            insert_cutoff(&db, candidate).await.unwrap()
        });
    }
    let mut inserted = 0;
    while let Some(result) = writes.join_next().await {
        let result = result.unwrap();
        inserted += usize::from(result.upserted_id.is_some());
        assert_eq!(result.modified_count, 0);
    }
    assert_eq!(inserted, 1);
    let original = marker(&db).await;
    let chosen = *original.get_datetime("cutoff").unwrap();
    match original.get_str("rule").unwrap() {
        "database_time" => assert_eq!(chosen, *original.get_datetime("written_at").unwrap()),
        "earliest_creation" => assert!(candidates.contains(&Some(chosen))),
        other => panic!("unexpected marker rule: {other}"),
    }
    let bytes = bson::to_vec(&original).unwrap();
    let (left, right) = tokio::join!(
        insert_cutoff(&db, Some(at("2020-01-01T00:00:00Z"))),
        insert_cutoff(&peer, None)
    );
    for result in [left, right] {
        let result = result.unwrap();
        assert_eq!(result.modified_count, 0);
        assert!(result.upserted_id.is_none());
    }
    let (left, right) = tokio::join!(initialize(&db), initialize(&peer));
    left.unwrap();
    right.unwrap();
    assert_eq!(bson::to_vec(&marker(&db).await).unwrap(), bytes);
    assert_eq!(bson::to_vec(&marker(&peer).await).unwrap(), bytes);
    assert_eq!(
        db.collection::<bson::Document>(COLLECTION)
            .count_documents(doc! {"_id": MARKER})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn malformed_markers_are_neither_repaired_nor_authority() {
    let db =
        crate::test_utils::connect_transaction_test_database("cma_legacy_marker_invalid").await;
    let rows = db.collection::<bson::Document>(COLLECTION);
    for saved in [
        doc! {"_id":MARKER},
        doc! {"_id":MARKER,"cutoff":null,"rule":"database_time","written_at":null},
        doc! {"_id":MARKER,"cutoff":"2026-01-01T00:00:00Z","rule":"database_time"},
        doc! {"_id":MARKER,"cutoff":at("2026-01-01T00:00:00Z"),"rule":"unknown"},
        doc! {"_id":MARKER,"cutoff":at("2999-01-01T00:00:00Z"),"rule":"database_time"},
    ] {
        rows.insert_one(&saved).await.unwrap();
        initialize(&db).await.unwrap();
        assert_eq!(
            bson::to_vec(&marker(&db).await).unwrap(),
            bson::to_vec(&saved).unwrap()
        );
        assert!(cutoff(&db).await.is_none());
        rows.delete_one(doc! {"_id":MARKER}).await.unwrap();
    }
    db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
        .insert_one(doc! {"_id":"bad-intent","source":"nyxbot","created_at":"malformed"})
        .await
        .unwrap();
    assert!(initialize(&db).await.is_err());
    assert!(cutoff(&db).await.is_none());
    db.drop().await.unwrap();
}

#[test]
fn prospective_runtime_policy_covers_only_the_reviewed_operations() {
    assert!(
        legacy_runtime_enrollment_policy()
            == LegacyRuntimeEnrollmentPolicy::AllowProspectiveBilling
    );
    for route in OPERATIONS_THAT_CAN_CAUSE_RUNTIME_ENROLLMENT {
        let route = route
            .replace("{id}", "example")
            .replace("{response_id}", "response");
        assert!(can_enroll("POST", &route));
        assert!(!can_enroll("DELETE", &route));
    }
    for route in [
        "/api/v1/sandboxes/example/pause",
        "/api/v1/agents/example/stop",
        "/api/v1/agents/example/files",
        "/api/v2/agents/example/responses/response/controls",
    ] {
        assert!(!can_enroll("POST", route));
    }
}
