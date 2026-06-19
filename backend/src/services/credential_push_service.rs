use std::sync::Arc;

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::node_service;
use crate::services::node_ws_manager::{CredentialUpdateParams, NodeWsManager};

/// API-key scope to apply during a rotation-time push fan-out.
///
/// Mirrors the node + service scope fields on `AuthUser`. A credential
/// shared across multiple `UserService`s can reach nodes the caller
/// does not own (node dim) AND services the caller does not have in
/// `allowed_service_ids` (service dim). The fan-out must enforce
/// *both* so external-key rotation cannot be used to rewrite
/// out-of-scope siblings (thirtieth-round Codex P1 — service dim
/// added, node dim inherited from the twenty-ninth-round fix).
pub struct ActorScope {
    pub allow_all_nodes: bool,
    pub allowed_node_ids: Vec<String>,
    pub allow_all_services: bool,
    pub allowed_service_ids: Vec<String>,
}

impl ActorScope {
    /// Construct an unrestricted scope — used by session / admin
    /// callers and internal services that aren't API-key gated.
    #[allow(dead_code)]
    pub fn unrestricted() -> Self {
        Self {
            allow_all_nodes: true,
            allowed_node_ids: vec![],
            allow_all_services: true,
            allowed_service_ids: vec![],
        }
    }

    fn permits_node(&self, node_id: &str) -> bool {
        self.allow_all_nodes || self.allowed_node_ids.iter().any(|id| id == node_id)
    }

    fn permits_service(&self, service_id: &str) -> bool {
        self.allow_all_services || self.allowed_service_ids.iter().any(|id| id == service_id)
    }
}

/// Fire-and-forget push that also verifies node ownership per target
/// before delivery. Used by the `/api/v1/api-keys/external/:id`
/// rotation path where the caller authenticated to edit the `UserApiKey`
/// but hasn't passed the per-node ownership check that `PUT /keys`
/// applies via `ensure_node_writable_by_actor`. Without this filter, an
/// org admin or scoped key editor could rewrite credentials on nodes
/// they don't control (twenty-first-round Codex P1).
///
/// Additionally honors `actor_node_scope`: a scoped API key with an
/// `allowed_node_ids` allow-list cannot use external-key rotation as a
/// back door to rewrite credentials on out-of-scope nodes owned by the
/// same user (thirtieth-round Codex P1). Mirrors the scope enforcement
/// `execute_tool` applies at proxy time.
///
/// Services whose node the actor cannot write to OR whose node is
/// outside the caller's API-key scope are silently skipped (logged as a
/// debug/info line). Services on nodes that pass both checks are pushed
/// through `push_credential_to_node_if_routed` as usual.
pub async fn push_credential_to_node_if_owned(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_ws_manager: &Arc<NodeWsManager>,
    user_id: &str,
    actor_user_id: &str,
    api_key_id: &str,
    actor_scope: ActorScope,
) {
    let services: Vec<UserService> = match db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! {
            "user_id": user_id,
            "api_key_id": api_key_id,
            "node_id": { "$ne": null },
            "is_active": true,
            "auth_method": { "$ne": "none" },
        })
        .await
    {
        Ok(cursor) => cursor.try_collect().await.unwrap_or_default(),
        Err(e) => {
            tracing::warn!(error = %e, "Failed to query UserServices for owned credential push");
            return;
        }
    };

    if services.is_empty() {
        return;
    }

    let api_key = match db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": api_key_id })
        .await
    {
        Ok(Some(k)) => k,
        _ => {
            tracing::warn!(api_key_id = %api_key_id, "UserApiKey not found for owned credential push");
            return;
        }
    };

    let credential = match decrypt_api_key_credential(&api_key, encryption_keys).await {
        Some(c) => c,
        None => {
            tracing::warn!(api_key_id = %api_key_id, "No credential to push");
            return;
        }
    };

    for svc in &services {
        let Some(node_id) = svc.node_id.as_deref() else {
            continue;
        };

        // API-key service scope check: when a credential is shared
        // across multiple services, a scoped key whose
        // `allowed_service_ids` authorizes only one sibling must not
        // use rotation of the shared credential to overwrite node-
        // local credentials for the other siblings — that would be a
        // back door past the service allow-list
        // (thirty-first-round Codex P1). Runs before the node-scope
        // check so we short-circuit even before touching the node
        // ownership query.
        if !actor_scope.permits_service(&svc.id) {
            tracing::info!(
                actor_user_id = %actor_user_id,
                service_id = %svc.id,
                service_slug = %svc.slug,
                "Skipping credential push: service is outside actor API-key scope"
            );
            continue;
        }

        // API-key node scope check: a key limited to a subset of nodes
        // must not use external-key rotation to rewrite credentials on
        // out-of-scope nodes, even when the underlying user owns them
        // (thirtieth-round Codex P1).
        if !actor_scope.permits_node(node_id) {
            tracing::info!(
                actor_user_id = %actor_user_id,
                node_id = %node_id,
                service_slug = %svc.slug,
                "Skipping credential push: target node is outside actor API-key scope"
            );
            continue;
        }

        // Per-node ownership check: only push to nodes the actor can
        // write to. Failure here is logged but doesn't abort the rest
        // of the rotation — some nodes may be legitimately out of
        // scope for this actor while others are rotate-worthy.
        if node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id)
            .await
            .is_err()
        {
            tracing::info!(
                actor_user_id = %actor_user_id,
                node_id = %node_id,
                service_slug = %svc.slug,
                "Skipping credential push: actor does not own the target node"
            );
            continue;
        }

        let target_url = match db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find_one(doc! { "_id": &svc.endpoint_id })
            .await
        {
            Ok(Some(ep)) if !ep.url.is_empty() => Some(ep.url),
            _ => None,
        };
        let params = build_credential_params(svc, &credential, target_url);
        if let Err(e) = node_ws_manager.send_credential_update(node_id, &params) {
            tracing::warn!(
                node_id = %node_id,
                service_slug = %svc.slug,
                error = %e,
                "Failed to push credential to owned node (node may be offline)"
            );
        }
    }
}

/// After a credential is stored or refreshed, check if any UserService
/// referencing this UserApiKey is node-routed. If so, push the credential
/// to the connected node.
///
/// This is fire-and-forget: errors are logged but not propagated.
pub async fn push_credential_to_node_if_routed(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_ws_manager: &Arc<NodeWsManager>,
    user_id: &str,
    api_key_id: &str,
) {
    // Find UserServices that reference this api_key and have a node_id.
    // Exclude services downgraded to `auth_method: "none"`: pushing the
    // rotated secret would silently re-enable injection on a node for a
    // service the user explicitly turned off. Nineteenth-round Codex P2.
    let services: Vec<UserService> = match db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! {
            "user_id": user_id,
            "api_key_id": api_key_id,
            "node_id": { "$ne": null },
            "is_active": true,
            "auth_method": { "$ne": "none" },
        })
        .await
    {
        Ok(cursor) => match cursor.try_collect().await {
            Ok(svcs) => svcs,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to query UserServices for credential push");
                return;
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "Failed to query UserServices for credential push");
            return;
        }
    };

    if services.is_empty() {
        return;
    }

    // Load the UserApiKey to get the decrypted credential
    let api_key = match db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": api_key_id })
        .await
    {
        Ok(Some(k)) => k,
        Ok(None) => {
            tracing::warn!(api_key_id = %api_key_id, "UserApiKey not found for credential push");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to load UserApiKey for credential push");
            return;
        }
    };

    // Decrypt the credential
    let credential = match decrypt_api_key_credential(&api_key, encryption_keys).await {
        Some(c) => c,
        None => {
            tracing::warn!(api_key_id = %api_key_id, "No credential to push");
            return;
        }
    };

    for svc in &services {
        let node_id = match &svc.node_id {
            Some(id) => id,
            None => continue,
        };

        // Load endpoint URL for target_url field
        let target_url = match db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find_one(doc! { "_id": &svc.endpoint_id })
            .await
        {
            Ok(Some(ep)) if !ep.url.is_empty() => Some(ep.url),
            _ => None,
        };

        let params = build_credential_params(svc, &credential, target_url);

        if let Err(e) = node_ws_manager.send_credential_update(node_id, &params) {
            tracing::warn!(
                node_id = %node_id,
                service_slug = %svc.slug,
                error = %e,
                "Failed to push credential to node (node may be offline)"
            );
        }
    }
}

/// Effective post-update fields that the strict push uses to build the
/// `credential_update` WS frame. The caller (the `PUT /keys` handler)
/// composes these from `body + view` before persisting the
/// `UserService` mutation, so a push failure leaves no partial state
/// behind (fourth-round Codex review P1).
pub struct StrictPushTarget<'a> {
    pub target_node_id: &'a str,
    pub service_slug: &'a str,
    pub auth_method: &'a str,
    pub auth_key_name: &'a str,
    pub target_url: Option<&'a str>,
}

/// Strict variant of `push_credential_to_node_if_routed` used by the
/// `PUT /keys` flow that just stored a fresh credential server-side. The
/// caller gates subsequent `UserService` writes and
/// `activate_node_managed_api_key` on a successful return here — if this
/// function errors (node offline, WS buffer full, serialization failure),
/// the handler aborts the request so the credential stays on the server
/// for a later retry instead of being silently lost (first Codex review
/// P1), and no routing/auth mutation has been persisted yet.
///
/// Takes `target` fields directly rather than re-reading `UserService`
/// from the database — the caller knows the *effective* post-update
/// values (`node_id`, `auth_method`, endpoint URL, …) and we need to
/// push to *those*, not to whatever the DB still shows from before.
///
/// **Best-effort delivery semantics.** "Strict" here means the WS frame
/// was queued on the node's outbound channel — it does NOT wait for a
/// `credential_update_ack` from the node. If the node accepts the frame
/// but later fails to persist it (keychain / config write error), the
/// handler still returns success and commits routing mutations.
/// Follow-up: add request-id + oneshot pending waiter so callers can
/// block on `credential_update_ack` with a timeout (twentieth-round
/// Codex P1). The existing push-before-mutation ordering and the
/// server-side credential preservation in `reconcile_provider_key_for_
/// service_routing` mean a failed node-side apply leaves the server copy
/// intact for the user to retry via the same PUT, so the worst case is
/// temporary proxy failure, not credential loss.
pub async fn push_credential_to_node_strict(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_ws_manager: &Arc<NodeWsManager>,
    user_id: &str,
    api_key_id: &str,
    target: StrictPushTarget<'_>,
) -> AppResult<()> {
    let api_key = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": api_key_id, "user_id": user_id })
        .await?
        .ok_or_else(|| {
            AppError::NotFound("API key disappeared before credential push could run".to_string())
        })?;

    let credential = decrypt_api_key_credential(&api_key, encryption_keys)
        .await
        .ok_or_else(|| {
            AppError::Internal(
                "Failed to decrypt credential for node push; secret cannot be delivered"
                    .to_string(),
            )
        })?;

    let params = build_credential_params_from_fields(
        target.service_slug,
        target.auth_method,
        target.auth_key_name,
        &credential,
        target.target_url.map(str::to_string),
    );

    // Capability-aware delivery. New node agents advertise
    // `credential_ack_correlation` in their `status_update` — the
    // backend then uses strict ack-wait so a node-side apply failure
    // fails the PUT. Older agents don't advertise the flag; we fall
    // back to the best-effort `send_credential_update` (returns Ok
    // once the frame is queued). This lets backend + node roll out
    // independently (twenty-seventh-round Codex P2).
    //
    // Wait briefly for capability resolution before classifying the
    // node. Without this, a PUT that lands in the short window after
    // the WS auth handshake but before the node's first
    // `status_update` would see `supports_credential_ack_correlation
    // == false` and silently downgrade to fire-and-forget on an
    // otherwise-strict agent (twenty-ninth-round Codex P2). If the
    // first status_update never arrives within the timeout we fall
    // through — old agents that never send one must still make
    // progress.
    node_ws_manager
        .await_capability_resolution(target.target_node_id, std::time::Duration::from_millis(500))
        .await;

    if node_ws_manager.supports_credential_ack_correlation(target.target_node_id) {
        node_ws_manager
            .send_credential_update_and_wait(
                target.target_node_id,
                &params,
                std::time::Duration::from_secs(10),
            )
            .await?;
    } else {
        node_ws_manager.send_credential_update(target.target_node_id, &params)?;
        tracing::info!(
            node_id = %target.target_node_id,
            service_slug = %target.service_slug,
            "Pushed credential in legacy mode (node has not advertised credential_ack_correlation)"
        );
    }
    Ok(())
}

/// Strict variant of `push_no_auth_to_node`: returns an error if the
/// node rejects the placeholder or never acknowledges it. Used by
/// `PUT /keys` to keep the `auth_method: "none"` downgrade atomic —
/// the caller runs this BEFORE any `UserService` / `UserEndpoint`
/// mutations land so a failed push leaves server state untouched and
/// the client can retry (fixes the non-atomic gap flagged in the
/// PR #437 review).
///
/// Legacy agents that don't advertise `credential_ack_correlation`
/// can't participate in the ack protocol and also don't understand the
/// `injection_method: "none"` placeholder. Rather than silently
/// accept the downgrade and leave the old secret live on the node,
/// this function refuses the downgrade with `NodeCredentialMissing`
/// so the caller surfaces a clear, actionable error. Operators who
/// really need the downgrade can upgrade the agent or manually run
/// `nyxid node credentials remove` first.
pub async fn push_no_auth_to_node_strict(
    node_ws_manager: &Arc<NodeWsManager>,
    node_id: &str,
    service_slug: &str,
    target_url: Option<&str>,
) -> AppResult<()> {
    let params = CredentialUpdateParams {
        service_slug: service_slug.to_string(),
        injection_method: "none".to_string(),
        header_name: None,
        header_value: None,
        param_name: None,
        param_value: None,
        target_url: target_url.map(str::to_string),
    };

    // Short-circuit on a disconnected node so the caller sees a
    // retryable `NodeOffline` (503 / 8001) instead of the
    // "legacy agent" `BadRequest` below —
    // `supports_credential_ack_correlation` returns `false` in both
    // cases, so without this guard an offline node was being told to
    // "upgrade the agent" even though retrying when it reconnects is
    // the right move.
    if !node_ws_manager.is_connected(node_id) {
        return Err(AppError::NodeOffline(format!(
            "Node {node_id} is not connected; cannot downgrade service '{service_slug}' to no-auth \
             without first clearing the credential on the node."
        )));
    }

    node_ws_manager
        .await_capability_resolution(node_id, std::time::Duration::from_millis(500))
        .await;

    // Re-check connection after the capability wait: the node may
    // have dropped while we awaited. Same reasoning as above —
    // surface `NodeOffline` for a retryable outage, not the
    // legacy-agent `BadRequest`.
    if !node_ws_manager.is_connected(node_id) {
        return Err(AppError::NodeOffline(format!(
            "Node {node_id} disconnected before no-auth placeholder could be pushed"
        )));
    }

    if node_ws_manager.supports_credential_ack_correlation(node_id) {
        node_ws_manager
            .send_credential_update_and_wait(node_id, &params, std::time::Duration::from_secs(10))
            .await
    } else {
        Err(AppError::BadRequest(format!(
            "Cannot downgrade service '{service_slug}' to no-auth: node '{node_id}' is a legacy \
             agent without credential_ack_correlation support and would keep injecting the old \
             secret. Upgrade the node agent or run `nyxid node credentials remove {service_slug}` \
             on that node first."
        )))
    }
}

/// After an OAuth callback stores a token, find any UserService records
/// for this user + provider that are node-routed, and push the credential.
///
/// This bridges the old provider system with the new UserService model:
/// looks up UserApiKey records that have the same provider_config_id.
pub async fn push_oauth_credential_to_nodes(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_ws_manager: &Arc<NodeWsManager>,
    user_id: &str,
    provider_config_id: &str,
) {
    // Find UserApiKeys linked to this provider
    let api_keys: Vec<UserApiKey> = match db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find(doc! {
            "user_id": user_id,
            "provider_config_id": provider_config_id,
            "status": "active",
            "credential_type": { "$ne": "node_managed" },
        })
        .await
    {
        Ok(cursor) => cursor.try_collect().await.unwrap_or_default(),
        Err(_) => return,
    };

    for api_key in &api_keys {
        push_credential_to_node_if_routed(
            db,
            encryption_keys,
            node_ws_manager,
            user_id,
            &api_key.id,
        )
        .await;
    }
}

/// Build CredentialUpdateParams from a UserService and decrypted credential.
fn build_credential_params(
    svc: &UserService,
    credential: &str,
    target_url: Option<String>,
) -> CredentialUpdateParams {
    build_credential_params_from_fields(
        &svc.slug,
        &svc.auth_method,
        &svc.auth_key_name,
        credential,
        target_url,
    )
}

/// Build `CredentialUpdateParams` from raw fields. Used by the strict
/// push path when the caller has post-update values that don't yet
/// reflect what's in the DB.
fn build_credential_params_from_fields(
    service_slug: &str,
    auth_method: &str,
    auth_key_name: &str,
    credential: &str,
    target_url: Option<String>,
) -> CredentialUpdateParams {
    match auth_method {
        "bearer" => CredentialUpdateParams {
            service_slug: service_slug.to_string(),
            injection_method: "header".to_string(),
            // Bearer auth is always injected under `Authorization`, matching
            // the direct-proxy behavior and mirroring how `basic` is handled
            // below. Stored `auth_key_name` may still be empty on services
            // originally created with `auth_method: "none"` or carry a
            // stale value from a previous `header`-auth configuration —
            // using it verbatim would push `Bearer …` under the wrong
            // header and break node-side auth (twenty-first-round Codex
            // P2). Users who want a custom header for a bearer-like
            // token should register the service with `auth_method:
            // "header"` and embed the `Bearer ` prefix in the credential.
            header_name: Some("Authorization".to_string()),
            header_value: Some(format!("Bearer {credential}")),
            param_name: None,
            param_value: None,
            target_url,
        },
        "header" => CredentialUpdateParams {
            service_slug: service_slug.to_string(),
            injection_method: "header".to_string(),
            header_name: Some(auth_key_name.to_string()),
            header_value: Some(credential.to_string()),
            param_name: None,
            param_value: None,
            target_url,
        },
        "query" => CredentialUpdateParams {
            service_slug: service_slug.to_string(),
            injection_method: "query_param".to_string(),
            header_name: None,
            header_value: None,
            param_name: Some(auth_key_name.to_string()),
            param_value: Some(credential.to_string()),
            target_url,
        },
        "basic" => CredentialUpdateParams {
            service_slug: service_slug.to_string(),
            injection_method: "header".to_string(),
            header_name: Some("Authorization".to_string()),
            header_value: Some(format!("Basic {credential}")),
            param_name: None,
            param_value: None,
            target_url,
        },
        "path" => CredentialUpdateParams {
            service_slug: service_slug.to_string(),
            injection_method: "path_prefix".to_string(),
            header_name: Some(auth_key_name.to_string()),
            header_value: Some(credential.to_string()),
            param_name: None,
            param_value: None,
            target_url,
        },
        "none" => CredentialUpdateParams {
            // No-auth placeholder: node keeps the slug entry (so
            // `proxy_executor` can resolve `target_url`) but stops
            // injecting any secret. Used when a server-held service
            // is downgraded to `auth_method: "none"` on the same
            // node (thirty-third-round Codex P1).
            service_slug: service_slug.to_string(),
            injection_method: "none".to_string(),
            header_name: None,
            header_value: None,
            param_name: None,
            param_value: None,
            target_url,
        },
        // AWS cloud-billing auth method (NyxID#716 + Codex review
        // BLOCKER 6): the JSON credential blob rides on `header_value`
        // (mirroring the node-side `CredentialConfig::new_aws_sigv4`
        // constructor that reuses the `header_value_encrypted` TOML
        // field). The node agent's ws_client + credential_store
        // re-parse the JSON inside proxy_executor at signing time.
        "aws_sigv4" => CredentialUpdateParams {
            service_slug: service_slug.to_string(),
            injection_method: "aws_sigv4".to_string(),
            header_name: None,
            header_value: Some(credential.to_string()),
            param_name: None,
            param_value: None,
            target_url,
        },
        _ => CredentialUpdateParams {
            service_slug: service_slug.to_string(),
            injection_method: "header".to_string(),
            header_name: Some(auth_key_name.to_string()),
            header_value: Some(credential.to_string()),
            param_name: None,
            param_value: None,
            target_url,
        },
    }
}

/// Decrypt the active credential from a UserApiKey.
async fn decrypt_api_key_credential(
    api_key: &UserApiKey,
    encryption_keys: &EncryptionKeys,
) -> Option<String> {
    let encrypted = match api_key.credential_type.as_str() {
        "oauth2" => api_key.access_token_encrypted.as_ref(),
        _ => api_key.credential_encrypted.as_ref(),
    }?;

    let decrypted_bytes = match encryption_keys.decrypt(encrypted).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to decrypt credential for push");
            return None;
        }
    };

    String::from_utf8(decrypted_bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ────────────────────────────────────────────────────────────────────
    // ActorScope — pure unit tests
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn unrestricted_permits_any_node() {
        let scope = ActorScope::unrestricted();
        assert!(scope.permits_node("node-1"));
        assert!(scope.permits_node("node-999"));
        assert!(scope.permits_node(""));
    }

    #[test]
    fn unrestricted_permits_any_service() {
        let scope = ActorScope::unrestricted();
        assert!(scope.permits_service("svc-1"));
        assert!(scope.permits_service("svc-999"));
        assert!(scope.permits_service(""));
    }

    #[test]
    fn scoped_node_allows_listed_ids() {
        let scope = ActorScope {
            allow_all_nodes: false,
            allowed_node_ids: vec!["node-a".to_string(), "node-b".to_string()],
            allow_all_services: true,
            allowed_service_ids: vec![],
        };
        assert!(scope.permits_node("node-a"));
        assert!(scope.permits_node("node-b"));
        assert!(!scope.permits_node("node-c"));
    }

    #[test]
    fn scoped_service_allows_listed_ids() {
        let scope = ActorScope {
            allow_all_nodes: true,
            allowed_node_ids: vec![],
            allow_all_services: false,
            allowed_service_ids: vec!["svc-x".to_string(), "svc-y".to_string()],
        };
        assert!(scope.permits_service("svc-x"));
        assert!(scope.permits_service("svc-y"));
        assert!(!scope.permits_service("svc-z"));
    }

    #[test]
    fn empty_allowed_lists_with_allow_all_false_denies_everything() {
        let scope = ActorScope {
            allow_all_nodes: false,
            allowed_node_ids: vec![],
            allow_all_services: false,
            allowed_service_ids: vec![],
        };
        assert!(!scope.permits_node("node-1"));
        assert!(!scope.permits_service("svc-1"));
        assert!(!scope.permits_node(""));
        assert!(!scope.permits_service(""));
    }

    #[test]
    fn allow_all_nodes_overrides_empty_allowed_list() {
        let scope = ActorScope {
            allow_all_nodes: true,
            allowed_node_ids: vec![],
            allow_all_services: false,
            allowed_service_ids: vec![],
        };
        assert!(scope.permits_node("any-node"));
        assert!(!scope.permits_service("any-service"));
    }

    #[test]
    fn allow_all_services_overrides_empty_allowed_list() {
        let scope = ActorScope {
            allow_all_nodes: false,
            allowed_node_ids: vec![],
            allow_all_services: true,
            allowed_service_ids: vec![],
        };
        assert!(!scope.permits_node("any-node"));
        assert!(scope.permits_service("any-service"));
    }

    #[test]
    fn duplicate_ids_in_allowed_list_still_permits() {
        let scope = ActorScope {
            allow_all_nodes: false,
            allowed_node_ids: vec!["node-a".to_string(), "node-a".to_string()],
            allow_all_services: false,
            allowed_service_ids: vec!["svc-x".to_string(), "svc-x".to_string()],
        };
        assert!(scope.permits_node("node-a"));
        assert!(scope.permits_service("svc-x"));
        assert!(!scope.permits_node("node-b"));
        assert!(!scope.permits_service("svc-y"));
    }

    #[test]
    fn permits_node_is_case_sensitive() {
        let scope = ActorScope {
            allow_all_nodes: false,
            allowed_node_ids: vec!["Node-A".to_string()],
            allow_all_services: true,
            allowed_service_ids: vec![],
        };
        assert!(scope.permits_node("Node-A"));
        assert!(!scope.permits_node("node-a"));
        assert!(!scope.permits_node("NODE-A"));
    }

    #[test]
    fn permits_service_is_case_sensitive() {
        let scope = ActorScope {
            allow_all_nodes: true,
            allowed_node_ids: vec![],
            allow_all_services: false,
            allowed_service_ids: vec!["Svc-X".to_string()],
        };
        assert!(scope.permits_service("Svc-X"));
        assert!(!scope.permits_service("svc-x"));
    }

    // ────────────────────────────────────────────────────────────────────
    // build_credential_params_from_fields — pure unit tests
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn build_params_bearer_uses_authorization_header() {
        let params =
            build_credential_params_from_fields("my-svc", "bearer", "ignored", "tok123", None);
        assert_eq!(params.injection_method, "header");
        assert_eq!(params.header_name, Some("Authorization".to_string()));
        assert_eq!(params.header_value, Some("Bearer tok123".to_string()));
        assert!(params.param_name.is_none());
        assert!(params.param_value.is_none());
        assert!(params.target_url.is_none());
    }

    #[test]
    fn build_params_header_uses_custom_header_name() {
        let params = build_credential_params_from_fields(
            "my-svc",
            "header",
            "X-API-Key",
            "secret",
            Some("https://api.example.com".to_string()),
        );
        assert_eq!(params.injection_method, "header");
        assert_eq!(params.header_name, Some("X-API-Key".to_string()));
        assert_eq!(params.header_value, Some("secret".to_string()));
        assert_eq!(
            params.target_url,
            Some("https://api.example.com".to_string())
        );
    }

    #[test]
    fn build_params_query_uses_param_fields() {
        let params =
            build_credential_params_from_fields("my-svc", "query", "api_key", "secret", None);
        assert_eq!(params.injection_method, "query_param");
        assert!(params.header_name.is_none());
        assert!(params.header_value.is_none());
        assert_eq!(params.param_name, Some("api_key".to_string()));
        assert_eq!(params.param_value, Some("secret".to_string()));
    }

    #[test]
    fn build_params_basic_uses_authorization_header() {
        let params =
            build_credential_params_from_fields("my-svc", "basic", "ignored", "dXNlcjpwYXNz", None);
        assert_eq!(params.injection_method, "header");
        assert_eq!(params.header_name, Some("Authorization".to_string()));
        assert_eq!(params.header_value, Some("Basic dXNlcjpwYXNz".to_string()));
    }

    #[test]
    fn build_params_path_uses_path_prefix() {
        let params = build_credential_params_from_fields("tg-bot", "path", "bot", "tok123", None);
        assert_eq!(params.injection_method, "path_prefix");
        assert_eq!(params.header_name, Some("bot".to_string()));
        assert_eq!(params.header_value, Some("tok123".to_string()));
    }

    #[test]
    fn build_params_none_has_no_credentials() {
        let params = build_credential_params_from_fields(
            "my-svc",
            "none",
            "",
            "",
            Some("https://example.com".to_string()),
        );
        assert_eq!(params.injection_method, "none");
        assert!(params.header_name.is_none());
        assert!(params.header_value.is_none());
        assert!(params.param_name.is_none());
        assert!(params.param_value.is_none());
        assert_eq!(params.target_url, Some("https://example.com".to_string()));
    }

    #[test]
    fn build_params_aws_sigv4_carries_credential_in_header_value() {
        let cred = r#"{"access_key":"AK","secret_key":"SK","region":"us-east-1"}"#;
        let params = build_credential_params_from_fields("bedrock", "aws_sigv4", "", cred, None);
        assert_eq!(params.injection_method, "aws_sigv4");
        assert!(params.header_name.is_none());
        assert_eq!(params.header_value, Some(cred.to_string()));
        assert!(params.param_name.is_none());
    }

    #[test]
    fn build_params_unknown_method_falls_back_to_header() {
        let params = build_credential_params_from_fields(
            "my-svc",
            "custom_unknown",
            "X-Custom",
            "value",
            None,
        );
        assert_eq!(params.injection_method, "header");
        assert_eq!(params.header_name, Some("X-Custom".to_string()));
        assert_eq!(params.header_value, Some("value".to_string()));
    }

    #[test]
    fn build_params_preserves_service_slug() {
        for method in [
            "bearer",
            "header",
            "query",
            "basic",
            "path",
            "none",
            "aws_sigv4",
        ] {
            let params =
                build_credential_params_from_fields("test-slug", method, "key", "val", None);
            assert_eq!(params.service_slug, "test-slug", "method={method}");
        }
    }

    // --- build_credential_params (via UserService) ---

    #[test]
    fn build_params_from_user_service_delegates_correctly() {
        let svc = UserService {
            id: "svc-1".to_string(),
            user_id: "user-1".to_string(),
            slug: "openai".to_string(),
            endpoint_id: "ep-1".to_string(),
            api_key_id: Some("ak-1".to_string()),
            auth_method: "bearer".to_string(),
            auth_key_name: "Authorization".to_string(),
            catalog_service_id: None,
            node_id: Some("node-1".to_string()),
            node_priority: 0,
            service_type: "http".to_string(),
            ssh_auth_mode: crate::models::ssh_auth_mode::SshAuthMode::ProxyOnly,
            admin_only: false,
            ssh_node_keys_stale: false,
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            is_active: true,
            source: None,
            source_id: None,
            source_app_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let params = build_credential_params(
            &svc,
            "sk-test-key",
            Some("https://api.openai.com".to_string()),
        );
        assert_eq!(params.service_slug, "openai");
        assert_eq!(params.injection_method, "header");
        assert_eq!(params.header_name, Some("Authorization".to_string()));
        assert_eq!(params.header_value, Some("Bearer sk-test-key".to_string()));
        assert_eq!(
            params.target_url,
            Some("https://api.openai.com".to_string())
        );
    }
}

#[cfg(test)]
mod no_auth_strict_push_tests {
    use super::*;
    use crate::services::node_ws_manager::{
        CredentialAckOutcome, NodeCapabilitiesMsg, NodeOutboundMessage,
    };
    use serde_json::Value;
    use tokio::sync::mpsc;

    /// Simulate a connected node that has advertised
    /// `credential_ack_correlation` (the modern agent) and acks any
    /// `credential_update` with Ok. Returns both the manager (wrapped
    /// in Arc for the caller) and the rx half so tests can inspect the
    /// frame that was sent.
    fn spawn_modern_agent() -> (
        Arc<NodeWsManager>,
        mpsc::Receiver<NodeOutboundMessage>,
        &'static str,
    ) {
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let (tx, rx) = mpsc::channel(256);
        mgr.register_connection("node-1", tx);
        mgr.record_capabilities(
            "node-1",
            &NodeCapabilitiesMsg {
                credential_ack_correlation: true,
                remote_credential_crypto_v1: false,
            },
        );
        mgr.mark_status_update_received("node-1");
        (mgr, rx, "node-1")
    }

    #[tokio::test]
    async fn strict_no_auth_push_sends_none_injection_and_awaits_ack() {
        let (mgr, mut rx, node_id) = spawn_modern_agent();

        let mgr_for_responder = mgr.clone();
        let ack_task = tokio::spawn(async move {
            let Some(NodeOutboundMessage::Text(msg)) = rx.recv().await else {
                panic!("expected credential_update frame");
            };
            let parsed: Value = serde_json::from_str(&msg).expect("valid json");
            assert_eq!(parsed["type"], "credential_update");
            assert_eq!(parsed["service_slug"], "demo");
            assert_eq!(parsed["injection_method"], "none");
            assert_eq!(parsed["target_url"], "https://new.example.com");
            // header/param fields MUST be absent so a legacy path
            // that mishandles them never sees a stale secret.
            assert!(parsed.get("header_name").is_none());
            assert!(parsed.get("header_value").is_none());
            assert!(parsed.get("param_name").is_none());
            assert!(parsed.get("param_value").is_none());

            let request_id = parsed["request_id"].as_str().expect("request id echoed");
            mgr_for_responder.deliver_credential_ack(node_id, request_id, CredentialAckOutcome::Ok);
            msg
        });

        let result =
            push_no_auth_to_node_strict(&mgr, node_id, "demo", Some("https://new.example.com"))
                .await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        ack_task.await.expect("responder panicked");
    }

    #[tokio::test]
    async fn strict_no_auth_push_rejects_legacy_agent_with_actionable_error() {
        // Connected but no capabilities recorded → legacy agent.
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let (tx, _rx) = mpsc::channel(256);
        mgr.register_connection("legacy-node", tx);
        mgr.mark_status_update_received("legacy-node");

        let err =
            push_no_auth_to_node_strict(&mgr, "legacy-node", "demo", Some("https://example.com"))
                .await
                .expect_err("legacy agent should reject no-auth downgrade");

        // We specifically surface BadRequest (not NodeOffline) so the
        // user sees a clear, actionable error instead of "transient".
        let AppError::BadRequest(msg) = err else {
            panic!("expected BadRequest, got {err:?}");
        };
        assert!(
            msg.contains("legacy agent"),
            "error must flag legacy-agent root cause: {msg}"
        );
        assert!(
            msg.contains("credentials remove"),
            "error must tell the user how to recover: {msg}"
        );
    }

    #[tokio::test]
    async fn strict_no_auth_push_disconnected_node_surfaces_as_node_offline() {
        // A disconnected node must NOT be confused with a legacy
        // agent: the user should be told the node is offline (a
        // retryable, transient condition) rather than being nudged to
        // upgrade or manually remove credentials. Both conditions
        // cause `supports_credential_ack_correlation` to return
        // false, so the helper has to check connection state
        // explicitly — this test locks that in.
        let mgr = Arc::new(NodeWsManager::new(30, 100));
        let err = push_no_auth_to_node_strict(&mgr, "missing-node", "demo", None)
            .await
            .expect_err("disconnected node must fail the strict downgrade");

        let AppError::NodeOffline(msg) = err else {
            panic!("expected NodeOffline for disconnected node, got {err:?}");
        };
        assert!(
            msg.contains("missing-node"),
            "error should name the offline node: {msg}"
        );
    }
}
