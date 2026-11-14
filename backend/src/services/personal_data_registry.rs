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
        why: "agent_key_login_requests stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "agent_service_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "agent_service_bindings stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "anonymous_endpoint_usage",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "anonymous_endpoint_usage is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "api_key_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "api_key_credentials stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "api_keys stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "approval_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "approval_grants stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "approval_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "approval_requests stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_acknowledgements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_acknowledgements stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_action_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_action_receipts stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_consents",
        owner: OwnerSelector::Fields(&["actor_user_id", "owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_consents stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        why: "assistant_agent_credentials stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_learning",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_learning_members",
        owner: OwnerSelector::Fields(&["member_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning_members stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_learning_proposals",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning_proposals stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_learning_rejections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "assistant_agents",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "Learning rejection rows join the personal assistant agent parent.",
    },
    Entry {
        collection: "assistant_agent_learning_runs",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning_runs stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_learning_skill_roots",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning_skill_roots stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_private_notes",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_private_notes stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_shares",
        owner: OwnerSelector::Fields(&["owner_id", "subject_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_shares stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agent_sharing",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_sharing stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agents stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_analytics_settings",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_analytics_settings is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "assistant_artifact_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_artifact_deliveries stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_async_operations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_async_operations stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_async_quotas",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_async_quotas stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_attachment_expiry",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_attachment_expiry stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_attachments",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_attachments stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_authored_draft_claims",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_authored_draft_claims stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_checkpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_checkpoints stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_cma_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_cma_agents stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_cma_creations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_cma_creations stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_cma_sandboxes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_cma_sandboxes stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_cma_session_origins",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "CMA session-origin metadata belongs to the account and is removed by local erasure.",
    },
    Entry {
        collection: "assistant_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_conversations stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_direct_handoffs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_direct_handoffs stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_group_agent_access",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_group_agent_access stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_group_agent_proposals",
        owner: OwnerSelector::Fields(&["proposed_by"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_agent_proposals stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_group_execution_slots",
        owner: OwnerSelector::Fields(&["original_requester_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Group execution slots retain the original requester owner and are removed by local erasure.",
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
        why: "assistant_group_invites stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_group_mention_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_mention_notifications stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_group_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_messages stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_group_read_states",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_read_states stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_group_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_requests stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_group_transfers",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Group transfer state preserves the shared handoff between opaque participant IDs.",
    },
    Entry {
        collection: "assistant_groups",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_groups stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_invocation_turns",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_invocation_turns stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_invocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_invocations stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_learning_migrations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_learning_migrations stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_learning_publication_targets",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("assistant_learning_reconciliation"),
        export: Export::Section("account_activity"),
        why: "Settled publication targets remain external-effect evidence; the learning reconciliation helper releases only un-dispatched reservations.",
    },
    Entry {
        collection: "assistant_message_feedback",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_message_feedback stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_messages stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_migrations is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "assistant_notification_push_budgets",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Per-account push budgets are local notification state and are removed with the account.",
    },
    Entry {
        collection: "assistant_notification_push_sources",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Push source registrations belong to the account and are removed by local erasure.",
    },
    Entry {
        collection: "assistant_notification_worker",
        owner: OwnerSelector::Fields(&["actor_type"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_notification_worker is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "assistant_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_notifications stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_onboarding",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_onboarding stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_profile_routes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_profile_routes is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "assistant_resources",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_resources stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_service_execution",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_service_execution stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_settings",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_settings stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_skill_draft_attempts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_skill_draft_attempts stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_tool_results",
        owner: OwnerSelector::Fields(&["user_id", "actor_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_tool_results stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_upload_limits",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_upload_limits is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "assistant_voice_announcements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_voice_announcements stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_voice_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_voice_requests stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_voice_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("voice_purge"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Voice sessions are purged by the voice lifecycle after the account requests erasure.",
    },
    Entry {
        collection: "assistant_voice_windows",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_voice_windows stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "assistant_wire_logs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_wire_logs stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "async_service_operation_artifacts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "async_service_operation_artifacts stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "audit_log",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::Audit),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "audit_log is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "auth_device_code_reservations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("public-code uniqueness fence"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "Public-code reservations are a cross-protocol uniqueness fence with no account owner and remain under their expiry lifecycle.",
    },
    Entry {
        collection: "auth_device_codes",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Approved device-login records belong to the approving person and are removed by local erasure.",
    },
    Entry {
        collection: "auth_device_codes_v2",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "The v2 approved device-login record belongs to the approving person and is removed by local erasure.",
    },
    Entry {
        collection: "authorization_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "authorization_codes stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "billing_account_balances",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_account_balances is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "billing_account_verify_lease",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_account_verify_lease is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "billing_account_verify_state",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_account_verify_state is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "billing_cutover_operations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_cutover_operations is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "billing_lago_carry",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_lago_carry is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "billing_ledger",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_ledger is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "billing_migration_errors",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_migration_errors is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "billing_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_migrations is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "billing_pool_recovery",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_pool_recovery is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "billing_rate_cache",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_rate_cache is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "billing_rate_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_rate_diagnostics is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "billing_tax_profiles",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_tax_profiles is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "billing_topup_sessions",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_topup_sessions is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "billing_wallet",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        why: "billing_wallet is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "billing_wallet_refresh_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_wallet_refresh_requests is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "catalog_delegation_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "catalog_delegation_grants stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "catalog_icons",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "catalog_icons is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "catalog_skill_operations",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "catalog_skill_operations is shared-resource history; opaque account identifiers remain while human attribution is redacted.",
    },
    Entry {
        collection: "catalog_skill_revisions",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "catalog_skill_revisions is shared-resource history; opaque account identifiers remain while human attribution is redacted.",
    },
    Entry {
        collection: "chain_verify_status",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "chain_verify_status is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "channel_activity_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_activity_notifications stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_adapter_accounts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "channel_adapter_accounts stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_adapter_allocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_allocations stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_adapter_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_bindings stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_adapter_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_events stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_adapter_resource_claims",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_resource_claims stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_adapter_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_sessions stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_bots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_bots stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_connect_links stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("channel lifecycle"),
        export: Export::Section("channels"),
        why: "channel_conversations is processed by the channel lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "channel_delivery_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_delivery_receipts stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_email_batches",
        owner: OwnerSelector::Fields(&["account_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("channels"),
        why: "channel_email_batches is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "channel_email_receipts",
        owner: OwnerSelector::Fields(&["account_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("channels"),
        why: "channel_email_receipts is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "channel_email_sends",
        owner: OwnerSelector::Fields(&["account_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("channels"),
        why: "channel_email_sends is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "channel_email_subscriptions",
        owner: OwnerSelector::Fields(&["account_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("channels"),
        why: "channel_email_subscriptions is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "channel_event_logs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "channel_conversations",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "Channel event rows join the personal conversation parent.",
    },
    Entry {
        collection: "channel_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_messages stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_reply_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_reply_deliveries stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_send_claims",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "channel_conversations",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "Channel send claims join the personal conversation parent.",
    },
    Entry {
        collection: "channel_text_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_text_deliveries stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "channel_webhook_rejections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Section("channels"),
        why: "channel_webhook_rejections is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "channel_whatsapp_native",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_whatsapp_native stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "cli_pairings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "cli_pairings stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "connect_links stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "consent_revocation_fences",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "consent_revocation_fences is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "consents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "consents stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "coordination_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_leases is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "coordination_rate_windows",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_rate_windows is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "coordination_replay_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_replay_records is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "coordination_slots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_slots is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "coordination_token_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_token_buckets is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "credit_allocation_requests",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Credit allocation requests are financial policy and settlement evidence keyed by an owner.",
    },
    Entry {
        collection: "credit_grants",
        owner: OwnerSelector::Fields(&["recipient_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        why: "credit_grants is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "credit_schedule_periods",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "credit_schedule_periods is a shared financial period or hold; retaining it preserves settlement state for every participant.",
    },
    Entry {
        collection: "credit_schedules",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "credit_schedules is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "device_codes",
        owner: OwnerSelector::Fields(&["approved_by_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "device_codes stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "device_onboard_credentials",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "device_onboard_credentials stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "device_pubkey_lockouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "device_pubkey_lockouts is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "downstream_services",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "downstream_services is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "durable_operation_executions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Durable execution outcomes are retained settlement evidence after owner cleanup.",
    },
    Entry {
        collection: "durable_operation_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Revoked durable grants remain as erasure and access-restriction evidence.",
    },
    Entry {
        collection: "encrypted_record_migration_findings",
        owner: OwnerSelector::Fields(&["owner_uuid"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "encrypted_record_migration_findings stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "encrypted_record_migration_runs",
        owner: OwnerSelector::Fields(&["owner_uuid"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "encrypted_record_migration_runs stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "event_dedup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "event_dedup_records is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "execution_enrollment_jobs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "execution_enrollment_jobs is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "execution_enrollment_repairs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "execution_enrollment_repairs is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "execution_permission_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "execution_permission_deliveries stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "feature_flag_definitions",
        owner: OwnerSelector::Fields(&["created_by"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "feature_flag_definitions is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "feature_flag_metadata",
        owner: OwnerSelector::Fields(&["owner"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "feature_flag_metadata is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "feature_flag_overrides",
        owner: OwnerSelector::Fields(&["target_key"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "feature_flag_overrides stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "generated_artifact_content_series",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "generated_artifact_content_series stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "groups",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "groups is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "invite_codes",
        owner: OwnerSelector::Fields(&["usages.user_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        why: "invite_codes is shared history; erase personal ownership and redact human-readable attribution.",
    },
    Entry {
        collection: "invocation_diagnostics",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "invocation_diagnostics stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "lark_base_write_photos",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "lark_base_write_photos stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "lark_base_writes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "lark_base_writes stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "login_approvals",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "login_approvals stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "login_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "login_codes stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "machine_authority_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_authority_leases is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_confirmations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_confirmations is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_contexts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_contexts is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_desktops",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_desktops is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_effect_outbox",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_effect_outbox is retained machine lifecycle evidence and remains under its owner and version fences.",
    },
    Entry {
        collection: "machine_external_grants",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_external_grants is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_free_slots",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_free_slots is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_jobs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_jobs is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_lifecycle_events",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_lifecycle_events is retained machine lifecycle evidence and remains under its owner and version fences.",
    },
    Entry {
        collection: "machine_oauth_consent_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_oauth_consent_requests is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_operations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_operations is retained machine lifecycle evidence and remains under its owner and version fences.",
    },
    Entry {
        collection: "machine_orphan_inventory",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_orphan_inventory is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_revocations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_revocations is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_runtime_heads",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_heads is retained machine runtime billing evidence used for financial reconciliation.",
    },
    Entry {
        collection: "machine_runtime_intervals",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_intervals is retained machine runtime billing evidence used for financial reconciliation.",
    },
    Entry {
        collection: "machine_runtime_quotes",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_quotes is retained machine runtime billing evidence used for financial reconciliation.",
    },
    Entry {
        collection: "machine_runtime_rates",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_rates is retained machine runtime billing evidence used for financial reconciliation.",
    },
    Entry {
        collection: "machine_runtime_runs",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_runs is retained machine runtime billing evidence used for financial reconciliation.",
    },
    Entry {
        collection: "machine_separation_compatibility",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_separation_compatibility is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_setups",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_setups is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_update_attempts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_update_attempts is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "machine_updates",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_updates is processed by the machine erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "managed_executions",
        owner: OwnerSelector::Fields(&["billing_owner_id", "actor_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Managed execution rows are financial settlement evidence and remain immutable after an owner leaves.",
    },
    Entry {
        collection: "managed_machine_bootstrap_deliveries",
        owner: OwnerSelector::Fields(&["recipient_key_digest"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "managed_machine_bootstrap_deliveries is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "managed_machines",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "managed_machines is retained machine lifecycle evidence and remains under its owner and version fences.",
    },
    Entry {
        collection: "mcp_session_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "mcp_session_notifications stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "mcp_session_slots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "mcp_session_slots stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "mcp_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        why: "mcp_sessions stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "mfa_challenges",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "mfa_challenges stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "mfa_factors",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "mfa_factors stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "node_pending_credentials",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("nodes"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Pending node credentials are removed by the guarded node drain for their owner.",
    },
    Entry {
        collection: "node_registration_tokens",
        owner: OwnerSelector::Fields(&["user_id", "issuer_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "node_registration_tokens stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "node_service_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("nodes"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Node-service bindings are removed by the guarded node drain for their owner.",
    },
    Entry {
        collection: "nodes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("nodes"),
        export: Export::Section("machines"),
        why: "Personal node credentials and connections are drained by the nodes lifecycle before removal.",
    },
    Entry {
        collection: "notification_channels",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "notification_channels stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "nyxbot_channels",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("channel_bots"),
        export: Export::Section("channels"),
        why: "A person-owned NyxBot channel is retired through the channel-bot lifecycle.",
    },
    Entry {
        collection: "nyxbot_event_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_event_bindings stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "nyxbot_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_events stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "nyxbot_gateway_releases",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Gateway release rows fence remote deletion; unsettled releases remain until the Discord drain is confirmed.",
    },
    Entry {
        collection: "nyxbot_gateway_responses",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_gateway_responses stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "nyxbot_image_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_image_deliveries stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "nyxbot_image_deliveries_v2",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_image_deliveries_v2 stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "nyxbot_threads",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_threads stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "nyxbot_watches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_watches stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "oauth_broker_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "oauth_broker_bindings stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "oauth_clients",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Section("services"),
        why: "oauth_clients is processed by the local_service_account_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "oauth_consent_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "oauth_consent_requests stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "oauth_delegation_authorities",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "oauth_delegation_authorities stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "oauth_states",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "oauth_states stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "openclaw_channel_mappings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "openclaw_channel_mappings stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "oracle_login_profiles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_login_profiles is processed by the local_service_account_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "oracle_login_snapshots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_login_snapshots is processed by the local_service_account_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "oracle_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "oracle_pools is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "oracle_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_sessions is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "oracle_tasks",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_tasks is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "oracle_worker_commands",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_worker_commands is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "oracle_workers",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_workers is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "org_channel_guest_budgets",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "org_channel_guest_budgets is a shared financial period or hold; retaining it preserves settlement state for every participant.",
    },
    Entry {
        collection: "org_channel_guest_holds",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "org_channel_guest_holds is a shared financial period or hold; retaining it preserves settlement state for every participant.",
    },
    Entry {
        collection: "org_channel_lanes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_channel_lanes is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "org_channel_link_challenges",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_channel_link_challenges is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "org_channel_messages",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_channel_messages is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "org_channel_sender_links",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_channel_sender_links is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "org_invites",
        owner: OwnerSelector::Fields(&["created_by"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "org_invites stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "org_memberships",
        owner: OwnerSelector::Fields(&["member_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "org_memberships stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "org_role_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_role_scopes is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "ownership_transfers",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "ownership_transfers is shared-resource history; opaque account identifiers remain while human attribution is redacted.",
    },
    Entry {
        collection: "permission_key_policies",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "permission_key_policies stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "personal_erasure_effects",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_effects is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "personal_erasure_jobs",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_jobs is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "personal_erasure_local_receipts",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_local_receipts is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "personal_erasure_preparations",
        owner: OwnerSelector::Fields(&["_id", "user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_preparations is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "personal_erasure_revocation_receipts",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_revocation_receipts is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "personal_owner_write_fences",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_owner_write_fences is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "platform_bot_cleanup",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_bot_cleanup is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_bot_configs",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_bot_configs is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_bot_installations",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_bot_installations is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_bot_runtime_controls",
        owner: OwnerSelector::Fields(&["personal_control_revision"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_bot_runtime_controls is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_credentials",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_credentials is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_delivery_tests",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_delivery_tests is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_discord_control_installation_history",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_discord_control_installation_history is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_discord_control_installations",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_discord_control_installations is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_entry_gateway_partitions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_entry_gateway_partitions is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_entry_gateway_transports",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_entry_gateway_transports is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_entry_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_entry_migrations is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_guide_selections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_guide_selections stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "platform_help_assignments",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_help_assignments is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_help_libraries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_help_libraries is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_help_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_help_revisions is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_manager_deliveries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_manager_deliveries is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_manager_ingress_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_manager_ingress_receipts is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_model_attempts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_model_attempts is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_model_routing_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_model_routing_revisions is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_model_tariffs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_model_tariffs is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_models",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_models is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_native_entry_receipts",
        owner: OwnerSelector::Fields(&["sender_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_native_entry_receipts is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_native_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_native_receipts is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_native_replies",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_native_replies stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "platform_native_reply_handles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_native_reply_handles is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_onboarding_binding_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_onboarding_binding_requests stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "platform_onboarding_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_onboarding_bindings stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "platform_onboarding_decision_spacing",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_onboarding_decision_spacing is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_personal_bindings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_personal_bindings is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_personal_decision_spacing",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_personal_decision_spacing stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "platform_personal_link_challenges",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_personal_link_challenges is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_personal_link_intents",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_personal_link_intents is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_personal_setup_selections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_personal_setup_selections stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "platform_personal_stop_effects",
        owner: OwnerSelector::Fields(&["personal_control_revision"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_personal_stop_effects is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_budget_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_budget_buckets is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_budget_loops",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_budget_loops is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_call_reservations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_call_reservations is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_cleanup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_cleanup_records is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_deployment_activations",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_public_deployment_activations is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_deployment_proofs",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_public_deployment_proofs is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_probe_consents",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_public_probe_consents is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_quotas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_quotas is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_readiness_probes",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_public_readiness_probes is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_readiness_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_readiness_records is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_revocation_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_revocation_scopes is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_sessions is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_public_turns",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_turns is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_settings is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_setup_notices",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_setup_notices is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_setup_steps",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_setup_steps is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "platform_setups",
        owner: OwnerSelector::Fields(&["approved_at"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_setups is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "pool_recovery_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "pool_recovery_diagnostics is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "provider_configs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "provider_configs is processed by the platform_or_channel_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "pushed_authorization_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "pushed_authorization_requests is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "refresh_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "refresh_tokens stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "reply_token_uses",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "reply_token_uses is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "roles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "roles is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "sandbox_billing_cursors",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_cursors is owner-scoped billing settlement state retained for financial reconciliation.",
    },
    Entry {
        collection: "sandbox_billing_evidence",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_evidence is owner-scoped billing settlement state retained for financial reconciliation.",
    },
    Entry {
        collection: "sandbox_billing_quotes",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_quotes is owner-scoped billing settlement state retained for financial reconciliation.",
    },
    Entry {
        collection: "sandbox_billing_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "sandbox_billing_settings is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "sandbox_billing_windows",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_windows is owner-scoped billing settlement state retained for financial reconciliation.",
    },
    Entry {
        collection: "sandbox_rate_cards",
        owner: OwnerSelector::Fields(&["created_by"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "sandbox_rate_cards is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "saved_logins",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "saved_logins stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "schema_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "schema_migrations is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "security_hmac_keysets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "security_hmac_keysets is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "security_remediation_progress",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "security_remediation_progress is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "security_replicas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "security_replicas is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "security_rollouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "security_rollouts is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "service_account_key_read_grants",
        owner: OwnerSelector::Fields(&["_id", "owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_account_key_read_grants stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "service_account_tokens",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Section("services"),
        why: "service_account_tokens is processed by the local_service_account_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "service_accounts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Section("services"),
        why: "service_accounts is processed by the local_service_account_lifecycle erasure helper for its lifecycle-specific ownership and safety guarantees.",
    },
    Entry {
        collection: "service_approval_configs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_approval_configs stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "service_change_events",
        owner: OwnerSelector::Fields(&["owner_id", "actor.person_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        why: "service_change_events is shared history; erase personal ownership and redact human-readable attribution.",
    },
    Entry {
        collection: "service_concurrency_leases",
        owner: OwnerSelector::Fields(&["users"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "service_concurrency_leases is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "service_endpoints",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "service_endpoints is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "service_history_heads",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "user_services",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "Per-service ordering head; removed with its personal user service parent.",
    },
    Entry {
        collection: "service_pool_health_observations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "service_pools",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "Pool health observations join the personal service-pool parent.",
    },
    Entry {
        collection: "service_pool_member_health",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_pool_member_health stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "service_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_pools stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "service_preferences",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_preferences stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "service_provider_requirements",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "service_provider_requirements is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "sessions stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "startup_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "startup_diagnostics is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "telegram_bot_claims",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "telegram_bot_claims stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "telegram_bot_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "telegram_bot_requests stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "telegram_managed_bot_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "telegram_managed_bot_events stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "telemetry_capture_accounts",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::Domain("telemetry_erasure_service"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "telemetry_capture_accounts is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "telemetry_dispatches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("telemetry_erasure_service"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "telemetry_dispatches is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "telemetry_erasure_destinations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "telemetry_erasure_destinations is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "telemetry_erasure_jobs",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "telemetry_erasure_jobs is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "telemetry_erasure_reconciliations",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "telemetry_erasure_reconciliations is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "trigger_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "trigger_deliveries stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "trigger_owner_limits",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "trigger_owner_limits stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "trigger_run_budgets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "trigger_run_budgets is platform, coordination, or shared state with no personal owner.",
    },
    Entry {
        collection: "trigger_runs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "trigger_runs stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "trigger_work",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "trigger_work stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "triggers",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "triggers stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "usage_allowance_periods",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_allowance_periods is a shared financial period or hold; retaining it preserves settlement state for every participant.",
    },
    Entry {
        collection: "usage_allowances",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_allowances is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "usage_meter",
        owner: OwnerSelector::Fields(&["billing_owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        why: "usage_meter is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "usage_rollup_daily",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_rollup_daily is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "usage_rollup_hourly",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_rollup_hourly is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "usage_rollup_state",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_rollup_state is retained integrity or settlement evidence; deletion would change the durable record.",
    },
    Entry {
        collection: "usage_workspaces",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("billing"),
        why: "usage_workspaces stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "user_api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_api_keys stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "user_endpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_endpoints stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "user_provider_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_provider_credentials stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "user_provider_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_provider_tokens stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "user_service_connections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_service_connections stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "user_services",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_services stores person-owned data selected by explicit owner fields and is erased in the local pass.",
    },
    Entry {
        collection: "users",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::Domain("source"),
        export: Export::Section("account"),
        why: "The users row is the account profile and authentication source; it is removed by the source phase.",
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

#[allow(dead_code)]
pub fn find(collection: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.collection == collection)
}

#[allow(dead_code)]
pub fn local_inventory_entries() -> impl Iterator<Item = &'static Entry> {
    ENTRIES
        .iter()
        .filter(|e| matches!(e.erasure, Erasure::LocalInventory))
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
            db.collection::<mongodb::bson::Document>(entry.collection)
                .create_index(
                    IndexModel::builder()
                        .keys(keys)
                        .options(
                            mongodb::options::IndexOptions::builder()
                                .name(index_name)
                                .build(),
                        )
                        .build(),
                )
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let associated = regex::Regex::new(
            r#"(?s)impl\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{.*?(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"]+)""#,
        )
        .unwrap();
        let uses = regex::Regex::new(
            r#"(?m)\buse\s+([A-Za-z_][A-Za-z0-9_:]*)::([A-Z][A-Z0-9_]*)(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;"#,
        )
        .unwrap();
        let use_statements = regex::Regex::new(r#"(?m)\buse\s+([^;]+);"#).unwrap();
        let uppercase = regex::Regex::new(r#"\b[A-Z][A-Z0-9_]*\b"#).unwrap();
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
                    let values = resolve(argument).unwrap_or_default();
                    if !values.is_empty() {
                        values
                    } else {
                        let mut values = BTreeSet::new();
                        for name in uppercase
                            .find_iter(argument)
                            .map(|capture| capture.as_str())
                        {
                            values.extend(resolve(name).unwrap_or_default());
                        }
                        values
                    }
                };
                if values.is_empty() {
                    // A run-time collection parameter cannot be resolved by a
                    // source scan. Constant-looking arguments must resolve;
                    // dynamic variables are covered by the call-site's own
                    // registry tests and are intentionally skipped here.
                    if argument.chars().all(|character| {
                        character.is_ascii_uppercase()
                            || character.is_ascii_digit()
                            || character == '_'
                            || character == ':'
                    }) {
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
                r#"use crate::models::POOLS; db.collection(POOLS); db.collection(AssistantWireLog::COLLECTION_NAME);"#.to_owned(),
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
