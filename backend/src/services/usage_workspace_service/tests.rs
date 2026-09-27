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
    assert!(legacy.draft.panels[0].wide);
    assert!(validate(&legacy).is_ok());
    for span in [1, 2, 3] {
        let mut value = config();
        let panel = &mut value.draft.panels[0];
        panel.span = Some(span);
        panel.height = Some("tall".into());
        panel.interval = Some("week".into());
        panel.measure = "prompt_tokens".into();
        assert!(validate(&value).is_ok());
        let restored: WorkspaceConfig =
            serde_json::from_value(serde_json::to_value(&value).unwrap()).unwrap();
        assert_eq!(restored.draft.panels[0].span, Some(span));
        assert_eq!(restored.draft.panels[0].interval.as_deref(), Some("week"));
    }
    for field in ["span", "height", "interval"] {
        let mut value = config();
        let panel = &mut value.draft.panels[0];
        match field {
            "span" => panel.span = Some(4),
            "height" => panel.height = Some("unbounded".into()),
            _ => panel.interval = Some("minute".into()),
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
async fn workspace_handlers_enforce_admin_writes_and_operator_reads() {
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
    let state = test_app_state(db.clone());
    for role in ["admin", "operator", "user"] {
        let uid = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&uid, UserType::Person);
        user.is_admin = role == "admin";
        user.is_operator = role == "operator";
        if role == "admin" {
            user.role_ids.push(roles.admin.clone());
        }
        if role == "operator" {
            user.role_ids.push(roles.operator.clone());
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
        assert_eq!(read.is_ok(), role != "user");
        let write = crate::handlers::admin_usage::save_workspace(
            axum::extract::State(state.clone()),
            test_auth_user(&uid),
            axum::Json(SaveWorkspaceRequest {
                revision: 0,
                config: config(),
            }),
        )
        .await;
        assert_eq!(write.is_ok(), role == "admin");
        let analytics = crate::handlers::admin_usage::get_analytics(
            axum::extract::State(state.clone()),
            test_auth_user(&uid),
            axum::extract::Query(AnalyticsQuery::default()),
        )
        .await;
        assert_eq!(analytics.is_ok(), role != "user");
    }
    db.drop().await.unwrap();
}
