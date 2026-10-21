//! Service boundary for the public/personal manager implementation.
//!
//! These values are neither persisted models nor HTTP DTOs. Producers own those
//! representations and validate every proof against live state before effects.
//! This module registers no routes, enables no mode and supplies no runtime.

pub use crate::models::platform_entry_projection::{
    StoredEntryIdentity as EntryIdentity, StoredEventIdentity as EventIdentity,
    StoredSenderIdentity as SenderIdentity,
};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use http::Method;
use mongodb::{ClientSession, Database};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::{errors::AppResult, models::credits::Credits};

pub const CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagerChatKind {
    Private,
    Group,
    Supergroup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagerCommandEligibility {
    OriginalPrivateMessage,
    Ineligible,
}

/// Constructed from authenticated original native ingress, never normalized HTTP input.
#[derive(Clone, Debug)]
pub struct VerifiedManagerUpdate {
    pub profile_id: String,
    pub manager_bot_id: String,
    pub entry: EntryIdentity,
    pub observation_id: String,
    pub sender: SenderIdentity,
    pub chat_kind: ManagerChatKind,
    pub event: EventIdentity,
    pub command_eligibility: ManagerCommandEligibility,
}

/// Transport-authenticated proof/control input; forwarded ordinary text uses TurnInput.
/// Consumers also compare this update/provenance with the persisted ingress claim.
pub struct OriginalManagerCommand {
    update: VerifiedManagerUpdate,
    input: TurnInput,
}

impl OriginalManagerCommand {
    pub(crate) fn from_verified(update: VerifiedManagerUpdate, input: TurnInput) -> Option<Self> {
        if update.chat_kind != ManagerChatKind::Private
            || update.command_eligibility != ManagerCommandEligibility::OriginalPrivateMessage
        {
            return None;
        }
        Some(Self { update, input })
    }

    pub fn update(&self) -> &VerifiedManagerUpdate {
        &self.update
    }

    pub fn text(&self) -> &str {
        &self.input.text
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagerAuthority {
    pub profile_id: String,
    pub manager_bot_id: String,
    pub entry: EntryIdentity,
    pub manager_authority_epoch: i64,
}

/// Missing persisted controls return `None`, never an enabled default.
#[derive(Clone, Debug)]
pub struct ManagerRuntimePolicy {
    pub manager: ManagerAuthority,
    pub personal_enabled: bool,
    pub public_enabled: bool,
    pub emergency_stopped: bool,
    pub personal_control_revision: i64,
    pub public_policy_revision: i64,
    pub public_policy: Option<PublicPolicySelection>,
}

#[derive(Clone, Debug)]
pub struct PublicPolicySelection {
    pub sponsor_owner_user_id: String,
    pub nyxagent_profile: String,
    pub total_daily_credits: Credits,
    pub sender_daily_credits: Credits,
}

#[derive(Clone, Debug)]
pub struct ExpectedRuntimeRevisions {
    pub manager_authority_epoch: i64,
    pub personal_control_revision: i64,
    pub public_policy_revision: i64,
}

#[derive(Clone, Debug)]
pub struct RuntimePatch {
    pub public_deployment_digest: Option<String>,
    pub initialize: bool,
    pub expected_profile_generation: Option<i64>,
    pub expected: ExpectedRuntimeRevisions,
    pub personal_enabled: Option<bool>,
    pub public_enabled: Option<bool>,
    pub emergency_stopped: Option<bool>,
    pub public_policy: PolicyPatch,
}

#[derive(Clone, Debug)]
pub enum PolicyPatch {
    Unchanged,
    Clear,
    Update(PublicPolicyPatch),
}

#[derive(Clone, Debug)]
pub struct PublicPolicyPatch {
    pub sponsor_owner_user_id: Option<String>,
    pub nyxagent_profile: Option<String>,
    pub total_daily_credits: Option<Credits>,
    pub sender_daily_credits: Option<Credits>,
}

/// No public-policy or legacy onboarding generation participates in this proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonalBindingAuthority {
    pub manager: ManagerAuthority,
    pub sender: SenderIdentity,
    pub binding_id: String,
    pub binding_generation: i64,
    pub personal_control_revision: i64,
    pub user_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformOriginSource {
    Ingress(IngressClaim),
    ThreadEvent { admission_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformTurnOrigin {
    pub authority: PersonalBindingAuthority,
    pub nyxbot_agent_id: String,
    pub source: PlatformOriginSource,
}

#[derive(Clone, Debug)]
pub struct PersonalThreadClaim {
    pub nyxbot_agent_id: String,
    pub conversation_id: String,
}

#[derive(Clone, Debug)]
pub enum ThreadAuthorityResolution {
    Personal(Box<PersonalBindingAuthority>),
    WebOnly,
}

#[derive(Clone, Debug)]
pub enum SenderResolution {
    Unlinked,
    Personal {
        authority: Box<PersonalBindingAuthority>,
        nyxbot_agent_id: Option<String>,
        conversation_id: Option<String>,
    },
    Unavailable(UnavailableReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    ConfirmationRejected,
    ControlInvalid,
    ControlRateLimited,
    UnsupportedInput,
    ModeDisabled,
    ManagerUnavailable,
    EmergencyStopped,
    PersonalDisabled,
    PublicDisabled,
    OwnerUnavailable,
    TransportUnavailable,
    OriginRevoked,
    EngineUnavailable,
    InsufficientFunds,
    CapacityLimited,
    PublicNotReady,
    PublicQuotaLimited,
    RuntimeUninitialized,
    SponsorUnavailable,
    PublicContractUnsupported,
    PublicAuthorityRejected,
    ProfileUnavailable,
    ProofExpired,
    PublicTurnFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenceOutcome {
    Authorized,
    Unavailable(UnavailableReason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnRef {
    pub conversation_id: String,
    pub turn_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressClaim {
    pub receipt_id: String,
    pub claim_id: String,
}

#[derive(Clone, Debug)]
pub struct AdmittedPersonalUpdate {
    pub ingress: IngressClaim,
    pub authority: PersonalBindingAuthority,
}

#[derive(Clone, Debug)]
pub struct PublicIngress {
    pub ingress: IngressClaim,
    pub manager: ManagerAuthority,
    pub sender: SenderIdentity,
    pub public_policy_revision: i64,
}

#[derive(Clone, Debug)]
pub enum ManagerAudience {
    /// A reserved original-private command, atomically claimed with its effect.
    Control,
    Personal(AdmittedPersonalUpdate),
    Public(PublicIngress),
    Fixed(UnavailableReason),
    PrivateChatHandoff,
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // One bounded admission value, passed directly between producers.
pub enum IngressAdmission {
    Claimed {
        ingress: IngressClaim,
        audience: ManagerAudience,
    },
    Duplicate {
        receipt_id: String,
    },
}

/// Server policy only: exact service, operation and model, with no destination URL.
#[derive(Clone, Debug)]
pub struct PublicModelOperation {
    pub reasoning_effort: Option<String>,
    pub service_id: String,
    pub operation: String,
    pub method: Method,
    pub canonical_path: String,
    pub protocol: PublicModelProtocol,
    pub execution_authority_digest: String,
    pub endpoint_generation: Option<i64>,
    pub model_id: String,
    pub max_input_bytes: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_response_bytes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicModelProtocol {
    OpenAiResponses,
    OpenAiCompletions,
    AnthropicMessages,
}

#[derive(Clone, Debug)]
pub struct PublicExecutionLimits {
    pub max_duration_secs: u32,
    pub max_model_calls: u32,
    pub max_tool_calls: u32,
    pub max_continuations: u32,
    /// Whole-loop totals, including repeated input on retries/continuations.
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_input_bytes: u32,
    pub max_context_bytes: u32,
    pub max_output_bytes: u32,
}

/// Part 3 derives this from admitted state. It carries no credential material.
#[derive(Clone, Debug)]
pub struct PublicExecutionContext {
    pub proof_record_id: String,
    pub ingress: PublicIngress,
    pub public_session_id: String,
    pub turn: TurnRef,
    pub sponsor_owner_user_id: String,
    pub policy_digest: String,
    pub nyxagent_profile: String,
    pub model_operations: Vec<PublicModelOperation>,
    pub limits: PublicExecutionLimits,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub enum NyxAgentExecutionContext {
    Owner {
        user_id: String,
        turn: TurnRef,
        platform_origin: Option<PlatformTurnOrigin>,
    },
    Public(PublicExecutionContext),
}

/// Deliberately has no Debug/serde implementation: user text is private.
#[derive(Default)]
pub struct TurnInput {
    pub channel_uploads: Vec<super::assistant_upload_service::PreparedChannelUpload>,
    pub text: String,
}

/// References existing retained replies; this is not a durable body replay queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyRef {
    pub turn: TurnRef,
    pub reply_id: String,
}

#[derive(Clone, Debug)]
pub enum DeliveryRef {
    Reply(ReplyRef),
    Fixed { ingress: IngressClaim },
}

#[derive(Clone, Debug)]
pub enum ExecutionEvent {
    ReplyReady(ReplyRef),
    Settled { turn: TurnRef },
}

#[derive(Clone, Debug)]
pub enum ReplyAuthority {
    Personal {
        origin: PlatformTurnOrigin,
        turn: TurnRef,
    },
    Public {
        ingress: PublicIngress,
        public_session_id: String,
        turn: TurnRef,
    },
    Fixed {
        ingress: IngressClaim,
    },
}

#[derive(Clone, Debug)]
pub struct ReplyDestination {
    pub manager: ManagerAuthority,
    pub sender: SenderIdentity,
    pub authority: ReplyAuthority,
    pub expires_at: DateTime<Utc>,
}

/// Ephemeral, already rendered text/card notice; never log or persist for replay.
pub enum ReplyContent {
    Text(String),
    Fixed(UnavailableReason),
    PrivateChatHandoff,
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // Started carries the complete authority; this is not a collection.
pub enum PersonalTurnStart {
    Started {
        turn: TurnRef,
        destination: ReplyDestination,
    },
    Unavailable(UnavailableReason),
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // Capture moves its owned proof directly into durable recovery.
pub enum ExactStopOutcome {
    Captured(CapturedPlatformStop),
    NoMatchingActiveTurn,
}

/// Part 2 captures effects; Part 1 persists its recovery record before commit.
#[derive(Clone, Debug)]
pub struct CapturedPlatformStop {
    pub origin: PlatformTurnOrigin,
    pub turn: TurnRef,
    pub stop_requested_at: DateTime<Utc>,
    pub machine_jobs: Vec<ExactMachineJob>,
    pub machine_scopes: Vec<ExactMachineScope>,
}

#[derive(Clone, Debug)]
pub struct ExactMachineScope {
    pub node_id: String,
    pub runtime_id: String,
}

#[derive(Clone, Debug)]
pub struct ExactMachineJob {
    pub node_id: String,
    pub runtime_id: String,
    pub job_id: String,
}

#[derive(Clone, Debug)]
pub struct ExactStopProgress {
    pub settlement: ExactStopSettlement,
    pub machine_jobs: Vec<ExactMachineJobProgress>,
    pub machine_scopes: Vec<ExactMachineScopeProgress>,
}

#[derive(Clone, Debug)]
pub struct ExactMachineScopeProgress {
    pub scope: ExactMachineScope,
    pub outcome: ExactMachineCancelOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactStopSettlement {
    Pending,
    Settled,
    AlreadySettled,
    ConversationDeleted,
}

#[derive(Clone, Debug)]
pub struct ExactMachineJobProgress {
    pub job: ExactMachineJob,
    pub outcome: ExactMachineCancelOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactMachineCancelOutcome {
    Cancelled,
    AlreadyFinished,
    NodeUnavailable,
    Unsupported,
    RuntimeChanged,
}

/// Exact stored card provenance; no free-form continuation or action arguments.
#[derive(Clone, Debug)]
pub struct PlatformCardDecision {
    pub acknowledgement_id: String,
    pub requested_turn: TurnRef,
    pub allowed: bool,
    pub voice_request_id: Option<String>,
    pub continuation_receipt_id: Option<String>,
    pub trigger_run_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum PlatformDecisionOutcome {
    Decided(PlatformCardDecision),
    NoPendingDecision,
    Unavailable(UnavailableReason),
}

#[derive(Clone, Debug)]
pub struct PublicReadinessEvidence {
    pub manager: ManagerAuthority,
    pub public_policy_revision: i64,
    pub policy_digest: String,
    pub proof_record_id: String,
    pub candidate_digest: String,
    pub evidence_digest: String,
    pub sponsor_owner_user_id: String,
    pub nyxagent_build: String,
    pub nyxagent_profile: String,
    pub model_operations: Vec<PublicModelOperation>,
    pub limits: PublicExecutionLimits,
    pub checked_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // Readiness carries one bounded proof across the producer boundary.
pub enum PublicReadiness {
    Ready(PublicReadinessEvidence),
    Unavailable(UnavailableReason),
}

#[derive(Clone, Debug)]
pub struct PublicProfile {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct PublicProfileDiscovery {
    pub profiles: Vec<PublicProfile>,
    pub default_profile: Option<String>,
    pub readiness: PublicReadiness,
}

#[derive(Clone, Debug)]
pub enum PublicRevocationScope {
    Sender {
        manager: ManagerAuthority,
        sender: SenderIdentity,
    },
    Manager {
        manager: ManagerAuthority,
    },
    Sponsor {
        sponsor_owner_user_id: String,
    },
    Session {
        public_session_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicRevocationReason {
    PersonalLinkConfirmed,
    SenderUnlinked,
    ManagerRetired,
    SessionExpired,
}

#[derive(Clone, Debug)]
pub struct PublicRevocationRecorded;

/// Internal execution/egress material, never an event, HTTP DTO or log value.
pub struct RuntimeCredential {
    pub api_key_id: String,
    pub raw_key: Zeroizing<String>,
}

impl std::fmt::Debug for RuntimeCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RuntimeCredential { [REDACTED] }")
    }
}

/// Server-built relative operation. No caller URLs, cookies or header overrides.
pub struct NyxAgentRequest {
    pub method: Method,
    pub relative_path: String,
    pub body: Option<serde_json::Value>,
    pub idempotency_key: String,
    pub billing_policy: Option<super::billing::route_inventory::BillingRoutePolicy>,
}

pub struct PreparedPublicTurn {
    pub context: PublicExecutionContext,
    pub credential: RuntimeCredential,
    pub input: TurnInput,
    pub instructions: String,
    pub upstream_session_id: Option<String>,
}

/// Part 3 encrypts these observed handles under the exact public turn/key fence.
pub struct ObservedUpstreamBinding {
    pub session_id: Option<String>,
    pub response_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum SettledExecution {
    Saved(ReplyRef),
}

#[derive(Clone, Debug)]
pub struct PersonalRevocationResult {
    pub revoked: bool,
    pub stop_effect_record_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RecoveryBatch {
    pub processed: u32,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub enum DeliveryOutcome {
    Sent { telegram_message_ids: Vec<i64> },
    NativeSent { platform_message_ids: Vec<String> },
    Duplicate,
    Deferred { retry_at: DateTime<Utc> },
    Refused(UnavailableReason),
    Uncertain,
}

/// Part 1. All session-taking methods join the caller's existing transaction.
#[async_trait]
pub trait ManagerIdentityProducer: Send + Sync {
    async fn runtime_policy(
        &self,
        db: &Database,
        profile_id: &str,
    ) -> AppResult<Option<ManagerRuntimePolicy>>;

    async fn resolve_sender(
        &self,
        db: &Database,
        session: &mut ClientSession,
        update: &VerifiedManagerUpdate,
    ) -> AppResult<SenderResolution>;

    async fn fence_personal_origin(
        &self,
        db: &Database,
        session: &mut ClientSession,
        authority: &PersonalBindingAuthority,
    ) -> AppResult<FenceOutcome>;

    async fn claim_binding_thread(
        &self,
        db: &Database,
        session: &mut ClientSession,
        authority: &PersonalBindingAuthority,
        nyxbot_agent_id: &str,
        candidate_conversation_id: &str,
    ) -> AppResult<PersonalThreadClaim>;

    async fn claim_binding_turn(
        &self,
        db: &Database,
        session: &mut ClientSession,
        origin: &PlatformTurnOrigin,
        turn: &TurnRef,
    ) -> AppResult<()>;

    /// Settlement is allowed after revoke; compare only the exact old pointer.
    async fn clear_binding_turn(
        &self,
        db: &Database,
        session: &mut ClientSession,
        user_id: &str,
        binding_id: &str,
        binding_generation: i64,
        turn: &TurnRef,
    ) -> AppResult<bool>;

    async fn clear_binding_thread_reference(
        &self,
        db: &Database,
        session: &mut ClientSession,
        user_id: &str,
        binding_id: &str,
        binding_generation: i64,
        conversation_id: &str,
    ) -> AppResult<bool>;

    async fn resolve_thread_authority(
        &self,
        db: &Database,
        session: &mut ClientSession,
        user_id: &str,
        binding_id: &str,
        binding_generation: i64,
        conversation_id: &str,
    ) -> AppResult<ThreadAuthorityResolution>;
}

/// Part 1 owns durable personal retirement/stop records and bounded recovery.
#[async_trait]
pub trait ManagerLifecycleProducer: Send + Sync {
    async fn unlink_sender_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
        command: &OriginalManagerCommand,
    ) -> AppResult<PersonalRevocationResult>;

    async fn recover_personal_lifecycle(
        &self,
        db: &Database,
        personal: &dyn PersonalTurnProducer,
        limit: u32,
    ) -> AppResult<RecoveryBatch>;
}

/// Part 2. A start uses ordinary owner permissions, funds and pool admission.
#[async_trait]
pub trait PersonalTurnProducer: Send + Sync {
    async fn start_personal_turn(
        &self,
        db: &Database,
        admitted: &AdmittedPersonalUpdate,
        input: TurnInput,
        events: mpsc::Sender<ExecutionEvent>,
    ) -> AppResult<PersonalTurnStart>;

    async fn stop_exact_platform_turn(
        &self,
        db: &Database,
        session: &mut ClientSession,
        origin: &PlatformTurnOrigin,
        turn: &TurnRef,
    ) -> AppResult<ExactStopOutcome>;

    async fn apply_exact_platform_stop(
        &self,
        effects: &CapturedPlatformStop,
    ) -> AppResult<ExactStopProgress>;

    async fn decide_platform_reply(
        &self,
        db: &Database,
        admitted: &AdmittedPersonalUpdate,
        conversation_id: &str,
        command: OriginalManagerCommand,
    ) -> AppResult<PlatformDecisionOutcome>;
}

/// Part 2. Public cannot enter owner credential convergence through this enum.
#[async_trait]
pub trait NyxAgentExecutionProducer: Send + Sync {
    async fn execute_nyxagent_turn(
        &self,
        db: &Database,
        context: NyxAgentExecutionContext,
        input: TurnInput,
        events: mpsc::Sender<ExecutionEvent>,
    ) -> AppResult<()>;
}

/// Part 3. Readiness and all billable calls use the public budget path.
#[async_trait]
pub trait PublicTurnProducer: Send + Sync {
    async fn discover_public_profiles(
        &self,
        db: &Database,
        reviewed: &ManagerRuntimePolicy,
    ) -> AppResult<PublicProfileDiscovery>;

    async fn probe_public_readiness(
        &self,
        db: &Database,
        reviewed: &ManagerRuntimePolicy,
    ) -> AppResult<PublicReadiness>;

    /// Local revalidation only; no network or billed probe in this transaction.
    async fn fence_public_readiness(
        &self,
        db: &Database,
        session: &mut ClientSession,
        reviewed: &ManagerRuntimePolicy,
        evidence: &PublicReadinessEvidence,
    ) -> AppResult<FenceOutcome>;

    async fn prepare_public_context(
        &self,
        db: &Database,
        admitted: &PublicIngress,
    ) -> AppResult<PublicExecutionContext>;

    async fn fence_public_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        context: &PublicExecutionContext,
    ) -> AppResult<FenceOutcome>;

    async fn run_public_turn(
        &self,
        db: &Database,
        context: PublicExecutionContext,
        input: TurnInput,
        events: mpsc::Sender<ExecutionEvent>,
    ) -> AppResult<TurnRef>;

    async fn fence_public_reply(
        &self,
        db: &Database,
        session: &mut ClientSession,
        destination: &ReplyDestination,
    ) -> AppResult<FenceOutcome>;

    async fn revoke_public_sender(
        &self,
        db: &Database,
        session: &mut ClientSession,
        manager: &ManagerAuthority,
        sender: &SenderIdentity,
    ) -> AppResult<()>;

    async fn revoke_public_sessions_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
        scope: &PublicRevocationScope,
        reason: PublicRevocationReason,
    ) -> AppResult<PublicRevocationRecorded>;
}

/// Part 3 supplies this service adapter over the existing metered proxy core.
#[async_trait]
pub trait NyxAgentEgress: Send + Sync {
    async fn preload_skills(
        &self,
        _row: &crate::models::assistant_conversation::AssistantConversation,
        _agent: &crate::models::assistant_agent::AssistantAgent,
    ) -> AppResult<String> {
        Ok(String::new())
    }

    /// Optional human-channel display note after the durable acknowledgement.
    /// The adapter owns delivery authorization; the runtime cancels this task on acceptance.
    fn admission_acknowledged(
        &self,
        _row: &crate::models::assistant_conversation::AssistantConversation,
    ) -> Option<tokio::task::JoinHandle<()>> {
        None
    }

    async fn request(
        &self,
        authority: &NyxAgentExecutionContext,
        credential: &RuntimeCredential,
        request: NyxAgentRequest,
    ) -> AppResult<axum::response::Response>;
}

/// Selected before owner preparation. Public recovery never calls owner hooks.
#[async_trait]
pub trait PublicNyxAgentRuntime: Send + Sync {
    async fn prepare(
        &self,
        db: &Database,
        context: &PublicExecutionContext,
        input: TurnInput,
    ) -> AppResult<PreparedPublicTurn>;

    async fn replace_credential(
        &self,
        db: &Database,
        context: &PublicExecutionContext,
        expected_key_id: &str,
    ) -> AppResult<RuntimeCredential>;

    async fn record_upstream_binding(
        &self,
        db: &Database,
        context: &PublicExecutionContext,
        credential_api_key_id: &str,
        binding: &ObservedUpstreamBinding,
    ) -> AppResult<()>;

    async fn settle(
        &self,
        db: &Database,
        context: &PublicExecutionContext,
        message_id: &str,
        result: &super::assistant_nyxagent::TurnResult,
    ) -> AppResult<SettledExecution>;
}

/// Part 5. All claims join the engine/card transaction before spawn or effects.
#[async_trait]
pub trait ManagerIngressTurnStore: Send + Sync {
    async fn claim_personal_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        admitted: &AdmittedPersonalUpdate,
        turn: &TurnRef,
    ) -> AppResult<()>;

    async fn settle_personal_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        ingress: &IngressClaim,
        reply: &ReplyRef,
    ) -> AppResult<()>;

    async fn claim_personal_decision(
        &self,
        db: &Database,
        session: &mut ClientSession,
        admitted: &AdmittedPersonalUpdate,
        command: &OriginalManagerCommand,
        decision: &PlatformCardDecision,
    ) -> AppResult<()>;

    async fn settle_personal_decision(
        &self,
        db: &Database,
        session: &mut ClientSession,
        ingress: &IngressClaim,
        decision: &PlatformCardDecision,
    ) -> AppResult<()>;

    async fn claim_public_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        context: &PublicExecutionContext,
    ) -> AppResult<()>;

    async fn settle_public_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        ingress: &IngressClaim,
        reply: &ReplyRef,
    ) -> AppResult<()>;
}

/// Startup composition only; every member must be a real registered producer.
/// Absence at startup keeps the new mode unmounted. There is no Default runtime.
pub struct PlatformManagerProducers {
    pub identity: Arc<dyn ManagerIdentityProducer>,
    pub lifecycle: Arc<dyn ManagerLifecycleProducer>,
    pub personal: Arc<dyn PersonalTurnProducer>,
    pub public: Arc<dyn PublicTurnProducer>,
    pub transport: Arc<dyn ManagerTransportProducer>,
}

/// Part 5. Runners never send Telegram requests themselves.
#[async_trait]
pub trait ManagerTransportProducer: Send + Sync {
    async fn admit_manager_update(
        &self,
        db: &Database,
        update: &VerifiedManagerUpdate,
    ) -> AppResult<IngressAdmission>;

    async fn deliver_platform_reply(
        &self,
        db: &Database,
        destination: &ReplyDestination,
        delivery: &DeliveryRef,
        content: ReplyContent,
    ) -> AppResult<DeliveryOutcome>;
}
