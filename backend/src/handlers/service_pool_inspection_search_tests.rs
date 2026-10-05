use super::*;

#[tokio::test]
async fn pool_inspection_name_search_precedes_paging_and_preserves_scope() {
    let mut fixture = fixture("pool_name_search", StatusCode::OK, "priority", false).await;
    let db = &fixture.state.db;
    let services: Vec<Document> = db
        .collection::<Document>("user_services")
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let first_id = services[0].get_str("_id").unwrap();
    let last = &services[1];
    let last_id = last.get_str("_id").unwrap();
    let original_catalog_id = last.get_str("catalog_service_id").unwrap().to_owned();
    let label = "Preferred member [West]";
    db.collection::<Document>("user_endpoints")
        .update_one(
            doc! {"_id":last.get_str("endpoint_id").unwrap()},
            doc! {"$set":{"label":label}},
        )
        .await
        .unwrap();
    let decrypts = fixture.state.encryption_keys.decrypt_stats();
    let page = inspect(&fixture, json!({"check_operation":false,"limit":1}), false).await;
    assert_eq!(page.candidates[0].user_service_id, first_id);
    for search in ["preferred MEMBER [west]", last.get_str("slug").unwrap()] {
        let found = inspect(
            &fixture,
            json!({"check_operation":false,"search":search,"limit":1}),
            false,
        )
        .await;
        assert_eq!(found.candidates.len(), 1);
        assert_eq!(found.candidates[0].user_service_id, last_id);
        assert_eq!(found.candidates[0].name, label);
        assert_ne!(found.candidates[0].group_name, "Custom connections");
        assert!(found.candidates[0].group_slug.is_some());
        assert!(!found.has_more);
    }
    // Only the later inventory row references this catalog. Matching its literal
    // original name or slug must happen before limit/skip, not on a loaded page.
    let catalog_id = Uuid::new_v4().to_string();
    let catalog_name = "Original [West].* (Pool)+";
    let catalog_slug = "original-west-pool";
    let mut catalog = db
        .collection::<Document>("downstream_services")
        .find_one(doc! {"_id":last.get_str("catalog_service_id").unwrap()})
        .await
        .unwrap()
        .unwrap();
    catalog.insert("_id", &catalog_id);
    catalog.insert("name", catalog_name);
    catalog.insert("slug", catalog_slug);
    db.collection::<Document>("downstream_services")
        .insert_one(catalog)
        .await
        .unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":last_id},
            doc! {"$set":{"catalog_service_id":&catalog_id}},
        )
        .await
        .unwrap();
    for search in ["original [west].* (pool)+", "ORIGINAL-WEST-POOL", ".*"] {
        let found = inspect(
            &fixture,
            json!({"check_operation":false,"search":search,"limit":1}),
            false,
        )
        .await;
        assert_eq!(found.candidates.len(), 1, "{search}");
        assert_eq!(found.candidates[0].user_service_id, last_id, "{search}");
        assert_eq!(found.candidates[0].group_name, catalog_name);
        assert_eq!(
            found.candidates[0].group_slug.as_deref(),
            Some(catalog_slug)
        );
        assert_eq!(
            found.candidates[0].catalog_service_id.as_deref(),
            Some(catalog_id.as_str())
        );
        assert!(!found.has_more);
        assert!(
            inspect(
                &fixture,
                json!({"check_operation":false,"search":search,"limit":1,"after":"1"}),
                false
            )
            .await
            .candidates
            .is_empty()
        );
    }
    assert!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":"[not-present]"}),
            false
        )
        .await
        .candidates
        .is_empty()
    );
    // Matching foreign-owned instances must never enter the candidate result.
    let mut foreign = last.clone();
    foreign.insert("_id", Uuid::new_v4().to_string());
    foreign.insert("user_id", Uuid::new_v4().to_string());
    foreign.insert("catalog_service_id", &catalog_id);
    db.collection::<Document>("user_services")
        .insert_one(foreign)
        .await
        .unwrap();
    assert_eq!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":label}),
            false
        )
        .await
        .candidates
        .len(),
        1
    );
    for search in [catalog_name, catalog_slug] {
        let found = inspect(
            &fixture,
            json!({"check_operation":false,"search":search,"limit":1}),
            false,
        )
        .await;
        assert_eq!(found.candidates.len(), 1);
        assert_eq!(found.candidates[0].user_service_id, last_id);
        assert!(
            !found.has_more,
            "foreign owned catalog match must not affect paging"
        );
    }
    fixture.auth.allow_all_services = false;
    fixture.auth.allowed_service_ids = vec![first_id.to_owned()];
    assert!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":label}),
            false
        )
        .await
        .candidates
        .is_empty()
    );
    for search in [catalog_name, catalog_slug] {
        assert!(
            inspect(
                &fixture,
                json!({"check_operation":false,"search":search,"limit":1}),
                false
            )
            .await
            .candidates
            .is_empty()
        );
    }
    let selected = inspect(&fixture, json!({"check_operation":false,"search":"no match","selected_only":true,"peer_ids":format!("{first_id},{last_id}"),"limit":1,"after":"999"}), false).await;
    assert_eq!(selected.candidates.len(), 1);
    assert_eq!(selected.candidates[0].user_service_id, first_id);
    fixture.auth.allow_all_services = true;
    // Platform resolution also displays the endpoint label, with no credential materialization.
    db.collection::<Document>("downstream_services").update_one(
        doc! {"_id":&original_catalog_id},
        doc! {"$set":{"auth_method":"bearer","service_category":"internal","requires_user_credential":false,"visibility":"public","credential_encrypted":mongodb::bson::Binary{subtype:mongodb::bson::spec::BinarySubtype::Generic,bytes:vec![1]}}},
    ).await.unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":last_id},
            doc! {"$set":{"catalog_service_id":&original_catalog_id,"credential_binding":"platform","auth_method":"bearer"}},
        )
        .await
        .unwrap();
    let platform = inspect(
        &fixture,
        json!({"check_operation":false,"search":label}),
        false,
    )
    .await;
    assert_eq!(platform.candidates.len(), 1);
    assert_eq!(platform.candidates[0].name, label);
    assert_eq!(platform.candidates[0].credential_binding, "platform");
    assert!(platform.candidates[0].eligible);
    // Matching-name pages retain stable pagination, without consuming unrelated rows.
    db.collection::<Document>("user_endpoints")
        .update_one(
            doc! {"_id":services[0].get_str("endpoint_id").unwrap()},
            doc! {"$set":{"label":"Another member [West]"}},
        )
        .await
        .unwrap();
    let page = inspect(
        &fixture,
        json!({"check_operation":false,"search":"member [West]","limit":1}),
        false,
    )
    .await;
    assert_eq!(page.candidates.len(), 1);
    assert!(page.has_more);
    let next = inspect(&fixture, json!({"check_operation":false,"search":"member [West]","limit":1,"after":page.next_cursor}), false).await;
    assert_eq!(next.candidates.len(), 1);
    assert!(!next.has_more);
    assert_ne!(
        page.candidates[0].user_service_id,
        next.candidates[0].user_service_id
    );
    assert_eq!(decrypts, fixture.state.encryption_keys.decrypt_stats());
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_inspection_renamed_key_labels_match_services_without_exposing_foreign_or_platform_keys()
 {
    let mut fixture = fixture("pool_renamed_key", StatusCode::OK, "priority", false).await;
    bind_first_credential(&fixture).await;
    let db = &fixture.state.db;
    let service = db
        .collection::<Document>("user_services")
        .find_one(doc! {"slug":"review-first"})
        .await
        .unwrap()
        .unwrap();
    let id = service.get_str("_id").unwrap();
    let key_id = service.get_str("api_key_id").unwrap();
    let owner = service.get_str("user_id").unwrap();
    let endpoint_id = service.get_str("endpoint_id").unwrap();
    db.collection::<Document>("user_endpoints")
        .update_one(
            doc! {"_id":endpoint_id},
            doc! {"$set":{"label":"Previous endpoint label"}},
        )
        .await
        .unwrap();
    let label = "Team [renamed].*";
    crate::services::user_api_key_service::update_api_key(
        db,
        &fixture.state.encryption_keys,
        owner,
        key_id,
        Some(label),
        None,
    )
    .await
    .unwrap();
    let view = crate::services::unified_key_service::get_key(
        db,
        &fixture.state.encryption_keys,
        owner,
        id,
    )
    .await
    .unwrap();
    assert_eq!(view.label, label);
    let decrypts = fixture.state.encryption_keys.decrypt_stats();
    let key_before = db
        .collection::<Document>("user_api_keys")
        .find_one(doc! {"_id":key_id})
        .await
        .unwrap()
        .unwrap();
    for status in ["active", "failed"] {
        db.collection::<Document>("user_api_keys")
            .update_one(doc! {"_id":key_id}, doc! {"$set":{"status":status}})
            .await
            .unwrap();
        for search in ["TEAM [renamed].*", "[renamed]", ".*"] {
            let found = inspect(
                &fixture,
                json!({"check_operation":false,"search":search,"limit":1}),
                false,
            )
            .await;
            assert_eq!(found.candidates.len(), 1);
            assert_eq!(found.candidates[0].user_service_id, id);
            assert_eq!(found.candidates[0].name, label);
            assert!(!found.has_more);
        }
        for (query, health) in [
            (
                json!({"check_operation":false,"selected_only":true,"peer_ids":id}),
                false,
            ),
            (json!({"check_operation":false}), true),
        ] {
            let found = inspect(&fixture, query, health).await;
            assert_eq!(
                found
                    .candidates
                    .iter()
                    .find(|r| r.user_service_id == id)
                    .unwrap()
                    .name,
                label
            );
        }
    }
    assert!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":"Previous endpoint label"}),
            false
        )
        .await
        .candidates
        .is_empty()
    );
    fixture.auth.allow_all_services = false;
    fixture.auth.allowed_service_ids = vec![fixture.second_member_id.clone()];
    assert!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":label}),
            false
        )
        .await
        .candidates
        .is_empty()
    );
    fixture.auth.allow_all_services = true;
    // Platform bindings use the endpoint label even if a personal key is retained.
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"credential_binding":"platform"}},
        )
        .await
        .unwrap();
    assert!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":label}),
            false
        )
        .await
        .candidates
        .is_empty()
    );
    let platform = inspect(
        &fixture,
        json!({"check_operation":false,"search":"Previous endpoint label"}),
        false,
    )
    .await;
    assert_eq!(platform.candidates[0].name, "Previous endpoint label");
    db.collection::<Document>("user_services")
        .update_one(doc! {"_id":id}, doc! {"$unset":{"credential_binding":""}})
        .await
        .unwrap();
    // An inconsistent foreign key reference cannot disclose its label in display or search.
    db.collection::<Document>("user_api_keys")
        .update_one(
            doc! {"_id":key_id},
            doc! {"$set":{"user_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert!(
        inspect(
            &fixture,
            json!({"check_operation":false,"search":label}),
            false
        )
        .await
        .candidates
        .is_empty()
    );
    let foreign = inspect(
        &fixture,
        json!({"check_operation":false,"search":"Previous endpoint label"}),
        false,
    )
    .await;
    assert_eq!(foreign.candidates[0].name, "Previous endpoint label");
    let key_after = db
        .collection::<Document>("user_api_keys")
        .find_one(doc! {"_id":key_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        key_before.get("last_used_at"),
        key_after.get("last_used_at")
    );
    assert_eq!(decrypts, fixture.state.encryption_keys.decrypt_stats());
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    db.drop().await.unwrap();
}
