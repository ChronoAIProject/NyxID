use super::AppError;

impl AppError {
    /// Only stable proxy refusals qualify. Keep this match exhaustive: adding an
    /// AppError requires a deliberate decision even when its HTTP status is 4xx.
    pub(crate) fn counts_for_refusal_throttle(&self) -> bool {
        let counted = match self {
            Self::BadRequest(_)
            | Self::CredentialUnavailable(_)
            | Self::RequestBodyTooLarge { .. }
            | Self::Forbidden(_)
            | Self::ForbiddenWithGuidance { .. }
            | Self::NotFound(_)
            | Self::ApprovalUnsupported
            | Self::ToolOperationNotPublished
            | Self::AssistantModelUnavailable
            | Self::AssistantAttachmentExpired
            | Self::ValidationError(_)
            | Self::ServiceSelectionInvalid { .. }
            | Self::InvalidRedirectUri
            | Self::InvalidScope(_)
            | Self::InvalidTarget(_)
            | Self::SystemRoleProtected(_)
            | Self::CircularGroupHierarchy
            | Self::SshPrincipalAmbiguous(_)
            | Self::SshAuthModeUnsupportedForOperation(_)
            | Self::UnsupportedGrantType(_)
            | Self::MachineCapabilityDisabled
            | Self::MachineNotAllowed
            | Self::MachinePathOutsideRoots
            | Self::MachineJobNotFound
            | Self::MachineConfirmationDeclined
            | Self::MachineLimitExceeded
            | Self::MachineNotIsolated
            | Self::MachineLoginNotFound
            | Self::MachineLoginOriginMismatch
            | Self::MachineLoginWrongField
            | Self::MachineComputerPermissionMissing
            | Self::MachineComputerToolUnsupported
            | Self::MachineAuthorityUnsupported
            | Self::MachinePermissionRevoked
            | Self::NodeNotFound(_)
            | Self::NodeRegistrationFailed(_)
            | Self::PendingCredentialDecryptFailed(_)
            | Self::PendingCredentialVersionUnsupported(_)
            | Self::PendingCredentialCiphertextTooLarge(_)
            | Self::ApiKeyScopeForbidden(_)
            | Self::GuestModelRouteRequired
            | Self::ApiKeyScopeInactive
            | Self::ApiKeyScopeNotFound(_)
            | Self::ApiKeyScopePlanNotFound(_)
            | Self::ApiKeyScopePlanDenied(_)
            | Self::ApiKeyScopePlanOwnerUnsupported(_)
            | Self::DurableGrantMissing(_)
            | Self::DurableGrantMismatch(_)
            | Self::DurableGrantExpired
            | Self::DurableGrantRevoked
            | Self::DurableGrantContractDrift
            | Self::DurableOperationConflict
            | Self::OrgNotFound(_)
            | Self::OrgMembershipRequired
            | Self::OrgRoleInsufficient(_)
            | Self::AnonymousIncompatibleService(_)
            | Self::ServicePoolNotFound(_)
            | Self::ServicePoolSlugTaken(_)
            | Self::ServicePoolMemberInvalid(_)
            | Self::AnonymousIncompatibleBilling(_) => true,
            Self::InvocationDiagnostic { source, .. } => source.counts_for_refusal_throttle(),
            Self::Unauthorized(_)
            | Self::OAuthAuthorityUnavailable
            | Self::SkillRead(_)
            | Self::AssistantMissingAccess(_)
            | Self::Conflict(_)
            | Self::TriggerOverlapSkip { .. }
            | Self::AssistantTurnActive
            | Self::VoiceQueueFull
            | Self::VoiceProviderUnavailable
            | Self::MonitoringUnavailable
            | Self::MonitoringNotConfigured
            | Self::DiagnosticNotReady
            // Crash reporting is a first-party diagnostics surface, not a proxy refusal.
            | Self::CrashReportingUnavailable
            | Self::CrashReportConsentRequired
            | Self::BillingRecoveryWaiting { .. }
            | Self::BillingRecoveryFailed { .. }
            | Self::VoiceStartFailed(_)
            | Self::AssistantTurnRequired
            | Self::OrgAgentAccessAuthoringUnavailable
            | Self::AssistantModelAuthorityChanged
            | Self::GrantCascadeConfirmationRequired(_)
            | Self::RateLimited
            | Self::RateLimitedWithRetry { .. }
            | Self::ServiceConcurrencyLimited
            | Self::SecurityReadinessUnavailable
            | Self::Internal(_)
            | Self::PoolAttemptTransport(_)
            | Self::DatabaseError(_)
            | Self::MemoryWrite(_)
            | Self::SkillDraftValidation(_)
            | Self::AuthenticationFailed(_)
            | Self::TokenExpired
            | Self::RefreshRefused { .. }
            | Self::MfaRequired { .. }
            | Self::MfaStepUpRequired
            | Self::PkceVerificationFailed
            | Self::InvalidDpopProof(_)
            | Self::RequiredServiceNotConnected { .. }
            | Self::RoleNotFound(_)
            | Self::GroupNotFound(_)
            | Self::ConsentNotFound
            | Self::RoleAlreadyAssigned
            | Self::GroupMembershipExists
            | Self::DuplicateSlug(_)
            | Self::ServiceAccountNotFound(_)
            | Self::ServiceAccountInactive
            | Self::SocialAuthFailed(_)
            | Self::SocialAuthConflict
            | Self::SocialAuthNoEmail
            | Self::SocialAuthDeactivated
            | Self::SocialAuthRegistrationClosed
            | Self::EmailSignupDisabled
            | Self::SshNodeKeyMissing(_)
            | Self::SshHostKeyMismatch(_)
            | Self::SshNodeExecChannelClosed(_)
            | Self::ConsentRequired { .. }
            | Self::ApprovalRequired { .. }
            | Self::ApprovalFailed { .. }
            | Self::ExternalTokenInvalid(_)
            | Self::ExternalProviderNotConfigured(_)
            | Self::MachineConfirmationPending
            | Self::MachineComputerUnavailable
            | Self::MachineOwnerInControl
            | Self::MachineBrowserUnavailable
            | Self::MachineDriverRestarting
            | Self::MachineDisplayUnavailable
            | Self::MachineTurnStopped
            | Self::MachineAuthorityStale
            | Self::MachineAuthorityBusy
            | Self::NodeOffline(_)
            | Self::NodeProxyTimeout
            | Self::NodeCredentialMissing(_)
            | Self::NodeHttpSignatureUnsupported
            | Self::WorkspaceDestinationsNotActivated
            | Self::WsProxyDownstream(_)
            | Self::ClientDisconnected
            | Self::PendingCredentialPubkeyAwaiting(_)
            | Self::PendingCredentialQueueFull(_)
            | Self::ApiKeyScopePlanRouteUnresolved(_)
            | Self::ApiKeyScopePlanStale(_)
            | Self::DurableGrantQuotaExhausted
            | Self::DurableOperationDuplicate
            | Self::DurableOperationOutcomeUncertain
            | Self::DeviceCodeNotFound
            | Self::DeviceCodeExpired
            | Self::DevicePollSignatureInvalid(_)
            | Self::DeviceUserCodeInvalid
            | Self::DeviceCodePending
            | Self::DeviceCodeAlreadyDelivered
            | Self::DeviceCodeRateLimited
            | Self::DeviceCodeLocked
            | Self::DeviceCodeSlowDown
            | Self::AuthDeviceCodeNotFound
            | Self::AuthDeviceCodeExpired
            | Self::AuthDeviceCodePending
            | Self::AuthDeviceCodeSlowDown
            | Self::AuthDeviceCodeDenied
            | Self::AuthDeviceCodeAlreadyDelivered
            | Self::AuthDeviceCodeRateLimited
            | Self::AuthDeviceUserCodeInvalid
            | Self::AgentKeyLoginNotFound
            | Self::AgentKeyLoginExpired
            | Self::AgentKeyLoginPending
            | Self::AgentKeyLoginSlowDown
            | Self::AgentKeyLoginDenied
            | Self::AgentKeyLoginAlreadyDelivered
            | Self::AgentKeyLoginRateLimited
            | Self::AgentKeyLoginUserCodeInvalid
            | Self::AgentKeyLoginKeyIneligible
            | Self::AgentKeyCredentialNotFound
            | Self::LoginCodeInvalid
            | Self::LoginCodeExpired
            | Self::LoginCodeCancelled
            | Self::LoginCodeRedeemed
            | Self::LoginCodeRateLimited
            | Self::ConnectLinkNotFound
            | Self::ConnectLinkExpired
            | Self::ConnectLinkAlreadyCompleted
            | Self::ConnectLinkCancelled
            | Self::ConnectLinkRateLimited
            | Self::ConnectLinkCompletionInProgress
            | Self::TriggerNotFound
            | Self::TriggerSecretInvalid
            | Self::TriggerRateLimited
            | Self::TriggerPayloadTooLarge
            | Self::TriggerDeliveryUnsupported
            | Self::TriggerDeliveryFailed
            | Self::TriggerDeliveryRecordNotFound
            | Self::ChannelBotNotFound(_)
            | Self::ChannelBotInactive(_)
            | Self::ChannelBotLimitReached(_)
            | Self::ChannelWebhookVerificationFailed(_)
            | Self::ChannelRelayFailed(_)
            | Self::ChannelPlatformError(_)
            | Self::ChannelPlatformEditUnsupported
            | Self::DeviceChannelReplyNotAllowed
            | Self::ChannelConversationNotReachable(_)
            | Self::ChannelMediaUnsupported
            | Self::ChannelMediaTooLarge
            | Self::ChannelMediaFetchFailed(_)
            | Self::ChannelAttachmentNotFound
            | Self::ChannelPlatformSendUnsupported
            | Self::ChannelConversationNotAddressable
            | Self::ChannelAgentInitiateNotAllowed
            | Self::OrgCannotAuthenticate
            | Self::OrgQueryTimeout
            | Self::AdminUsageQueryTimeout
            | Self::OrgSlugTaken(_)
            | Self::OrgInviteInvalid(_)
            | Self::OrgInviteExpired
            | Self::OrgApprovalNoAdmin(_)
            | Self::InviteCodeInvalid
            | Self::InviteCodeExhausted
            | Self::InviteCodeDeactivated
            | Self::InviteCodeAlreadyRedeemed
            | Self::OraclePoolNotFound(_)
            | Self::OraclePoolSlugTaken(_)
            | Self::OraclePoolInactive(_)
            | Self::OracleWorkerTokenInvalid
            | Self::OracleQueueFull(_)
            | Self::OracleQuotaExceeded(_)
            | Self::OracleTaskNotFound(_)
            | Self::OracleSessionNotFound(_)
            | Self::OracleSessionClosed(_)
            | Self::OraclePayloadTooLarge(_)
            | Self::OracleExtractDisabled(_)
            | Self::OracleWorkerNotFound(_)
            | Self::OracleWorkerCapabilityUnsupported(_)
            | Self::OracleWorkerCommandNotFound(_)
            | Self::OracleWorkerLabelUnavailable(_)
            | Self::OracleLoginSnapshotNotFound(_)
            | Self::OracleWorkerCredentialRenewalRequired
            | Self::OraclePoolActiveWork
            | Self::OracleRetired
            | Self::ServicePoolNoViableMember(_)
            | Self::ServicePoolAttemptsExhausted { .. }
            | Self::ServicePoolInfrastructureUnavailable
            | Self::ServicePoolUpstreamEncodingUnsupported
            | Self::ServicePoolCoolingDown
            | Self::ServicePoolDeadlineExceeded { .. }
            | Self::InsufficientCredits
            | Self::BillingNotConfigured(_)
            | Self::BillingProviderUnavailable(_)
            | Self::ManagedExecutionPending(_)
            | Self::ErasurePreparing
            | Self::PlanEntitlementRequired(_)
            | Self::WalletSuspended => false,
        };
        counted
            && self.status_code().is_client_error()
            && self.status_code() != axum::http::StatusCode::UNAUTHORIZED
            && self.status_code() != axum::http::StatusCode::TOO_MANY_REQUESTS
            && self.retry_after_seconds().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mw::refusal_throttle::{Admission, RefusalThrottle, request_key};

    #[test]
    fn excluded_variants_never_accumulate_after_one_hundred_repetitions() {
        let cases = [
            AppError::Unauthorized("synthetic".into()),
            AppError::OAuthAuthorityUnavailable,
            AppError::Conflict("synthetic".into()),
            AppError::AssistantTurnActive,
            AppError::VoiceQueueFull,
            AppError::VoiceProviderUnavailable,
            AppError::MonitoringUnavailable,
            AppError::MonitoringNotConfigured,
            AppError::DiagnosticNotReady,
            AppError::CrashReportingUnavailable,
            AppError::CrashReportConsentRequired,
            AppError::AssistantTurnRequired,
            AppError::AssistantModelAuthorityChanged,
            AppError::RateLimited,
            AppError::ServiceConcurrencyLimited,
            AppError::SecurityReadinessUnavailable,
            AppError::Internal("synthetic".into()),
            AppError::AuthenticationFailed("synthetic".into()),
            AppError::TokenExpired,
            AppError::MfaStepUpRequired,
            AppError::PkceVerificationFailed,
            AppError::InvalidDpopProof("synthetic".into()),
            AppError::RoleNotFound("synthetic".into()),
            AppError::GroupNotFound("synthetic".into()),
            AppError::ConsentNotFound,
            AppError::RoleAlreadyAssigned,
            AppError::GroupMembershipExists,
            AppError::DuplicateSlug("synthetic".into()),
            AppError::ServiceAccountNotFound("synthetic".into()),
            AppError::ServiceAccountInactive,
            AppError::SocialAuthFailed("synthetic".into()),
            AppError::SocialAuthConflict,
            AppError::SocialAuthNoEmail,
            AppError::SocialAuthDeactivated,
            AppError::SocialAuthRegistrationClosed,
            AppError::EmailSignupDisabled,
            AppError::SshNodeKeyMissing("synthetic".into()),
            AppError::SshHostKeyMismatch("synthetic".into()),
            AppError::SshNodeExecChannelClosed("synthetic".into()),
            AppError::ExternalTokenInvalid("synthetic".into()),
            AppError::ExternalProviderNotConfigured("synthetic".into()),
            AppError::MachineConfirmationPending,
            AppError::MachineComputerUnavailable,
            AppError::MachineOwnerInControl,
            AppError::MachineBrowserUnavailable,
            AppError::MachineDriverRestarting,
            AppError::MachineDisplayUnavailable,
            AppError::MachineTurnStopped,
            AppError::MachineAuthorityStale,
            AppError::MachineAuthorityBusy,
            AppError::NodeOffline("synthetic".into()),
            AppError::NodeProxyTimeout,
            AppError::NodeCredentialMissing("synthetic".into()),
            AppError::NodeHttpSignatureUnsupported,
            AppError::WorkspaceDestinationsNotActivated,
            AppError::WsProxyDownstream("synthetic".into()),
            AppError::ClientDisconnected,
            AppError::PendingCredentialPubkeyAwaiting("synthetic".into()),
            AppError::PendingCredentialQueueFull("synthetic".into()),
            AppError::ApiKeyScopePlanRouteUnresolved("synthetic".into()),
            AppError::ApiKeyScopePlanStale("synthetic".into()),
            AppError::DurableGrantQuotaExhausted,
            AppError::DurableOperationDuplicate,
            AppError::DurableOperationOutcomeUncertain,
            AppError::DeviceCodeNotFound,
            AppError::DeviceCodeExpired,
            AppError::DevicePollSignatureInvalid("synthetic".into()),
            AppError::DeviceUserCodeInvalid,
            AppError::DeviceCodePending,
            AppError::DeviceCodeAlreadyDelivered,
            AppError::DeviceCodeRateLimited,
            AppError::DeviceCodeLocked,
            AppError::DeviceCodeSlowDown,
            AppError::AuthDeviceCodeNotFound,
            AppError::AuthDeviceCodeExpired,
            AppError::AuthDeviceCodePending,
            AppError::AuthDeviceCodeSlowDown,
            AppError::AuthDeviceCodeDenied,
            AppError::AuthDeviceCodeAlreadyDelivered,
            AppError::AuthDeviceCodeRateLimited,
            AppError::AuthDeviceUserCodeInvalid,
            AppError::AgentKeyLoginNotFound,
            AppError::AgentKeyLoginExpired,
            AppError::AgentKeyLoginPending,
            AppError::AgentKeyLoginSlowDown,
            AppError::AgentKeyLoginDenied,
            AppError::AgentKeyLoginAlreadyDelivered,
            AppError::AgentKeyLoginRateLimited,
            AppError::AgentKeyLoginUserCodeInvalid,
            AppError::AgentKeyLoginKeyIneligible,
            AppError::AgentKeyCredentialNotFound,
            AppError::LoginCodeInvalid,
            AppError::LoginCodeExpired,
            AppError::LoginCodeCancelled,
            AppError::LoginCodeRedeemed,
            AppError::LoginCodeRateLimited,
            AppError::ConnectLinkNotFound,
            AppError::ConnectLinkExpired,
            AppError::ConnectLinkAlreadyCompleted,
            AppError::ConnectLinkCancelled,
            AppError::ConnectLinkRateLimited,
            AppError::ConnectLinkCompletionInProgress,
            AppError::TriggerNotFound,
            AppError::TriggerSecretInvalid,
            AppError::TriggerRateLimited,
            AppError::TriggerPayloadTooLarge,
            AppError::TriggerDeliveryUnsupported,
            AppError::TriggerDeliveryFailed,
            AppError::TriggerDeliveryRecordNotFound,
            AppError::ChannelBotNotFound("synthetic".into()),
            AppError::ChannelBotInactive("synthetic".into()),
            AppError::ChannelBotLimitReached("synthetic".into()),
            AppError::ChannelWebhookVerificationFailed("synthetic".into()),
            AppError::ChannelRelayFailed("synthetic".into()),
            AppError::ChannelPlatformError("synthetic".into()),
            AppError::ChannelPlatformEditUnsupported,
            AppError::DeviceChannelReplyNotAllowed,
            AppError::ChannelConversationNotReachable("synthetic".into()),
            AppError::ChannelMediaUnsupported,
            AppError::ChannelMediaTooLarge,
            AppError::ChannelMediaFetchFailed("synthetic".into()),
            AppError::ChannelAttachmentNotFound,
            AppError::ChannelPlatformSendUnsupported,
            AppError::ChannelConversationNotAddressable,
            AppError::ChannelAgentInitiateNotAllowed,
            AppError::OrgCannotAuthenticate,
            AppError::OrgQueryTimeout,
            AppError::AdminUsageQueryTimeout,
            AppError::OrgSlugTaken("synthetic".into()),
            AppError::OrgInviteInvalid("synthetic".into()),
            AppError::OrgInviteExpired,
            AppError::OrgApprovalNoAdmin("synthetic".into()),
            AppError::InviteCodeInvalid,
            AppError::InviteCodeExhausted,
            AppError::InviteCodeDeactivated,
            AppError::InviteCodeAlreadyRedeemed,
            AppError::OraclePoolNotFound("synthetic".into()),
            AppError::OraclePoolSlugTaken("synthetic".into()),
            AppError::OraclePoolInactive("synthetic".into()),
            AppError::OracleWorkerTokenInvalid,
            AppError::OracleQueueFull("synthetic".into()),
            AppError::OracleQuotaExceeded("synthetic".into()),
            AppError::OracleTaskNotFound("synthetic".into()),
            AppError::OracleSessionNotFound("synthetic".into()),
            AppError::OracleSessionClosed("synthetic".into()),
            AppError::OraclePayloadTooLarge("synthetic".into()),
            AppError::OracleExtractDisabled("synthetic".into()),
            AppError::OracleWorkerNotFound("synthetic".into()),
            AppError::OracleWorkerCapabilityUnsupported("synthetic".into()),
            AppError::OracleWorkerCommandNotFound("synthetic".into()),
            AppError::OracleWorkerLabelUnavailable("synthetic".into()),
            AppError::OracleLoginSnapshotNotFound("synthetic".into()),
            AppError::OracleWorkerCredentialRenewalRequired,
            AppError::OraclePoolActiveWork,
            AppError::OracleRetired,
            AppError::ServicePoolNoViableMember("synthetic".into()),
            AppError::ServicePoolInfrastructureUnavailable,
            AppError::ServicePoolUpstreamEncodingUnsupported,
            AppError::ServicePoolCoolingDown,
            AppError::InsufficientCredits,
            AppError::BillingNotConfigured("synthetic".into()),
            AppError::BillingProviderUnavailable("synthetic".into()),
            AppError::ManagedExecutionPending("synthetic".into()),
            AppError::ErasurePreparing,
            AppError::PlanEntitlementRequired("synthetic".into()),
            AppError::WalletSuspended,
            AppError::RateLimitedWithRetry { retry_after: 30 },
            AppError::MfaRequired {
                session_token: "synthetic".into(),
            },
            AppError::ConsentRequired {
                consent_url: "https://example.test".into(),
            },
            AppError::ApprovalRequired {
                request_id: "synthetic".into(),
            },
            AppError::ApprovalFailed {
                request_id: "synthetic".into(),
                approve_url: "https://example.test".into(),
                reason: "pending".into(),
            },
            AppError::RefreshRefused {
                source: Box::new(AppError::TokenExpired),
                reason: super::super::refresh::RefreshRefusalReason::RefreshExpired,
            },
            AppError::BillingRecoveryWaiting {
                source: Box::new(AppError::NotFound("synthetic".into())),
            },
            AppError::BillingRecoveryFailed {
                source: Box::new(AppError::NotFound("synthetic".into())),
                failed_units: 1,
            },
            AppError::ServicePoolAttemptsExhausted { attempts: vec![] },
            AppError::ServicePoolDeadlineExceeded { attempts: vec![] },
            AppError::AssistantMissingAccess(Box::new(serde_json::json!({}))),
            AppError::SkillRead(super::super::skill_read::SkillReadReason::SkillNotBound),
            super::super::voice_start::Stage::ProviderAnswer
                .error(AppError::NotFound("synthetic".into())),
            AppError::GrantCascadeConfirmationRequired(Box::new(
                super::super::GrantCascadePayload {
                    provider_slug: "synthetic".into(),
                    provider_name: "synthetic".into(),
                    revokes_grant: false,
                    siblings: vec![],
                    unaffected_other_app: vec![],
                    token_scope_available: false,
                },
            )),
            AppError::DatabaseError(mongodb::error::Error::custom("synthetic")),
            AppError::MemoryWrite(super::super::memory_write::MemoryWriteReason::MemoryConflict),
            super::super::skill_draft::SkillDraftValidation::at("synthetic", "test").into(),
            AppError::RequiredServiceNotConnected {
                service_slug: "synthetic".into(),
                service_name: "synthetic".into(),
            },
            AppError::PoolAttemptTransport(super::super::PoolAttemptTransportKind::AfterDispatch),
        ];
        for error in cases {
            let now = std::time::Instant::now();
            let limiter = RefusalThrottle::with_clock(std::sync::Arc::new(move || now), 8);
            let key = request_key(
                "key",
                "synthetic",
                "GET",
                "service",
                "/items",
                None,
                &[0; 32],
            );
            for _ in 0..100 {
                assert_eq!(limiter.admit(&key), Admission::Ordinary, "{error:?}");
                assert!(!error.counts_for_refusal_throttle(), "{error:?}");
                limiter.finish(
                    key,
                    false,
                    error
                        .counts_for_refusal_throttle()
                        .then(|| error.error_code()),
                );
            }
        }
    }
    #[tokio::test]
    async fn quiet_rate_limit_preserves_wire_response_without_warning() {
        use axum::{body::to_bytes, response::IntoResponse};
        use std::{
            io::Write,
            sync::{Arc, Mutex},
        };
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl Write for Capture {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || Capture(writer.clone()))
            .finish();
        let quiet = tracing::subscriber::with_default(subscriber, || {
            for _ in 0..100 {
                let _ = AppError::quiet_rate_limit_response(30);
            }
            AppError::quiet_rate_limit_response(30)
        });
        assert!(output.lock().unwrap().is_empty());
        let ordinary = AppError::RateLimitedWithRetry { retry_after: 30 }.into_response();
        assert_eq!(quiet.status(), ordinary.status());
        assert_eq!(quiet.headers(), ordinary.headers());
        assert_eq!(
            to_bytes(quiet.into_body(), usize::MAX).await.unwrap(),
            to_bytes(ordinary.into_body(), usize::MAX).await.unwrap()
        );
    }
}
