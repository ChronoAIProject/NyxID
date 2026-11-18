use super::*;
use crate::services::telegram_new_service::{creation_suggestions, username_suggestions};

async fn regeneration_button(server: &MockServer) -> String {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .rev()
        .filter_map(|r| r.body_json::<Value>().ok())
        .find_map(|body| {
            body["reply_markup"]["inline_keyboard"]
                .as_array()?
                .iter()
                .flat_map(|row| row.as_array().into_iter().flatten())
                .find(|button| button["text"] == "Regenerate username")
                .and_then(|button| button["callback_data"].as_str())
                .map(str::to_owned)
        })
        .unwrap()
}

fn callback(data: &str, update_id: i64) -> Value {
    json!({"update_id":update_id,"callback_query":{"id":"callback", "from":{"id":700,"is_bot":false},"data":data,
        "message":{"message_id":50,"from":{"id":100,"is_bot":true},"chat":{"id":700,"type":"private"}}}})
}

#[tokio::test]
async fn telegram_new_username_regeneration_preserves_hosted_agent_and_creation_authority() {
    let (state, actor, server) = fixture().await;
    let (request, start, _) = hosted_request(&state, &actor, &server, false).await;
    let base = server.uri();
    let service = service(&state, &base);
    assert!(
        service
            .regenerate_username(&actor, &request.id)
            .await
            .is_err()
    );
    let headers = headers(&state).await;
    webhook(&service, &headers, command(&start, 10)).await;
    let before = service.get(&actor, &request.id).await.unwrap();
    let old_names = username_suggestions(&before);
    assert!(
        service
            .regenerate_username("wrong-actor", &request.id)
            .await
            .is_err()
    );
    let saved = service
        .regenerate_username(&actor, &request.id)
        .await
        .unwrap();
    assert_ne!(old_names, username_suggestions(&saved));
    let mut old = bson::to_document(&before).unwrap();
    let mut new = bson::to_document(&saved).unwrap();
    old.remove("username_seed");
    new.remove("username_seed");
    assert_eq!(old, new, "Only cosmetic suggestions may change");
    let (_, primary) = service.creation_link(&actor, &request.id).await.unwrap();
    let choices = creation_suggestions(&saved, &primary).unwrap();
    assert_eq!(choices.len(), 3);
    for (name, url) in &choices {
        let parsed = url::Url::parse(url).unwrap();
        assert_eq!(parsed.path(), format!("/newbot/NyxSetupBot/{name}"));
        assert_eq!(
            parsed.query_pairs().find(|(k, _)| k == "name").unwrap().1,
            saved.label
        );
    }
    webhook(
        &service,
        &headers,
        json!({"update_id":11,"managed_bot":{"user":{"id":700,"is_bot":false},"bot":bot()}}),
    )
    .await;
    let mut created = message(json!({"managed_bot_created":{"bot":bot()}}));
    created["update_id"] = json!(12);
    webhook(&service, &headers, created).await;
    provider_connection(&server, &request.id, 200).await;
    service.complete_pending_creations().await.unwrap();
    let completed = service.get(&actor, &request.id).await.unwrap();
    assert_eq!(completed.status, Status::Connected);
    assert_eq!(
        bson::to_bson(&completed.creation_context).unwrap(),
        bson::to_bson(&request.creation_context).unwrap()
    );
    assert!(
        service
            .regenerate_username(&actor, &request.id)
            .await
            .is_err()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_username_buttons_validate_sender_manager_provenance_and_stale_clicks() {
    let (state, actor, server) = fixture().await;
    let (request, start) = start_request(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    webhook(&service, &headers, command(&start, 10)).await;
    let data = regeneration_button(&server).await;
    assert!(data.len() <= 64);
    let original = service.get(&actor, &request.id).await.unwrap();
    let before = server.received_requests().await.unwrap().len();
    for change in ["sender", "manager", "forward", "quote", "group"] {
        let mut update = callback(&data, 11);
        match change {
            "sender" => {
                update["callback_query"]["from"]["id"] = json!(701);
                update["callback_query"]["message"]["chat"]["id"] = json!(701);
            }
            "manager" => update["callback_query"]["message"]["from"]["id"] = json!(101),
            "forward" => update["callback_query"]["message"]["forward_from"] = json!({"id":700}),
            "quote" => update["callback_query"]["message"]["quote"] = json!({"text":"reference"}),
            _ => update["callback_query"]["message"]["chat"]["type"] = json!("group"),
        }
        webhook(&service, &headers, update).await;
    }
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    assert_eq!(
        username_suggestions(&service.get(&actor, &request.id).await.unwrap()),
        username_suggestions(&original)
    );
    webhook(&service, &headers, callback(&data, 12)).await;
    let saved = service.get(&actor, &request.id).await.unwrap();
    assert_ne!(
        username_suggestions(&saved),
        username_suggestions(&original)
    );
    assert_eq!(saved.start_update_id, original.start_update_id);
    assert_eq!(saved.revision, original.revision);
    let sent = server.received_requests().await.unwrap();
    let instructions = sent[sent.len() - 2].body_json::<Value>().unwrap();
    let keyboard = sent.last().unwrap().body_json::<Value>().unwrap();
    assert_eq!(
        instructions["reply_markup"]["inline_keyboard"][0][0]["text"],
        "Open creation form"
    );
    assert!(
        instructions["text"]
            .as_str()
            .unwrap()
            .contains("have not been checked")
    );
    assert_eq!(
        keyboard["reply_markup"]["keyboard"][0][0]["request_managed_bot"]["suggested_username"],
        username_suggestions(&saved)[0]
    );
    webhook(&service, &headers, callback(&data, 13)).await;
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        sent.len() + 1,
        "Stale click only acknowledges"
    );
    assert_eq!(
        service
            .get(&actor, &request.id)
            .await
            .unwrap()
            .username_seed,
        saved.username_seed
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_username_regeneration_refuses_closed_hosted_context_and_legacy_rows_default()
{
    let (state, actor, server) = fixture().await;
    let (request, start, watch) = hosted_request(&state, &actor, &server, false).await;
    let base = server.uri();
    let service = service(&state, &base);
    webhook(&service, &headers(&state).await, command(&start, 10)).await;
    let saved = service.get(&actor, &request.id).await.unwrap();
    assert!(saved.username_seed.is_none());
    assert_ne!(username_suggestions(&saved)[0], "nyx_bot");
    state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME)
        .delete_one(doc! {"_id":watch})
        .await
        .unwrap();
    assert!(
        service
            .regenerate_username(&actor, &request.id)
            .await
            .is_err()
    );
    assert!(
        service
            .get(&actor, &request.id)
            .await
            .unwrap()
            .username_seed
            .is_none()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_unbound_username_suggestions_regenerate_without_creating_a_request() {
    let (state, _, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    webhook(&service, &headers, command("/start", 10)).await;
    let before = server.received_requests().await.unwrap();
    let first = before.last().unwrap().body_json::<Value>().unwrap()["reply_markup"]["keyboard"][0]
        [0]["request_managed_bot"]["suggested_username"]
        .clone();
    let data = regeneration_button(&server).await;
    webhook(&service, &headers, callback(&data, 11)).await;
    let after = server.received_requests().await.unwrap();
    let next = &after.last().unwrap().body_json::<Value>().unwrap()["reply_markup"]["keyboard"][0]
        [0]["request_managed_bot"]["suggested_username"];
    assert_ne!(&first, next);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(REQUESTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_username_handler_requires_human_empty_body_and_rate_limit() {
    use crate::mw::auth::AuthMethod;
    use axum::{
        body::Bytes,
        extract::{Path, State},
    };
    let (state, actor, server) = fixture().await;
    let (request, start) = start_request(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    webhook(&service, &headers(&state).await, command(&start, 10)).await;
    let before = server.received_requests().await.unwrap().len();
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::ServiceAccount,
        AuthMethod::Delegated,
        AuthMethod::Relay,
    ] {
        let mut auth = crate::test_utils::test_auth_user(&actor);
        auth.auth_method = method;
        assert!(matches!(
            crate::handlers::telegram_new::regenerate_username(
                State(state.clone()),
                auth,
                Path(request.id.clone()),
                Bytes::new()
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
    }
    assert!(matches!(
        crate::handlers::telegram_new::regenerate_username(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor),
            Path(request.id.clone()),
            Bytes::from_static(b"{\"label\":\"Changed\"}")
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    for _ in 0..4 {
        let (headers, axum::Json(response)) = crate::handlers::telegram_new::regenerate_username(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor),
            Path(request.id.clone()),
            Bytes::new(),
        )
        .await
        .unwrap();
        assert_eq!(headers["cache-control"], "no-store");
        assert_eq!(headers["referrer-policy"], "no-referrer");
        assert_eq!(response.id, request.id);
        assert_eq!(response.label, request.label);
        assert_eq!(response.username_suggestions.len(), 3);
        assert!(
            serde_json::to_value(&response)
                .unwrap()
                .get("username_seed")
                .is_none()
        );
    }
    assert!(matches!(
        crate::handlers::telegram_new::regenerate_username(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor),
            Path(request.id.clone()),
            Bytes::new()
        )
        .await,
        Err(AppError::RateLimited)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    state.db.drop().await.unwrap();
}
