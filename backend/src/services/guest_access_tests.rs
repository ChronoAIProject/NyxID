use super::{
    agent_operation_scope_service as scopes, assistant_team_service as team,
    feature_flag_service as flags,
};
use crate::{
    models::{
        agent_operation_scope::OperationSelection,
        assistant_agent::{AgentGrants, AssistantAgent, GuestAccess},
    },
    services::assistant_authority_tests::{Fixture, connected, fixture},
};
use mongodb::bson::{self, doc};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

async fn rollout(f: &Fixture, enabled: bool) {
    flags::set_platform_override(
        &f.state.db,
        flags::GUEST_OPERATION_SCOPES_FLAG_KEY,
        &flags::FlagTarget::Global,
        enabled,
        &f.owner,
    )
    .await
    .unwrap();
}
fn selection(revision: i64, ids: Vec<String>, all: bool) -> OperationSelection {
    serde_json::from_value(
        json!({"expected_revision":revision,"endpoint_ids":ids,"all_operations":all}),
    )
    .unwrap()
}
async fn save(
    f: &Fixture,
    service: &str,
    input: OperationSelection,
) -> crate::errors::AppResult<AssistantAgent> {
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::GuestOperations {
            base: Box::new(team::GrantChange::Guests(BTreeMap::new())),
            selections: BTreeMap::from([(service.into(), input)]),
        },
    )
    .await
}
async fn raw(
    f: &Fixture,
    service: &str,
    method: &str,
    path: &str,
    guest: bool,
    ws: bool,
) -> crate::errors::AppResult<bool> {
    let mut auth = f.auth.clone();
    let mut chat = f.chat.clone();
    chat.guest = guest;
    auth.assistant_chat = Some(Arc::new(chat));
    scopes::check_non_mcp_context(
        &f.state.db,
        &auth,
        false,
        service,
        None,
        method,
        Some(&super::proxy_authorization::CanonicalPath::from_mcp_literal(path).unwrap()),
        scopes::GuestHttpContext {
            websocket: ws,
            query: None,
        },
    )
    .await
}

#[tokio::test]
async fn guest_scopes_gate_revision_owner_isolation_and_grant_lifecycle() {
    let f = Box::pin(fixture("guest_scope_lifecycle")).await;
    let id = connected(&f.state.db, &f.owner, "guest-scope", "http://127.0.0.1:9").await;
    let grants = AgentGrants {
        service_ids: vec![id.clone()],
        ..Default::default()
    };
    let original = team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(grants.clone()),
    )
    .await
    .unwrap();
    assert!(original.guest_operation_scopes.is_empty());
    assert!(raw(&f, &id, "GET", "/any", true, false).await.unwrap());
    let endpoint = crate::test_utils::test_service_endpoint(&id, "safe", "GET", "/allowed/{id}");
    f.state
        .db
        .collection::<crate::models::service_endpoint::ServiceEndpoint>(
            crate::models::service_endpoint::COLLECTION_NAME,
        )
        .insert_one(&endpoint)
        .await
        .unwrap();
    assert!(save(&f, &id, selection(0, vec![], false)).await.is_err());
    rollout(&f, true).await;
    assert!(
        save(
            &f,
            &id,
            selection(0, vec!["foreign-endpoint".into()], false)
        )
        .await
        .is_err()
    );
    let saved = save(&f, &id, selection(0, vec![endpoint.id.clone()], false))
        .await
        .unwrap();
    assert!(saved.operation_scopes.is_empty());
    assert_eq!(saved.grants, original.grants);
    assert_eq!(saved.guest_operation_scope_revisions[&id], 1);
    assert!(save(&f, &id, selection(0, vec![], false)).await.is_err());
    assert!(
        raw(&f, &id, "GET", "/allowed/one", true, false)
            .await
            .is_ok()
    );
    for (method, path, ws) in [
        ("GET", "/other", false),
        ("POST", "/allowed/one", false),
        ("GET", "/allowed/one", true),
    ] {
        assert!(raw(&f, &id, method, path, true, ws).await.is_err());
        assert!(raw(&f, &id, method, path, false, ws).await.is_ok());
    }
    // Revocation reaches an existing thread without rotating or narrowing its owner key.
    let denied = save(&f, &id, selection(1, vec![], false)).await.unwrap();
    assert!(
        raw(&f, &id, "GET", "/allowed/one", true, false)
            .await
            .is_err()
    );
    let key = super::key_service::get_api_key(&f.state.db, &f.owner, &f.row.credential_api_key_id)
        .await
        .unwrap();
    assert!(key.allowed_service_ids.contains(&id));
    assert!(key.assistant_operation_scopes.is_empty());
    rollout(&f, false).await;
    assert!(save(&f, &id, selection(2, vec![], true)).await.is_err());
    assert!(
        raw(&f, &id, "GET", "/allowed/one", true, false)
            .await
            .is_err()
    );
    // The previous model's exact enum remains readable; unknown sibling fields
    // are ignored by serde (the old AssistantAgent has no deny_unknown_fields).
    #[derive(serde::Deserialize)]
    struct LegacyAgent {
        guest_access: BTreeMap<String, GuestAccess>,
        grants: AgentGrants,
    }
    let encoded = bson::to_document(&denied).unwrap();
    let legacy: LegacyAgent = bson::from_document(encoded.clone()).unwrap();
    assert!(legacy.guest_access.is_empty());
    assert_eq!(legacy.grants, grants);
    let mut old = encoded;
    old.remove("guest_operation_scopes");
    old.remove("guest_operation_scope_revisions");
    let legacy: AssistantAgent = bson::from_document(old).unwrap();
    assert!(legacy.guest_operation_scopes.is_empty());
    let kept = team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants::default()),
    )
    .await
    .unwrap();
    assert_eq!(kept.guest_operation_scopes, denied.guest_operation_scopes);
    let removed = team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Remove(grants.clone()),
    )
    .await
    .unwrap();
    assert!(removed.guest_operation_scopes.is_empty());
    let regranted = team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(grants),
    )
    .await
    .unwrap();
    assert!(regranted.guest_operation_scopes.is_empty());
    assert_eq!(regranted.guest_operation_scope_revisions[&id], 3);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_scopes_http_management_is_human_only_and_requires_owner() {
    use crate::handlers::assistant_team::set_guest_operations;
    use axum::{
        Json,
        extract::{Path, State},
    };
    let f = Box::pin(fixture("guest_scope_http_acl")).await;
    let id = connected(&f.state.db, &f.owner, "guest-scope", "http://127.0.0.1:9").await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    rollout(&f, true).await;
    for auth in [
        f.auth.clone(),
        crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string()),
    ] {
        assert!(
            set_guest_operations(
                State(f.state.clone()),
                auth,
                Path((f.chat.agent_id.clone(), id.clone())),
                Json(selection(0, vec![], false))
            )
            .await
            .is_err()
        );
    }
    let response = set_guest_operations(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        Path((f.chat.agent_id.clone(), id.clone())),
        Json(selection(0, vec![], false)),
    )
    .await
    .unwrap();
    assert_eq!(response.0["revision"], 1);
    assert!(raw(&f, &id, "GET", "/anything", true, false).await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_append_raw_contract_refuses_overwrite_and_remote_lookalikes() {
    let f = Box::pin(fixture("guest_append_raw")).await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "api-google-workspace".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let endpoint = crate::test_utils::test_service_endpoint(
        &catalog.id,
        "sheets_append_values",
        "POST",
        "/v4/spreadsheets/{spreadsheetId}/values/{range}:append",
    );
    f.state
        .db
        .collection::<crate::models::service_endpoint::ServiceEndpoint>(
            crate::models::service_endpoint::COLLECTION_NAME,
        )
        .insert_one(&endpoint)
        .await
        .unwrap();
    let path = super::proxy_authorization::CanonicalPath::from_mcp_literal(
        "/v4/spreadsheets/sheet/values/A1:append",
    )
    .unwrap();
    for (query, allowed) in [
        (None, false),
        (Some("insertDataOption=OVERWRITE"), false),
        (Some("insertDataOption=INSERT_ROWS"), true),
        (
            Some("insertDataOption=INSERT_ROWS&insertDataOption=OVERWRITE"),
            false,
        ),
    ] {
        let effects = scopes::unscoped_guest_effects(
            &f.state.db,
            &catalog.id,
            None,
            &reqwest::Method::POST,
            Some(&path),
            query,
        )
        .await
        .unwrap();
        assert_eq!(effects.uses, allowed);
        assert!(effects.destructive);
        assert!(!effects.reads);
    }
    // A matching name at another route cannot acquire the append attestation.
    f.state
        .db
        .collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&endpoint.id},
            doc! {"$set":{"path":"/untrusted"}},
        )
        .await
        .unwrap();
    let path = super::proxy_authorization::CanonicalPath::from_mcp_literal("/untrusted").unwrap();
    let effects = scopes::unscoped_guest_effects(
        &f.state.db,
        &catalog.id,
        None,
        &reqwest::Method::POST,
        Some(&path),
        Some("insertDataOption=INSERT_ROWS"),
    )
    .await
    .unwrap();
    assert!(!effects.uses);
    // A missing row must not turn a compiled append/overwrite route into an
    // unclassified POST (as with a generic mounted-spec fallback).
    f.state
        .db
        .collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .delete_one(doc! {"_id": &endpoint.id})
        .await
        .unwrap();
    let path = super::proxy_authorization::CanonicalPath::from_mcp_literal(
        "/v4/spreadsheets/sheet/values/A1:append",
    )
    .unwrap();
    for (query, allowed) in [
        ("insertDataOption=INSERT_ROWS", true),
        ("insertDataOption=OVERWRITE", false),
    ] {
        let effects = scopes::unscoped_guest_effects(
            &f.state.db,
            &catalog.id,
            None,
            &reqwest::Method::POST,
            Some(&path),
            Some(query),
        )
        .await
        .unwrap();
        assert_eq!(effects.uses, allowed);
        assert!(effects.destructive);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_scopes_cover_catalog_and_ungranted_inference_aliases() {
    let f = Box::pin(fixture("guest_scope_catalog_alias")).await;
    let catalog = crate::test_utils::test_auto_connected_catalog_service();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let id = connected(&f.state.db, &f.owner, "model", "http://127.0.0.1:9").await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &id},
            doc! {"$set": {"catalog_service_id": &catalog.id}},
        )
        .await
        .unwrap();
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    rollout(&f, true).await;
    let saved = save(&f, &id, selection(0, vec![], false)).await.unwrap();
    assert_eq!(
        saved.guest_operation_scopes[&id]
            .catalog_service_id
            .as_deref(),
        Some(catalog.id.as_str())
    );
    let path = super::proxy_authorization::CanonicalPath::from_mcp_literal("/v1/chat/completions")
        .unwrap();
    for guest in [true, false] {
        let mut auth = f.auth.clone();
        auth.allow_all_services = false;
        auth.allowed_service_ids = vec![id.clone()];
        let mut chat = f.chat.clone();
        chat.guest = guest;
        auth.assistant_chat = Some(Arc::new(chat));
        for instance in [Some(id.as_str()), None, Some("ungranted-instance")] {
            let (service, catalog_id) = scopes::execution_identity(&auth, instance, &catalog.id);
            let result = scopes::check_non_mcp_context(
                &f.state.db,
                &auth,
                false,
                service,
                catalog_id,
                "POST",
                Some(&path),
                Default::default(),
            )
            .await;
            assert_eq!(result.is_err(), guest);
        }
    }
    f.state.db.drop().await.unwrap();
}
