//! Org membership and ownership-resolution service.
//!
//! In the NyxID "Org = User" model, an organization is a [`User`] with
//! `user_type = Org`. The `org_memberships` collection records which person
//! users belong to each org and what role they have. This service is the
//! single source of truth for:
//!
//! 1. Org user creation (the org's underlying `users` row).
//! 2. Membership CRUD (add / list / role change / revoke).
//! 3. The proxy fallback query (with a wall-clock timeout) used by
//!    [`crate::services::proxy_service`] when a personal `UserService`
//!    lookup misses.
//! 4. The [`OwnerAccess`] helper that other handlers use to extend their
//!    "must be owner" checks to "owner OR admin of the owning org".

use std::time::Duration;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::FindOptions;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::org_membership::{
    COLLECTION_NAME as ORG_MEMBERSHIPS, MemberScopeSource, OrgMembership, OrgRole,
};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::services::org_slug;

/// Wall-clock timeout for the proxy fallback membership query.
///
/// Users with their own personal `UserService` never reach this query (the
/// personal lookup short-circuits first). Only users without a personal
/// match pay this round-trip. The cap bounds blast radius if MongoDB is
/// degraded -- proxy 404s for non-org users return in bounded time rather
/// than hanging.
pub const ORG_FALLBACK_TIMEOUT: Duration = Duration::from_millis(500);

// ─────────────────────────────────────────────────────────────────────────────
// Org user CRUD
// ─────────────────────────────────────────────────────────────────────────────

/// Create the underlying `User` row for a new org and return its id.
///
/// Org users have `user_type = Org`, no password, no MFA, and cannot log in
/// via any auth path -- those are blocked in `auth_service::ensure_person_user`.
pub async fn create_org_user(
    db: &mongodb::Database,
    display_name: &str,
    contact_email: Option<&str>,
    avatar_url: Option<&str>,
) -> AppResult<User> {
    let trimmed_name = display_name.trim();
    if trimmed_name.is_empty() {
        return Err(AppError::ValidationError(
            "Org display name is required".to_string(),
        ));
    }
    if trimmed_name.len() > 128 {
        return Err(AppError::ValidationError(
            "Org display name must be at most 128 characters".to_string(),
        ));
    }

    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    let slug = org_slug::reserve_slug(db, &org_slug::slugify(trimmed_name), None).await?;
    // Synthetic placeholder when the admin doesn't provide a contact email.
    // The partial-unique index on `users.email` only constrains
    // user_type=person, so collisions across orgs are fine -- but we still
    // generate a unique-looking value to keep logs and admin UIs readable.
    let email = contact_email
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| synthetic_org_email(&id));

    let org = User {
        id: id.clone(),
        email,
        password_hash: None,
        display_name: Some(trimmed_name.to_string()),
        slug: Some(slug),
        avatar_url: avatar_url.map(|s| s.to_string()),
        email_verified: false,
        email_verification_token: None,
        password_reset_token: None,
        password_reset_expires_at: None,
        is_active: true,
        is_admin: false,
        is_operator: false,
        role_ids: vec![],
        group_ids: vec![],
        invite_code_id: None,
        mfa_enabled: false,
        social_provider: None,
        social_provider_id: None,
        user_type: UserType::Org,
        primary_org_id: None,
        created_at: now,
        updated_at: now,
        last_login_at: None,
        profile_config: Default::default(),
    };

    db.collection::<User>(USERS).insert_one(&org).await?;
    tracing::info!(org_user_id = %id, "Org user created");
    Ok(org)
}

/// Look up an org user by id. Returns `OrgNotFound` when missing or the
/// user is not actually an org account.
pub async fn get_org_user(db: &mongodb::Database, org_user_id: &str) -> AppResult<User> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": org_user_id })
        .await?
        .ok_or_else(|| AppError::OrgNotFound(org_user_id.to_string()))?;
    if !user.user_type.is_org() {
        return Err(AppError::OrgNotFound(org_user_id.to_string()));
    }
    Ok(user)
}

/// Look up an org by UUID or slug.
pub async fn find_org_by_key(db: &mongodb::Database, key: &str) -> AppResult<User> {
    if Uuid::parse_str(key).is_ok() {
        return get_org_user(db, key).await;
    }

    db.collection::<User>(USERS)
        .find_one(doc! { "user_type": "org", "slug": key })
        .await?
        .ok_or_else(|| AppError::OrgNotFound(key.to_string()))
}

/// Suffix used for the synthetic placeholder email generated when an org is
/// created without an explicit contact email. Kept as a const so UI/API
/// normalizers can hide it behind a single check.
pub const ORG_PLACEHOLDER_EMAIL_SUFFIX: &str = "@nyxid.local";

/// Build the exact synthetic placeholder email for an org with the given
/// id. The only value `create_org_user` ever generates is
/// `org-<id>@nyxid.local`, so normalizers compare against this exact string
/// rather than a `starts_with("org-") && ends_with("@nyxid.local")` pattern
/// — the loose check incorrectly hides legitimate user-supplied addresses
/// like `org-support@nyxid.local`.
fn synthetic_org_email(org_user_id: &str) -> String {
    format!("org-{}{}", org_user_id, ORG_PLACEHOLDER_EMAIL_SUFFIX)
}

/// Return the org's user-visible contact email, or `None` when the stored
/// email is the synthetic `org-<id>@nyxid.local` placeholder for *this*
/// org. Any other address — including a real `org-support@nyxid.local`
/// the admin explicitly set — is returned verbatim.
pub fn contact_email_for_display(user: &User) -> Option<String> {
    let email = user.email.trim();
    if email.is_empty() {
        return None;
    }
    if email.eq_ignore_ascii_case(&synthetic_org_email(&user.id)) {
        return None;
    }
    Some(email.to_string())
}

/// Update org metadata. Supports `display_name`, `avatar_url`, and
/// `contact_email`. Pass `Some(value)` to set/clear each field; `None` leaves
/// the field untouched. For `contact_email`, an empty string clears back to
/// the synthetic placeholder so audit/admin surfaces stay legible.
pub async fn update_org_user(
    db: &mongodb::Database,
    org_user_id: &str,
    display_name: Option<&str>,
    slug: Option<&str>,
    avatar_url: Option<&str>,
    contact_email: Option<&str>,
    remote_credential_integrity_verification_opt_out: Option<bool>,
) -> AppResult<User> {
    // Verify it's an org first.
    let existing = get_org_user(db, org_user_id).await?;

    let mut update = doc! {};
    if let Some(name) = display_name {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(AppError::ValidationError(
                "Org display name cannot be empty".to_string(),
            ));
        }
        update.insert("display_name", trimmed);
    }
    if let Some(slug) = slug {
        update.insert("slug", slug);
    }
    if let Some(avatar) = avatar_url {
        let trimmed = avatar.trim();
        if trimmed.is_empty() {
            update.insert("avatar_url", bson::Bson::Null);
        } else {
            update.insert("avatar_url", trimmed);
        }
    }
    if let Some(email) = contact_email {
        let trimmed = email.trim();
        if trimmed.is_empty() {
            // Restore the synthetic placeholder so admin/audit surfaces
            // still show a stable, unique-looking identifier.
            update.insert("email", synthetic_org_email(&existing.id));
        } else {
            // Defensive validation — handler-level validator already runs,
            // but this keeps the service safe for direct callers.
            if !trimmed.contains('@') {
                return Err(AppError::ValidationError(
                    "contact_email must be a valid email".to_string(),
                ));
            }
            update.insert("email", trimmed.to_lowercase());
        }
    }
    if let Some(opt_out) = remote_credential_integrity_verification_opt_out {
        update.insert(
            "profile_config.release_integrity.remote_credential_integrity_verification_opt_out",
            opt_out,
        );
    }
    update.insert("updated_at", bson::DateTime::from_chrono(Utc::now()));

    db.collection::<User>(USERS)
        .update_one(doc! { "_id": org_user_id }, doc! { "$set": update })
        .await?;

    get_org_user(db, org_user_id).await
}

pub fn remote_credential_integrity_verification_opt_out(user: &User) -> bool {
    user.user_type.is_org()
        && user
            .profile_config
            .release_integrity
            .remote_credential_integrity_verification_opt_out
}

/// Delete an org user.
///
/// Refuses to delete when the org still owns *live* shared resources.
/// Without the org user record, `resolve_owner_access` would treat
/// surviving resources as orphaned and deny every read/write -- nobody
/// could clean them up through the API. We therefore force the admin
/// to transfer or delete the live ones first, and cascade-delete the
/// historical state that has no meaning without the org.
///
/// **Blockers** (must be empty before deletion proceeds): *active*
/// user services / legacy service connections / NyxID API keys /
/// service accounts / developer OAuth clients / channel bots /
/// channel conversations / credential nodes / custom catalog
/// services (all soft-deleted via `is_active = false`), *non-revoked*
/// provider tokens, hard-deleted endpoints / external API keys /
/// per-service approval configs, *active* approval grants, and
/// *pending* approval requests. The soft-delete filters are critical
/// here -- without them, an org that ever had a service would be
/// permanently undeletable, because the soft-deleted row stays in
/// the collection forever.
///
/// **Cascaded** (deleted alongside the org user record): memberships,
/// invites, decided approval requests (approved/rejected/expired),
/// expired/revoked approval grants, soft-deleted blocker tombstones
/// (user services, legacy service connections, API keys, service
/// accounts, OAuth clients, bots, conversations, nodes, custom
/// catalog services), `service_endpoints` and
/// `service_provider_requirements` joined through the org's owned
/// downstream service ids, agent service bindings, service-account
/// tokens and SA-owned provider tokens (joined through the org's
/// owned SA ids), refresh_tokens and consent grants joined through
/// the org's owned developer OAuth client ids,
/// oauth_states for in-flight provider connect flows (matched on
/// `user_id` OR `target_user_id` for org-targeted flows), all
/// `user_provider_tokens` owned by the org (closes the in-flight
/// OAuth callback race), channel messages, channel event logs
/// (joined through the org's conversation ids), OpenClaw channel
/// mappings, the notification channel row, all node registration
/// tokens, all node service bindings owned by the org, and
/// user-provided OAuth client credentials. These rows are dead state
/// once the org is gone; no API call could ever read or mutate them
/// again. The audit log lives in its own collection and survives
/// intact.
pub async fn delete_org_user(db: &mongodb::Database, org_user_id: &str) -> AppResult<()> {
    let _ = get_org_user(db, org_user_id).await?;

    // (collection_name, blocker_filter, human_label)
    // Each blocker_filter is a doc that already includes the owner check
    // AND the live-state filter for that collection's delete semantics
    // (soft vs. hard). See the doc comment above for which collections
    // use which.
    let now_bson = bson::DateTime::from_chrono(Utc::now());
    let blocker_specs: Vec<(&str, mongodb::bson::Document, &str)> = vec![
        (
            crate::models::user_service::COLLECTION_NAME,
            // Soft-deleted UserServices stay in the collection with
            // `is_active = false` -- those must NOT block deletion.
            doc! { "user_id": org_user_id, "is_active": true },
            "user services",
        ),
        (
            crate::models::user_service_connection::COLLECTION_NAME,
            // Legacy pre-migration credential. Still treated as a live
            // credential by `proxy_service::user_has_legacy_personal_connection`.
            // Soft-deleted via `is_active = false`. The admin must call
            // `DELETE /connections/{service_id}` first.
            doc! { "user_id": org_user_id, "is_active": true },
            "legacy service connections",
        ),
        (
            crate::models::user_endpoint::COLLECTION_NAME,
            // Hard-deleted; no live filter needed.
            doc! { "user_id": org_user_id },
            "endpoints",
        ),
        (
            crate::models::user_api_key::COLLECTION_NAME,
            // Hard-deleted; no live filter needed.
            doc! { "user_id": org_user_id },
            "external API keys",
        ),
        (
            crate::models::api_key::COLLECTION_NAME,
            // Soft-deleted via `is_active = false`.
            doc! { "user_id": org_user_id, "is_active": true },
            "NyxID API keys",
        ),
        (
            crate::models::user_provider_token::COLLECTION_NAME,
            // Soft-deleted via `status = "revoked"`.
            doc! { "user_id": org_user_id, "status": { "$ne": "revoked" } },
            "provider tokens",
        ),
        (
            crate::models::service_approval_config::COLLECTION_NAME,
            // Hard-deleted; no live filter needed.
            doc! { "user_id": org_user_id },
            "approval configs",
        ),
        (
            crate::models::approval_grant::COLLECTION_NAME,
            doc! {
                "user_id": org_user_id,
                "revoked": false,
                "expires_at": { "$gt": &now_bson },
            },
            "active approval grants",
        ),
        (
            crate::models::approval_request::COLLECTION_NAME,
            doc! { "user_id": org_user_id, "status": "pending" },
            "pending approval requests",
        ),
        (
            crate::models::service_account::COLLECTION_NAME,
            // Soft-deleted via `is_active = false`.
            doc! { "owner_user_id": org_user_id, "is_active": true },
            "service accounts",
        ),
        (
            crate::models::oauth_client::COLLECTION_NAME,
            // Soft-deleted via `is_active = false`.
            doc! { "created_by": org_user_id, "is_active": true },
            "developer OAuth clients",
        ),
        (
            crate::models::channel_bot::COLLECTION_NAME,
            // Soft-deleted via `is_active = false`. Active bots have a
            // live platform webhook pointing at this NyxID instance --
            // the admin must `DELETE /channel-bots/{id}` first so the
            // webhook is deregistered on the platform side. After org
            // deletion there's no API path that can do that cleanup,
            // so we refuse to leave a dangling integration behind.
            doc! { "user_id": org_user_id, "is_active": true },
            "channel bots",
        ),
        (
            crate::models::channel_conversation::COLLECTION_NAME,
            // Soft-deleted via `is_active = false`. Conversations are
            // routing rules; an active row is reachable to inbound
            // webhooks even after the bot itself is deactivated, so
            // the admin must clean these up alongside the bots.
            doc! { "user_id": org_user_id, "is_active": true },
            "channel conversations",
        ),
        (
            crate::models::node::COLLECTION_NAME,
            // Soft-deleted via `is_active = false`. An active node row
            // is what `node_service::authenticate_node` consults on
            // every WS reconnect, so a dangling org-owned node would
            // keep accepting agent connections and proxying traffic
            // on behalf of a non-existent org. The admin must call
            // `DELETE /nodes/{id}` first, which deactivates the node
            // and its bindings; the agent fails on next heartbeat.
            doc! { "user_id": org_user_id, "is_active": true },
            "credential nodes",
        ),
        (
            crate::models::downstream_service::COLLECTION_NAME,
            // Custom catalog entries created via `POST /services` by
            // an org-owned API key. Soft-deleted via `is_active =
            // false`. An *active* row stays visible to every other
            // authenticated user via the normal `/services` listing
            // (the visibility filter doesn't depend on the creator
            // being alive), and once the org user is gone the
            // built-in `require_admin_or_creator` cleanup gate fails
            // for everyone except a global admin. Force the admin to
            // call `DELETE /services/{id}` first.
            doc! { "created_by": org_user_id, "is_active": true },
            "custom catalog services",
        ),
    ];

    let mut blockers: Vec<String> = Vec::new();
    for (collection_name, filter, label) in &blocker_specs {
        let count = db
            .collection::<bson::Document>(collection_name)
            .count_documents(filter.clone())
            .await?;
        if count > 0 {
            blockers.push(format!("{count} {label}"));
        }
    }

    if !blockers.is_empty() {
        return Err(AppError::Conflict(format!(
            "Cannot delete org while it still owns {}. Transfer or delete them first.",
            blockers.join(", ")
        )));
    }

    // Cascade dead state once the live blockers are clear. Historical
    // approval requests and dead grants would otherwise be unreachable
    // through the API (`resolve_owner_access` no longer recognizes the
    // org), so deleting them keeps the audit log -- which lives in its
    // own collection -- as the only record.
    db.collection::<bson::Document>(crate::models::approval_request::COLLECTION_NAME)
        .delete_many(doc! {
            "user_id": org_user_id,
            "status": { "$in": ["approved", "rejected", "expired"] },
        })
        .await?;
    db.collection::<bson::Document>(crate::models::approval_grant::COLLECTION_NAME)
        .delete_many(doc! {
            "user_id": org_user_id,
            "$or": [
                { "revoked": true },
                { "expires_at": { "$lte": &now_bson } },
            ],
        })
        .await?;
    // Cascade soft-deleted blocker rows. The live blocker check above
    // already filtered them out, so they're tombstones referencing the
    // about-to-be-deleted org user_id. Leaving them behind would
    // accumulate dangling rows in MongoDB; the API can never reach
    // them after the org user is gone.
    db.collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id, "is_active": false })
        .await?;
    db.collection::<bson::Document>(crate::models::user_service_connection::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id, "is_active": false })
        .await?;
    db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id, "is_active": false })
        .await?;
    // Agent service bindings reference an api_key_id but carry a
    // denormalized `user_id` for query efficiency, so we can clean
    // them up by org user_id directly. The api_keys delete above
    // doesn't touch them; the binding API has no way to reach them
    // once the parent api_key tombstone is gone.
    db.collection::<bson::Document>(crate::models::agent_service_binding::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id })
        .await?;
    // OAuth states for in-flight provider connect flows. Cascade
    // EARLY so any callback or device-code poll that arrives after
    // this point cannot consume a still-valid state row and create a
    // fresh `user_provider_tokens` entry for the about-to-be-deleted
    // org. Match on either `user_id` or `target_user_id` because
    // org-targeted flows store the org id in `target_user_id` (with
    // `user_id` set to the human admin who initiated the flow).
    //
    // The matching `user_provider_tokens` cascade below uses an
    // unfiltered `user_id` match (rather than the previous "revoked
    // only" filter) so that any token row that managed to land in
    // the small race window between this cascade and the user
    // record delete still gets cleaned up.
    db.collection::<bson::Document>(crate::models::oauth_state::COLLECTION_NAME)
        .delete_many(doc! {
            "$or": [
                { "user_id": org_user_id },
                { "target_user_id": org_user_id },
            ],
        })
        .await?;
    // Service accounts: snapshot owned ids BEFORE deleting the SA rows
    // so we can clean up `service_account_tokens` AND any SA-owned
    // `user_provider_tokens` afterwards. The SA delete path only marks
    // SA tokens as `revoked: true` and never touches provider tokens.
    // SA-owned provider tokens are stored with `user_id == sa_id`
    // (see `handlers/admin_sa_providers::store_api_key` etc.), so the
    // snapshot is the only path back to them once the SA row is gone.
    // Mirrors the cleanup that `admin_user_service::delete_user` does
    // for person users.
    let owned_sa_ids: Vec<String> = db
        .collection::<bson::Document>(crate::models::service_account::COLLECTION_NAME)
        .distinct("_id", doc! { "owner_user_id": org_user_id })
        .await?
        .into_iter()
        .filter_map(|value| match value {
            bson::Bson::String(id) => Some(id),
            _ => None,
        })
        .collect();
    db.collection::<bson::Document>(crate::models::service_account::COLLECTION_NAME)
        .delete_many(doc! { "owner_user_id": org_user_id, "is_active": false })
        .await?;
    if !owned_sa_ids.is_empty() {
        let sa_id_array: Vec<bson::Bson> = owned_sa_ids
            .iter()
            .cloned()
            .map(bson::Bson::String)
            .collect();
        db.collection::<bson::Document>(crate::models::service_account_token::COLLECTION_NAME)
            .delete_many(doc! { "service_account_id": { "$in": &sa_id_array } })
            .await?;
        // SA-owned provider tokens (user_provider_tokens.user_id is
        // overloaded with the SA id for SA-owned connections).
        db.collection::<bson::Document>(crate::models::user_provider_token::COLLECTION_NAME)
            .delete_many(doc! { "user_id": { "$in": &sa_id_array } })
            .await?;
        // Same overload for any in-flight oauth states the SA may
        // have started but never finished.
        db.collection::<bson::Document>(crate::models::oauth_state::COLLECTION_NAME)
            .delete_many(doc! {
                "$or": [
                    { "user_id": { "$in": &sa_id_array } },
                    { "target_user_id": { "$in": &sa_id_array } },
                ],
            })
            .await?;
    }
    // Drop ALL provider tokens owned by the org, not just revoked ones.
    // The blocker check above already required no non-revoked tokens at
    // start time, so what's caught here is the union of (a) the revoked
    // tombstones we used to clean up and (b) anything that landed in
    // the race window between the blocker check and this cascade --
    // most plausibly an in-flight OAuth callback that beat the
    // `oauth_states` cascade above.
    db.collection::<bson::Document>(crate::models::user_provider_token::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id })
        .await?;
    // Developer OAuth clients: snapshot owned client ids BEFORE deleting
    // the client tombstones so we can clean up `refresh_tokens` whose
    // `client_id` references them. Without this, refresh tokens minted
    // by an org-owned developer app would linger forever -- the live
    // validation in `token_service::refresh_tokens` already rejects
    // them on the next refresh attempt, so this is belt-and-suspenders
    // cleanup that keeps the collection from accumulating dead rows.
    let owned_oauth_client_ids: Vec<String> = db
        .collection::<bson::Document>(crate::models::oauth_client::COLLECTION_NAME)
        .distinct("_id", doc! { "created_by": org_user_id })
        .await?
        .into_iter()
        .filter_map(|value| match value {
            bson::Bson::String(id) => Some(id),
            _ => None,
        })
        .collect();
    db.collection::<bson::Document>(crate::models::oauth_client::COLLECTION_NAME)
        .delete_many(doc! { "created_by": org_user_id, "is_active": false })
        .await?;
    if !owned_oauth_client_ids.is_empty() {
        let oauth_client_id_array: Vec<bson::Bson> = owned_oauth_client_ids
            .iter()
            .cloned()
            .map(bson::Bson::String)
            .collect();
        db.collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
            .delete_many(doc! { "client_id": { "$in": &oauth_client_id_array } })
            .await?;
        // Consent grants users gave to the org's developer apps. Hard-
        // deleted, no DELETE handler that targets them by client, and
        // they remain enumerable via `/consents` and the admin
        // listing after the org is gone (see `handlers/consent.rs`
        // and `handlers/admin.rs`). Cascade by client_id so deleted
        // org apps disappear from every user's "Authorized Apps"
        // surface.
        db.collection::<bson::Document>(crate::models::consent::COLLECTION_NAME)
            .delete_many(doc! { "client_id": { "$in": &oauth_client_id_array } })
            .await?;
    }
    // Credential nodes. Active nodes are blocked above so what remains
    // here is soft-deleted node tombstones plus their associated
    // bindings and any outstanding registration tokens for the org.
    //
    // Order matters: clear the registration tokens BEFORE the user
    // record is deleted at the end of this function, so the WS
    // registration path (`node_service::register_node`) cannot consume
    // a still-valid token and create a fresh node row owned by the
    // about-to-be-deleted org. There is still a small race window
    // between the cascade and the user delete, but the impact is
    // limited to a soft-deleted node row that the next admin cleanup
    // sweep can pick up.
    //
    // Bindings owned by the org also need a cascade even when the
    // physical node is the admin's personal hardware: org-shared
    // services routed through a personal node create a binding row
    // with `user_id = org_user_id` (so proxy resolution finds it
    // under the effective owner). Those rows are useless once the
    // org is gone but they leak forever otherwise.
    db.collection::<bson::Document>(crate::models::node_registration_token::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id })
        .await?;
    db.collection::<bson::Document>(crate::models::node_service_binding::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id })
        .await?;
    db.collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id, "is_active": false })
        .await?;
    // User-provided OAuth client credentials (per-user override of the
    // service-level OAuth app). Hard-deleted, no platform-side cleanup,
    // no DELETE handler that would let the admin clear them ahead of
    // time. The encrypted blobs are useless without the org user, so
    // cascade-only by user_id matches the openclaw_channel_mappings
    // and notification_channels pattern.
    db.collection::<bson::Document>(crate::models::user_provider_credentials::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id })
        .await?;
    // Custom catalog services created via `POST /services` by an
    // org-owned API key. Active rows are blocked above, so what's
    // left here is soft-deleted tombstones plus their child
    // `service_endpoints` and `service_provider_requirements` rows
    // (which key off `service_id`, not `created_by`).
    //
    // Snapshot owned downstream service ids BEFORE deleting the
    // tombstones, then use the snapshot to clean up the children.
    // Without that ordering the children would lose their only path
    // back to the org. Mirrors the `channel_event_logs` and
    // `service_account_tokens` patterns above.
    let owned_service_ids: Vec<String> = db
        .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .distinct("_id", doc! { "created_by": org_user_id })
        .await?
        .into_iter()
        .filter_map(|value| match value {
            bson::Bson::String(id) => Some(id),
            _ => None,
        })
        .collect();
    db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .delete_many(doc! { "created_by": org_user_id, "is_active": false })
        .await?;
    if !owned_service_ids.is_empty() {
        let svc_id_array: Vec<bson::Bson> = owned_service_ids
            .iter()
            .cloned()
            .map(bson::Bson::String)
            .collect();
        db.collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
            .delete_many(doc! { "service_id": { "$in": &svc_id_array } })
            .await?;
        db.collection::<bson::Document>(
            crate::models::service_provider_requirement::COLLECTION_NAME,
        )
        .delete_many(doc! { "service_id": { "$in": &svc_id_array } })
        .await?;
    }
    // Channel relay state. Active rows are blocked above, so what's left
    // here is soft-deleted bot/conversation tombstones plus any
    // append-only message and event-log records that referenced them.
    //
    // Snapshot conversation_ids BEFORE deleting the conversations, then
    // use the snapshot to delete `channel_event_logs` (which key off
    // `conversation_id`, not `user_id`). Without this, deleting the
    // conversations first would leave the event log with no way back to
    // the org -- it'd accumulate forever.
    let conv_ids: Vec<String> = db
        .collection::<crate::models::channel_conversation::ChannelConversation>(
            crate::models::channel_conversation::COLLECTION_NAME,
        )
        .find(doc! { "user_id": org_user_id })
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|c| c.id)
        .collect();
    if !conv_ids.is_empty() {
        let conv_id_array: Vec<bson::Bson> =
            conv_ids.iter().cloned().map(bson::Bson::String).collect();
        db.collection::<bson::Document>(crate::models::channel_event_log::COLLECTION_NAME)
            .delete_many(doc! { "conversation_id": { "$in": &conv_id_array } })
            .await?;
    }
    db.collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id })
        .await?;
    db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id })
        .await?;
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id })
        .await?;
    // OpenClaw channel mappings. Hard-deleted, no blocker, no platform-
    // side cleanup: NyxID never registers anything with OpenClaw -- the
    // user manually pastes the per-mapping webhook secret into their
    // OpenClaw plugin. Once we delete the row the next inbound webhook
    // fails HMAC verification and the user re-creates the mapping if
    // they want. There's also no `DELETE /integrations/openclaw/mappings`
    // endpoint today, so cascade-only is the only sensible move; making
    // this a hard blocker would render any org with a mapping
    // permanently undeletable.
    db.collection::<bson::Document>(crate::services::openclaw_channel_service::MAPPINGS_COLLECTION)
        .delete_many(doc! { "nyxid_user_id": org_user_id })
        .await?;
    // Notification channels. Cascade-only -- there's no blocker because
    // an org user cannot meaningfully receive a notification anyway
    // (the row is dead state from the moment it was created), and the
    // embedded device tokens have no platform-side cleanup beyond
    // letting FCM/APNs garbage-collect dormant subscriptions. The row
    // would otherwise be created when an org-owned API key hits any
    // `/notifications/*` endpoint via `get_or_create_channel`.
    db.collection::<bson::Document>(crate::models::notification_channel::COLLECTION_NAME)
        .delete_many(doc! { "user_id": org_user_id })
        .await?;
    // Cascade memberships
    crate::services::org_role_scope_service::delete_all_for_org(db, org_user_id).await?;
    db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .delete_many(doc! { "org_user_id": org_user_id })
        .await?;
    // Cascade invites
    db.collection::<bson::Document>(crate::models::org_invite::COLLECTION_NAME)
        .delete_many(doc! { "org_user_id": org_user_id })
        .await?;
    // Hard-delete the org user record itself
    db.collection::<User>(USERS)
        .delete_one(doc! { "_id": org_user_id })
        .await?;

    tracing::info!(org_user_id = %org_user_id, "Org user deleted");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Membership CRUD
// ─────────────────────────────────────────────────────────────────────────────

/// Insert a new membership row, or reactivate a previously-revoked one.
///
/// The unique index on `(org_user_id, member_user_id)` covers both active
/// and revoked rows so that audit history is preserved across revoke/rejoin
/// cycles. To allow re-invites without losing that history, this function
/// updates the existing row in place when a revoked entry exists for the
/// same pair: it resets `revoked_at` to null, refreshes `created_at` to the
/// rejoin time, and applies the new role / scope. Active rows still return
/// `Conflict` to surface the obvious mistake.
pub async fn create_membership(
    db: &mongodb::Database,
    org_user_id: &str,
    member_user_id: &str,
    role: OrgRole,
    scope_source: MemberScopeSource,
    allowed_service_ids: Option<Vec<String>>,
) -> AppResult<OrgMembership> {
    let allowed_service_ids = if scope_source == MemberScopeSource::Inherit {
        None
    } else {
        allowed_service_ids
    };

    // Validate the org actually exists and is an org.
    let _ = get_org_user(db, org_user_id).await?;
    // Validate the member exists and is a person.
    let member = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": member_user_id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("user {member_user_id}")))?;
    if !member.user_type.is_person() {
        return Err(AppError::ValidationError(
            "Members must be person accounts, not orgs".to_string(),
        ));
    }

    let collection = db.collection::<OrgMembership>(ORG_MEMBERSHIPS);

    // Look up any existing row (active or revoked) for this pair.
    let existing = collection
        .find_one(doc! {
            "org_user_id": org_user_id,
            "member_user_id": member_user_id,
        })
        .await?;

    if let Some(row) = existing {
        if row.is_active() {
            return Err(AppError::Conflict(
                "User is already a member of this org".to_string(),
            ));
        }

        // Reactivate revoked row in-place.
        let now = Utc::now();
        let now_bson = bson::DateTime::from_chrono(now);
        let allowed = match &allowed_service_ids {
            None => bson::Bson::Null,
            Some(ids) => bson::to_bson(ids).map_err(|e| AppError::Internal(e.to_string()))?,
        };
        collection
            .update_one(
                doc! { "_id": &row.id },
                doc! { "$set": {
                    "role": role.as_str(),
                    "scope_source": scope_source.as_str(),
                    "allowed_service_ids": allowed,
                    "revoked_at": bson::Bson::Null,
                    "created_at": now_bson,
                }},
            )
            .await?;
        return Ok(OrgMembership {
            id: row.id,
            org_user_id: org_user_id.to_string(),
            member_user_id: member_user_id.to_string(),
            role,
            scope_source,
            allowed_service_ids,
            created_at: now,
            revoked_at: None,
        });
    }

    let now = Utc::now();
    let membership = OrgMembership {
        id: Uuid::new_v4().to_string(),
        org_user_id: org_user_id.to_string(),
        member_user_id: member_user_id.to_string(),
        role,
        scope_source,
        allowed_service_ids,
        created_at: now,
        revoked_at: None,
    };

    match collection.insert_one(&membership).await {
        Ok(_) => Ok(membership),
        Err(e) if is_duplicate_key_error(&e) => Err(AppError::Conflict(
            "User is already a member of this org".to_string(),
        )),
        Err(e) => Err(AppError::DatabaseError(e)),
    }
}

/// List the memberships of a given member user. Active by default;
/// pass `include_revoked = true` to include soft-deleted rows.
pub async fn list_memberships_for_member(
    db: &mongodb::Database,
    member_user_id: &str,
    include_revoked: bool,
) -> AppResult<Vec<OrgMembership>> {
    let mut filter = doc! { "member_user_id": member_user_id };
    if !include_revoked {
        filter.insert("revoked_at", bson::Bson::Null);
    }

    let cursor = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find(filter)
        .await?;

    let memberships: Vec<OrgMembership> = cursor.try_collect().await?;
    Ok(memberships)
}

/// List members of an org. Active by default.
pub async fn list_members_for_org(
    db: &mongodb::Database,
    org_user_id: &str,
    include_revoked: bool,
) -> AppResult<Vec<OrgMembership>> {
    let mut filter = doc! { "org_user_id": org_user_id };
    if !include_revoked {
        filter.insert("revoked_at", bson::Bson::Null);
    }

    let cursor = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find(filter)
        .await?;

    let memberships: Vec<OrgMembership> = cursor.try_collect().await?;
    Ok(memberships)
}

/// Look up a single membership by `(org_user_id, member_user_id)`.
/// Active rows only -- revoked memberships return `None`.
pub async fn get_active_membership(
    db: &mongodb::Database,
    org_user_id: &str,
    member_user_id: &str,
) -> AppResult<Option<OrgMembership>> {
    let row = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find_one(doc! {
            "org_user_id": org_user_id,
            "member_user_id": member_user_id,
            "revoked_at": bson::Bson::Null,
        })
        .await?;
    Ok(row)
}

/// Update role and/or scope for an existing active membership.
pub async fn update_membership(
    db: &mongodb::Database,
    membership_id: &str,
    role: Option<OrgRole>,
    scope_source: Option<MemberScopeSource>,
    allowed_service_ids: Option<Option<Vec<String>>>,
) -> AppResult<OrgMembership> {
    let mut update = doc! {};
    if let Some(role) = role {
        update.insert("role", role.as_str());
    }
    match scope_source {
        Some(MemberScopeSource::Inherit) => {
            update.insert("scope_source", MemberScopeSource::Inherit.as_str());
            update.insert("allowed_service_ids", bson::Bson::Null);
        }
        Some(MemberScopeSource::Override) => {
            update.insert("scope_source", MemberScopeSource::Override.as_str());
            match allowed_service_ids {
                None => {
                    update.insert("allowed_service_ids", bson::Bson::Null);
                }
                Some(None) => {
                    update.insert("allowed_service_ids", bson::Bson::Null);
                }
                Some(Some(ids)) => {
                    update.insert("allowed_service_ids", ids);
                }
            };
        }
        None => {
            if let Some(scope) = allowed_service_ids {
                update.insert("scope_source", MemberScopeSource::Override.as_str());
                match scope {
                    None => update.insert("allowed_service_ids", bson::Bson::Null),
                    Some(ids) => update.insert("allowed_service_ids", ids),
                };
            }
        }
    }
    if update.is_empty() {
        return Err(AppError::BadRequest("No fields to update".to_string()));
    }

    db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .update_one(
            doc! { "_id": membership_id, "revoked_at": bson::Bson::Null },
            doc! { "$set": update },
        )
        .await?;

    db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find_one(doc! { "_id": membership_id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("membership {membership_id}")))
}

/// Soft-delete a membership by setting `revoked_at`.
pub async fn revoke_membership(
    db: &mongodb::Database,
    org_user_id: &str,
    member_user_id: &str,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let result = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .update_one(
            doc! {
                "org_user_id": org_user_id,
                "member_user_id": member_user_id,
                "revoked_at": bson::Bson::Null,
            },
            doc! { "$set": { "revoked_at": now } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "active membership not found".to_string(),
        ));
    }
    Ok(())
}

/// True when the actor is an active admin of the given org.
pub async fn is_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    org_user_id: &str,
) -> AppResult<bool> {
    let row = get_active_membership(db, org_user_id, actor_user_id).await?;
    Ok(row.map(|m| m.role.can_admin()).unwrap_or(false))
}

/// True when the actor has any active membership in the given org.
pub async fn is_member(
    db: &mongodb::Database,
    actor_user_id: &str,
    org_user_id: &str,
) -> AppResult<bool> {
    Ok(get_active_membership(db, org_user_id, actor_user_id)
        .await?
        .is_some())
}

/// List the user_ids of every active admin of the given org.
///
/// Returns an empty `Vec` for non-orgs, orgs with no admins, or unknown
/// org ids -- callers should treat the result as informational, not
/// authoritative. Used by the approval pipeline to fan out org-policy
/// notifications to every admin.
pub async fn list_admin_user_ids(
    db: &mongodb::Database,
    org_user_id: &str,
) -> AppResult<Vec<String>> {
    let cursor = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find(doc! {
            "org_user_id": org_user_id,
            "role": "admin",
            "revoked_at": bson::Bson::Null,
        })
        .await?;
    let memberships: Vec<OrgMembership> = cursor.try_collect().await?;
    Ok(memberships.into_iter().map(|m| m.member_user_id).collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// Proxy fallback membership query (timeout-bounded)
// ─────────────────────────────────────────────────────────────────────────────

/// Find all active memberships for `member_user_id`, bounded by a wall-clock
/// timeout (see [`ORG_FALLBACK_TIMEOUT`]). The result is **already ordered**
/// for proxy resolution: the user's `primary_org_id` (if set) comes first,
/// then the rest by ascending `created_at` (earliest membership wins).
///
/// Returns `OrgQueryTimeout` if the underlying MongoDB query exceeds the
/// timeout. Returns `Ok(vec![])` when the user has no memberships.
pub async fn find_active_memberships_with_timeout(
    db: &mongodb::Database,
    member_user_id: &str,
) -> AppResult<Vec<OrgMembership>> {
    // Look up the user once to get primary_org_id. The user document is also
    // already cached at the call site (proxy_service has the AuthUser) but we
    // can't reach into that here, so a single round-trip is the simplest API.
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": member_user_id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("user {member_user_id}")))?;
    let primary_org_id = user.primary_org_id.clone();

    let opts = FindOptions::builder()
        .sort(doc! { "created_at": 1 })
        .build();

    let query = async {
        let cursor = db
            .collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .find(doc! {
                "member_user_id": member_user_id,
                "revoked_at": bson::Bson::Null,
            })
            .with_options(opts)
            .await?;
        let memberships: Vec<OrgMembership> = cursor.try_collect().await?;
        Ok::<_, AppError>(memberships)
    };

    let memberships = match tokio::time::timeout(ORG_FALLBACK_TIMEOUT, query).await {
        Ok(Ok(rows)) => rows,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            tracing::warn!(
                member_user_id = %member_user_id,
                "Org membership query exceeded fallback timeout"
            );
            return Err(AppError::OrgQueryTimeout);
        }
    };

    // Apply primary_org_id ordering: pull primary to the front, keep rest in
    // created_at order.
    let mut ordered = memberships;
    if let Some(primary) = primary_org_id.as_deref()
        && let Some(idx) = ordered.iter().position(|m| m.org_user_id == primary)
    {
        let primary_row = ordered.remove(idx);
        ordered.insert(0, primary_row);
    }
    Ok(ordered)
}

// ─────────────────────────────────────────────────────────────────────────────
// Ownership resolution helper
// ─────────────────────────────────────────────────────────────────────────────

/// Outcome of [`resolve_owner_access`]. Distinguishes direct ownership from
/// org-mediated access so callers can make role-aware decisions.
///
/// The org variants carry the membership's effective service scope so
/// callers can gate per-resource reads/writes via [`Self::allows_resource`].
/// Resources outside the scope must be treated as not-found for both read
/// and write; never leak metadata a member is not entitled to see.
///
/// # Security invariant
///
/// **Every handler that writes to a user_service / user_endpoint /
/// user_api_key / unified key MUST pass through `can_write() && allows_resource(id)`
/// before calling any mutation.** The write_owner helpers in
/// `handlers/{keys,user_services_handler,user_endpoints,user_api_keys_external}.rs`
/// already enforce this; any new handler that mutates these collections
/// must do the same.
///
/// Rationale: members and viewers ONLY return `can_write = false`. If a new
/// handler skipped the check and called the service layer directly with the
/// actor's user_id, a member could write to an org-owned resource because
/// the service layer only filters by `user_id`. The write_owner helper
/// substitutes the effective owner id AND gates on role -- both are load-bearing.
///
/// Middleware already blocks delegated tokens and service-account tokens
/// from reaching these endpoints, so the only remaining auth path is
/// session / personal API key, both of which surface the caller's own
/// user_id via `AuthUser`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerAccess {
    /// The actor IS the resource owner. Full access.
    Direct,
    /// The actor is an Admin of the org that owns this resource. Full access,
    /// subject to the admin's own effective service scope (admins rarely
    /// restrict this, but the scope is honored for correctness).
    AsOrgAdmin {
        org_user_id: String,
        membership_id: String,
        allowed_service_ids: Option<Vec<String>>,
    },
    /// The actor is a Member or Viewer of the owning org. Read-only access.
    AsOrgMember {
        org_user_id: String,
        membership_id: String,
        role: OrgRole,
        allowed_service_ids: Option<Vec<String>>,
    },
    /// The actor has no relationship to the resource owner.
    Forbidden,
}

impl OwnerAccess {
    /// Whether the actor can perform a write/mutate operation on the resource.
    /// Does not account for per-resource scope -- callers must ALSO call
    /// [`Self::allows_resource`] with the target resource id.
    pub fn can_write(&self) -> bool {
        matches!(self, OwnerAccess::Direct | OwnerAccess::AsOrgAdmin { .. })
    }

    /// Whether the actor can read the resource. Anything but `Forbidden`.
    /// Does not account for per-resource scope -- callers must ALSO call
    /// [`Self::allows_resource`] with the target resource id.
    pub fn can_read(&self) -> bool {
        !matches!(self, OwnerAccess::Forbidden)
    }

    /// Whether the actor is allowed to touch the given `user_service` /
    /// resource id under this access grant. `Direct` ownership and
    /// `Forbidden` always return true / false respectively. Org-mediated
    /// access honors the membership's effective service scope: `None` means
    /// unrestricted access; `Some(list)` means only those ids are
    /// reachable through the membership.
    ///
    /// Call this **in addition to** `can_read`/`can_write` in every handler
    /// that touches an individual key/service/endpoint owned by an org. A
    /// scoped member must never read metadata for services outside their
    /// scope.
    pub fn allows_resource(&self, resource_id: &str) -> bool {
        match self {
            OwnerAccess::Direct => true,
            OwnerAccess::Forbidden => false,
            OwnerAccess::AsOrgAdmin {
                allowed_service_ids,
                ..
            }
            | OwnerAccess::AsOrgMember {
                allowed_service_ids,
                ..
            } => match allowed_service_ids {
                None => true,
                Some(ids) => ids.iter().any(|id| id == resource_id),
            },
        }
    }

    /// Like [`Self::allows_resource`] but for resources that map to a set
    /// of `UserService` ids (e.g. an endpoint or external API key referenced
    /// by one or more `UserService` rows). Returns `true` if **any** of the
    /// candidate ids passes the scope check, or if the membership is
    /// unscoped.
    ///
    /// Empty input is treated as "no service binds this resource yet". Such
    /// orphans are only writable by callers without a scope (Direct owners
    /// or unscoped admins) -- a scoped admin has no concrete claim on a
    /// resource that maps to no service in their scope.
    pub fn allows_any_resource(&self, resource_ids: &[String]) -> bool {
        match self {
            OwnerAccess::Direct => true,
            OwnerAccess::Forbidden => false,
            OwnerAccess::AsOrgAdmin {
                allowed_service_ids,
                ..
            }
            | OwnerAccess::AsOrgMember {
                allowed_service_ids,
                ..
            } => match allowed_service_ids {
                None => true,
                Some(scope) => resource_ids
                    .iter()
                    .any(|rid| scope.iter().any(|s| s == rid)),
            },
        }
    }
}

/// Decide what level of access `actor_user_id` has to a resource owned by
/// `target_owner_id`. The owner may be a person (the actor themselves, or
/// someone else) or an org (in which case membership + role is consulted).
pub async fn resolve_owner_access(
    db: &mongodb::Database,
    actor_user_id: &str,
    target_owner_id: &str,
) -> AppResult<OwnerAccess> {
    if actor_user_id == target_owner_id {
        return Ok(OwnerAccess::Direct);
    }

    // Check whether target_owner_id is an org. If it's a person we don't own
    // (and it's not us), it's just forbidden.
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": target_owner_id })
        .await?;
    let owner = match owner {
        Some(u) if u.user_type.is_org() => u,
        _ => return Ok(OwnerAccess::Forbidden),
    };

    let membership = get_active_membership(db, &owner.id, actor_user_id).await?;
    let Some(m) = membership else {
        return Ok(OwnerAccess::Forbidden);
    };
    let effective_scope =
        crate::services::org_role_scope_service::effective_scope_for_membership(db, &m).await?;
    let membership_id = m.id.clone();

    Ok(match m.role {
        OrgRole::Admin => OwnerAccess::AsOrgAdmin {
            org_user_id: owner.id,
            membership_id,
            allowed_service_ids: effective_scope,
        },
        OrgRole::Member | OrgRole::Viewer => OwnerAccess::AsOrgMember {
            org_user_id: owner.id,
            membership_id,
            role: m.role,
            allowed_service_ids: effective_scope,
        },
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    matches!(
        e.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we))
            if we.code == 11000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_timeout_constant_is_500ms() {
        // Documented in the implementation plan and CLAUDE.md. If this is
        // ever changed, update both docs.
        assert_eq!(ORG_FALLBACK_TIMEOUT, Duration::from_millis(500));
    }

    #[test]
    fn owner_access_can_write_can_read() {
        assert!(OwnerAccess::Direct.can_write());
        assert!(OwnerAccess::Direct.can_read());

        let admin = OwnerAccess::AsOrgAdmin {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            allowed_service_ids: None,
        };
        assert!(admin.can_write());
        assert!(admin.can_read());

        let member = OwnerAccess::AsOrgMember {
            org_user_id: "org-1".to_string(),
            membership_id: "m-2".to_string(),
            role: OrgRole::Member,
            allowed_service_ids: None,
        };
        assert!(!member.can_write());
        assert!(member.can_read());

        let viewer = OwnerAccess::AsOrgMember {
            org_user_id: "org-1".to_string(),
            membership_id: "m-3".to_string(),
            role: OrgRole::Viewer,
            allowed_service_ids: None,
        };
        assert!(!viewer.can_write());
        assert!(viewer.can_read());

        assert!(!OwnerAccess::Forbidden.can_write());
        assert!(!OwnerAccess::Forbidden.can_read());
    }

    #[test]
    fn allows_resource_direct_always_true() {
        assert!(OwnerAccess::Direct.allows_resource("any"));
    }

    #[test]
    fn allows_resource_forbidden_always_false() {
        assert!(!OwnerAccess::Forbidden.allows_resource("any"));
    }

    #[test]
    fn allows_resource_respects_scope_on_org_admin() {
        let admin = OwnerAccess::AsOrgAdmin {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            allowed_service_ids: Some(vec!["svc-1".to_string(), "svc-2".to_string()]),
        };
        assert!(admin.allows_resource("svc-1"));
        assert!(admin.allows_resource("svc-2"));
        assert!(!admin.allows_resource("svc-3"));
    }

    #[test]
    fn allows_resource_respects_scope_on_org_member() {
        let member = OwnerAccess::AsOrgMember {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            role: OrgRole::Member,
            allowed_service_ids: Some(vec!["svc-1".to_string()]),
        };
        assert!(member.allows_resource("svc-1"));
        assert!(!member.allows_resource("svc-2"));
    }

    #[test]
    fn allows_resource_no_scope_means_unrestricted() {
        let member = OwnerAccess::AsOrgMember {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            role: OrgRole::Member,
            allowed_service_ids: None,
        };
        assert!(member.allows_resource("any-svc"));
    }

    #[test]
    fn allows_resource_empty_scope_blocks_everything() {
        let member = OwnerAccess::AsOrgMember {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            role: OrgRole::Member,
            allowed_service_ids: Some(vec![]),
        };
        assert!(!member.allows_resource("svc-1"));
    }

    fn make_org_user(email: &str) -> User {
        use chrono::Utc;
        let now = Utc::now();
        User {
            id: "11111111-2222-3333-4444-555555555555".to_string(),
            email: email.to_string(),
            password_hash: None,
            display_name: Some("Test Org".to_string()),
            slug: Some("test-org".to_string()),
            avatar_url: None,
            email_verified: false,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: true,
            is_admin: false,
            is_operator: false,
            role_ids: vec![],
            group_ids: vec![],
            invite_code_id: None,
            mfa_enabled: false,
            social_provider: None,
            social_provider_id: None,
            user_type: UserType::Org,
            primary_org_id: None,
            created_at: now,
            updated_at: now,
            last_login_at: None,
            profile_config: Default::default(),
        }
    }

    #[test]
    fn contact_email_for_display_hides_placeholder() {
        let user = make_org_user(&format!(
            "org-11111111-2222-3333-4444-555555555555{}",
            ORG_PLACEHOLDER_EMAIL_SUFFIX
        ));
        assert_eq!(contact_email_for_display(&user), None);
    }

    #[test]
    fn contact_email_for_display_returns_real_email() {
        let user = make_org_user("contact@acme.test");
        assert_eq!(
            contact_email_for_display(&user),
            Some("contact@acme.test".to_string())
        );
    }

    #[test]
    fn contact_email_for_display_empty_is_none() {
        let user = make_org_user("");
        assert_eq!(contact_email_for_display(&user), None);
    }

    #[test]
    fn contact_email_for_display_passes_through_other_nyxid_local_emails() {
        // Only the exact `org-<this_org_id>@nyxid.local` form is treated
        // as the placeholder. A real user who happens to use
        // `foo@nyxid.local` is still surfaced.
        let user = make_org_user("foo@nyxid.local");
        assert_eq!(
            contact_email_for_display(&user),
            Some("foo@nyxid.local".to_string())
        );
    }

    #[test]
    fn contact_email_for_display_surfaces_org_prefixed_real_emails() {
        // Regression: the old check hid every `org-*@nyxid.local` address,
        // including admin-configured ones like `org-support@nyxid.local`.
        // It must now only hide the synthetic id-based placeholder.
        let user = make_org_user("org-support@nyxid.local");
        assert_eq!(
            contact_email_for_display(&user),
            Some("org-support@nyxid.local".to_string())
        );
    }

    #[test]
    fn contact_email_for_display_hides_placeholder_case_insensitive() {
        // MongoDB should never store mixed case here (the service
        // lowercases user-supplied emails and generates the placeholder
        // lowercase), but guard against data that came in via a different
        // path.
        let user = make_org_user(&format!(
            "ORG-11111111-2222-3333-4444-555555555555{}",
            ORG_PLACEHOLDER_EMAIL_SUFFIX
        ));
        assert_eq!(contact_email_for_display(&user), None);
    }

    // ── synthetic_org_email ──

    #[test]
    fn synthetic_org_email_format() {
        let email = synthetic_org_email("abc-123");
        assert_eq!(email, "org-abc-123@nyxid.local");
    }

    #[test]
    fn synthetic_org_email_with_uuid() {
        let id = "11111111-2222-3333-4444-555555555555";
        let email = synthetic_org_email(id);
        assert_eq!(email, format!("org-{}{}", id, ORG_PLACEHOLDER_EMAIL_SUFFIX));
    }

    #[test]
    fn synthetic_org_email_empty_id() {
        let email = synthetic_org_email("");
        assert_eq!(email, format!("org-{}", ORG_PLACEHOLDER_EMAIL_SUFFIX));
    }

    // ── contact_email_for_display (additional edge cases) ──

    #[test]
    fn contact_email_for_display_whitespace_only_is_none() {
        let user = make_org_user("   ");
        assert_eq!(contact_email_for_display(&user), None);
    }

    #[test]
    fn contact_email_for_display_trims_leading_trailing_spaces() {
        let user = make_org_user("  real@example.com  ");
        assert_eq!(
            contact_email_for_display(&user),
            Some("real@example.com".to_string())
        );
    }

    #[test]
    fn contact_email_for_display_different_org_id_placeholder_not_hidden() {
        // A placeholder that belongs to a DIFFERENT org id should NOT be
        // hidden for this org.
        let user = make_org_user("org-different-id@nyxid.local");
        assert_eq!(
            contact_email_for_display(&user),
            Some("org-different-id@nyxid.local".to_string())
        );
    }

    // ── OwnerAccess::allows_any_resource ──

    #[test]
    fn allows_any_resource_direct_always_true() {
        assert!(OwnerAccess::Direct.allows_any_resource(&[]));
        assert!(OwnerAccess::Direct.allows_any_resource(&["svc-1".to_string()]));
        assert!(
            OwnerAccess::Direct.allows_any_resource(&["svc-1".to_string(), "svc-2".to_string(),])
        );
    }

    #[test]
    fn allows_any_resource_forbidden_always_false() {
        assert!(!OwnerAccess::Forbidden.allows_any_resource(&[]));
        assert!(!OwnerAccess::Forbidden.allows_any_resource(&["svc-1".to_string()]));
    }

    #[test]
    fn allows_any_resource_unscoped_admin_allows_all() {
        let admin = OwnerAccess::AsOrgAdmin {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            allowed_service_ids: None,
        };
        assert!(admin.allows_any_resource(&[]));
        assert!(admin.allows_any_resource(&["svc-1".to_string()]));
    }

    #[test]
    fn allows_any_resource_scoped_admin_matches_any_in_scope() {
        let admin = OwnerAccess::AsOrgAdmin {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            allowed_service_ids: Some(vec!["svc-1".to_string(), "svc-2".to_string()]),
        };
        // One match is enough
        assert!(admin.allows_any_resource(&["svc-1".to_string(), "svc-99".to_string()]));
        // No match
        assert!(!admin.allows_any_resource(&["svc-99".to_string()]));
    }

    #[test]
    fn allows_any_resource_scoped_admin_empty_input_returns_false() {
        // Empty resource ids with a scoped admin means "orphan resource" --
        // no concrete claim.
        let admin = OwnerAccess::AsOrgAdmin {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            allowed_service_ids: Some(vec!["svc-1".to_string()]),
        };
        assert!(!admin.allows_any_resource(&[]));
    }

    #[test]
    fn allows_any_resource_scoped_member_matches() {
        let member = OwnerAccess::AsOrgMember {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            role: OrgRole::Member,
            allowed_service_ids: Some(vec!["svc-3".to_string()]),
        };
        assert!(member.allows_any_resource(&["svc-3".to_string()]));
        assert!(!member.allows_any_resource(&["svc-4".to_string()]));
    }

    #[test]
    fn allows_any_resource_unscoped_member_allows_all() {
        let member = OwnerAccess::AsOrgMember {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            role: OrgRole::Viewer,
            allowed_service_ids: None,
        };
        assert!(member.allows_any_resource(&["svc-1".to_string()]));
        assert!(member.allows_any_resource(&[]));
    }

    #[test]
    fn allows_any_resource_empty_scope_blocks_everything() {
        let member = OwnerAccess::AsOrgMember {
            org_user_id: "org".to_string(),
            membership_id: "m".to_string(),
            role: OrgRole::Member,
            allowed_service_ids: Some(vec![]),
        };
        assert!(!member.allows_any_resource(&["svc-1".to_string()]));
        assert!(!member.allows_any_resource(&[]));
    }

    // ── OwnerAccess structural / variant coverage ──

    #[test]
    fn owner_access_org_admin_can_write_and_read() {
        let admin = OwnerAccess::AsOrgAdmin {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            allowed_service_ids: Some(vec!["svc-1".to_string()]),
        };
        assert!(admin.can_write());
        assert!(admin.can_read());
    }

    #[test]
    fn owner_access_viewer_cannot_write_but_can_read() {
        let viewer = OwnerAccess::AsOrgMember {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            role: OrgRole::Viewer,
            allowed_service_ids: Some(vec![]),
        };
        assert!(!viewer.can_write());
        assert!(viewer.can_read());
    }

    #[test]
    fn owner_access_debug_and_clone() {
        let access = OwnerAccess::AsOrgAdmin {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            allowed_service_ids: None,
        };
        let cloned = access.clone();
        assert_eq!(access, cloned);
        // Verify Debug impl does not panic
        let _ = format!("{:?}", access);
    }

    #[test]
    fn owner_access_equality() {
        let a = OwnerAccess::Direct;
        let b = OwnerAccess::Direct;
        assert_eq!(a, b);

        let c = OwnerAccess::Forbidden;
        assert_ne!(a, c);
    }
}
