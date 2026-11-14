//! The single source of truth for personal MongoDB data, erasure, and export.
//! Keep this table deliberately boring: reviewable data policy decisions live here.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerSelector {
    Fields(&'static [&'static str]),
    NotPersonal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedCategory {
    Audit,
    FinancialBilling,
    MachineLifecycle,
    ErasureBookkeeping,
    SharedAttribution,
}

impl RetainedCategory {
    pub const fn disclosure_phrase(self) -> &'static str {
        match self {
            Self::Audit => "hash-chained audit records",
            Self::FinancialBilling => "financial and billing records",
            Self::MachineLifecycle => "machine lifecycle records",
            Self::ErasureBookkeeping => "erasure and access-restriction bookkeeping",
            Self::SharedAttribution => {
                "shared-resource attribution with an opaque account identifier"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Erasure {
    LocalInventory,
    JoinedChild { parent: &'static str },
    Domain(&'static str),
    Redacted,
    Retained(RetainedCategory),
    NotPersonal(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclusionReason {
    SecretMaterial,
    TransientCoordination,
    ErasureBookkeeping,
    RetainedRecord,
    NotPersonal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Export {
    Section(&'static str),
    Excluded(ExclusionReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub collection: &'static str,
    pub owner: OwnerSelector,
    pub erasure: Erasure,
    pub export: Export,
    pub why: &'static str,
}

pub static ENTRIES: &[Entry] = &[
    Entry {
        collection: "agent_key_login_requests",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The agent_key_login_requests collection stores agent key login requests records; the approved_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "agent_service_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The agent_service_bindings collection stores agent service bindings records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "anonymous_endpoint_usage",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The anonymous_endpoint_usage collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "api_key_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The api_key_credentials collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The api_keys collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "approval_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The approval_grants collection stores approval grants records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "approval_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The approval_requests collection stores approval requests records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_acknowledgements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_acknowledgements collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_action_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The assistant_action_receipts collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_consents",
        owner: OwnerSelector::Fields(&["actor_user_id", "owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agent_consents collection stores assistant conversations, actions, or account settings; the actor_user_id, owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        why: "The assistant_agent_credentials collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_learning",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agent_learning collection stores assistant conversations, actions, or account settings; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_learning_members",
        owner: OwnerSelector::Fields(&["member_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agent_learning_members collection stores assistant conversations, actions, or account settings; the member_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_learning_proposals",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agent_learning_proposals collection stores assistant conversations, actions, or account settings; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_learning_rejections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "assistant_agents",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "assistant_agent_learning_rejections has no independent owner and follows assistant_agents; child-first cleanup removes it with that parent.",
    },
    Entry {
        collection: "assistant_agent_learning_runs",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agent_learning_runs collection stores assistant conversations, actions, or account settings; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_learning_skill_roots",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agent_learning_skill_roots collection stores assistant conversations, actions, or account settings; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_private_notes",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agent_private_notes collection stores assistant conversations, actions, or account settings; the actor_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_shares",
        owner: OwnerSelector::Fields(&["owner_id", "subject_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agent_shares collection stores assistant conversations, actions, or account settings; the owner_id, subject_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agent_sharing",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agent_sharing collection stores assistant conversations, actions, or account settings; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_agents collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_analytics_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The assistant_analytics_settings collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "assistant_artifact_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_artifact_deliveries collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_async_operations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_async_operations collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_async_quotas",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_async_quotas collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_attachment_expiry",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_attachment_expiry collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_attachments",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_attachments collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_authored_draft_claims",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_authored_draft_claims collection stores assistant conversations, actions, or account settings; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_checkpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_checkpoints collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_cma_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_cma_agents collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_cma_creations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_cma_creations collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_cma_sandboxes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_cma_sandboxes collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_cma_session_origins",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The assistant_cma_session_origins collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_conversations collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_direct_handoffs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_direct_handoffs collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_group_agent_access",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The assistant_group_agent_access collection stores assistant conversations, actions, or account settings; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_group_agent_proposals",
        owner: OwnerSelector::Fields(&["proposed_by"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_group_agent_proposals collection stores assistant conversations, actions, or account settings; the proposed_by selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_group_execution_slots",
        owner: OwnerSelector::Fields(&["original_requester_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The assistant_group_execution_slots collection stores assistant conversations, actions, or account settings; the original_requester_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_group_invites",
        owner: OwnerSelector::Fields(&[
            "issuer_user_id",
            "accepted_by_user_id",
            "intended_user_id",
        ]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_group_invites collection stores assistant conversations, actions, or account settings; the issuer_user_id, accepted_by_user_id, intended_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_group_mention_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_group_mention_notifications collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_group_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_group_messages collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_group_read_states",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_group_read_states collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_group_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_group_requests collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_group_transfers",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "assistant_group_transfers records shared-resource attribution; retention preserves the opaque account linkage needed by other participants.",
    },
    Entry {
        collection: "assistant_groups",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_groups collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_invocation_turns",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_invocation_turns collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_invocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_invocations collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_learning_migrations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The assistant_learning_migrations collection stores assistant conversations, actions, or account settings; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_learning_publication_targets",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("assistant_learning_reconciliation"),
        export: Export::Section("account_activity"),
        why: "The assistant_learning_publication_targets collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "assistant_message_feedback",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_message_feedback collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_messages collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The assistant_migrations collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "assistant_notification_push_budgets",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The assistant_notification_push_budgets collection stores assistant conversations, actions, or account settings; the _id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_notification_push_sources",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The assistant_notification_push_sources collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_notification_worker",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The assistant_notification_worker collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "assistant_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_notifications collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_onboarding",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_onboarding collection stores assistant conversations, actions, or account settings; the _id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_profile_routes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The assistant_profile_routes collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "assistant_resources",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_resources collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_service_execution",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_service_execution collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_settings",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_settings collection stores assistant conversations, actions, or account settings; the _id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_skill_draft_attempts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_skill_draft_attempts collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_tool_results",
        owner: OwnerSelector::Fields(&["user_id", "actor_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_tool_results collection stores assistant conversations, actions, or account settings; the user_id, actor_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_upload_limits",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The assistant_upload_limits collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "assistant_voice_announcements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_voice_announcements collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_voice_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The assistant_voice_requests collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_voice_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("voice_purge"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The assistant_voice_sessions collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "assistant_voice_windows",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The assistant_voice_windows collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "assistant_wire_logs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The assistant_wire_logs collection stores assistant conversations, actions, or account settings; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "async_service_operation_artifacts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The async_service_operation_artifacts collection stores async service operation artifacts records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "audit_log",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::Audit),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "audit_log is hash-chained audit evidence; retention preserves tamper-evident accountability.",
    },
    Entry {
        collection: "auth_device_code_reservations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("public-code uniqueness fence"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "The auth_device_code_reservations collection is a public-code uniqueness fence; its lifecycle is shared and is never treated as a person-owned record.",
    },
    Entry {
        collection: "auth_device_codes",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The auth_device_codes collection stores authentication and account security state; the approved_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "auth_device_codes_v2",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The auth_device_codes_v2 collection stores authentication and account security state; the approved_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "authorization_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The authorization_codes collection stores authentication and account security state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "billing_account_balances",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_account_balances is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "billing_account_verify_lease",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The billing_account_verify_lease collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "billing_account_verify_state",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The billing_account_verify_state collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "billing_cutover_operations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The billing_cutover_operations collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "billing_lago_carry",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_lago_carry is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "billing_ledger",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_ledger is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "billing_migration_errors",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The billing_migration_errors collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "billing_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The billing_migrations collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "billing_pool_recovery",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_pool_recovery is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "billing_rate_cache",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_rate_cache is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "billing_rate_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The billing_rate_diagnostics collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "billing_tax_profiles",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_tax_profiles is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "billing_topup_sessions",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_topup_sessions is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "billing_wallet",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        why: "billing_wallet is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "billing_wallet_refresh_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The billing_wallet_refresh_requests collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "catalog_delegation_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The catalog_delegation_grants collection stores catalog delegation grants records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "catalog_icons",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The catalog_icons collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "catalog_skill_operations",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "catalog_skill_operations records shared-resource attribution; retention preserves the opaque account linkage needed by other participants.",
    },
    Entry {
        collection: "catalog_skill_revisions",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "catalog_skill_revisions records shared-resource attribution; retention preserves the opaque account linkage needed by other participants.",
    },
    Entry {
        collection: "chain_verify_status",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The chain_verify_status collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "channel_activity_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_activity_notifications collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_adapter_accounts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The channel_adapter_accounts collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_adapter_allocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_adapter_allocations collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_adapter_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_adapter_bindings collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_adapter_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_adapter_events collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_adapter_resource_claims",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_adapter_resource_claims collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_adapter_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_adapter_sessions collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_bots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_bots collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_connect_links collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("channel lifecycle"),
        export: Export::Section("channels"),
        why: "The channel_conversations collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "channel_delivery_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_delivery_receipts collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_email_batches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_email_batches collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_email_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_email_receipts collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_email_sends",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_email_sends collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_email_subscriptions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_email_subscriptions collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_event_logs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "channel_conversations",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "channel_event_logs has no independent owner and follows channel_conversations; child-first cleanup removes it with that parent.",
    },
    Entry {
        collection: "channel_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_messages collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_reply_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_reply_deliveries collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_send_claims",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "channel_conversations",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "channel_send_claims has no independent owner and follows channel_conversations; child-first cleanup removes it with that parent.",
    },
    Entry {
        collection: "channel_text_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_text_deliveries collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "channel_webhook_rejections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The channel_webhook_rejections collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "channel_whatsapp_native",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "The channel_whatsapp_native collection stores linked-channel routing and delivery state; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "cli_pairings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The cli_pairings collection stores cli pairings records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The connect_links collection stores connect links records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "consent_revocation_fences",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "consent_revocation_fences records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "consents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The consents collection stores authentication and account security state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "coordination_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The coordination_leases collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "coordination_rate_windows",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The coordination_rate_windows collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "coordination_replay_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The coordination_replay_records collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "coordination_slots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The coordination_slots collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "coordination_token_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The coordination_token_buckets collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "credit_allocation_requests",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "credit_allocation_requests is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "credit_grants",
        owner: OwnerSelector::Fields(&["recipient_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        why: "credit_grants is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "credit_schedule_periods",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "credit_schedule_periods is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "credit_schedules",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "credit_schedules is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "device_codes",
        owner: OwnerSelector::Fields(&["approved_by_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "The device_codes collection stores device codes records; the approved_by_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "device_onboard_credentials",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The device_onboard_credentials collection stores device onboard credentials records; the owner_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "device_pubkey_lockouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The device_pubkey_lockouts collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "downstream_services",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "downstream_services records shared-resource attribution; retention preserves the opaque account linkage needed by other participants.",
    },
    Entry {
        collection: "durable_operation_executions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "durable_operation_executions records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "durable_operation_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "durable_operation_grants records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "encrypted_record_migration_findings",
        owner: OwnerSelector::Fields(&["owner_uuid"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The encrypted_record_migration_findings collection stores encrypted record migration findings records; the owner_uuid selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "encrypted_record_migration_runs",
        owner: OwnerSelector::Fields(&["owner_uuid"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The encrypted_record_migration_runs collection stores encrypted record migration runs records; the owner_uuid selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "event_dedup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The event_dedup_records collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "execution_enrollment_jobs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The execution_enrollment_jobs collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "execution_enrollment_repairs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The execution_enrollment_repairs collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "execution_permission_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The execution_permission_deliveries collection stores execution permission deliveries records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "feature_flag_definitions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The feature_flag_definitions collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "feature_flag_metadata",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The feature_flag_metadata collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "feature_flag_overrides",
        owner: OwnerSelector::Fields(&["target_key"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The feature_flag_overrides collection stores feature flag overrides records; the target_key selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "generated_artifact_content_series",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The generated_artifact_content_series collection stores uploaded or generated file metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "groups",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The groups collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "invite_codes",
        owner: OwnerSelector::Fields(&["usages.user_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        why: "invite_codes is shared history; personal names and email fields are redacted while counters and opaque attribution remain intact.",
    },
    Entry {
        collection: "invocation_diagnostics",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The invocation_diagnostics collection stores invocation diagnostics records; the actor_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "lark_base_write_photos",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The lark_base_write_photos collection stores uploaded or generated file metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "lark_base_writes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The lark_base_writes collection stores uploaded or generated file metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "login_approvals",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "The login_approvals collection stores authentication and account security state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "login_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "The login_codes collection stores authentication and account security state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "machine_authority_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_authority_leases collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_confirmations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_confirmations collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_contexts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_contexts collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_desktops",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_desktops collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_effect_outbox",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_effect_outbox records machine lifecycle evidence; retention preserves the owner-visible lifecycle and billing trail.",
    },
    Entry {
        collection: "machine_external_grants",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_external_grants collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_free_slots",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_free_slots collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_jobs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_jobs collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_lifecycle_events",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_lifecycle_events records machine lifecycle evidence; retention preserves the owner-visible lifecycle and billing trail.",
    },
    Entry {
        collection: "machine_oauth_consent_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_oauth_consent_requests collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_operations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_operations records machine lifecycle evidence; retention preserves the owner-visible lifecycle and billing trail.",
    },
    Entry {
        collection: "machine_orphan_inventory",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_orphan_inventory collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_revocations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_revocations collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_runtime_heads",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_heads is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "machine_runtime_intervals",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_intervals is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "machine_runtime_quotes",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_quotes is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "machine_runtime_rates",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_rates is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "machine_runtime_runs",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_runs is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "machine_separation_compatibility",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_separation_compatibility collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_setups",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_setups collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_update_attempts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_update_attempts collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "machine_updates",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "The machine_updates collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "managed_executions",
        owner: OwnerSelector::Fields(&["billing_owner_id", "actor_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "managed_executions is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "managed_machine_bootstrap_deliveries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The managed_machine_bootstrap_deliveries collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "managed_machines",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "managed_machines records machine lifecycle evidence; retention preserves the owner-visible lifecycle and billing trail.",
    },
    Entry {
        collection: "mcp_session_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The mcp_session_notifications collection stores mcp session notifications records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "mcp_session_slots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The mcp_session_slots collection stores mcp session slots records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "mcp_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        why: "The mcp_sessions collection stores mcp sessions records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "mfa_challenges",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The mfa_challenges collection stores authentication and account security state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "mfa_factors",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "The mfa_factors collection stores authentication and account security state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "node_pending_credentials",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("nodes"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The node_pending_credentials collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "node_registration_tokens",
        owner: OwnerSelector::Fields(&["user_id", "issuer_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The node_registration_tokens collection stores machine or node access state; the user_id, issuer_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "node_service_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("nodes"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The node_service_bindings collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "nodes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("nodes"),
        export: Export::Section("machines"),
        why: "The nodes collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "notification_channels",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "The notification_channels collection stores notification channels records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "nyxbot_channels",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("channel_bots"),
        export: Export::Section("channels"),
        why: "The nyxbot_channels collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "nyxbot_event_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The nyxbot_event_bindings collection stores nyxbot event bindings records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "nyxbot_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The nyxbot_events collection stores nyxbot events records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "nyxbot_gateway_releases",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "nyxbot_gateway_releases records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "nyxbot_gateway_responses",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The nyxbot_gateway_responses collection stores nyxbot gateway responses records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "nyxbot_image_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The nyxbot_image_deliveries collection stores nyxbot image deliveries records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "nyxbot_image_deliveries_v2",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The nyxbot_image_deliveries_v2 collection stores nyxbot image deliveries v2 records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "nyxbot_threads",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The nyxbot_threads collection stores nyxbot threads records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "nyxbot_watches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "The nyxbot_watches collection stores nyxbot watches records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "oauth_broker_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The oauth_broker_bindings collection stores oauth broker bindings records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "oauth_clients",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Section("services"),
        why: "The oauth_clients collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "oauth_consent_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The oauth_consent_requests collection stores oauth consent requests records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "oauth_delegation_authorities",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The oauth_delegation_authorities collection stores oauth delegation authorities records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "oauth_states",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The oauth_states collection stores oauth states records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "openclaw_channel_mappings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The openclaw_channel_mappings collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "oracle_login_profiles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The oracle_login_profiles collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "oracle_login_snapshots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The oracle_login_snapshots collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "oracle_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("oracle"),
        export: Export::Section("account"),
        why: "The oracle_pools collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "oracle_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The oracle_sessions collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "oracle_tasks",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The oracle_tasks collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "oracle_worker_commands",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The oracle_worker_commands collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "oracle_workers",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The oracle_workers collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "org_channel_guest_budgets",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "org_channel_guest_budgets is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "org_channel_guest_holds",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "org_channel_guest_holds is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "org_channel_lanes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The org_channel_lanes collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "org_channel_link_challenges",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The org_channel_link_challenges collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "org_channel_messages",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The org_channel_messages collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "org_channel_sender_links",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The org_channel_sender_links collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "org_invites",
        owner: OwnerSelector::Fields(&["created_by"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The org_invites collection stores org invites records; the created_by selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "org_memberships",
        owner: OwnerSelector::Fields(&["member_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The org_memberships collection stores org memberships records; the member_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "org_role_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The org_role_scopes collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "ownership_transfers",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "ownership_transfers records shared-resource attribution; retention preserves the opaque account linkage needed by other participants.",
    },
    Entry {
        collection: "permission_key_policies",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The permission_key_policies collection stores permission key policies records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "personal_erasure_effects",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_effects records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "personal_erasure_jobs",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_jobs records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "personal_erasure_local_receipts",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_local_receipts records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "personal_erasure_preparations",
        owner: OwnerSelector::Fields(&["_id", "user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "The personal_erasure_preparations collection stores personal erasure preparations records; the _id, user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "personal_erasure_revocation_receipts",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_revocation_receipts records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "personal_owner_write_fences",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_owner_write_fences records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "platform_bot_cleanup",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_bot_cleanup collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_bot_configs",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_bot_configs collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_bot_installations",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_bot_installations collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_bot_runtime_controls",
        owner: OwnerSelector::Fields(&["personal_control_revision"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_bot_runtime_controls collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_credentials",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_credentials collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_delivery_tests",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_delivery_tests collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_discord_control_installation_history",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_discord_control_installation_history collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_discord_control_installations",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_discord_control_installations collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_entry_gateway_partitions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_entry_gateway_partitions collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_entry_gateway_transports",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_entry_gateway_transports collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_entry_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_entry_migrations collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_guide_selections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The platform_guide_selections collection stores personal platform setup state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "platform_help_assignments",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_help_assignments collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_help_libraries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_help_libraries collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_help_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_help_revisions collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_manager_deliveries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_manager_deliveries collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_manager_ingress_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_manager_ingress_receipts collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_model_attempts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_model_attempts collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_model_routing_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_model_routing_revisions collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_model_tariffs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_model_tariffs collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_models",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_models collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_native_entry_receipts",
        owner: OwnerSelector::Fields(&["sender_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_native_entry_receipts collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_native_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_native_receipts collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_native_replies",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The platform_native_replies collection stores personal platform setup state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "platform_native_reply_handles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_native_reply_handles collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_onboarding_binding_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The platform_onboarding_binding_requests collection stores personal platform setup state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "platform_onboarding_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The platform_onboarding_bindings collection stores personal platform setup state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "platform_onboarding_decision_spacing",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_onboarding_decision_spacing collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_personal_bindings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_personal_bindings collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_personal_decision_spacing",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The platform_personal_decision_spacing collection stores personal platform setup state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "platform_personal_link_challenges",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_personal_link_challenges collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_personal_link_intents",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_personal_link_intents collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_personal_setup_selections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The platform_personal_setup_selections collection stores personal platform setup state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "platform_personal_stop_effects",
        owner: OwnerSelector::Fields(&["personal_control_revision"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_personal_stop_effects collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_public_budget_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_public_budget_buckets collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_public_budget_loops",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_public_budget_loops collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_public_call_reservations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_public_call_reservations collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_public_cleanup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_public_cleanup_records collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_public_deployment_activations",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_public_deployment_activations collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_public_deployment_proofs",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_public_deployment_proofs collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_public_probe_consents",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_public_probe_consents collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_public_quotas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_public_quotas collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_public_readiness_probes",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_public_readiness_probes collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_public_readiness_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_public_readiness_records collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_public_revocation_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_public_revocation_scopes collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_public_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_public_sessions collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_public_turns",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_public_turns collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The platform_settings collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "platform_setup_notices",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_setup_notices collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_setup_steps",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_setup_steps collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "platform_setups",
        owner: OwnerSelector::Fields(&["approved_at"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "The platform_setups collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "pool_recovery_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The pool_recovery_diagnostics collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "provider_configs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The provider_configs collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "pushed_authorization_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The pushed_authorization_requests collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "refresh_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "The refresh_tokens collection stores authentication and account security state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "reply_token_uses",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The reply_token_uses collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "roles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The roles collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "sandbox_billing_cursors",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_cursors is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "sandbox_billing_evidence",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_evidence is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "sandbox_billing_quotes",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_quotes is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "sandbox_billing_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The sandbox_billing_settings collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "sandbox_billing_windows",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_windows is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "sandbox_rate_cards",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The sandbox_rate_cards collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "saved_logins",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The saved_logins collection stores saved logins records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "schema_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The schema_migrations collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "security_hmac_keysets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The security_hmac_keysets collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "security_remediation_progress",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The security_remediation_progress collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "security_replicas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The security_replicas collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "security_rollouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The security_rollouts collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "service_account_key_read_grants",
        owner: OwnerSelector::Fields(&["_id", "owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The service_account_key_read_grants collection stores connected-service and credential metadata; the _id, owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "service_account_tokens",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Section("services"),
        why: "The service_account_tokens collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "service_accounts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Section("services"),
        why: "The service_accounts collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "service_approval_configs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The service_approval_configs collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "service_change_events",
        owner: OwnerSelector::Fields(&["owner_id", "actor.person_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        why: "service_change_events is shared history; personal names and email fields are redacted while counters and opaque attribution remain intact.",
    },
    Entry {
        collection: "service_concurrency_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The service_concurrency_leases collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "service_endpoints",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The service_endpoints collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "service_history_heads",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "user_services",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "service_history_heads has no independent owner and follows user_services; child-first cleanup removes it with that parent.",
    },
    Entry {
        collection: "service_pool_health_observations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "service_pools",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "service_pool_health_observations has no independent owner and follows service_pools; child-first cleanup removes it with that parent.",
    },
    Entry {
        collection: "service_pool_member_health",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The service_pool_member_health collection stores connected-service and credential metadata; the owner_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "service_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The service_pools collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "service_preferences",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The service_preferences collection stores connected-service and credential metadata; the _id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "service_provider_requirements",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The service_provider_requirements collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "The sessions collection stores authentication and account security state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "startup_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The startup_diagnostics collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "telegram_bot_claims",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The telegram_bot_claims collection stores linked-channel routing and delivery state; the owner_user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "telegram_bot_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The telegram_bot_requests collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "telegram_managed_bot_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "The telegram_managed_bot_events collection stores linked-channel routing and delivery state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "telemetry_capture_accounts",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::Domain("telemetry_erasure_service"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "The telemetry_capture_accounts collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "telemetry_dispatches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("telemetry_erasure_service"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "The telemetry_dispatches collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
    Entry {
        collection: "telemetry_erasure_destinations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The telemetry_erasure_destinations collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "telemetry_erasure_jobs",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "telemetry_erasure_jobs records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "telemetry_erasure_reconciliations",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "telemetry_erasure_reconciliations records erasure or access-restriction evidence; retention preserves the cleanup and safety fence.",
    },
    Entry {
        collection: "trigger_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The trigger_deliveries collection stores automation configuration and run state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "trigger_owner_limits",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The trigger_owner_limits collection stores automation configuration and run state; the _id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "trigger_run_budgets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "The trigger_run_budgets collection stores deployment-wide coordination or configuration and has no person-owned rows for account erasure.",
    },
    Entry {
        collection: "trigger_runs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The trigger_runs collection stores automation configuration and run state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "trigger_work",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The trigger_work collection stores automation configuration and run state; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "triggers",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The triggers collection stores triggers records; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "usage_allowance_periods",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_allowance_periods is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "usage_allowances",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_allowances is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "usage_meter",
        owner: OwnerSelector::Fields(&["billing_owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        why: "usage_meter is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "usage_rollup_daily",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_rollup_daily is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "usage_rollup_hourly",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_rollup_hourly is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "usage_rollup_state",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_rollup_state is financial settlement evidence; retention preserves balances, locks, and reconciliation history.",
    },
    Entry {
        collection: "usage_workspaces",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("billing"),
        why: "The usage_workspaces collection stores usage workspaces records; the _id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "user_api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The user_api_keys collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "user_endpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The user_endpoints collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "user_provider_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The user_provider_credentials collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "user_provider_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The user_provider_tokens collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "user_service_connections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The user_service_connections collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "user_services",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "The user_services collection stores connected-service and credential metadata; the user_id selector identifies only the departing person's rows for bounded deletion.",
    },
    Entry {
        collection: "users",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::Domain("source"),
        export: Export::Section("account"),
        why: "The users collection is account-linked lifecycle state; its owning domain helper settles external effects before cleanup.",
    },
];

pub fn entries() -> &'static [Entry] {
    debug_assert!(
        RetainedCategory::Audit
            .disclosure_phrase()
            .contains("audit")
    );
    ENTRIES
}

pub fn find(collection: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.collection == collection)
}

pub fn export_sections() -> impl Iterator<Item = &'static str> {
    ENTRIES
        .iter()
        .filter_map(|e| match e.export {
            Export::Section(name) => Some(name),
            Export::Excluded(_) => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
}

/// Ensure the exact owner selectors used by local erasure and the orphan
/// sweep have supporting indexes. `_id` is provided by MongoDB and needs no
/// additional definition. Keeping the definitions here makes the registry's
/// predicates and their bounded-page execution auditable together.
pub(crate) async fn ensure_indexes(db: &mongodb::Database) -> crate::errors::AppResult<()> {
    use mongodb::IndexModel;
    use std::collections::BTreeMap;

    let mut by_collection = BTreeMap::<&str, Vec<IndexModel>>::new();
    for entry in ENTRIES {
        let (Erasure::LocalInventory | Erasure::Redacted) = entry.erasure else {
            continue;
        };
        let OwnerSelector::Fields(fields) = entry.owner else {
            continue;
        };
        for field in fields {
            if *field == "_id" || field.is_empty() {
                continue;
            }
            let index_name = format!("personal_erasure_owner_{}", field.replace('.', "_"));
            let mut keys = mongodb::bson::Document::new();
            keys.insert(*field, 1_i32);
            by_collection.entry(entry.collection).or_default().push(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .name(index_name)
                            .build(),
                    )
                    .build(),
            );
        }
    }
    for (collection, indexes) in by_collection {
        db.collection::<mongodb::bson::Document>(collection)
            .create_indexes(indexes)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;
    use futures::TryStreamExt;
    use std::{collections::BTreeSet, fs, path::Path};

    #[test]
    fn registry_has_unique_collection_names() {
        let names: BTreeSet<_> = ENTRIES.iter().map(|e| e.collection).collect();
        assert_eq!(names.len(), ENTRIES.len());
    }

    fn strip_cfg_test_modules(source: &str) -> String {
        let marker = "#[cfg(test)]";
        let mut output = String::with_capacity(source.len());
        let mut cursor = 0;
        while let Some(relative) = source[cursor..].find(marker) {
            let start = cursor + relative;
            output.push_str(&source[cursor..start]);
            let after = start + marker.len();
            let Some(open_relative) = source[after..].find('{') else {
                cursor = source[after..]
                    .find('\n')
                    .map(|offset| after + offset + 1)
                    .unwrap_or(source.len());
                continue;
            };
            let open = after + open_relative;
            let prefix = &source[after..open];
            if prefix.contains(';') {
                cursor = after + prefix.find(';').unwrap() + 1;
                continue;
            }
            let mut depth = 0usize;
            let mut end = source.len();
            for (offset, byte) in source[open..].bytes().enumerate() {
                match byte {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + offset + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            cursor = end;
        }
        output.push_str(&source[cursor..]);
        output
    }

    fn scan_collection_calls(
        sources: &[(String, String)],
    ) -> Result<BTreeSet<String>, Vec<String>> {
        use std::collections::HashMap;
        let declaration = regex::Regex::new(
            r#"(?m)(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"]+)""#,
        )
        .unwrap();
        let constant_alias = regex::Regex::new(
            r#"(?m)(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*([A-Za-z_][A-Za-z0-9_:]*)\s*;"#,
        )
        .unwrap();
        let associated_alias = regex::Regex::new(
            r#"(?s)impl\s+([A-Za-z_][A-Za-z0-9_:]*)\s*\{.*?(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*([A-Za-z_][A-Za-z0-9_:]*)\s*;"#,
        )
        .unwrap();
        let associated = regex::Regex::new(
            r#"(?s)impl\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{.*?(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"]+)""#,
        )
        .unwrap();
        let uses = regex::Regex::new(
            r#"(?m)\buse\s+([A-Za-z_][A-Za-z0-9_:]*)::([A-Z][A-Z0-9_]*)(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;"#,
        )
        .unwrap();
        let use_statements = regex::Regex::new(r#"(?m)\buse\s+([^;]+);"#).unwrap();
        let calls =
            regex::Regex::new(r#"\.collection(?:\s*::\s*<[^()]*>)?\s*\(\s*([^,\)]*)"#).unwrap();

        let mut global = HashMap::<String, BTreeSet<String>>::new();
        let mut global_aliases = HashMap::<String, BTreeSet<String>>::new();
        let mut local = Vec::<HashMap<String, String>>::new();
        let mut aliases = Vec::<HashMap<String, String>>::new();
        fn split_top_level(value: &str) -> Vec<&str> {
            let mut parts = Vec::new();
            let mut depth = 0usize;
            let mut start = 0usize;
            for (index, byte) in value.bytes().enumerate() {
                match byte {
                    b'{' => depth += 1,
                    b'}' => depth = depth.saturating_sub(1),
                    b',' if depth == 0 => {
                        parts.push(value[start..index].trim());
                        start = index + 1;
                    }
                    _ => {}
                }
            }
            parts.push(value[start..].trim());
            parts
        }
        fn collect_imports(prefix: &str, value: &str, aliases: &mut HashMap<String, String>) {
            for member in split_top_level(value) {
                let Some(open) = member.find('{') else {
                    let mut words = member.split_whitespace();
                    let path = words.next().unwrap_or_default();
                    if path.is_empty() {
                        continue;
                    }
                    let alias = match (words.next(), words.next()) {
                        (Some("as"), Some(alias)) => alias,
                        _ => path.rsplit("::").next().unwrap_or(path),
                    };
                    aliases.insert(alias.to_owned(), format!("{prefix}{path}"));
                    continue;
                };
                let Some(close) = member.rfind('}') else {
                    continue;
                };
                if close <= open {
                    continue;
                }
                let nested_prefix =
                    format!("{prefix}{}", member[..open].trim().trim_end_matches("::"));
                collect_imports(
                    &(nested_prefix + "::"),
                    member[open + 1..close].trim(),
                    aliases,
                );
            }
        }
        for (label, source) in sources {
            let mut this = HashMap::new();
            for captures in declaration.captures_iter(source) {
                this.insert(captures[1].to_owned(), captures[2].to_owned());
                global
                    .entry(captures[1].to_owned())
                    .or_default()
                    .insert(captures[2].to_owned());
                if let Some(stem) = std::path::Path::new(label)
                    .file_stem()
                    .and_then(|s| s.to_str())
                {
                    let module = if label.contains("/models/") {
                        "models"
                    } else if label.contains("/services/") {
                        "services"
                    } else {
                        "src"
                    };
                    global
                        .entry(format!("crate::{module}::{stem}::{}", &captures[1]))
                        .or_default()
                        .insert(captures[2].to_owned());
                }
            }
            let mut this_aliases = HashMap::new();
            for captures in constant_alias.captures_iter(source) {
                this_aliases.insert(captures[1].to_owned(), captures[2].to_owned());
            }
            for captures in associated_alias.captures_iter(source) {
                this_aliases.insert(
                    format!("{}::{}", &captures[1], &captures[2]),
                    captures[3].to_owned(),
                );
            }
            for captures in uses.captures_iter(source) {
                let alias = captures
                    .get(3)
                    .map(|value| value.as_str())
                    .unwrap_or_else(|| captures.get(2).unwrap().as_str());
                this_aliases.insert(
                    alias.to_owned(),
                    captures[1].to_owned() + "::" + &captures[2],
                );
            }
            // Handle grouped imports (`use crate::models::{foo::COLLECTION_NAME
            // as FOO, bar::COLLECTION_NAME as BAR};`) as well as ordinary
            // imports that the focused expression above does not match.
            for captures in use_statements.captures_iter(source) {
                let statement = captures[1].trim();
                if let Some(open) = statement.find('{')
                    && let Some(close) = statement.rfind('}')
                    && close > open
                {
                    collect_imports(
                        &(statement[..open].trim().trim_end_matches("::").to_owned() + "::"),
                        &statement[open + 1..close],
                        &mut this_aliases,
                    );
                }
            }
            for (alias, path) in &this_aliases {
                global_aliases
                    .entry(alias.clone())
                    .or_default()
                    .insert(path.clone());
            }
            for captures in associated.captures_iter(source) {
                let key = format!("{}::{}", &captures[1], &captures[2]);
                global
                    .entry(key)
                    .or_default()
                    .insert(captures[3].to_owned());
            }
            local.push(this);
            aliases.push(this_aliases);
        }

        // Resolve aliases after all source files have contributed their
        // declarations. Model modules commonly expose a short collection
        // alias that points at an associated model constant, while services
        // refer to that alias through `crate::` or `super::` paths. Keeping
        // the resolution exact avoids the old substring-based false positives
        // while still handling those legitimate cross-module forms.
        for this_aliases in &aliases {
            for (alias, target) in this_aliases {
                let values = global
                    .get(target)
                    .cloned()
                    .or_else(|| {
                        target
                            .rsplit("::")
                            .next()
                            .and_then(|name| global.get(name).cloned())
                    })
                    .unwrap_or_default();
                if !values.is_empty() {
                    global
                        .entry(alias.clone())
                        .or_default()
                        .extend(values);
                }
            }
        }

        let mut seen = BTreeSet::new();
        let mut errors = Vec::new();
        for ((label, source), (this, this_aliases)) in
            sources.iter().zip(local.into_iter().zip(aliases))
        {
            for call in calls.captures_iter(source) {
                let argument = call[1].trim();
                if argument.is_empty() {
                    continue;
                }
                let resolve = |name: &str| {
                    this.get(name)
                        .map(|value| BTreeSet::from([value.clone()]))
                        .or_else(|| {
                            this_aliases.get(name).and_then(|path| {
                                global.get(path).cloned().or_else(|| {
                                    path.rsplit("::")
                                        .next()
                                        .and_then(|name| global.get(name).cloned())
                                })
                            })
                        })
                        .or_else(|| global.get(name).cloned())
                        .or_else(|| {
                            global_aliases.get(name).and_then(|paths| {
                                let values = paths
                                    .iter()
                                    .filter_map(|path| global.get(path))
                                    .flat_map(|values| values.iter().cloned())
                                    .collect::<BTreeSet<_>>();
                                (!values.is_empty()).then_some(values)
                            })
                        })
                        .or_else(|| {
                            name.rsplit("::")
                                .next()
                                .and_then(|name| global.get(name).cloned())
                        })
                };
                let values = if argument.starts_with('"') && argument.ends_with('"') {
                    BTreeSet::from([argument.trim_matches('"').to_owned()])
                } else {
                    // Resolve the complete argument only.  Searching for
                    // uppercase substrings made a call such as
                    // `collection(format!("{A}{B}"))` appear to use both
                    // constants and made the stale-entry check meaningless.
                    // Runtime expressions are intentionally outside this
                    // source inventory; constant-looking arguments must be
                    // exact aliases or associated constants.
                    resolve(argument).unwrap_or_default()
                };
                if values.is_empty() {
                    // A run-time collection parameter cannot be resolved by a
                    // source scan. Constant-looking arguments must resolve;
                    // dynamic variables are covered by the call-site's own
                    // registry tests and are intentionally skipped here.
                    let looks_like_constant = argument.rsplit("::").next().is_some_and(|name| {
                        !name.is_empty()
                            && name.chars().all(|character| {
                                character.is_ascii_uppercase()
                                    || character.is_ascii_digit()
                                    || character == '_'
                            })
                    });
                    if looks_like_constant {
                        errors.push(format!(
                            "{label}: unresolved collection constant {argument}"
                        ));
                    }
                } else {
                    seen.extend(values);
                }
            }
        }
        if errors.is_empty() {
            Ok(seen)
        } else {
            Err(errors)
        }
    }

    fn validate_scan(
        sources: &[(String, String)],
        registered: &BTreeSet<String>,
    ) -> Result<(), Vec<String>> {
        let seen = scan_collection_calls(sources)?;
        let mut errors = Vec::new();
        errors.extend(
            seen.iter()
                .filter(|name| !registered.contains(*name))
                .map(|name| format!("unregistered collection: {name}")),
        );
        errors.extend(
            registered
                .iter()
                .filter(|name| !seen.contains(*name))
                .map(|name| format!("stale registry entry: {name}")),
        );
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    #[test]
    fn every_production_collection_literal_is_registered() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let registry = root.join("services/personal_data_registry.rs");
        let mut files = Vec::new();
        fn gather(path: &Path, registry: &Path, files: &mut Vec<(String, String)>) {
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for item in entries.flatten() {
                let p = item.path();
                if p.is_dir() {
                    gather(&p, registry, files);
                    continue;
                }
                if p.extension().and_then(|e| e.to_str()) != Some("rs")
                    || p == registry
                    || p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.contains("test"))
                {
                    continue;
                }
                let Ok(source) = fs::read_to_string(&p) else {
                    continue;
                };
                files.push((p.display().to_string(), strip_cfg_test_modules(&source)));
            }
        }
        gather(&root, &registry, &mut files);
        let seen = scan_collection_calls(&files)
            .unwrap_or_else(|errors| panic!("unresolved production collection calls: {errors:?}"));
        let inventory_names =
            crate::services::personal_erasure_service::local::inventory("source-scan-owner")
                .into_iter()
                .map(|(collection, _)| collection.to_owned());
        let seen: BTreeSet<_> = seen.into_iter().chain(inventory_names).collect();
        let stale: Vec<_> = ENTRIES
            .iter()
            .filter(|entry| !seen.contains(entry.collection))
            .map(|entry| entry.collection)
            .collect();
        assert!(stale.is_empty(), "stale registry entries: {stale:?}");
        let registered: BTreeSet<_> = ENTRIES.iter().map(|entry| entry.collection).collect();
        let missing: Vec<_> = seen
            .iter()
            .filter(|collection| !registered.contains(collection.as_str()))
            .cloned()
            .collect();
        assert!(
            missing.is_empty(),
            "unregistered collection calls: {missing:?}"
        );
    }

    #[test]
    fn source_scanner_rejects_unregistered_constants_literals_and_stale_entries() {
        let registered = BTreeSet::from(["known".to_owned()]);
        let constant = vec![(
            "synthetic.rs".to_owned(),
            r#"const UNKNOWN: &str = "unknown"; db.collection(UNKNOWN);"#.to_owned(),
        )];
        let error = validate_scan(&constant, &registered).unwrap_err();
        assert!(
            error
                .iter()
                .any(|e| e.contains("unregistered collection: unknown"))
        );
        let literal = vec![(
            "synthetic.rs".to_owned(),
            r#"db.collection("literal_unknown");"#.to_owned(),
        )];
        let error = validate_scan(&literal, &registered).unwrap_err();
        assert!(
            error
                .iter()
                .any(|e| e.contains("unregistered collection: literal_unknown"))
        );
        let stale = vec![(
            "synthetic.rs".to_owned(),
            r#"db.collection("known");"#.to_owned(),
        )];
        let registered_with_stale = BTreeSet::from(["known".to_owned(), "stale".to_owned()]);
        let error = validate_scan(&stale, &registered_with_stale).unwrap_err();
        assert!(
            error
                .iter()
                .any(|e| e.contains("stale registry entry: stale"))
        );
    }

    #[test]
    fn source_scanner_resolves_aliases_and_associated_constants() {
        let sources = vec![
            (
                "models.rs".to_owned(),
                r#"pub const POOLS: &str = "pool_rows"; struct AssistantWireLog; impl AssistantWireLog { pub const COLLECTION_NAME: &str = "wire_rows"; }"#.to_owned(),
            ),
            (
                "use.rs".to_owned(),
                r#"use crate::models::POOLS as POOL_COLLECTION; use crate::models::AssistantWireLog; db.collection(POOL_COLLECTION); db.collection(AssistantWireLog::COLLECTION_NAME);"#.to_owned(),
            ),
        ];
        let seen = scan_collection_calls(&sources).unwrap();
        assert_eq!(
            seen,
            BTreeSet::from(["pool_rows".to_owned(), "wire_rows".to_owned()])
        );
    }

    #[test]
    fn retained_categories_have_policy_phrases() {
        let policy = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../frontend/public/legal/privacy.md"),
        )
        .unwrap_or_default();
        for category in [
            RetainedCategory::Audit,
            RetainedCategory::FinancialBilling,
            RetainedCategory::MachineLifecycle,
            RetainedCategory::ErasureBookkeeping,
            RetainedCategory::SharedAttribution,
        ] {
            assert!(
                policy.contains(category.disclosure_phrase()),
                "missing policy phrase: {}",
                category.disclosure_phrase()
            );
        }
    }

    #[test]
    fn local_inventory_and_registry_have_no_unclassified_collections() {
        let inventory = crate::services::personal_erasure_service::local::inventory("probe");
        let inventory_names: BTreeSet<_> = inventory.iter().map(|(name, _)| *name).collect();
        let missing_from_registry: Vec<_> = inventory_names
            .iter()
            .filter(|name| find(name).is_none())
            .copied()
            .collect();
        assert!(
            missing_from_registry.is_empty(),
            "local inventory collections missing from registry: {missing_from_registry:?}"
        );

        let missing_from_inventory: Vec<_> = ENTRIES
            .iter()
            .filter(|entry| matches!(entry.erasure, Erasure::LocalInventory))
            .filter(|entry| !inventory_names.contains(entry.collection))
            .map(|entry| entry.collection)
            .collect();
        assert!(
            missing_from_inventory.is_empty(),
            "registry LocalInventory collections missing from local::inventory: {missing_from_inventory:?}"
        );
    }

    #[test]
    fn local_inventory_owner_selectors_match_registry_predicates_and_indexes() {
        let owner = "00000000-0000-0000-0000-000000000001";
        let filters =
            crate::services::personal_erasure_service::local::inventory_owner_fields(owner);
        for entry in ENTRIES {
            let Erasure::LocalInventory = entry.erasure else {
                continue;
            };
            let OwnerSelector::Fields(fields) = entry.owner else {
                panic!(
                    "local inventory entry {} has no owner selector",
                    entry.collection
                );
            };
            let actual = filters
                .get(entry.collection)
                .unwrap_or_else(|| panic!("missing inventory filter for {}", entry.collection));
            let expected = fields.iter().map(|field| (*field).to_owned()).collect();
            assert_eq!(
                actual, &expected,
                "owner predicate drift for {}",
                entry.collection
            );
            for field in fields {
                assert!(
                    *field == "_id" || !field.is_empty(),
                    "empty owner field for {}",
                    entry.collection
                );
            }
        }
    }

    #[tokio::test]
    async fn ensure_indexes_creates_every_personal_owner_predicate_index() {
        let db =
            crate::test_utils::connect_transaction_test_database("privacy_registry_indexes").await;
        super::ensure_indexes(&db).await.unwrap();
        for entry in ENTRIES {
            let (Erasure::LocalInventory | Erasure::Redacted) = entry.erasure else {
                continue;
            };
            let OwnerSelector::Fields(fields) = entry.owner else {
                continue;
            };
            // A collection with only an `_id` owner selector has no custom
            // index to create on an empty test database. Seed one document so
            // MongoDB materializes the namespace; `_id_` is the supporting
            // index.
            if fields.iter().all(|field| *field == "_id") {
                db.collection::<mongodb::bson::Document>(entry.collection)
                    .insert_one(doc! {"_id": "index-fixture"})
                    .await
                    .unwrap();
            }
            let indexes = db
                .collection::<mongodb::bson::Document>(entry.collection)
                .list_indexes()
                .await
                .unwrap();
            let indexes: Vec<_> = indexes.try_collect().await.unwrap();
            for field in fields {
                if *field == "_id" {
                    continue;
                }
                assert!(
                    indexes
                        .iter()
                        .any(|index| index.keys.get_i32(field).ok() == Some(1)),
                    "{} needs an owner index for {}",
                    entry.collection,
                    field
                );
            }
        }
        db.drop().await.unwrap();
    }

    #[test]
    fn table_driven_erasure_cases_cover_every_mutating_registry_entry() {
        let mut cases = 0usize;
        for entry in ENTRIES {
            match entry.erasure {
                Erasure::LocalInventory | Erasure::Redacted => {
                    assert!(
                        matches!(entry.owner, OwnerSelector::Fields(fields) if !fields.is_empty()),
                        "mutating entry {} must name its owner predicate",
                        entry.collection
                    );
                    cases += 1;
                }
                Erasure::JoinedChild { parent } => {
                    assert!(!parent.is_empty(), "joined child parent is empty");
                    cases += 1;
                }
                Erasure::Retained(_) | Erasure::Domain(_) | Erasure::NotPersonal(_) => {}
            }
        }
        assert!(cases > 0);
    }

    #[test]
    fn corrected_owner_selectors_match_model_field_names() {
        for (collection, expected) in [
            ("assistant_agent_learning_members", &["member_user_id"][..]),
            ("assistant_agent_learning_proposals", &["owner_id"][..]),
            ("assistant_agent_learning_runs", &["owner_id"][..]),
            ("assistant_agent_learning_skill_roots", &["owner_id"][..]),
            ("assistant_group_agent_access", &["owner_id"][..]),
            ("org_memberships", &["member_user_id"][..]),
            ("billing_wallet", &["owner_id"][..]),
            ("credit_grants", &["recipient_user_id"][..]),
            ("usage_meter", &["billing_owner_id"][..]),
        ] {
            let entry = find(collection).unwrap_or_else(|| panic!("missing {collection}"));
            assert_eq!(entry.owner, OwnerSelector::Fields(expected));
        }
    }

    #[test]
    fn every_personal_owner_selector_names_a_model_or_document_field() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/models");
        let mut fields = BTreeSet::new();
        fn gather(path: &Path, fields: &mut BTreeSet<String>) {
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    gather(&path, fields);
                } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                    if let Ok(source) = fs::read_to_string(path) {
                        for capture in
                            regex::Regex::new(r"(?m)^\s*(?:pub\s+)?([a-zA-Z_][a-zA-Z0-9_]*)\s*:")
                                .unwrap()
                                .captures_iter(&source)
                        {
                            fields.insert(capture[1].to_owned());
                        }
                    }
                }
            }
        }
        gather(&root, &mut fields);
        let documented = BTreeSet::from([
            "actor".to_owned(),
            "usages".to_owned(),
            "target_user_ids".to_owned(),
            "target_group_ids".to_owned(),
            "owner_id".to_owned(),
            "user_id".to_owned(),
            "_id".to_owned(),
        ]);
        for entry in ENTRIES {
            let OwnerSelector::Fields(owner_fields) = entry.owner else {
                continue;
            };
            for field in owner_fields {
                let leaf = field.rsplit('.').next().unwrap_or(field);
                assert!(
                    *field == "_id" || fields.contains(leaf) || documented.contains(leaf),
                    "{} uses owner field {} absent from model/document declarations",
                    entry.collection,
                    field
                );
            }
        }
    }

    #[test]
    fn every_export_section_has_registry_entries() {
        let sections: BTreeSet<_> = export_sections().collect();
        assert!(sections.contains("account"));
        assert!(sections.contains("billing"));
        for section in sections {
            assert!(
                ENTRIES
                    .iter()
                    .any(|entry| entry.export == Export::Section(section)),
                "export section has no registry entry: {section}"
            );
        }
    }
}
