//! Closed CMA error projection and turn-scoped refusal handling.
use bson::{Document, doc};
use chrono::{Duration, Utc};
use mongodb::Database;
use serde_json::{Value, json};

use crate::{
    errors::AppResult,
    models::{assistant_conversation::AgentRole, downstream_service::DownstreamService},
    services::{
        assistant_acknowledgement_service::ChatAuthority, proxy_service::ProxyTarget,
        transaction_service::durable_database,
    },
};

const AUTH_REFUSAL_KIND: &str = "cma_authentication_refusal";
const NOT_AUTHENTICATED: &str = "not_authenticated";
pub(crate) const AUTH_REFUSAL_MARKER: &str = "cma_turn_authentication_refused";

/// The resolved connection and turn own the refusal, independently of tool
/// aliases, individual operations, and the replica handling the callback.
pub(crate) struct AuthenticationRefusal {
    id: String,
    scope: Document,
}

impl AuthenticationRefusal {
    pub(crate) fn for_call(
        chat: Option<&ChatAuthority>,
        target: &ProxyTarget,
        connection_id: &str,
        direct: bool,
    ) -> Option<Self> {
        if target.catalog_slug.as_deref() != Some("cma") {
            return None;
        }
        // A server-routed alias using only the catalog's owner assertion has
        // the same CMA authentication authority as the native tools.
        if direct
            && target.auth_method == "none"
            && target.target_id.is_none()
            && target.connection_id.is_none()
            && target.user_service_default_headers.is_empty()
            && target.catalog_default_headers.configured().is_empty()
            && matches!(
                target.service.identity_propagation_mode.as_str(),
                "jwt" | "both"
            )
            && !target.service.forward_access_token
            && !target.service.inject_delegation_token
        {
            return Self::for_catalog(chat, &target.service, &target.base_url);
        }
        Self::for_binding(
            chat,
            json!([connection_id, &target.service.id, &target.target_id]),
        )
    }

    pub(crate) fn for_catalog(
        chat: Option<&ChatAuthority>,
        service: &DownstreamService,
        base_url: &str,
    ) -> Option<Self> {
        Self::for_binding(
            chat,
            json!({
                "catalog": &service.id,
                "destination": base_url.trim_end_matches('/'),
                "identity_mode": &service.identity_propagation_mode,
                "audience": &service.identity_jwt_audience,
                "include_user_id": service.identity_include_user_id,
                "include_email": service.identity_include_email,
                "include_name": service.identity_include_name,
            }),
        )
    }

    fn for_binding(chat: Option<&ChatAuthority>, binding: Value) -> Option<Self> {
        let chat = chat.filter(|chat| {
            chat.turn_live && !chat.turn_stopped && chat.role == AgentRole::Orchestrator
        })?;
        let turn = chat.turn_id.as_deref()?;
        let identity = super::digest(json!([
            &chat.user_id,
            &chat.conversation_id,
            &chat.api_key_id,
            turn,
            binding
        ]));
        Some(Self {
            id: format!("{AUTH_REFUSAL_KIND}:{identity}"),
            scope: doc! {
                "kind": AUTH_REFUSAL_KIND,
                "user_id": &chat.user_id,
                "conversation_id": &chat.conversation_id,
                "api_key_id": &chat.api_key_id,
                "turn_id": turn,
            },
        })
    }

    pub(crate) async fn already_refused(&self, db: &Database) -> AppResult<bool> {
        Ok(durable_database(db)
            .collection::<Document>(super::COLLECTION)
            .find_one(doc! {"_id": &self.id, "expires_at": {"$gt": bson::DateTime::now()}})
            .projection(doc! {"_id": 1})
            .await?
            .is_some())
    }

    pub(crate) async fn record(&self, db: &Database) -> AppResult<()> {
        let mut fields = self.scope.clone();
        fields.insert(
            "expires_at",
            bson::DateTime::from_chrono(
                Utc::now()
                    + Duration::seconds(crate::services::assistant_nyxagent::ACTIVE_TURN_TTL_SECS),
            ),
        );
        durable_database(db)
            .collection::<Document>(super::COLLECTION)
            .update_one(doc! {"_id": &self.id}, doc! {"$setOnInsert": fields})
            .upsert(true)
            .await?;
        Ok(())
    }

    pub(crate) fn reply() -> Value {
        json!({
            "error": NOT_AUTHENTICATED,
            "status": 401,
            "retryable": false,
            "message": "CMA could not verify this connection. Ask the owner to check the CMA connection in NyxID and reconnect it if prompted, then send a new message.",
            "instructions": "End this turn with the recovery message. Skip further CMA discovery, retries, and alternate CMA calls for this task."
        })
    }
}

pub(crate) fn definitive_authentication_refusal(status: u16, body: &str) -> bool {
    definitive_authentication_refusal_bytes(status, body.as_bytes())
}

pub(crate) fn definitive_authentication_refusal_bytes(status: u16, body: &[u8]) -> bool {
    status == 401
        && body.len() <= 64 * 1024
        && serde_json::from_slice::<Value>(body)
            .is_ok_and(|value| value["code"].as_str() == Some(NOT_AUTHENTICATED))
}

pub(crate) fn credential_setup_result(
    catalog_slug: Option<&str>,
    service: &DownstreamService,
    status: u16,
    body: &str,
) -> Option<String> {
    if catalog_slug != Some("cma") || status != 409 || body.len() > 64 * 1024 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let code = value
        .get("code")
        .or_else(|| value.get("error"))
        .and_then(serde_json::Value::as_str);
    if code != Some("sandbox_credential_setup_required") {
        return None;
    }
    let mut url = url::Url::parse(&service.base_url).ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .extend(["auth", "login"]);
    Some(serde_json::json!({"error":"sandbox_credential_setup_required","login_url":url.as_str(),"message":format!("Sign in once at {url}, then retry the original request with the same Idempotency-Key."),"instructions":"Keep the exact original request body and Idempotency-Key. Do not silently create a new request or change its parent. If CMA rejects the same-key recovery after login, report the compatibility error."}).to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_link_is_never_taken_from_remote_problem_text() {
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.base_url = "https://cma.example.test".into();
        let problem = r#"{"code":"sandbox_credential_setup_required","login_url":"https://untrusted.test/collect","detail":"rotate your key"}"#;
        let result = credential_setup_result(Some("cma"), &catalog, 409, problem).unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["login_url"], "https://cma.example.test/auth/login");
        assert!(!result.contains("untrusted.test"));
        assert!(
            value["message"]
                .as_str()
                .unwrap()
                .contains("same Idempotency-Key")
        );
        assert!(credential_setup_result(Some("alias"), &catalog, 409, problem).is_none());
        assert!(credential_setup_result(Some("cma"), &catalog, 403, problem).is_none());
        assert!(
            credential_setup_result(Some("cma"), &catalog, 409, "{\"code\":\"different\"}")
                .is_none()
        );
    }
    #[test]
    fn malformed_catalog_destinations_are_not_links() {
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        for base in [
            "javascript:alert(1)",
            "https://user:secret@cma.test",
            "https://cma.test/?token=secret",
        ] {
            catalog.base_url = base.into();
            assert!(
                credential_setup_result(
                    Some("cma"),
                    &catalog,
                    409,
                    r#"{"code":"sandbox_credential_setup_required"}"#
                )
                .is_none()
            );
        }
    }
}
