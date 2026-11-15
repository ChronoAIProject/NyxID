//! Query-plan and scale checks for the actual export selectors.
use super::*;

fn uses_index(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.get("stage").and_then(|value| value.as_str()) == Some("IXSCAN")
            || map.values().any(uses_index),
        serde_json::Value::Array(values) => values.iter().any(uses_index),
        _ => false,
    }
}

async fn explain(db: &mongodb::Database, collection: &str, filter: Document) {
    let result = db.run_command(doc! {"explain": {"find": collection, "filter": filter}, "verbosity": "executionStats"}).await.unwrap();
    let plan = serde_json::to_value(result.get_document("queryPlanner").unwrap().get_document("winningPlan").unwrap()).unwrap();
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
        let personal_data_registry::ExportSelector::JoinedPerson {parent, field, owner_field} = entry.export_selector else {continue;};
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
        explain(&db,parent,owner.clone()).await;
        let (ids,truncated) = resolve_ids(&db,parent,owner).await.unwrap();
        assert!(!truncated);
        assert!(ids.contains(&Bson::String(parent_id)));
        let filter = doc! {field:{"$in":ids}};
        explain(&db,entry.collection,filter.clone()).await;
        assert!(source_pipeline(entry.collection,filter.clone()).iter().all(|stage| !stage.contains_key("$lookup")));
        let mut budget = ExportBudget::new(MAX_BYTES);
        let value = rows(&db,entry.collection,filter,fields_for(entry.collection).unwrap(),doc! {"created_at":-1},&mut budget).await.unwrap();
        assert_eq!(value.total_count,1, "{}", entry.collection);
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn parent_scope_resolution_caps_ids_and_marks_the_incomplete_scope() {
    let db = crate::test_utils::connect_transaction_test_database("export_parent_cap").await;
    db.collection::<Document>("mcp_sessions").create_index(mongodb::IndexModel::builder().keys(doc! {"user_id":1}).build()).await.unwrap();
    for start in (0..PARENT_ID_CAP+1).step_by(1000) {
        db.collection::<Document>("mcp_sessions").insert_many((start..(start+1000).min(PARENT_ID_CAP+1)).map(|index|doc! {"_id":format!("parent-{index:05}"),"user_id":"person"})).await.unwrap();
    }
    let (ids,truncated)=resolve_ids(&db,"mcp_sessions",doc! {"user_id":"person"}).await.unwrap();
    assert!(truncated);
    assert_eq!(ids.len(),PARENT_ID_CAP);
    db.drop().await.unwrap();
}
