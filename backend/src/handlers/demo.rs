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
