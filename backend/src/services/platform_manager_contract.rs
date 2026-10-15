//! Service boundary for the public/personal manager implementation.
//!
//! These values are neither persisted models nor HTTP DTOs. Producers own those
//! representations and validate every proof against live state before effects.
//! This module registers no routes, enables no mode and supplies no runtime.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mongodb::{ClientSession, Database};
use tokio::sync::mpsc;

use crate::{errors::AppResult, models::credits::Credits};

pub const CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagerChatKind {
    Private,
    Group,
    Supergroup,
}

/// Constructed only from authenticated native Telegram ingress, never HTTP input.
#[derive(Clone, Debug)]
pub struct VerifiedManagerUpdate {
    pub profile_id: String,
    pub manager_bot_id: String,
    pub telegram_bot_id: i64,
    pub observation_id: String,
    pub sender_id: i64,
    pub chat_id: i64,
    pub chat_kind: ManagerChatKind,
    pub update_id: i64,
    pub message_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagerAuthority {
    pub profile_id: String,
    pub manager_bot_id: String,
    pub telegram_bot_id: i64,
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

/// No public-policy or legacy onboarding generation participates in this proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformTurnOrigin {
    pub manager: ManagerAuthority,
    pub sender_id: i64,
    pub chat_id: i64,
    pub binding_id: String,
    pub binding_generation: i64,
    pub personal_control_revision: i64,
    pub user_id: String,
    pub nyxbot_agent_id: String,
}

#[derive(Clone, Debug)]
pub enum SenderResolution {
    Unlinked,
    Personal {
        origin: PlatformTurnOrigin,
        conversation_id: Option<String>,
    },
    Unavailable(UnavailableReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
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

#[derive(Clone, Debug)]
pub struct IngressClaim {
    pub receipt_id: String,
    pub claim_id: String,
}

#[derive(Clone, Debug)]
pub struct AdmittedPersonalUpdate {
    pub ingress: IngressClaim,
    pub origin: PlatformTurnOrigin,
}

#[derive(Clone, Debug)]
pub struct PublicIngress {
    pub ingress: IngressClaim,
    pub manager: ManagerAuthority,
    pub sender_id: i64,
    pub chat_id: i64,
    pub public_policy_revision: i64,
}

#[derive(Clone, Debug)]
pub enum ManagerAudience {
    Personal(AdmittedPersonalUpdate),
    Public(PublicIngress),
    Fixed(UnavailableReason),
    PrivateChatHandoff,
}

#[derive(Clone, Debug)]
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
    pub service_id: String,
    pub operation: String,
    pub model_id: String,
}

#[derive(Clone, Debug)]
pub struct PublicExecutionLimits {
    pub max_duration_secs: u32,
    pub max_model_calls: u32,
    pub max_tool_calls: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
}

/// Part 3 derives this from admitted state. It carries no credential material.
#[derive(Clone, Debug)]
pub struct PublicExecutionContext {
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
pub struct TurnInput {
    pub text: String,
}

/// References existing retained replies; this is not a durable body replay queue.
#[derive(Clone, Debug)]
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
    pub sender_id: i64,
    pub chat_id: i64,
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
pub enum PersonalTurnStart {
    Started {
        turn: TurnRef,
        destination: ReplyDestination,
    },
    Unavailable(UnavailableReason),
}

#[derive(Clone, Debug)]
pub enum ExactStopOutcome {
    StopRecorded { cancellation_id: String },
    NoMatchingActiveTurn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformDecisionOutcome {
    Decided,
    NoPendingDecision,
    Unavailable(UnavailableReason),
}

#[derive(Clone, Debug)]
pub struct PublicReadinessEvidence {
    pub manager: ManagerAuthority,
    pub public_policy_revision: i64,
    pub policy_digest: String,
    pub sponsor_owner_user_id: String,
    pub nyxagent_build: String,
    pub nyxagent_profile: String,
    pub model_operations: Vec<PublicModelOperation>,
    pub limits: PublicExecutionLimits,
    pub checked_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub enum PublicReadiness {
    Ready(PublicReadinessEvidence),
    Unavailable(UnavailableReason),
}

#[derive(Clone, Debug)]
pub enum DeliveryOutcome {
    Sent { telegram_message_ids: Vec<i64> },
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
        origin: &PlatformTurnOrigin,
    ) -> AppResult<FenceOutcome>;

    async fn claim_binding_thread(
        &self,
        db: &Database,
        session: &mut ClientSession,
        origin: &PlatformTurnOrigin,
        conversation_id: &str,
    ) -> AppResult<String>;

    async fn clear_binding_thread_reference(
        &self,
        db: &Database,
        session: &mut ClientSession,
        origin: &PlatformTurnOrigin,
        conversation_id: &str,
    ) -> AppResult<bool>;
}

/// Part 2. A start uses ordinary owner permissions, funds and pool admission.
#[async_trait]
pub trait PersonalTurnProducer: Send + Sync {
    async fn start_personal_turn(
        &self,
        db: &Database,
        admitted: &AdmittedPersonalUpdate,
        input: TurnInput,
    ) -> AppResult<PersonalTurnStart>;

    async fn stop_exact_platform_turn(
        &self,
        db: &Database,
        session: &mut ClientSession,
        origin: &PlatformTurnOrigin,
        turn: &TurnRef,
    ) -> AppResult<ExactStopOutcome>;

    async fn decide_platform_reply(
        &self,
        db: &Database,
        admitted: &AdmittedPersonalUpdate,
        conversation_id: &str,
        input: TurnInput,
    ) -> AppResult<PlatformDecisionOutcome>;

    async fn clear_binding_thread(
        &self,
        db: &Database,
        session: &mut ClientSession,
        origin: &PlatformTurnOrigin,
        conversation_id: &str,
    ) -> AppResult<bool>;
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
    async fn probe_public_readiness(
        &self,
        db: &Database,
        reviewed: &ManagerRuntimePolicy,
    ) -> AppResult<PublicReadiness>;

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
    ) -> AppResult<TurnRef>;

    async fn fence_public_reply(
        &self,
        db: &Database,
        session: &mut ClientSession,
        destination: &ReplyDestination,
    ) -> AppResult<FenceOutcome>;
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
