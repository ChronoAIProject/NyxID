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

    #[tokio::test]
    async fn assistant_titles_utility_provider_applies_generated_title_end_to_end() {
        let (state, _, mock) = utility_fixture().await;
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
        let request: engine::TurnRequest = serde_json::from_value(
            json!({"conversation_id":row.id,"text":"hello there what can you help me to do?"}),
        )
        .unwrap();
        let start: engine::TurnStart = (&request).into();
        let active = Box::pin(engine::begin_turn(
            &state.db,
            ACTOR,
            &start,
            &state.encryption_keys,
        ))
        .await
        .unwrap();
        assert_eq!(active.title, "hello there what can you help me to do?");
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
        assert_eq!(mock.received_requests().await.unwrap().len(), 2);
    }
}
