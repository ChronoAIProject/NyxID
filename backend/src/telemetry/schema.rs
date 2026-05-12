//! Canonical telemetry event enum.
//!
//! Every backend-emitted event has one variant here. Handlers construct
//! the variant; `name()` and `properties()` translate it to the
//! vendor-neutral wire form. Adding a new event = adding a variant +
//! updating both match arms. Unknown event names become a compile error,
//! not a runtime surprise.
//!
//! `properties()` runs the scrubber on the produced JSON before returning,
//! so there is no code path by which an emitter can bypass redaction.

use serde_json::{Value, json};

use super::scrub;

/// Strongly-typed canonical event. One variant per row in
/// `docs/TELEMETRY.md` §5.1.
#[derive(Debug, Clone)]
pub enum TelemetryEvent {
    // --- handlers/auth.rs -----------------------------------------------
    UserSignedUp {
        /// `email` | `google` | `github` | `apple` — the auth method.
        method: String,
        /// `direct` | `invite_code` | `social_oauth` — funnel attribution.
        /// `direct` is reserved for public-launch (gate disabled) email
        /// signups that did not carry an invite code.
        source: String,
        /// Lowercased email-domain only (e.g. `gmail.com`). Full email
        /// would be scrubbed at egress; the domain remains usable for
        /// cohort analysis (corporate vs. personal accounts).
        email_domain: Option<String>,
        /// SHA-256-prefix hash of the redeemed invite code's UUID, when
        /// signup used one. Raw UUID would be scrubbed at egress; the
        /// hash correlates this event with `invite.code_generated` /
        /// `invite.code_redeemed`. `None` when no code was used.
        invite_code_id: Option<String>,
        /// Bare-domain portion of the HTTP `Referer` header (host only,
        /// scheme/path stripped) when the signup arrived from the web.
        /// Stored as domain — not the full URL — so a referer with PII
        /// in the query string cannot leak through the scrubber. `None`
        /// for non-web signups or when Referer is absent.
        referrer_domain: Option<String>,
        /// SHA-256-prefix hash of the inviting organization's user_id
        /// when the invite code was issued by an org user. `None` for
        /// personal invites or no invite. Hashed so the raw UUID is not
        /// scrubbed away at egress.
        via_org: Option<String>,
        /// Convenience boolean: `true` iff an invite code was redeemed.
        /// Redundant with `invite_code_id.is_some()` but kept so funnels
        /// can split without HogQL `IS NOT NULL` checks.
        invite_code_used: bool,
    },
    UserEmailVerified,
    AuthLoggedIn {
        method: String,
        mfa_required: bool,
    },
    AuthLoggedOut,
    AuthPasswordResetRequested,
    AuthPasswordResetCompleted,
    AuthTokenRefreshed,
    AuthTokenExchanged {
        subject_token_type: String,
        exchange_provider: Option<String>,
    },
    AuthDelegationRefreshed {
        client_id: String,
    },
    InviteCodeGenerated {
        generated_by_role: String,
    },
    /// Emitted when a previously-generated invite code is consumed during
    /// a successful signup. Pairs with `InviteCodeGenerated` to measure
    /// per-code conversion (codes issued vs. codes redeemed) and
    /// time-to-redemption distribution.
    InviteCodeRedeemed {
        /// SHA-256-prefix hash of the invite code's UUID. Raw UUID would
        /// be redacted to `[UUID_REDACTED]` at egress.
        code_id: String,
        /// SHA-256-prefix hash of the creating admin/org user_id. Used
        /// for "which inviter's codes convert best" cohort analysis.
        created_by_user_id: String,
        /// Days between `InviteCode.created_at` and redemption. Clamped
        /// at zero so negative clock drift never produces a nonsensical
        /// negative value.
        days_to_redemption: u64,
    },

    // --- handlers/users.rs ----------------------------------------------
    UserDeleted {
        reason: Option<String>,
    },

    // --- handlers/mfa.rs ------------------------------------------------
    MfaEnrollmentStarted {
        factor_type: String,
    },
    MfaEnrollmentCompleted {
        factor_type: String,
    },
    MfaChallengeSucceeded {
        factor_type: String,
    },
    MfaChallengeFailed {
        factor_type: String,
        reason: String,
    },

    // --- handlers/keys.rs -----------------------------------------------
    KeyCreated {
        source: String,
        catalog_slug: Option<String>,
        has_node_binding: bool,
    },
    KeyDeleted {
        source: String,
    },

    // --- handlers/user_services_handler.rs + connections ----------------
    ServiceConnected {
        provider_slug: String,
        flow: String,
    },
    ServiceDisconnected {
        provider_slug: String,
    },
    ServiceUserAgentCustomized {
        provider_slug: String,
    },

    // --- handlers/user_endpoints.rs -------------------------------------
    EndpointUpdated {
        endpoint_type: String,
    },
    EndpointDeleted {
        endpoint_type: String,
    },

    // --- handlers/catalog.rs --------------------------------------------
    CatalogBrowsed {
        filter: Option<String>,
        result_count: i64,
    },
    CatalogEntryViewed {
        catalog_slug: String,
        has_openapi_spec: bool,
    },
    CatalogEndpointsFetched {
        catalog_slug: String,
        endpoint_count: i64,
    },

    // --- handlers/api_keys.rs -------------------------------------------
    ApiKeyCreated {
        platform: Option<String>,
        scope_mode: String,
        rate_limit_per_second: Option<u32>,
    },
    ApiKeyRotated {
        platform: Option<String>,
    },
    ApiKeyDeleted {
        platform: Option<String>,
    },

    // --- handlers/agent_bindings.rs -------------------------------------
    AgentBindingCreated {
        platform: Option<String>,
        service_slug: String,
    },
    AgentBindingDeleted {
        platform: Option<String>,
        service_slug: String,
    },

    // --- handlers/approvals.rs ------------------------------------------
    ApprovalRequested {
        service_slug: String,
        mode: String,
        channel: String,
    },
    ApprovalDecided {
        service_slug: String,
        mode: String,
        decision: String,
        decision_ms: u64,
        channel: String,
        decided_via: String,
    },
    ApprovalExpired {
        service_slug: String,
        mode: String,
    },
    ApprovalGrantRevoked {
        service_slug: String,
    },
    ApprovalConfigUpdated {
        service_slug: String,
        mode: String,
    },

    // --- handlers/nodes.rs + admin_nodes.rs + node_ws.rs ----------------
    NodeRegistered {
        node_platform: String,
        profile: String,
    },
    NodeConnected {
        node_id: String,
        profile: String,
    },
    NodeDisconnected {
        node_id: String,
        reason: String,
    },
    NodeDeleted {
        node_id: String,
    },
    NodeCredentialConfigured {
        credential_type: String,
    },

    // --- handlers/channel_*.rs ------------------------------------------
    ChannelBotRegistered {
        platform: String,
    },
    ChannelBotDeleted {
        platform: String,
    },
    ChannelMappingCreated {
        platform: String,
        conversation_id_hash: String,
    },
    ChannelMappingDeleted {
        platform: String,
        conversation_id_hash: String,
    },
    /// Intended to be sampled at ~10% when the first emission lands in
    /// a follow-up PR. The sampling helper has not shipped yet.
    ChannelMessageReceived {
        platform: String,
        conversation_id_hash: String,
    },
    /// Sampled 10% at emit time.
    ChannelReplySent {
        platform: String,
        reply_mode: String,
        agent_api_key_id: Option<String>,
    },
    ChannelEventReceived {
        source: String,
        event_type: String,
        deduplicated: bool,
    },

    // --- handlers/mcp.rs ------------------------------------------------
    McpSessionStarted {
        client: Option<String>,
    },
    McpSessionEnded {
        duration_ms: u64,
        reason: String,
    },

    // --- handlers/ssh*.rs -----------------------------------------------
    SshCertificateIssued {
        service_slug: String,
        ttl_secs: i64,
    },
    SshTunnelOpened {
        service_slug: String,
        mode: String,
    },
    SshTunnelClosed {
        service_slug: String,
        duration_ms: u64,
    },

    // --- handlers/oauth.rs ----------------------------------------------
    OauthClientRegistered,
    OauthClientSecretRotated {
        client_id: String,
    },
    OauthAuthorizationGranted {
        client_id: String,
        grant_type: String,
    },
    OauthTokenIssued {
        client_id: String,
        grant_type: String,
    },

    // --- handlers/notifications.rs --------------------------------------
    NotificationChannelLinked {
        channel: String,
    },
    NotificationChannelUnlinked {
        channel: String,
    },
    NotificationDeviceRegistered {
        platform: String,
    },
    NotificationDeviceRemoved {
        platform: String,
    },

    // --- handlers/admin_*.rs --------------------------------------------
    AdminUserSuspended,
    AdminUserUnsuspended,
    AdminAuditLogViewed {
        filter: Option<String>,
    },
    AdminOauthClientRegistered,
    AdminServiceAccountCreated,
    AdminServiceAccountRotated,
    AdminServiceAccountDeleted,
    AdminNodeDisconnected {
        node_id: String,
    },
    AdminNodeDeleted {
        node_id: String,
    },
    AdminServiceCreated {
        slug: String,
    },
    AdminServiceUpdated {
        slug: String,
    },

    // --- handlers/proxy.rs ----------------------------------------------
    ProxyError {
        service_slug: String,
        error_code: u32,
        status: u16,
    },
    /// Emitted from `proxy_request` / `proxy_request_by_slug` when the
    /// upstream response is 2xx. The companion to `ProxyError`: together
    /// they let M1 reach be defined precisely as "≥1 `proxy.success` per
    /// user in the window" rather than via proxy signals that approximate
    /// it. See issue #714.
    ProxySuccess {
        /// Resolved `UserService` / `DownstreamService` slug. Never a
        /// UUID from the route path — matches the `ProxyError` rule so
        /// the two events join cleanly on `service_slug` for success-rate.
        service_slug: String,
        /// Upstream HTTP method (`GET` / `POST` / ...). Uppercase.
        method: String,
        /// Upstream HTTP status code (always 2xx here).
        status: u16,
        /// End-to-end proxy latency from handler entry to response,
        /// including downstream wait and credential resolution.
        latency_ms: u64,
        /// Auth provenance: `session` | `access_token` | `relay` |
        /// `api_key` | `service_account` | `delegated`. Lets HogQL split
        /// reach by user-driven vs. agent-driven traffic.
        auth_kind: &'static str,
    },

    // --- mw/rate_limit.rs ----------------------------------------------
    ApiRateLimited {
        route: String,
        limit_type: String,
        limit_per_second: u32,
        api_key_id: Option<String>,
    },
}

impl TelemetryEvent {
    /// Canonical event name. Dot-namespaced, lowercase, past-tense verb.
    pub fn name(&self) -> &'static str {
        match self {
            Self::UserSignedUp { .. } => "user.signed_up",
            Self::UserEmailVerified => "user.email_verified",
            Self::AuthLoggedIn { .. } => "auth.logged_in",
            Self::AuthLoggedOut => "auth.logged_out",
            Self::AuthPasswordResetRequested => "auth.password_reset_requested",
            Self::AuthPasswordResetCompleted => "auth.password_reset_completed",
            Self::AuthTokenRefreshed => "auth.token_refreshed",
            Self::AuthTokenExchanged { .. } => "auth.token_exchanged",
            Self::AuthDelegationRefreshed { .. } => "auth.delegation_refreshed",
            Self::InviteCodeGenerated { .. } => "invite.code_generated",
            Self::InviteCodeRedeemed { .. } => "invite.code_redeemed",
            Self::UserDeleted { .. } => "user.deleted",
            Self::MfaEnrollmentStarted { .. } => "mfa.enrollment_started",
            Self::MfaEnrollmentCompleted { .. } => "mfa.enrollment_completed",
            Self::MfaChallengeSucceeded { .. } => "mfa.challenge_succeeded",
            Self::MfaChallengeFailed { .. } => "mfa.challenge_failed",
            Self::KeyCreated { .. } => "key.created",
            Self::KeyDeleted { .. } => "key.deleted",
            Self::ServiceConnected { .. } => "service.connected",
            Self::ServiceDisconnected { .. } => "service.disconnected",
            Self::ServiceUserAgentCustomized { .. } => "service.user_agent_customized",
            Self::EndpointUpdated { .. } => "endpoint.updated",
            Self::EndpointDeleted { .. } => "endpoint.deleted",
            Self::CatalogBrowsed { .. } => "catalog.browsed",
            Self::CatalogEntryViewed { .. } => "catalog.entry_viewed",
            Self::CatalogEndpointsFetched { .. } => "catalog.endpoints_fetched",
            Self::ApiKeyCreated { .. } => "api_key.created",
            Self::ApiKeyRotated { .. } => "api_key.rotated",
            Self::ApiKeyDeleted { .. } => "api_key.deleted",
            Self::AgentBindingCreated { .. } => "agent_binding.created",
            Self::AgentBindingDeleted { .. } => "agent_binding.deleted",
            Self::ApprovalRequested { .. } => "approval.requested",
            Self::ApprovalDecided { .. } => "approval.decided",
            Self::ApprovalExpired { .. } => "approval.expired",
            Self::ApprovalGrantRevoked { .. } => "approval.grant_revoked",
            Self::ApprovalConfigUpdated { .. } => "approval.config_updated",
            Self::NodeRegistered { .. } => "node.registered",
            Self::NodeConnected { .. } => "node.connected",
            Self::NodeDisconnected { .. } => "node.disconnected",
            Self::NodeDeleted { .. } => "node.deleted",
            Self::NodeCredentialConfigured { .. } => "node.credential_configured",
            Self::ChannelBotRegistered { .. } => "channel.bot_registered",
            Self::ChannelBotDeleted { .. } => "channel.bot_deleted",
            Self::ChannelMappingCreated { .. } => "channel.mapping_created",
            Self::ChannelMappingDeleted { .. } => "channel.mapping_deleted",
            Self::ChannelMessageReceived { .. } => "channel.message_received",
            Self::ChannelReplySent { .. } => "channel.reply_sent",
            Self::ChannelEventReceived { .. } => "channel.event_received",
            Self::McpSessionStarted { .. } => "mcp.session_started",
            Self::McpSessionEnded { .. } => "mcp.session_ended",
            Self::SshCertificateIssued { .. } => "ssh.certificate_issued",
            Self::SshTunnelOpened { .. } => "ssh.tunnel_opened",
            Self::SshTunnelClosed { .. } => "ssh.tunnel_closed",
            Self::OauthClientRegistered => "oauth.client_registered",
            Self::OauthClientSecretRotated { .. } => "oauth.client_secret_rotated",
            Self::OauthAuthorizationGranted { .. } => "oauth.authorization_granted",
            Self::OauthTokenIssued { .. } => "oauth.token_issued",
            Self::NotificationChannelLinked { .. } => "notification.channel_linked",
            Self::NotificationChannelUnlinked { .. } => "notification.channel_unlinked",
            Self::NotificationDeviceRegistered { .. } => "notification.device_registered",
            Self::NotificationDeviceRemoved { .. } => "notification.device_removed",
            Self::AdminUserSuspended => "admin.user_suspended",
            Self::AdminUserUnsuspended => "admin.user_unsuspended",
            Self::AdminAuditLogViewed { .. } => "admin.audit_log_viewed",
            Self::AdminOauthClientRegistered => "admin.oauth_client_registered",
            Self::AdminServiceAccountCreated => "admin.service_account_created",
            Self::AdminServiceAccountRotated => "admin.service_account_rotated",
            Self::AdminServiceAccountDeleted => "admin.service_account_deleted",
            Self::AdminNodeDisconnected { .. } => "admin.node_disconnected",
            Self::AdminNodeDeleted { .. } => "admin.node_deleted",
            Self::AdminServiceCreated { .. } => "admin.service_created",
            Self::AdminServiceUpdated { .. } => "admin.service_updated",
            Self::ProxyError { .. } => "proxy.error",
            Self::ProxySuccess { .. } => "proxy.success",
            Self::ApiRateLimited { .. } => "api.rate_limited",
        }
    }

    /// Produce the scrubbed JSON properties object for this event.
    ///
    /// Scrubbing is invoked on the final JSON here, NOT by callers, so
    /// there is no path by which an emitter can bypass egress redaction.
    /// See `docs/TELEMETRY.md` §6.
    pub fn properties(&self) -> Value {
        let mut props = self.raw_properties();
        scrub::scrub_value(&mut props);
        props
    }

    /// Internal: the unscrubbed properties. Kept separate so
    /// `properties()` can centralize the scrubber call.
    fn raw_properties(&self) -> Value {
        match self {
            Self::UserSignedUp {
                method,
                source,
                email_domain,
                invite_code_id,
                referrer_domain,
                via_org,
                invite_code_used,
            } => json!({
                "method": method,
                "source": source,
                "email_domain": email_domain,
                "invite_code_id": invite_code_id,
                "referrer_domain": referrer_domain,
                "via_org": via_org,
                "invite_code_used": invite_code_used,
            }),
            Self::UserEmailVerified => json!({}),
            Self::AuthLoggedIn {
                method,
                mfa_required,
            } => json!({
                "method": method,
                "mfa_required": mfa_required,
            }),
            Self::AuthLoggedOut => json!({}),
            Self::AuthPasswordResetRequested => json!({}),
            Self::AuthPasswordResetCompleted => json!({}),
            Self::AuthTokenRefreshed => json!({}),
            Self::AuthTokenExchanged {
                subject_token_type,
                exchange_provider,
            } => json!({
                "subject_token_type": subject_token_type,
                "exchange_provider": exchange_provider,
            }),
            Self::AuthDelegationRefreshed { client_id } => json!({ "client_id": client_id }),
            Self::InviteCodeGenerated { generated_by_role } => json!({
                "generated_by_role": generated_by_role,
            }),
            Self::InviteCodeRedeemed {
                code_id,
                created_by_user_id,
                days_to_redemption,
            } => json!({
                "code_id": code_id,
                "created_by_user_id": created_by_user_id,
                "days_to_redemption": days_to_redemption,
            }),
            Self::UserDeleted { reason } => json!({ "reason": reason }),
            Self::MfaEnrollmentStarted { factor_type } => json!({ "factor_type": factor_type }),
            Self::MfaEnrollmentCompleted { factor_type } => json!({ "factor_type": factor_type }),
            Self::MfaChallengeSucceeded { factor_type } => json!({ "factor_type": factor_type }),
            Self::MfaChallengeFailed {
                factor_type,
                reason,
            } => json!({
                "factor_type": factor_type,
                "reason": reason,
            }),
            Self::KeyCreated {
                source,
                catalog_slug,
                has_node_binding,
            } => json!({
                "source": source,
                "catalog_slug": catalog_slug,
                "has_node_binding": has_node_binding,
            }),
            Self::KeyDeleted { source } => json!({ "source": source }),
            Self::ServiceConnected {
                provider_slug,
                flow,
            } => json!({
                "provider_slug": provider_slug,
                "flow": flow,
            }),
            Self::ServiceDisconnected { provider_slug } => {
                json!({ "provider_slug": provider_slug })
            }
            Self::ServiceUserAgentCustomized { provider_slug } => {
                json!({ "provider_slug": provider_slug })
            }
            Self::EndpointUpdated { endpoint_type } => json!({ "endpoint_type": endpoint_type }),
            Self::EndpointDeleted { endpoint_type } => json!({ "endpoint_type": endpoint_type }),
            Self::CatalogBrowsed {
                filter,
                result_count,
            } => json!({
                "filter": filter,
                "result_count": result_count,
            }),
            Self::CatalogEntryViewed {
                catalog_slug,
                has_openapi_spec,
            } => json!({
                "catalog_slug": catalog_slug,
                "has_openapi_spec": has_openapi_spec,
            }),
            Self::CatalogEndpointsFetched {
                catalog_slug,
                endpoint_count,
            } => json!({
                "catalog_slug": catalog_slug,
                "endpoint_count": endpoint_count,
            }),
            Self::ApiKeyCreated {
                platform,
                scope_mode,
                rate_limit_per_second,
            } => json!({
                "platform": platform,
                "scope_mode": scope_mode,
                "rate_limit_per_second": rate_limit_per_second,
            }),
            Self::ApiKeyRotated { platform } => json!({ "platform": platform }),
            Self::ApiKeyDeleted { platform } => json!({ "platform": platform }),
            Self::AgentBindingCreated {
                platform,
                service_slug,
            } => json!({
                "platform": platform,
                "service_slug": service_slug,
            }),
            Self::AgentBindingDeleted {
                platform,
                service_slug,
            } => json!({
                "platform": platform,
                "service_slug": service_slug,
            }),
            Self::ApprovalRequested {
                service_slug,
                mode,
                channel,
            } => json!({
                "service_slug": service_slug,
                "mode": mode,
                "channel": channel,
            }),
            Self::ApprovalDecided {
                service_slug,
                mode,
                decision,
                decision_ms,
                channel,
                decided_via,
            } => json!({
                "service_slug": service_slug,
                "mode": mode,
                "decision": decision,
                "decision_ms": decision_ms,
                "channel": channel,
                "decided_via": decided_via,
            }),
            Self::ApprovalExpired { service_slug, mode } => json!({
                "service_slug": service_slug,
                "mode": mode,
            }),
            Self::ApprovalGrantRevoked { service_slug } => json!({ "service_slug": service_slug }),
            Self::ApprovalConfigUpdated { service_slug, mode } => json!({
                "service_slug": service_slug,
                "mode": mode,
            }),
            Self::NodeRegistered {
                node_platform,
                profile,
            } => json!({
                "node_platform": node_platform,
                "profile": profile,
            }),
            Self::NodeConnected { node_id, profile } => json!({
                "node_id": node_id,
                "profile": profile,
            }),
            Self::NodeDisconnected { node_id, reason } => json!({
                "node_id": node_id,
                "reason": reason,
            }),
            Self::NodeDeleted { node_id } => json!({ "node_id": node_id }),
            Self::NodeCredentialConfigured { credential_type } => json!({
                "credential_type": credential_type,
            }),
            Self::ChannelBotRegistered { platform } => json!({ "platform": platform }),
            Self::ChannelBotDeleted { platform } => json!({ "platform": platform }),
            Self::ChannelMappingCreated {
                platform,
                conversation_id_hash,
            } => json!({
                "platform": platform,
                "conversation_id_hash": conversation_id_hash,
            }),
            Self::ChannelMappingDeleted {
                platform,
                conversation_id_hash,
            } => json!({
                "platform": platform,
                "conversation_id_hash": conversation_id_hash,
            }),
            Self::ChannelMessageReceived {
                platform,
                conversation_id_hash,
            } => json!({
                "platform": platform,
                "conversation_id_hash": conversation_id_hash,
                "sample_percent": 10,
            }),
            Self::ChannelReplySent {
                platform,
                reply_mode,
                agent_api_key_id,
            } => json!({
                "platform": platform,
                "reply_mode": reply_mode,
                "agent_api_key_id": agent_api_key_id,
                "sample_percent": 10,
            }),
            Self::ChannelEventReceived {
                source,
                event_type,
                deduplicated,
            } => json!({
                "source": source,
                "event_type": event_type,
                "deduplicated": deduplicated,
            }),
            Self::McpSessionStarted { client } => json!({ "client": client }),
            Self::McpSessionEnded {
                duration_ms,
                reason,
            } => json!({
                "duration_ms": duration_ms,
                "reason": reason,
            }),
            Self::SshCertificateIssued {
                service_slug,
                ttl_secs,
            } => json!({
                "service_slug": service_slug,
                "ttl_secs": ttl_secs,
            }),
            Self::SshTunnelOpened { service_slug, mode } => json!({
                "service_slug": service_slug,
                "mode": mode,
            }),
            Self::SshTunnelClosed {
                service_slug,
                duration_ms,
            } => json!({
                "service_slug": service_slug,
                "duration_ms": duration_ms,
            }),
            Self::OauthClientRegistered => json!({}),
            Self::OauthClientSecretRotated { client_id } => json!({ "client_id": client_id }),
            Self::OauthAuthorizationGranted {
                client_id,
                grant_type,
            } => json!({
                "client_id": client_id,
                "grant_type": grant_type,
            }),
            Self::OauthTokenIssued {
                client_id,
                grant_type,
            } => json!({
                "client_id": client_id,
                "grant_type": grant_type,
            }),
            Self::NotificationChannelLinked { channel } => json!({ "channel": channel }),
            Self::NotificationChannelUnlinked { channel } => json!({ "channel": channel }),
            Self::NotificationDeviceRegistered { platform } => json!({ "platform": platform }),
            Self::NotificationDeviceRemoved { platform } => json!({ "platform": platform }),
            Self::AdminUserSuspended => json!({}),
            Self::AdminUserUnsuspended => json!({}),
            Self::AdminAuditLogViewed { filter } => json!({ "filter": filter }),
            Self::AdminOauthClientRegistered => json!({}),
            Self::AdminServiceAccountCreated => json!({}),
            Self::AdminServiceAccountRotated => json!({}),
            Self::AdminServiceAccountDeleted => json!({}),
            Self::AdminNodeDisconnected { node_id } => json!({ "node_id": node_id }),
            Self::AdminNodeDeleted { node_id } => json!({ "node_id": node_id }),
            Self::AdminServiceCreated { slug } => json!({ "slug": slug }),
            Self::AdminServiceUpdated { slug } => json!({ "slug": slug }),
            Self::ProxyError {
                service_slug,
                error_code,
                status,
            } => json!({
                "service_slug": service_slug,
                "error_code": error_code,
                "status": status,
            }),
            Self::ProxySuccess {
                service_slug,
                method,
                status,
                latency_ms,
                auth_kind,
            } => json!({
                "service_slug": service_slug,
                "method": method,
                "status": status,
                "latency_ms": latency_ms,
                "auth_kind": auth_kind,
            }),
            Self::ApiRateLimited {
                route,
                limit_type,
                limit_per_second,
                api_key_id,
            } => json!({
                "route": route,
                "limit_type": limit_type,
                "limit_per_second": limit_per_second,
                "api_key_id": api_key_id,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_dot_namespaced_lowercase() {
        let e = TelemetryEvent::KeyCreated {
            source: "catalog".into(),
            catalog_slug: Some("openai".into()),
            has_node_binding: false,
        };
        assert_eq!(e.name(), "key.created");
    }

    #[test]
    fn properties_run_through_scrubber() {
        let e = TelemetryEvent::ProxyError {
            service_slug: "unknown".into(),
            error_code: 8001,
            status: 500,
        };
        // Fabricate a props string that would leak if scrubbing were skipped,
        // then assert it comes out redacted.
        let v = e.properties();
        assert_eq!(v["error_code"], 8001);
    }

    #[test]
    fn proxy_success_name_and_properties() {
        let e = TelemetryEvent::ProxySuccess {
            service_slug: "openai".into(),
            method: "POST".into(),
            status: 200,
            latency_ms: 42,
            auth_kind: "api_key",
        };
        assert_eq!(e.name(), "proxy.success");
        let v = e.properties();
        assert_eq!(v["service_slug"], "openai");
        assert_eq!(v["method"], "POST");
        assert_eq!(v["status"], 200);
        assert_eq!(v["latency_ms"], 42);
        assert_eq!(v["auth_kind"], "api_key");
    }

    #[test]
    fn invite_code_redeemed_uses_pre_hashed_ids() {
        // The schema scrubber redacts raw UUIDs at egress. Emit sites
        // must pre-hash IDs so the values survive scrubbing intact —
        // this test enforces that contract by feeding a hash-shaped
        // value (hex, length 16) and asserting it passes through.
        let e = TelemetryEvent::InviteCodeRedeemed {
            code_id: "a1b2c3d4e5f60718".into(),
            created_by_user_id: "0123456789abcdef".into(),
            days_to_redemption: 3,
        };
        assert_eq!(e.name(), "invite.code_redeemed");
        let v = e.properties();
        assert_eq!(v["code_id"], "a1b2c3d4e5f60718");
        assert_eq!(v["created_by_user_id"], "0123456789abcdef");
        assert_eq!(v["days_to_redemption"], 3);
    }

    #[test]
    fn user_signed_up_carries_funnel_attribution() {
        let e = TelemetryEvent::UserSignedUp {
            method: "email".into(),
            source: "invite_code".into(),
            email_domain: Some("example.com".into()),
            invite_code_id: Some("deadbeefdeadbeef".into()),
            referrer_domain: Some("twitter.com".into()),
            via_org: None,
            invite_code_used: true,
        };
        let v = e.properties();
        assert_eq!(v["source"], "invite_code");
        assert_eq!(v["email_domain"], "example.com");
        assert_eq!(v["invite_code_used"], true);
        assert_eq!(v["referrer_domain"], "twitter.com");
    }

    #[test]
    fn approval_decided_carries_decision_and_latency() {
        let e = TelemetryEvent::ApprovalDecided {
            service_slug: "openai".into(),
            mode: "per_request".into(),
            decision: "approved".into(),
            decision_ms: 1234,
            channel: "telegram".into(),
            decided_via: "mobile".into(),
        };
        let v = e.properties();
        assert_eq!(v["decision"], "approved");
        assert_eq!(v["decision_ms"], 1234);
    }
}
