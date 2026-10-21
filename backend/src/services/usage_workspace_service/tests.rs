use super::*;
fn config() -> WorkspaceConfig {
    serde_json::from_value(serde_json::json!({
        "version": 1,
        "draft": { "id": uuid::Uuid::new_v4().to_string(), "name": "Overview", "layout": "overview",
            "filters": { "period": "7d", "from": null, "to": null, "services": [], "actors": [], "owners": [] },
            "panels": [{ "id": uuid::Uuid::new_v4().to_string(), "title": "Cost", "chart": "combo", "measure": "cost", "metric": "tokens", "breakdown": "service", "top": 5, "wide": true }] },
        "saved_views": []
    })).unwrap()
}
#[test]
fn workspace_rejects_invalid_queries_and_duplicate_panels() {
    assert!(validate(&config()).is_ok());
    let mut invalid = config();
    invalid.draft.filters.period = "custom".into();
    assert!(validate(&invalid).is_err());
    let mut invalid = config();
    invalid.draft.panels[0].measure = "$where".into();
    assert!(validate(&invalid).is_err());
    let mut invalid = config();
    invalid.draft.panels = vec![invalid.draft.panels[0].clone(); 13];
    assert!(validate(&invalid).is_err());
    let mut invalid = config();
    invalid.draft.filters.services = vec!["one,two".into()];
    assert!(validate(&invalid).is_err());
}

#[test]
fn workspace_preserves_legacy_width_and_validates_panel_layout_and_interval() {
    let legacy = config();
    assert!(legacy.draft.panels[0].span.is_none());
    assert!(legacy.draft.panels[0].table_display.is_none());
    assert!(legacy.draft.panels[0].wide);
    assert!(validate(&legacy).is_ok());
    for span in [1, 2, 3] {
        let mut value = config();
        let panel = &mut value.draft.panels[0];
        panel.span = Some(span);
        panel.height = Some("tall".into());
        panel.interval = Some("week".into());
        panel.table_display = Some("accordion".into());
        panel.measure = "prompt_tokens".into();
        assert!(validate(&value).is_ok());
        let restored: WorkspaceConfig =
            serde_json::from_value(serde_json::to_value(&value).unwrap()).unwrap();
        assert_eq!(restored.draft.panels[0].span, Some(span));
        assert_eq!(restored.draft.panels[0].interval.as_deref(), Some("week"));
        assert_eq!(
            restored.draft.panels[0].table_display.as_deref(),
            Some("accordion")
        );
    }
    for field in ["span", "height", "interval", "table_display"] {
        let mut value = config();
        let panel = &mut value.draft.panels[0];
        match field {
            "span" => panel.span = Some(4),
            "height" => panel.height = Some("unbounded".into()),
            "interval" => panel.interval = Some("minute".into()),
            _ => panel.table_display = Some("missing".into()),
        }
        assert!(validate(&value).is_err());
    }
}
#[test]
fn workspace_accepts_large_boards_without_a_panel_count_limit() {
    let mut board = config();
    let panel = board.draft.panels[0].clone();
    board.draft.panels = (0..1000)
        .map(|_| WorkspacePanel {
            id: uuid::Uuid::new_v4().to_string(),
            ..panel.clone()
        })
        .collect();
    assert!(validate(&board).is_ok());
    board.saved_views = (0..10)
        .map(|_| WorkspaceView {
            id: uuid::Uuid::new_v4().to_string(),
            ..board.draft.clone()
        })
        .collect();
    assert!(
        matches!(validate(&board), Err(AppError::ValidationError(message)) if message.contains("1 MiB"))
    );
}
#[tokio::test]
async fn workspace_persistence_is_private_and_rejects_stale_writes() {
    let db = crate::test_utils::connect_test_database("usage_workspace")
        .await
        .unwrap();
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    assert!(get(&db, &alice).await.unwrap().config.is_none());
    let mut large = config();
    let panel = large.draft.panels[0].clone();
    large.draft.panels = (0..40)
        .map(|_| WorkspacePanel {
            id: uuid::Uuid::new_v4().to_string(),
            ..panel.clone()
        })
        .collect();
    let saved = save(
        &db,
        &alice,
        SaveWorkspaceRequest {
            revision: 0,
            config: large,
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        get(&db, &alice)
            .await
            .unwrap()
            .config
            .unwrap()
            .draft
            .panels
            .len(),
        40
    );
    assert!(get(&db, &bob).await.unwrap().config.is_none());
    assert!(matches!(
        save(
            &db,
            &alice,
            SaveWorkspaceRequest {
                revision: 0,
                config: config()
            }
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let (first, second) = tokio::join!(
        save(
            &db,
            &alice,
            SaveWorkspaceRequest {
                revision: 1,
                config: config()
            }
        ),
        save(
            &db,
            &alice,
            SaveWorkspaceRequest {
                revision: 1,
                config: config()
            }
        ),
    );
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(get(&db, &alice).await.unwrap().revision, 2);
    let row = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! { "_id": &alice })
        .await
        .unwrap()
        .unwrap();
    assert!(row.get_datetime("updated_at").is_ok());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn workspace_handlers_enforce_admin_writes_and_usage_reader_reads() {
    use crate::models::user::UserType;
    use crate::test_utils::{test_app_state, test_auth_user, test_user};
    let db = crate::test_utils::connect_test_database("usage_workspace_auth")
        .await
        .unwrap();
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&db)
        .await
        .unwrap();
    crate::services::billing::usage_rollup::ensure_indexes(&db)
        .await
        .unwrap();
    let usage_viewer = db
        .collection::<crate::models::role::Role>(crate::models::role::COLLECTION_NAME)
        .find_one(doc! { "slug": crate::models::role::USAGE_VIEWER_ROLE_SLUG })
        .await
        .unwrap()
        .expect("usage viewer seeded");
    let group_id = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::group::Group>(crate::models::group::COLLECTION_NAME)
        .insert_one(crate::models::group::Group {
            id: group_id.clone(),
            name: "Usage readers".to_string(),
            slug: "usage-readers".to_string(),
            description: None,
            role_ids: vec![usage_viewer.id.clone()],
            parent_group_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    // An existing person with no usage: usage-only readers must not learn
    // who it is by filtering the List report on its id.
    let idle_id = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(test_user(&idle_id, UserType::Person))
        .await
        .unwrap();
    for role in [
        "admin",
        "operator",
        "usage_viewer",
        "group_usage_viewer",
        "user",
    ] {
        let can_read = role != "user";
        let uid = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&uid, UserType::Person);
        user.is_admin = role == "admin";
        user.is_operator = role == "operator";
        match role {
            "admin" => user.role_ids.push(roles.admin.clone()),
            "operator" => user.role_ids.push(roles.operator.clone()),
            "usage_viewer" => user.role_ids.push(usage_viewer.id.clone()),
            "group_usage_viewer" => user.group_ids.push(group_id.clone()),
            _ => {}
        }
        db.collection::<crate::models::user::User>("users")
            .insert_one(user)
            .await
            .unwrap();
        let read = crate::handlers::admin_usage::get_workspace(
            axum::extract::State(state.clone()),
            test_auth_user(&uid),
        )
        .await;
        assert_eq!(read.is_ok(), can_read, "{role} workspace read");
        let write = crate::handlers::admin_usage::save_workspace(
            axum::extract::State(state.clone()),
            test_auth_user(&uid),
            axum::Json(SaveWorkspaceRequest {
                revision: 0,
                config: config(),
            }),
        )
        .await;
        assert_eq!(write.is_ok(), role == "admin", "{role} workspace write");
        let analytics = crate::handlers::admin_usage::get_analytics(
            axum::extract::State(state.clone()),
            test_auth_user(&uid),
            axum::extract::Query(AnalyticsQuery::default()),
        )
        .await;
        assert_eq!(analytics.is_ok(), can_read, "{role} analytics");
        let identities = crate::handlers::admin_usage::get_identities(
            axum::extract::State(state.clone()),
            test_auth_user(&uid),
            axum::extract::Query(
                crate::services::admin_usage_service::identities::IdentityQuery {
                    kind: Some("owners".to_string()),
                    search: None,
                    ids: None,
                },
            ),
        )
        .await;
        assert_eq!(identities.is_ok(), can_read, "{role} identities");
        for (target, label) in [(&idle_id, "idle"), (&uid, "self")] {
            let report = crate::handlers::admin_usage::get_usage(
                axum::extract::State(state.clone()),
                test_auth_user(&uid),
                crate::telemetry::TelemetryContext::default(),
                axum::extract::Query(crate::services::admin_usage_service::AdminUsageQuery {
                    user: Some(target.clone()),
                    ..Default::default()
                }),
            )
            .await;
            if !can_read {
                assert!(report.is_err(), "{role} list");
                continue;
            }
            let selected = report.unwrap().0.selected_user.expect("selected user");
            let disclosed = selected.email.is_some();
            let expect_disclosed = label == "self" || role == "admin" || role == "operator";
            assert_eq!(disclosed, expect_disclosed, "{role} {label} selected_user");
            if !expect_disclosed {
                assert_eq!(selected.display_name, "Unknown user");
                assert_eq!(selected.user_type, "unknown");
            }
        }
        if role == "usage_viewer" || role == "admin" {
            let labels = crate::handlers::admin_usage::get_identities(
                axum::extract::State(state.clone()),
                test_auth_user(&uid),
                axum::extract::Query(
                    crate::services::admin_usage_service::identities::IdentityQuery {
                        kind: None,
                        search: None,
                        ids: Some(uid.clone()),
                    },
                ),
            )
            .await
            .expect("identity labels");
            // This caller has no usage: admins label any identity, usage-only
            // readers only identities present in usage data.
            assert_eq!(
                labels.0.options.len(),
                usize::from(role == "admin"),
                "{role} labels"
            );
        }
    }
    db.drop().await.unwrap();
}

#[test]
fn nyxbot_internal_model_breakdown_does_not_expand_saved_workspace_contract() {
    let mut value = config();
    value.draft.panels[0].breakdown = "model".into();
    assert!(validate(&value).is_err());
    let mut json = serde_json::to_value(config()).unwrap();
    json["draft"]["filters"]["model"] = serde_json::json!("alias");
    assert!(serde_json::from_value::<WorkspaceConfig>(json).is_err());
}
