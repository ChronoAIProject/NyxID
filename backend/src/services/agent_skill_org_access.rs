//! Ornn's org-grant lookup needs a bearer in addition to the person's signed
//! identity. This capability can only read one live membership via GET /orgs;
//! it is deliberately not an access token and grants no account/proxy scope.
use chrono::Utc;
use jsonwebtoken::{Algorithm, Header, Validation, decode, encode};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::assistant_acknowledgement_service::{self as acks, ChatAuthority};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::assistant_agent::AssistantAgent,
};

pub(crate) const TOKEN_TYPE: &str = "nyxid-ornn-org-read+jwt";
const AUDIENCE: &str = "nyxid:ornn-org-membership";

// No Debug: this is a bearer capability, never diagnostic data.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    sub: String,
    iss: String,
    aud: String,
    exp: i64,
    iat: i64,
    org: String,
    agent: String,
    connection: String,
    pins: String,
    turn: Option<Turn>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Turn {
    key: String,
    conversation: String,
    turn: String,
    invocation: Option<String>,
}

fn denied() -> AppError {
    AppError::Forbidden("Ornn organization lookup is unavailable".into())
}

fn pins(agent: &AssistantAgent) -> AppResult<String> {
    let bytes = serde_json::to_vec(&agent.skills)
        .map_err(|_| AppError::Internal("Could not bind skill references".into()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// Ornn uses signed identity with no credential or an independent header.
/// Other injectors can replace Authorization or rewrite the exact pin path.
pub(crate) fn compatible_auth(method: &str, key: &str) -> bool {
    method == "none"
        || (method == "header"
            && !key.trim().is_empty()
            && !key.trim().eq_ignore_ascii_case("authorization"))
}

fn usable(connection: &crate::models::user_service::UserService) -> bool {
    connection.credential_binding.as_deref() != Some("platform")
        && !connection.inject_delegation_token
        && compatible_auth(&connection.auth_method, &connection.auth_key_name)
        && matches!(
            connection.identity_propagation_mode.as_str(),
            "jwt" | "both"
        )
}

/// Metadata selection only. The exact proxy path retains credentials, policy,
/// billing and egress fences. Never try another principal after Ornn denies.
pub(crate) async fn connection(
    db: &mongodb::Database,
    access: &super::org_agent_service::RequestAccess,
    catalog: &str,
) -> AppResult<String> {
    use crate::models::user_service::{COLLECTION_NAME, UserService};
    use futures::TryStreamExt;
    let mut rows = db.collection::<UserService>(COLLECTION_NAME)
        .find(doc! {"user_id":access.owner(),"catalog_service_id":catalog,"is_active":true,"deleted_at":null})
        .sort(doc! {"_id":1}).await?;
    while let Some(row) = rows.try_next().await? {
        if access.allows_service(&row) && usable(&row) {
            return Ok(row.id);
        }
    }
    Err(AppError::SkillRead(
        crate::errors::skill_read::SkillReadReason::OwnerConnectionUnavailable,
    ))
}

pub(crate) fn issue(
    state: &AppState,
    actor: &str,
    agent: &AssistantAgent,
    connection: &str,
    chat: Option<&ChatAuthority>,
) -> AppResult<Zeroizing<String>> {
    let turn = if let Some(chat) = chat {
        if chat.guest
            || !chat.turn_live
            || chat.turn_stopped
            || chat.user_id != actor
            || chat.agent_id != agent.id
        {
            return Err(AppError::AssistantTurnRequired);
        }
        Some(Turn {
            key: chat.api_key_id.clone(),
            conversation: chat.conversation_id.clone(),
            turn: chat
                .turn_id
                .clone()
                .ok_or(AppError::AssistantTurnRequired)?,
            invocation: chat.invocation_id.clone(),
        })
    } else {
        None
    };
    let now = Utc::now().timestamp();
    let claims = Claims {
        sub: actor.into(),
        iss: state.config.jwt_issuer.clone(),
        aud: AUDIENCE.into(),
        iat: now,
        exp: now + 60,
        org: agent.user_id.clone(),
        agent: agent.id.clone(),
        connection: connection.into(),
        pins: pins(agent)?,
        turn,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some(TOKEN_TYPE.into());
    header.kid = Some(state.jwt_keys.kid.clone());
    encode(&header, &claims, &state.jwt_keys.encoding)
        .map(Zeroizing::new)
        .map_err(|_| AppError::Internal("Could not issue Ornn lookup capability".into()))
}

/// The existing Ornn /orgs client consumes only these membership fields.
#[derive(Serialize)]
pub(crate) struct Membership {
    id: String,
    display_name: Option<String>,
    your_role: &'static str,
}

pub(crate) async fn lookup(state: &AppState, token: &str) -> AppResult<Membership> {
    if jsonwebtoken::decode_header(token)
        .map_err(|_| denied())?
        .typ
        .as_deref()
        != Some(TOKEN_TYPE)
    {
        return Err(denied());
    }
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[AUDIENCE]);
    validation.set_issuer(&[&state.config.jwt_issuer]);
    validation.leeway = 0;
    let claims = decode::<Claims>(token, state.jwt_keys.decoding_key(token)?, &validation)
        .map_err(|_| denied())?
        .claims;
    let now = Utc::now().timestamp();
    if claims.exp <= now || claims.iat > now || claims.exp.checked_sub(claims.iat) != Some(60) {
        return Err(denied());
    }
    let actor = state
        .db
        .collection::<crate::models::user::User>("users")
        .find_one(doc! {"_id":&claims.sub,"is_active":true})
        .await?
        .filter(|user| user.user_type.is_person())
        .ok_or_else(denied)?;
    let org = state
        .db
        .collection::<crate::models::user::User>("users")
        .find_one(doc! {"_id":&claims.org,"is_active":true,"user_type":"org"})
        .await?
        .ok_or_else(denied)?;
    let agent = state
        .db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(
            doc! {"_id":&claims.agent,"user_id":&org.id,"kind":"specialist","destroyed_at":null},
        )
        .await?
        .ok_or_else(denied)?;
    if pins(&agent)? != claims.pins {
        return Err(denied());
    }
    let access = super::org_agent_service::resolve_agent_access(&state.db, &actor.id, &agent)
        .await?
        .ok_or_else(denied)?;
    let connection = state
        .db
        .collection::<crate::models::user_service::UserService>("user_services")
        .find_one(
            doc! {"_id":&claims.connection,"user_id":&org.id,"is_active":true,"deleted_at":null},
        )
        .await?
        .ok_or_else(denied)?;
    if !access.allows_service(&connection) || !usable(&connection)
        || state.db.collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
            .find_one(doc! {"_id":connection.catalog_service_id.as_deref().unwrap_or_default(),"slug":"ornn-api","is_active":true})
            .await?.is_none()
    { return Err(denied()); }
    if let Some(turn) = claims.turn {
        let key = super::key_service::get_api_key(&state.db, &actor.id, &turn.key).await?;
        if !key.is_active || key.expires_at.is_some_and(|at| at <= Utc::now()) {
            return Err(denied());
        }
        let auth = Box::pin(crate::mw::auth::api_key_auth_user(
            &state.db, &key, None, None, None,
        ))
        .await?;
        let live = acks::for_key_with_access(
            &state.db,
            &actor.id,
            Some(&key.id),
            auth.org_agent_access.as_ref(),
        )
        .await?
        .ok_or_else(denied)?;
        if live.guest
            || !live.turn_live
            || live.turn_stopped
            || live.agent_id != agent.id
            || live.conversation_id != turn.conversation
            || live.turn_id.as_deref() != Some(&turn.turn)
            || live.invocation_id != turn.invocation
        {
            return Err(denied());
        }
        live.ensure_live_turn(&state.db, None, None, "assistant.skill_org_lookup")
            .await?;
    }
    Ok(Membership {
        id: org.id,
        display_name: org.display_name,
        your_role: if access.is_admin() { "admin" } else { "member" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{assistant_nyxagent as engine, org_agent_tests::Fixture};
    use crate::test_utils::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    #[test]
    fn org_skill_auth_preserves_callback_bearer_and_exact_path() {
        assert!(compatible_auth("none", ""));
        assert!(compatible_auth("header", "X-Ornn-Credential"));
        assert!(!compatible_auth("header", " Authorization "));
        for method in [
            "bearer",
            "bot_bearer",
            "basic",
            "aws_sigv4",
            "token_exchange",
            "github_git",
            "path",
            "query",
            "body",
        ] {
            assert!(!compatible_auth(method, ""), "{method}");
        }
    }

    #[tokio::test]
    async fn org_skill_lookup_is_purpose_limited_and_live_bound() {
        let f = Fixture::new("org_skill_lookup_capability").await;
        let (agent, thread) = f.create().await;
        // The callback must not turn into the actor's ordinary all-org list.
        let other_org = uuid::Uuid::new_v4().to_string();
        f.state
            .db
            .collection("users")
            .insert_one(test_user(&other_org, crate::models::user::UserType::Org))
            .await
            .unwrap();
        f.state
            .db
            .collection("org_memberships")
            .insert_one(test_membership(
                &other_org,
                &f.admin,
                crate::models::org_membership::OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        let connection = f.service(&f.org, "ornn-api").await;
        let mut catalog = test_auto_connected_catalog_service();
        catalog.slug = "ornn-api".into();
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                "downstream_services",
            )
            .insert_one(&catalog)
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .update_one(
                doc! {"_id":&connection},
                doc! {"$set":{"catalog_service_id":&catalog.id,"identity_propagation_mode":"jwt"}},
            )
            .await
            .unwrap();
        let thread = engine::begin_turn(
            &f.state.db,
            &f.admin,
            &engine::TurnRequest {
                conversation_id: Some(thread.id),
                text: "Guide".into(),
                agent_id: None,
                model: None,
                access_mode: None,
                acknowledgement_id: None,
                retry_of: None,
                attachment_ids: vec![],
            },
            &f.state.encryption_keys,
        )
        .await
        .unwrap();
        let chat = acks::for_key(&f.state.db, &f.admin, Some(&thread.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
        let token = issue(&f.state, &f.admin, &agent, &connection, Some(&chat)).unwrap();
        assert_eq!(lookup(&f.state, &token).await.unwrap().id, f.org);
        // Ordinary JWT/access/MCP authentication cannot parse this purpose and
        // audience, and it has no generic account-read or execution scope.
        assert!(
            crate::crypto::jwt::verify_token(&f.state.jwt_keys, &f.state.config, &token).is_err()
        );
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        for (method, path, allowed) in [
            ("GET", "/api/v1/orgs", true),
            ("HEAD", "/api/v1/orgs", false),
            ("POST", "/api/v1/orgs", false),
            ("GET", "/api/v1/users/me", false),
            ("GET", "/api/v1/keys", false),
            ("GET", "/api/v1/mcp/config", false),
            ("POST", "/api/v1/llm/gateway/v1/chat/completions", false),
            ("GET", "/api/v1/orgs/other-org/members", false),
            ("GET", "/api/v1/proxy/ornn-api/anything", false),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {}", token.as_str()))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status().is_success(),
                allowed,
                "{method} {path}: {}",
                response.status()
            );
            if !allowed {
                assert!(
                    matches!(
                        response.status(),
                        StatusCode::UNAUTHORIZED
                            | StatusCode::FORBIDDEN
                            | StatusCode::METHOD_NOT_ALLOWED
                    ),
                    "unexpected refusal for {method} {path}: {}",
                    response.status()
                );
            }
            if allowed {
                assert_eq!(response.status(), StatusCode::OK);
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                        .unwrap();
                assert_eq!(body["orgs"].as_array().unwrap().len(), 1);
                assert_eq!(body["orgs"][0]["id"], f.org);
            }
        }
        // Re-sign expired and wrong-audience claims as a trusted issuer to
        // test validation independently of signature corruption.
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token.split('.').nth(1).unwrap())
            .unwrap();
        let original: Value = serde_json::from_slice(&payload).unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        for (field, value) in [
            ("exp", json!(Utc::now().timestamp() - 1)),
            ("aud", json!("different-audience")),
            ("org", json!(f.outsider)),
            ("pins", json!("changed-pins")),
        ] {
            let mut claims = original.clone();
            claims[field] = value;
            let bad = encode(&header, &claims, &f.state.jwt_keys.encoding).unwrap();
            assert!(lookup(&f.state, &bad).await.is_err());
        }
        let mut bad = token.to_string();
        bad.push('x');
        assert!(lookup(&f.state, &bad).await.is_err());
        let mut guest = chat.clone();
        guest.guest = true;
        assert!(issue(&f.state, &f.admin, &agent, &connection, Some(&guest)).is_err());
        f.state
            .db
            .collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":&chat.api_key_id},
                doc! {"$set":{"is_active":false}},
            )
            .await
            .unwrap();
        assert!(lookup(&f.state, &token).await.is_err());
        f.state
            .db
            .collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":&chat.api_key_id},
                doc! {"$set":{"is_active":true}},
            )
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>("assistant_conversations")
            .update_one(
                doc! {"_id":&chat.conversation_id},
                doc! {"$set":{"active_turn.turn_id":"another-turn"}},
            )
            .await
            .unwrap();
        assert!(lookup(&f.state, &token).await.is_err());
        let preview = issue(&f.state, &f.admin, &agent, &connection, None).unwrap();
        assert!(lookup(&f.state, &preview).await.is_ok());
        f.revoke(&f.admin).await;
        assert!(lookup(&f.state, &preview).await.is_err());
        f.state.db.drop().await.unwrap();
    }
}
