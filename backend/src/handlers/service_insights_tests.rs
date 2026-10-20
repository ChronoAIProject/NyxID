use axum::extract::{Query, State};

use crate::{
    errors::AppError,
    handlers::service_insights::{InsightsQuery, get_insights},
    mw::auth::AuthMethod,
    test_utils::{test_app_state_no_db, test_auth_user},
};

#[tokio::test]
async fn agent_and_service_account_tokens_cannot_enumerate_key_inventory_through_insights() {
    let state = test_app_state_no_db().await;
    let actor_id = uuid::Uuid::new_v4().to_string();
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::ServiceAccount,
        AuthMethod::Relay,
    ] {
        let mut auth = test_auth_user(&actor_id);
        auth.auth_method = method;
        let result = get_insights(
            State(state.clone()),
            auth,
            Query(InsightsQuery {
                ids: uuid::Uuid::new_v4().to_string(),
                api_key_id: None,
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }
}
