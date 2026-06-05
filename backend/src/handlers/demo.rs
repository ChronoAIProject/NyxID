use axum::Json;
use chrono::Utc;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::mw::auth::AuthUser;

#[derive(Serialize, ToSchema)]
pub struct DemoResponse {
    pub ok: bool,
    pub message: String,
    pub timestamp: String,
    pub user_id: String,
    pub request_id: String,
}

/// GET /api/v1/demo
///
/// Returns a canned 200 so first-time users can verify they reach
/// NyxID's authenticated surface without configuring any downstream
/// service or external credential. No real downstream call is made.
#[utoipa::path(
    get,
    path = "/api/v1/demo",
    responses(
        (status = 200, description = "Demo response confirming auth + routing", body = DemoResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
    ),
    tag = "Demo"
)]
pub async fn get_demo(auth_user: AuthUser) -> Json<DemoResponse> {
    Json(DemoResponse {
        ok: true,
        message: "Hello from NyxID. Your auth and routing are wired correctly. Connect a real AI Service to start proxying downstream APIs.".to_string(),
        timestamp: Utc::now().to_rfc3339(),
        user_id: auth_user.user_id.to_string(),
        request_id: Uuid::new_v4().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::get_demo;
    use crate::test_utils::test_auth_user;

    #[tokio::test]
    async fn get_demo_returns_authenticated_demo_payload() {
        let user_id = uuid::Uuid::new_v4().to_string();

        let axum::Json(response) = get_demo(test_auth_user(&user_id)).await;

        assert!(response.ok);
        assert_eq!(
            response.message,
            "Hello from NyxID. Your auth and routing are wired correctly. Connect a real AI Service to start proxying downstream APIs."
        );
        assert_eq!(response.user_id, user_id);
        assert!(chrono::DateTime::parse_from_rfc3339(&response.timestamp).is_ok());
        assert!(uuid::Uuid::parse_str(&response.request_id).is_ok());

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["user_id"], user_id);
        assert!(json["timestamp"].as_str().is_some());
        assert!(json["request_id"].as_str().is_some());
    }
}
