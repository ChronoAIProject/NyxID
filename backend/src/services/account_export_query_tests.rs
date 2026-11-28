//! Query-plan and scale checks for the actual export selectors.
use super::*;

fn uses_index(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.get("stage").and_then(|value| value.as_str()) == Some("IXSCAN")
                || map.values().any(uses_index)
        }
        serde_json::Value::Array(values) => values.iter().any(uses_index),
        _ => false,
    }
}

async fn explain(db: &mongodb::Database, collection: &str, filter: Document) {
    let result = db
        .run_command(
            doc! {"explain": {"find": collection, "filter": filter, "sort": if ["mcp_sessions","platform_bot_installations","platform_setups"].contains(&collection) {parent_scope_sort(collection)} else if ["assistant_attachments","assistant_checkpoints","assistant_group_requests","org_channel_messages"].contains(&collection) {doc! {"created_at":-1,"_id":-1}} else {doc! {}}, "limit":PARENT_ID_CAP as i64 + 1}, "verbosity": "executionStats"},
        )
        .await
        .unwrap();
    let plan = serde_json::to_value(
        result
            .get_document("queryPlanner")
            .unwrap()
            .get_document("winningPlan")
            .unwrap(),
    )
    .unwrap();
    assert!(uses_index(&plan), "{collection}: {plan}");
    eprintln!("export plan {collection}: IXSCAN");
}

#[tokio::test]
async fn joined_views_resolve_indexed_parent_and_child_scopes_without_lookups() {
    let db = crate::test_utils::connect_transaction_test_database("export_join_plans").await;
    crate::db::ensure_indexes(&db).await.unwrap();
    super::super::platform_provisioning_service::ensure_account_export_indexes(&db)
        .await
        .unwrap();
    for entry in personal_data_registry::entries() {
        let personal_data_registry::ExportSelector::JoinedPerson {
            parent,
            field,
            owner_field,
        } = entry.export_selector
        else {
            continue;
        };
        let parent_id = format!("{}-parent", entry.collection);
        for index in 0..200 {
            let id = if index == 0 {
                parent_id.clone()
            } else {
                format!("{}-other-{index}", entry.collection)
            };
            db.collection::<Document>(parent).insert_one(doc! {"_id": &id, owner_field: if index == 0 {"person"} else {"other"}, "channel_bot_id":&id,"setup_id":&id,"idempotency_scope": &id, "idempotency_key": &id, "state": "cancelled"}).await.unwrap();
            db.collection::<Document>(entry.collection).insert_one(doc! {"_id": format!("{}-{index}",entry.collection), field: &id, "sequence": index, "revision": index, "kind": &id, "created_at": bson::DateTime::now()}).await.unwrap();
        }
        let owner = doc! {owner_field:"person"};
        explain(&db, parent, owner.clone()).await;
        let (ids, truncated) = resolve_ids(&db, parent, owner).await.unwrap();
        assert!(!truncated);
        assert!(ids.contains(&Bson::String(parent_id)));
        let filter = doc! {field:{"$in":ids}};
        explain(&db, entry.collection, filter.clone()).await;
        assert!(
            source_pipeline(entry.collection, filter.clone())
                .iter()
                .all(|stage| !stage.contains_key("$lookup"))
        );
        let mut budget = ExportBudget::new(MAX_BYTES);
        let value = rows(
            &db,
            entry.collection,
            filter,
            fields_for(entry.collection).unwrap(),
            doc! {"created_at":-1},
            &mut budget,
        )
        .await
        .unwrap();
        assert_eq!(value.total_count, 1, "{}", entry.collection);
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mcp_export_parent_scope_survives_a_session_migration() {
    use crate::models::mcp_notification::COLLECTION_NAME;
    use crate::models::mcp_session::{MCP_SESSION_COLLECTION, McpSessionStore};
    use crate::services::mcp_session_storage::{READINESS, migrate_batch};
    let db = crate::test_utils::connect_transaction_test_database("export_mcp_migration").await;
    let store = McpSessionStore::with_db(db.clone());
    let wire = store.create("person").await.unwrap().unwrap();
    store
        .send_notification(&wire, serde_json::json!({"private": true}))
        .await
        .unwrap();
    let (ids, truncated) =
        joined_parent_ids(&db, MCP_SESSION_COLLECTION, doc! {"user_id":"person"})
            .await
            .unwrap();
    assert!(!truncated);
    db.collection::<Document>(crate::models::security_capability_activation::COLLECTION_NAME).insert_one(doc!{
        "_id":uuid::Uuid::new_v4().to_string(), "record_kind":crate::models::security_capability_activation::RECORD_KIND,
        "capability": READINESS, "active_since":bson::DateTime::now()
    }).await.unwrap();
    assert_eq!(migrate_batch(&db, &mut None).await.unwrap(), 1);
    let view = rows(
        &db,
        COLLECTION_NAME,
        doc! {"session_id":{"$in":ids}},
        fields_for(COLLECTION_NAME).unwrap(),
        doc! {"created_at":-1},
        &mut ExportBudget::new(MAX_BYTES),
    )
    .await
    .unwrap();
    assert_eq!(view.total_count, 1);
    let exported = serde_json::to_string(&view).unwrap();
    assert!(!exported.contains(&wire));
    assert!(!exported.contains(&crate::services::mcp_session_storage::hash(&wire)));
    assert!(!exported.contains("private"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn parent_scope_resolution_caps_ids_and_marks_the_incomplete_scope() {
    let db = crate::test_utils::connect_transaction_test_database("export_parent_cap").await;
    db.collection::<Document>("mcp_sessions")
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"user_id":1})
                .build(),
        )
        .await
        .unwrap();
    for start in (0..PARENT_ID_CAP + 1).step_by(1000) {
        db.collection::<Document>("mcp_sessions")
            .insert_many(
                (start..(start + 1000).min(PARENT_ID_CAP + 1))
                    .map(|index| doc! {"_id":format!("parent-{index:05}"),"user_id":"person"}),
            )
            .await
            .unwrap();
    }
    let (ids, truncated) = resolve_ids(&db, "mcp_sessions", doc! {"user_id":"person"})
        .await
        .unwrap();
    assert!(truncated);
    assert_eq!(ids.len(), PARENT_ID_CAP);
    db.collection::<crate::models::user::User>("users")
        .insert_one(crate::test_utils::test_user(
            "person",
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let export = build(&db, "person").await.unwrap();
    assert!(export.sections["account_activity"]["mcp_session_notifications"].scope_truncated);
    assert!(export.sections["account_activity"]["mcp_session_notifications"].truncated);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn fifty_thousand_messages_use_batched_indexed_reads_within_timeout() {
    let db = crate::test_utils::connect_transaction_test_database("export_message_scale").await;
    db.collection::<Document>("assistant_conversations")
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"user_id":1,"updated_at":-1,"_id":-1})
                .build(),
        )
        .await
        .unwrap();
    super::super::assistant_nyxagent::ensure_account_export_indexes(&db)
        .await
        .unwrap();
    db.collection::<Document>("assistant_conversations")
        .insert_many(
            (0..100).map(|i| doc! {"_id":format!("conversation-{i:03}"),"user_id":"person"}),
        )
        .await
        .unwrap();
    for base in (0..50_000).step_by(1000) {
        db.collection::<Document>("assistant_messages").insert_many((base..base+1000).map(|i|doc! {"_id":format!("message-{i}"),"user_id":"person","conversation_id":format!("conversation-{:03}",i/500),"seq":i%500,"created_at":bson::DateTime::from_millis(i64::from(i)),"role":"assistant","origin":"user","text":"x".repeat(4096)})).await.unwrap();
    }
    let started = std::time::Instant::now();
    let (filter, truncated) = message_scope(&db, "person").await.unwrap();
    assert!(!truncated);
    let plan = db.run_command(doc! {"explain":{"aggregate":"assistant_messages","pipeline":[{"$match":&filter},{"$sort":{"created_at":-1,"_id":-1}},{"$limit":10001}],"cursor":{}},"verbosity":"executionStats"}).await.unwrap();
    let winning = plan
        .get_document("queryPlanner")
        .unwrap()
        .get_document("winningPlan")
        .unwrap();
    let plan_json = serde_json::to_value(winning).unwrap();
    assert!(uses_index(&plan_json), "{plan_json}");
    assert!(
        !plan_json.to_string().contains("\"SORT\""),
        "blocking sort: {plan_json}"
    );
    let mut budget = ExportBudget::new(MAX_BYTES);
    let result = query_rows(
        &db,
        "assistant_messages",
        filter,
        fields_for("assistant_messages").unwrap(),
        doc! {"created_at":-1,"_id":-1},
        &mut budget,
    )
    .await
    .unwrap();
    let elapsed = started.elapsed();
    eprintln!(
        "50,000-message export (4 KiB text/row): {:?}, {} rows, bounded lower count {}, IXSCAN",
        elapsed,
        result.items.len(),
        result.total_count
    );
    assert_eq!(result.total_count, SUBSECTION_ROW_CAP as u64 + 1);
    assert!(!result.total_count_exact);
    assert!(result.truncated);
    assert!(!result.unavailable);
    assert!(!result.items.is_empty() && result.items.len() <= SUBSECTION_ROW_CAP as usize);
    assert!(elapsed < QUERY_TIMEOUT, "{elapsed:?}");
    db.drop().await.unwrap();
}

#[tokio::test]
async fn unbounded_export_selectors_use_selective_prefix_indexes() {
    let db = crate::test_utils::connect_transaction_test_database("export_actor_plans").await;
    super::super::assistant_upload_retention::ensure_account_export_indexes(&db)
        .await
        .unwrap();
    super::super::collaboration_group_service::ensure_account_export_indexes(&db)
        .await
        .unwrap();
    super::super::org_channel_service::ensure_account_export_indexes(&db)
        .await
        .unwrap();
    super::super::assistant_nyxagent::ensure_account_export_indexes(&db)
        .await
        .unwrap();
    for &(collection, field) in personal_data_registry::EXPORT_SELECTOR_INDEX_GAPS {
        let entry = personal_data_registry::find(collection).unwrap();
        let filter = owner_filter(entry, "person").unwrap();
        assert_eq!(filter, doc! {field:"person"});
        let rows = db.collection::<Document>(collection);
        rows.insert_many((0..5_010).map(|i| {
            let mut row = doc! {
                "_id":format!("{collection}-{i}"), field:if i < 10 {"person"} else {"other"},
                "created_at":bson::DateTime::now()
            };
            if field != "user_id" {
                row.insert("user_id", "resource-owner");
            }
            row
        }))
        .await
        .unwrap();
        explain(&db, collection, filter.clone()).await;
        assert_eq!(rows.count_documents(filter).await.unwrap(), 10);
        // Reuse an existing selective prefix regardless of its name; no duplicate
        // indexes, drops or rebuilds on repeated feature initialization.
        let before = rows
            .list_indexes()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        super::super::assistant_upload_retention::ensure_account_export_indexes(&db)
            .await
            .unwrap();
        super::super::collaboration_group_service::ensure_account_export_indexes(&db)
            .await
            .unwrap();
        super::super::org_channel_service::ensure_account_export_indexes(&db)
            .await
            .unwrap();
        super::super::assistant_nyxagent::ensure_account_export_indexes(&db)
            .await
            .unwrap();
        let after = rows
            .list_indexes()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(
            bson::to_bson(&before).unwrap(),
            bson::to_bson(&after).unwrap()
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_deliveries_do_not_hide_private_conversations_or_unrouted_replies() {
    let db =
        crate::test_utils::connect_transaction_test_database("export_pending_deliveries").await;
    db.collection::<Document>("assistant_conversations").insert_one(doc! {
        "_id":"private", "user_id":"person", "title":"Private title", "deliver_also":[{"channel":"pending"}]
    }).await.unwrap();
    db.collection::<Document>("assistant_messages").insert_many([
        doc! {"_id":"private-reply", "user_id":"person", "conversation_id":"private", "role":"assistant", "origin":"user", "text":"private reply"},
        doc! {"_id":"routed-reply", "user_id":"person", "conversation_id":"private", "role":"assistant", "origin":"user", "platform_reply":{"destination":"channel"}, "text":"routed reply"},
        doc! {"_id":"platform-origin", "user_id":"person", "conversation_id":"private", "role":"assistant", "origin":"user", "platform_origin":{"platform":"telegram"}, "text":"platform reply"},
    ]).await.unwrap();
    let mut budget = ExportBudget::new(MAX_BYTES);
    let conversations = query_rows(
        &db,
        "assistant_conversations",
        doc! {"user_id":"person"},
        fields_for("assistant_conversations").unwrap(),
        doc! {"updated_at":-1},
        &mut budget,
    )
    .await
    .unwrap();
    assert_eq!(conversations.items.len(), 1);
    assert_eq!(conversations.items[0]["title"], "Private title");
    let (scope, _) = message_scope(&db, "person").await.unwrap();
    let messages = query_rows(
        &db,
        "assistant_messages",
        scope,
        fields_for("assistant_messages").unwrap(),
        doc! {"created_at":-1},
        &mut budget,
    )
    .await
    .unwrap();
    assert_eq!(messages.total_count, 1);
    assert!(messages.total_count_exact);
    assert_eq!(messages.items[0]["text"], "private reply");
    db.drop().await.unwrap();
}
