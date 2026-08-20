#![allow(dead_code)]

use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Serialize;
use std::collections::HashMap;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::consent::Consent;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, consent_service, oauth_broker_service};

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct ConsentItem {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub scopes: String,
    pub allow_all_services: bool,
    pub allowed_service_ids: Vec<String>,
    pub allowed_services: Vec<ConsentAllowedServiceItem>,
    pub legacy_unrestricted: bool,
    pub granted_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConsentAllowedServiceItem {
    pub id: String,
    pub slug: Option<String>,
    pub label: String,
    pub catalog_service_name: Option<String>,
    pub deleted: bool,
}

#[derive(Debug, Serialize)]
pub struct ConsentListResponse {
    pub consents: Vec<ConsentItem>,
}

#[derive(Debug, Serialize)]
pub struct ConsentRevokeResponse {
    pub message: String,
}

/// Safe consent state for assistant postcondition reads. Client names,
/// scopes, and service labels stay on the detail response and never enter
/// this projection.
#[derive(Debug, Serialize)]
pub struct ConsentAuthorizationEvidenceResponse {
    pub id: String,
    pub client_id: String,
    pub allow_all_services: bool,
    pub allowed_service_ids: Vec<String>,
    pub granted_at: String,
    pub expires_at: Option<String>,
}

impl ConsentAuthorizationEvidenceResponse {
    fn from_consent(consent: &Consent) -> Self {
        Self {
            id: consent.id.clone(),
            client_id: consent.client_id.clone(),
            allow_all_services: consent.allow_all_services,
            allowed_service_ids: consent.allowed_service_ids.clone().unwrap_or_default(),
            granted_at: consent.granted_at.to_rfc3339(),
            expires_at: consent.expires_at.map(|value| value.to_rfc3339()),
        }
    }
}

// --- Handlers ---

/// GET /api/v1/users/me/consents
pub async fn list_my_consents(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ConsentListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let consents = consent_service::list_user_consents(&state.db, &user_id).await?;

    let mut items = Vec::with_capacity(consents.len());
    for c in consents {
        // Look up client name
        let client_name = state
            .db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": &c.client_id })
            .await?
            .map(|cl| cl.client_name)
            .unwrap_or_else(|| c.client_id.clone());
        let legacy_unrestricted = is_legacy_unrestricted(&c);
        let allowed_service_ids = response_allowed_service_ids(&c);
        let allowed_services =
            resolve_allowed_services(&state.db, &user_id, &allowed_service_ids).await?;

        items.push(ConsentItem {
            id: c.id,
            client_id: c.client_id,
            client_name,
            scopes: c.scopes,
            allow_all_services: c.allow_all_services,
            allowed_service_ids,
            allowed_services,
            legacy_unrestricted,
            granted_at: c.granted_at.to_rfc3339(),
            expires_at: c.expires_at.map(|t| t.to_rfc3339()),
        });
    }

    Ok(Json(ConsentListResponse { consents: items }))
}

/// DELETE /api/v1/users/me/consents/:client_id
pub async fn revoke_my_consent(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
) -> AppResult<Json<ConsentRevokeResponse>> {
    let user_id = auth_user.user_id.to_string();
    let result = consent_service::revoke_consent(&state.db, &user_id, &client_id).await?;
    let revoked_broker_bindings = oauth_broker_service::revoke_bindings_for_user_client(
        &state.db,
        state.encryption_keys.clone(),
        &state.http_client,
        &client_id,
        &user_id,
        "user_revoked",
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oauth_consent_revoked",
        Some(serde_json::json!({
            "client_id": client_id,
            "revoked_refresh_tokens": result.revoked_refresh_tokens,
            "revoked_broker_bindings": revoked_broker_bindings,
        })),
    );

    Ok(Json(ConsentRevokeResponse {
        message: "Consent revoked".to_string(),
    }))
}

/// GET /api/v1/users/me/consents/{client_id}/authorization
///
/// A revoked consent is proven by this exact route returning body-free 404.
pub async fn get_my_consent_authorization(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
) -> AppResult<Json<ConsentAuthorizationEvidenceResponse>> {
    let user_id = auth_user.user_id.to_string();
    let consent = state
        .db
        .collection::<Consent>(crate::models::consent::COLLECTION_NAME)
        .find_one(doc! { "user_id": &user_id, "client_id": &client_id })
        .await?
        .ok_or(AppError::ConsentNotFound)?;
    Ok(Json(ConsentAuthorizationEvidenceResponse::from_consent(
        &consent,
    )))
}

fn is_legacy_unrestricted(consent: &Consent) -> bool {
    consent.allowed_service_ids.is_none() && !consent.allow_all_services
}

fn response_allowed_service_ids(consent: &Consent) -> Vec<String> {
    consent.allowed_service_ids.clone().unwrap_or_default()
}

async fn resolve_allowed_services(
    db: &mongodb::Database,
    user_id: &str,
    allowed_service_ids: &[String],
) -> AppResult<Vec<ConsentAllowedServiceItem>> {
    if allowed_service_ids.is_empty() {
        return Ok(Vec::new());
    }

    let services: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! { "_id": { "$in": allowed_service_ids }, "user_id": user_id })
        .await?
        .try_collect()
        .await?;
    let endpoint_ids: Vec<String> = services
        .iter()
        .map(|service| service.endpoint_id.clone())
        .collect();
    let catalog_service_ids: Vec<String> = services
        .iter()
        .filter_map(|service| service.catalog_service_id.clone())
        .collect();

    let endpoints: Vec<UserEndpoint> = if endpoint_ids.is_empty() {
        Vec::new()
    } else {
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .find(doc! { "_id": { "$in": endpoint_ids } })
            .await?
            .try_collect()
            .await?
    };
    let catalog_services: Vec<DownstreamService> = if catalog_service_ids.is_empty() {
        Vec::new()
    } else {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": catalog_service_ids } })
            .await?
            .try_collect()
            .await?
    };

    let services_by_id: HashMap<String, UserService> = services
        .into_iter()
        .map(|service| (service.id.clone(), service))
        .collect();
    let endpoints_by_id: HashMap<String, UserEndpoint> = endpoints
        .into_iter()
        .map(|endpoint| (endpoint.id.clone(), endpoint))
        .collect();
    let catalog_names_by_id: HashMap<String, String> = catalog_services
        .into_iter()
        .map(|service| (service.id.clone(), service.name))
        .collect();

    let mut resolved = Vec::with_capacity(allowed_service_ids.len());
    for service_id in allowed_service_ids {
        let Some(service) = services_by_id.get(service_id) else {
            resolved.push(ConsentAllowedServiceItem {
                id: service_id.clone(),
                slug: None,
                label: service_id.clone(),
                catalog_service_name: None,
                deleted: true,
            });
            continue;
        };

        let label = endpoints_by_id
            .get(&service.endpoint_id)
            .map(|endpoint| endpoint.label.clone())
            .unwrap_or_else(|| service.slug.clone());
        let catalog_service_name = service
            .catalog_service_id
            .as_ref()
            .and_then(|id| catalog_names_by_id.get(id).cloned());

        resolved.push(ConsentAllowedServiceItem {
            id: service.id.clone(),
            slug: Some(service.slug.clone()),
            label,
            catalog_service_name,
            deleted: false,
        });
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{connect_test_database, test_user_endpoint, test_user_service};
    use chrono::Utc;

    // ---- ConsentItem serialization ----

    #[test]
    fn consent_item_serializes_all_fields() {
        let item = ConsentItem {
            id: "consent-1".to_string(),
            client_id: "client-abc".to_string(),
            client_name: "My App".to_string(),
            scopes: "openid profile email".to_string(),
            allow_all_services: false,
            allowed_service_ids: vec!["svc-1".to_string()],
            allowed_services: vec![ConsentAllowedServiceItem {
                id: "svc-1".to_string(),
                slug: Some("openai".to_string()),
                label: "OpenAI key".to_string(),
                catalog_service_name: Some("OpenAI".to_string()),
                deleted: false,
            }],
            legacy_unrestricted: false,
            granted_at: "2026-01-01T00:00:00+00:00".to_string(),
            expires_at: Some("2027-01-01T00:00:00+00:00".to_string()),
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["id"], "consent-1");
        assert_eq!(json["client_id"], "client-abc");
        assert_eq!(json["client_name"], "My App");
        assert_eq!(json["scopes"], "openid profile email");
        assert_eq!(json["allow_all_services"], false);
        assert_eq!(json["allowed_service_ids"], serde_json::json!(["svc-1"]));
        assert_eq!(json["allowed_services"][0]["id"], "svc-1");
        assert_eq!(json["allowed_services"][0]["slug"], "openai");
        assert_eq!(json["allowed_services"][0]["label"], "OpenAI key");
        assert_eq!(
            json["allowed_services"][0]["catalog_service_name"],
            "OpenAI"
        );
        assert_eq!(json["allowed_services"][0]["deleted"], false);
        assert_eq!(json["legacy_unrestricted"], false);
        assert_eq!(json["granted_at"], "2026-01-01T00:00:00+00:00");
        assert_eq!(json["expires_at"], "2027-01-01T00:00:00+00:00");
    }

    #[test]
    fn consent_item_with_no_expiry() {
        let item = ConsentItem {
            id: "consent-2".to_string(),
            client_id: "client-xyz".to_string(),
            client_name: "Other App".to_string(),
            scopes: "openid".to_string(),
            allow_all_services: true,
            allowed_service_ids: vec![],
            allowed_services: vec![],
            legacy_unrestricted: false,
            granted_at: "2026-01-01T00:00:00+00:00".to_string(),
            expires_at: None,
        };
        let json = serde_json::to_value(&item).unwrap();
        assert!(json["expires_at"].is_null());
    }

    #[test]
    fn consent_service_access_flags_cover_explicit_and_legacy_rows() {
        let now = Utc::now();
        let mut consent = Consent {
            id: "consent-1".to_string(),
            user_id: "user-1".to_string(),
            client_id: "client-1".to_string(),
            scopes: "openid".to_string(),
            allow_all_services: false,
            allowed_service_ids: Some(vec!["svc-1".to_string()]),
            granted_at: now,
            expires_at: None,
        };

        assert!(!is_legacy_unrestricted(&consent));
        assert_eq!(response_allowed_service_ids(&consent), vec!["svc-1"]);

        consent.allowed_service_ids = Some(Vec::new());
        assert!(!is_legacy_unrestricted(&consent));
        assert!(response_allowed_service_ids(&consent).is_empty());

        consent.allowed_service_ids = Some(Vec::new());
        consent.allow_all_services = true;
        assert!(!is_legacy_unrestricted(&consent));
        assert!(response_allowed_service_ids(&consent).is_empty());

        consent.allowed_service_ids = None;
        consent.allow_all_services = false;
        assert!(is_legacy_unrestricted(&consent));
        assert!(response_allowed_service_ids(&consent).is_empty());
    }

    // ---- ConsentListResponse serialization ----

    #[test]
    fn consent_list_response_empty() {
        let resp = ConsentListResponse { consents: vec![] };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["consents"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn resolve_allowed_services_returns_names_and_deleted_placeholders() {
        let Some(db) = connect_test_database("consent_allowed_services").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let other_user_id = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let foreign_service_id = uuid::Uuid::new_v4().to_string();
        let missing_service_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &user_id,
                "Primary OpenAI",
                "https://api.openai.example/v1",
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([
                test_user_service(&service_id, &user_id, "openai", &endpoint_id, None, None),
                test_user_service(
                    &foreign_service_id,
                    &other_user_id,
                    "foreign",
                    "foreign-endpoint",
                    None,
                    None,
                ),
            ])
            .await
            .unwrap();

        let items = resolve_allowed_services(
            &db,
            &user_id,
            &[
                service_id.clone(),
                missing_service_id.clone(),
                foreign_service_id.clone(),
            ],
        )
        .await
        .unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, service_id);
        assert_eq!(items[0].slug.as_deref(), Some("openai"));
        assert_eq!(items[0].label, "Primary OpenAI");
        assert!(!items[0].deleted);
        assert_eq!(items[1].id, missing_service_id);
        assert!(items[1].slug.is_none());
        assert!(items[1].deleted);
        assert_eq!(items[2].id, foreign_service_id);
        assert!(items[2].slug.is_none());
        assert!(items[2].deleted);
    }

    // ---- ConsentRevokeResponse serialization ----

    #[test]
    fn consent_revoke_response_message() {
        let resp = ConsentRevokeResponse {
            message: "Consent revoked".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["message"], "Consent revoked");
    }
}
