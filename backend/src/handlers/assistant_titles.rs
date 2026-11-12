//! HTTP title adapter retaining the authenticated caller at its boundary.
use crate::{AppState, mw::auth::AuthUser, services::assistant_title_service as titles};

pub(super) fn spawn(state: AppState, auth: AuthUser, id: String) {
    titles::spawn(state, auth.user_id.to_string(), id);
}

#[cfg(test)]
pub(super) async fn generate(
    state: &AppState,
    auth: &AuthUser,
    id: &str,
) -> crate::errors::AppResult<()> {
    titles::generate(state, &auth.user_id.to_string(), id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        assistant_nyxagent as engine,
        assistant_oneshot_inference::tests::{ACTOR, utility_completion, utility_fixture},
        assistant_team_service as team,
    };
    use serde_json::json;

    async fn title_request_language_hint(tag: Option<&str>, question: &str) {
        let (state, _, mock) = utility_fixture().await;
        // The fake provider supplies an arbitrary title, not language detection.
        // Verify the actual utility request instead of claiming a locale forces output.
        utility_completion(&mock).await;
        let agent = Box::pin(team::ensure_nyxbot(&state.db, ACTOR))
            .await
            .unwrap();
        let mut session = state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let row = Box::pin(team::create_thread_for(
            &state.db,
            &state.encryption_keys,
            ACTOR,
            &agent,
            "New chat",
            &mut session,
        ))
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        let request: engine::TurnRequest =
            serde_json::from_value(json!({"conversation_id":row.id,"text":question})).unwrap();
        let mut start: engine::TurnStart = (&request).into();
        start.title_language = tag.map(str::to_owned);
        let active = Box::pin(engine::begin_turn(
            &state.db,
            ACTOR,
            &start,
            &state.encryption_keys,
        ))
        .await
        .unwrap();
        assert_eq!(active.title, question);
        Box::pin(engine::finish_turn(
            &state.db,
            &active,
            &active.credential_api_key_id,
            &uuid::Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: "I can help you research and plan tasks".into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        ))
        .await
        .unwrap();
        Box::pin(generate(
            &state,
            &crate::test_utils::test_auth_user(ACTOR),
            &row.id,
        ))
        .await
        .unwrap();
        let current = engine::get(&state.db, ACTOR, &row.id).await.unwrap();
        assert_eq!(current.title, "Discover NyxBot capabilities");
        assert_eq!(
            current.title_source,
            crate::models::assistant_conversation::TitleSource::Generated
        );
        let requests = mock.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        let body: serde_json::Value = requests[1].body_json().unwrap();
        let input: serde_json::Value =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(input["ui_language"], json!(tag));
        assert_eq!(input["first_user_message"], question);
        assert_eq!(input["assistant"], "I can help you research and plan tasks");
        assert!(input.get("output_language").is_none());
        let instruction = body["messages"][0]["content"].as_str().unwrap();
        assert!(
            instruction
                .contains("Write the title in the language first_user_message is written in.")
        );
        assert!(
            instruction
                .contains("Only if that language is unclear, use ui_language when it is not null.")
        );
        assert!(instruction.contains("Never choose a language from the assistant reply."));
    }
    #[tokio::test]
    async fn assistant_titles_english_ui_is_only_a_hint_for_chinese_message() {
        title_request_language_hint(Some("en-US"), "帮我连接日历以规划我的一天").await;
    }
    #[tokio::test]
    async fn assistant_titles_chinese_ui_is_only_a_hint_for_english_message() {
        title_request_language_hint(Some("zh-CN"), "Help me connect my calendar").await;
    }
    #[tokio::test]
    async fn assistant_titles_browser_locale_is_only_a_hint_for_english_message() {
        title_request_language_hint(Some("ru-RU"), "Help me connect my calendar").await;
    }
    #[tokio::test]
    async fn assistant_titles_missing_ui_hint_is_null() {
        title_request_language_hint(None, "Help me connect my calendar").await;
    }
}
