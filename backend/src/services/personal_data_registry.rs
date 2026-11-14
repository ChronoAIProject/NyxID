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
        why: "agent_key_login_requests stores person-owned rows keyed by approved_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "agent_service_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "agent_service_bindings stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "anonymous_endpoint_usage",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "anonymous_endpoint_usage is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "api_key_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "api_key_credentials stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "api_keys stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "approval_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "approval_grants stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "approval_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "approval_requests stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_acknowledgements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_acknowledgements stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_action_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_action_receipts stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_consents",
        owner: OwnerSelector::Fields(&["actor_user_id", "owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_consents stores person-owned rows keyed by actor_user_id, owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        why: "assistant_agent_credentials stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning_members",
        owner: OwnerSelector::Fields(&["member_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning_members stores person-owned rows keyed by member_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning_proposals",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning_proposals stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning_rejections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "assistant_agents",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "assistant_agent_learning_rejections is lifecycle data owned through assistant_agents; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "assistant_agent_learning_runs",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning_runs stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning_skill_roots",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_learning_skill_roots stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_private_notes",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_private_notes stores person-owned rows keyed by actor_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_shares",
        owner: OwnerSelector::Fields(&["owner_id", "subject_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_shares stores person-owned rows keyed by owner_id, subject_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_sharing",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agent_sharing stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_agents stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_analytics_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_analytics_settings is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_artifact_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_artifact_deliveries stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_async_operations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_async_operations stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_async_quotas",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_async_quotas stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_attachment_expiry",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_attachment_expiry stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_attachments",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_attachments stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_authored_draft_claims",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_authored_draft_claims stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_checkpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_checkpoints stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_cma_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_cma_agents stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_cma_creations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_cma_creations stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_cma_sandboxes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_cma_sandboxes stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_cma_session_origins",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_cma_session_origins stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_conversations stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_direct_handoffs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_direct_handoffs stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_agent_access",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_group_agent_access stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_agent_proposals",
        owner: OwnerSelector::Fields(&["proposed_by"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_agent_proposals stores person-owned rows keyed by proposed_by; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_execution_slots",
        owner: OwnerSelector::Fields(&["original_requester_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_group_execution_slots stores person-owned rows keyed by original_requester_id; bounded local cleanup removes those rows after the erasure admission fence.",
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
        why: "assistant_group_invites stores person-owned rows keyed by issuer_user_id, accepted_by_user_id, intended_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_mention_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_mention_notifications stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_messages stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_read_states",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_read_states stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_group_requests stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_transfers",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "assistant_group_transfers is retained as SharedAttribution evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "assistant_groups",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_groups stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_invocation_turns",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_invocation_turns stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_invocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_invocations stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_learning_migrations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_learning_migrations stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_learning_publication_targets",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("assistant_learning_reconciliation"),
        export: Export::Section("account_activity"),
        why: "assistant_learning_publication_targets is cleaned by the assistant_learning_reconciliation lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "assistant_message_feedback",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_message_feedback stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_messages stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_migrations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_notification_push_budgets",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "assistant_notification_push_budgets stores person-owned rows keyed by _id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_notification_push_sources",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "assistant_notification_push_sources stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_notification_worker",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_notification_worker is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_notifications stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_onboarding",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_onboarding stores person-owned rows keyed by _id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_profile_routes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_profile_routes is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_resources",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_resources stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_service_execution",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_service_execution stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_settings",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_settings stores person-owned rows keyed by _id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_skill_draft_attempts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_skill_draft_attempts stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_tool_results",
        owner: OwnerSelector::Fields(&["user_id", "actor_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_tool_results stores person-owned rows keyed by user_id, actor_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_upload_limits",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_upload_limits is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_voice_announcements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_voice_announcements stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_voice_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "assistant_voice_requests stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_voice_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("voice_purge"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "assistant_voice_sessions is cleaned by the voice_purge lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "assistant_voice_windows",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_voice_windows stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_wire_logs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "assistant_wire_logs stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "async_service_operation_artifacts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "async_service_operation_artifacts stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "audit_log",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::Audit),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "audit_log is retained as Audit evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "auth_device_code_reservations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("public-code uniqueness fence"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "auth_device_code_reservations is public-code uniqueness fence and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "auth_device_codes",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "auth_device_codes stores person-owned rows keyed by approved_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "auth_device_codes_v2",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "auth_device_codes_v2 stores person-owned rows keyed by approved_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "authorization_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "authorization_codes stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "billing_account_balances",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_account_balances is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_account_verify_lease",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_account_verify_lease is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_account_verify_state",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_account_verify_state is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_cutover_operations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_cutover_operations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_lago_carry",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_lago_carry is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_ledger",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_ledger is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_migration_errors",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_migration_errors is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_migrations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_pool_recovery",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_pool_recovery is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_rate_cache",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_rate_cache is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_rate_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_rate_diagnostics is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_tax_profiles",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_tax_profiles is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_topup_sessions",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "billing_topup_sessions is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_wallet",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        why: "billing_wallet is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_wallet_refresh_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "billing_wallet_refresh_requests is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "catalog_delegation_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "catalog_delegation_grants stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "catalog_icons",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "catalog_icons is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "catalog_skill_operations",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "catalog_skill_operations is retained as SharedAttribution evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "catalog_skill_revisions",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "catalog_skill_revisions is retained as SharedAttribution evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "chain_verify_status",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "chain_verify_status is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "channel_activity_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_activity_notifications stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_accounts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "channel_adapter_accounts stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_allocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_allocations stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_bindings stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_events stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_resource_claims",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_resource_claims stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_adapter_sessions stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_bots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_bots stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_connect_links stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("channel lifecycle"),
        export: Export::Section("channels"),
        why: "channel_conversations is cleaned by the channel lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "channel_delivery_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_delivery_receipts stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_email_batches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_email_batches stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_email_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_email_receipts stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_email_sends",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_email_sends stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_email_subscriptions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_email_subscriptions stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_event_logs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "channel_conversations",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "channel_event_logs is lifecycle data owned through channel_conversations; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "channel_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_messages stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_reply_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_reply_deliveries stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_send_claims",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "channel_conversations",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "channel_send_claims is lifecycle data owned through channel_conversations; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "channel_text_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_text_deliveries stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "channel_webhook_rejections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "channel_webhook_rejections is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "channel_whatsapp_native",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "channel_whatsapp_native stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "cli_pairings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "cli_pairings stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "connect_links stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "consent_revocation_fences",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "consent_revocation_fences is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "consents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "consents stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "coordination_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_leases is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "coordination_rate_windows",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_rate_windows is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "coordination_replay_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_replay_records is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "coordination_slots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_slots is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "coordination_token_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "coordination_token_buckets is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "credit_allocation_requests",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "credit_allocation_requests is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "credit_grants",
        owner: OwnerSelector::Fields(&["recipient_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        why: "credit_grants is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "credit_schedule_periods",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "credit_schedule_periods is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "credit_schedules",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "credit_schedules is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "device_codes",
        owner: OwnerSelector::Fields(&["approved_by_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "device_codes stores person-owned rows keyed by approved_by_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "device_onboard_credentials",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "device_onboard_credentials stores person-owned rows keyed by owner_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "device_pubkey_lockouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "device_pubkey_lockouts is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "downstream_services",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "downstream_services is retained as SharedAttribution evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "durable_operation_executions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "durable_operation_executions is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "durable_operation_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "durable_operation_grants is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "encrypted_record_migration_findings",
        owner: OwnerSelector::Fields(&["owner_uuid"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "encrypted_record_migration_findings stores person-owned rows keyed by owner_uuid; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "encrypted_record_migration_runs",
        owner: OwnerSelector::Fields(&["owner_uuid"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "encrypted_record_migration_runs stores person-owned rows keyed by owner_uuid; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "event_dedup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "event_dedup_records is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "execution_enrollment_jobs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "execution_enrollment_jobs is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "execution_enrollment_repairs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "execution_enrollment_repairs is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "execution_permission_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "execution_permission_deliveries stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "feature_flag_definitions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "feature_flag_definitions is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "feature_flag_metadata",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "feature_flag_metadata is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "feature_flag_overrides",
        owner: OwnerSelector::Fields(&["target_key"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "feature_flag_overrides stores person-owned rows keyed by target_key; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "generated_artifact_content_series",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "generated_artifact_content_series stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "groups",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "groups is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "invite_codes",
        owner: OwnerSelector::Fields(&["usages.user_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        why: "invite_codes contains shared history; cleanup removes personal ownership or human-readable attribution while preserving shared counters and opaque identifiers.",
    },
    Entry {
        collection: "invocation_diagnostics",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "invocation_diagnostics stores person-owned rows keyed by actor_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "lark_base_write_photos",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "lark_base_write_photos stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "lark_base_writes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "lark_base_writes stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "login_approvals",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "login_approvals stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "login_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "login_codes stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "machine_authority_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "machine_authority_leases is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_confirmations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_confirmations is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_contexts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "machine_contexts is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_desktops",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "machine_desktops is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_effect_outbox",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_effect_outbox is retained as MachineLifecycle evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_external_grants",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "machine_external_grants is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_free_slots",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_free_slots is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_jobs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_jobs is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_lifecycle_events",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_lifecycle_events is retained as MachineLifecycle evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_oauth_consent_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "machine_oauth_consent_requests is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_operations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_operations is retained as MachineLifecycle evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_orphan_inventory",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "machine_orphan_inventory is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_revocations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "machine_revocations is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_runtime_heads",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_heads is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_runtime_intervals",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_intervals is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_runtime_quotes",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_quotes is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_runtime_rates",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_rates is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_runtime_runs",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "machine_runtime_runs is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_separation_compatibility",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "machine_separation_compatibility is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_setups",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "machine_setups is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_update_attempts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "machine_update_attempts is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "machine_updates",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "machine_updates is cleaned by the machine lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "managed_executions",
        owner: OwnerSelector::Fields(&["billing_owner_id", "actor_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "managed_executions is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "managed_machine_bootstrap_deliveries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "managed_machine_bootstrap_deliveries is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "managed_machines",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "managed_machines is retained as MachineLifecycle evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "mcp_session_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "mcp_session_notifications stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "mcp_session_slots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "mcp_session_slots stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "mcp_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        why: "mcp_sessions stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "mfa_challenges",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "mfa_challenges stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "mfa_factors",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "mfa_factors stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "node_pending_credentials",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("nodes"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "node_pending_credentials is cleaned by the nodes lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "node_registration_tokens",
        owner: OwnerSelector::Fields(&["user_id", "issuer_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "node_registration_tokens stores person-owned rows keyed by user_id, issuer_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "node_service_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("nodes"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "node_service_bindings is cleaned by the nodes lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "nodes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("nodes"),
        export: Export::Section("machines"),
        why: "nodes is cleaned by the nodes lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "notification_channels",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "notification_channels stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_channels",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("channel_bots"),
        export: Export::Section("channels"),
        why: "nyxbot_channels is cleaned by the channel_bots lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "nyxbot_event_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_event_bindings stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_events stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_gateway_releases",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "nyxbot_gateway_releases is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "nyxbot_gateway_responses",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_gateway_responses stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_image_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_image_deliveries stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_image_deliveries_v2",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_image_deliveries_v2 stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_threads",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_threads stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_watches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "nyxbot_watches stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "oauth_broker_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "oauth_broker_bindings stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "oauth_clients",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "oauth_clients is cleaned by the local_service_account_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "oauth_consent_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "oauth_consent_requests stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "oauth_delegation_authorities",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "oauth_delegation_authorities stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "oauth_states",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "oauth_states stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "openclaw_channel_mappings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "openclaw_channel_mappings stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "oracle_login_profiles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_login_profiles is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "oracle_login_snapshots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_login_snapshots is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "oracle_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("oracle"),
        export: Export::Section("account"),
        why: "oracle_pools is cleaned by the oracle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "oracle_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_sessions is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "oracle_tasks",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_tasks is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "oracle_worker_commands",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_worker_commands is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "oracle_workers",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "oracle_workers is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "org_channel_guest_budgets",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "org_channel_guest_budgets is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "org_channel_guest_holds",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "org_channel_guest_holds is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "org_channel_lanes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_channel_lanes is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "org_channel_link_challenges",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_channel_link_challenges is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "org_channel_messages",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_channel_messages is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "org_channel_sender_links",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_channel_sender_links is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "org_invites",
        owner: OwnerSelector::Fields(&["created_by"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "org_invites stores person-owned rows keyed by created_by; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "org_memberships",
        owner: OwnerSelector::Fields(&["member_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "org_memberships stores person-owned rows keyed by member_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "org_role_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "org_role_scopes is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "ownership_transfers",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "ownership_transfers is retained as SharedAttribution evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "permission_key_policies",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "permission_key_policies stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "personal_erasure_effects",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_effects is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "personal_erasure_jobs",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_jobs is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "personal_erasure_local_receipts",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_local_receipts is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "personal_erasure_preparations",
        owner: OwnerSelector::Fields(&["_id", "user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_preparations stores person-owned rows keyed by _id, user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "personal_erasure_revocation_receipts",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_erasure_revocation_receipts is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "personal_owner_write_fences",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "personal_owner_write_fences is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "platform_bot_cleanup",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_bot_cleanup is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_bot_configs",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_bot_configs is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_bot_installations",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_bot_installations is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_bot_runtime_controls",
        owner: OwnerSelector::Fields(&["personal_control_revision"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_bot_runtime_controls is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_credentials",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_credentials is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_delivery_tests",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_delivery_tests is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_discord_control_installation_history",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_discord_control_installation_history is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_discord_control_installations",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_discord_control_installations is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_entry_gateway_partitions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_entry_gateway_partitions is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_entry_gateway_transports",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_entry_gateway_transports is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_entry_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_entry_migrations is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_guide_selections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_guide_selections stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "platform_help_assignments",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_help_assignments is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_help_libraries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_help_libraries is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_help_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_help_revisions is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_manager_deliveries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_manager_deliveries is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_manager_ingress_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_manager_ingress_receipts is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_model_attempts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_model_attempts is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_model_routing_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_model_routing_revisions is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_model_tariffs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_model_tariffs is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_models",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_models is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_native_entry_receipts",
        owner: OwnerSelector::Fields(&["sender_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_native_entry_receipts is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_native_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_native_receipts is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_native_replies",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_native_replies stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "platform_native_reply_handles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_native_reply_handles is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_onboarding_binding_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_onboarding_binding_requests stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "platform_onboarding_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_onboarding_bindings stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "platform_onboarding_decision_spacing",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_onboarding_decision_spacing is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_personal_bindings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_personal_bindings is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_personal_decision_spacing",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_personal_decision_spacing stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "platform_personal_link_challenges",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_personal_link_challenges is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_personal_link_intents",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_personal_link_intents is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_personal_setup_selections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "platform_personal_setup_selections stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "platform_personal_stop_effects",
        owner: OwnerSelector::Fields(&["personal_control_revision"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_personal_stop_effects is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_public_budget_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_budget_buckets is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_budget_loops",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_budget_loops is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_call_reservations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_call_reservations is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_cleanup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_cleanup_records is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_deployment_activations",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_public_deployment_activations is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_public_deployment_proofs",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_public_deployment_proofs is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_public_probe_consents",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_public_probe_consents is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_public_quotas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_quotas is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_readiness_probes",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_public_readiness_probes is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_public_readiness_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_readiness_records is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_revocation_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_revocation_scopes is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_sessions is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_turns",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_public_turns is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "platform_settings is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_setup_notices",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_setup_notices is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_setup_steps",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_setup_steps is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "platform_setups",
        owner: OwnerSelector::Fields(&["approved_at"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "platform_setups is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "pool_recovery_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "pool_recovery_diagnostics is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "provider_configs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "provider_configs is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "pushed_authorization_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "pushed_authorization_requests is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "refresh_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "refresh_tokens stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "reply_token_uses",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "reply_token_uses is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "roles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "roles is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "sandbox_billing_cursors",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_cursors is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "sandbox_billing_evidence",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_evidence is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "sandbox_billing_quotes",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_quotes is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "sandbox_billing_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "sandbox_billing_settings is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "sandbox_billing_windows",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "sandbox_billing_windows is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "sandbox_rate_cards",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "sandbox_rate_cards is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "saved_logins",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "saved_logins stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "schema_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "schema_migrations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "security_hmac_keysets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "security_hmac_keysets is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "security_remediation_progress",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "security_remediation_progress is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "security_replicas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "security_replicas is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "security_rollouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "security_rollouts is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "service_account_key_read_grants",
        owner: OwnerSelector::Fields(&["_id", "owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_account_key_read_grants stores person-owned rows keyed by _id, owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "service_account_tokens",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        why: "service_account_tokens is cleaned by the local_service_account_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "service_accounts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "service_accounts is cleaned by the local_service_account_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "service_approval_configs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_approval_configs stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "service_change_events",
        owner: OwnerSelector::Fields(&["owner_id", "actor.person_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        why: "service_change_events contains shared history; cleanup removes personal ownership or human-readable attribution while preserving shared counters and opaque identifiers.",
    },
    Entry {
        collection: "service_concurrency_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "service_concurrency_leases is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "service_endpoints",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "service_endpoints is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "service_history_heads",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "user_services",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "service_history_heads is lifecycle data owned through user_services; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "service_pool_health_observations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "service_pools",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        why: "service_pool_health_observations is lifecycle data owned through service_pools; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "service_pool_member_health",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_pool_member_health stores person-owned rows keyed by owner_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "service_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_pools stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "service_preferences",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "service_preferences stores person-owned rows keyed by _id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "service_provider_requirements",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "service_provider_requirements is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "sessions stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "startup_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "startup_diagnostics is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "telegram_bot_claims",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "telegram_bot_claims stores person-owned rows keyed by owner_user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "telegram_bot_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "telegram_bot_requests stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "telegram_managed_bot_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "telegram_managed_bot_events stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "telemetry_capture_accounts",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::Domain("telemetry_erasure_service"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "telemetry_capture_accounts is cleaned by the telemetry_erasure_service lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "telemetry_dispatches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("telemetry_erasure_service"),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        why: "telemetry_dispatches is cleaned by the telemetry_erasure_service lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "telemetry_erasure_destinations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "telemetry_erasure_destinations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "telemetry_erasure_jobs",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "telemetry_erasure_jobs is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "telemetry_erasure_reconciliations",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "telemetry_erasure_reconciliations is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "trigger_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "trigger_deliveries stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "trigger_owner_limits",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "trigger_owner_limits stores person-owned rows keyed by _id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "trigger_run_budgets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "trigger_run_budgets is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "trigger_runs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "trigger_runs stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "trigger_work",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "trigger_work stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "triggers",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "triggers stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "usage_allowance_periods",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_allowance_periods is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_allowances",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_allowances is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_meter",
        owner: OwnerSelector::Fields(&["billing_owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        why: "usage_meter is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_rollup_daily",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_rollup_daily is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_rollup_hourly",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_rollup_hourly is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_rollup_state",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "usage_rollup_state is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_workspaces",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("billing"),
        why: "usage_workspaces stores person-owned rows keyed by _id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "user_api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_api_keys stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "user_endpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_endpoints stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "user_provider_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_provider_credentials stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "user_provider_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_provider_tokens stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "user_service_connections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_service_connections stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "user_services",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "user_services stores person-owned rows keyed by user_id; bounded local cleanup removes those rows after the erasure admission fence.",
    },
    Entry {
        collection: "users",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::Domain("source"),
        export: Export::Section("account"),
        why: "users is cleaned by the source lifecycle, which applies the domain-specific external-effect and ownership fences.",
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

#[cfg(test)]
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

    #[test]
    fn every_registry_entry_has_collection_specific_rationale() {
        let boilerplate = "Reviewed against the production collection call sites.";
        let mut rationales = BTreeSet::new();
        for entry in ENTRIES {
            assert!(
                entry.why.contains(entry.collection),
                "rationale does not name {}",
                entry.collection
            );
            assert_ne!(
                entry.why, boilerplate,
                "boilerplate rationale for {}",
                entry.collection
            );
            assert!(
                rationales.insert(entry.why),
                "duplicate rationale for {}",
                entry.collection
            );
        }
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
                    global.entry(alias.clone()).or_default().extend(values);
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
        let source_seen = scan_collection_calls(&files)
            .unwrap_or_else(|errors| panic!("unresolved production collection calls: {errors:?}"));
        let registered: BTreeSet<String> = ENTRIES
            .iter()
            .map(|entry| entry.collection.to_owned())
            .collect();
        if let Err(source_errors) = validate_scan(&files, &registered) {
            // validate_scan also checks stale entries; production inventory has
            // a separate final-sweep path, so report the exact source drift.
            let unregistered: Vec<_> = source_errors
                .into_iter()
                .filter(|error| error.starts_with("unregistered collection:"))
                .collect();
            assert!(
                unregistered.is_empty(),
                "unregistered collection calls: {unregistered:?}"
            );
        }
        let inventory_names: BTreeSet<_> =
            crate::services::personal_erasure_service::local::inventory("source-scan-owner")
                .into_iter()
                .map(|(collection, _)| collection.to_owned())
                .collect();
        let covered: BTreeSet<_> = source_seen.union(&inventory_names).cloned().collect();
        let stale: Vec<_> = ENTRIES
            .iter()
            .filter(|entry| {
                !source_seen.contains(entry.collection)
                    && !inventory_names.contains(entry.collection)
            })
            .map(|entry| entry.collection)
            .collect();
        assert!(stale.is_empty(), "stale registry entry: {stale:?}");
        let missing: Vec<_> = covered
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
        let field_regex =
            regex::Regex::new(r"(?m)^\s*(?:pub\s+)?([a-zA-Z_][a-zA-Z0-9_]*)\s*:").unwrap();
        fn gather(path: &Path, fields: &mut BTreeSet<String>, field_regex: &regex::Regex) {
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    gather(&path, fields, field_regex);
                } else if path.extension().and_then(|value| value.to_str()) == Some("rs")
                    && let Ok(source) = fs::read_to_string(path)
                {
                    for capture in field_regex.captures_iter(&source) {
                        fields.insert(capture[1].to_owned());
                    }
                }
            }
        }
        gather(&root, &mut fields, &field_regex);
        let documented = BTreeSet::from([
            "actor".to_owned(),
            "usages".to_owned(),
            "target_user_ids".to_owned(),
            "target_group_ids".to_owned(),
            "owner_id".to_owned(),
            "user_id".to_owned(),
            "_id".to_owned(),
            "owner_uuid".to_owned(),
            "approved_user_id".to_owned(),
            "approved_by_user_id".to_owned(),
            "owner_user_id".to_owned(),
            "member_user_id".to_owned(),
            "recipient_user_id".to_owned(),
            "billing_owner_id".to_owned(),
            "actor_user_id".to_owned(),
            "actor_id".to_owned(),
            "subject_id".to_owned(),
            "original_requester_id".to_owned(),
            "issuer_user_id".to_owned(),
            "accepted_by_user_id".to_owned(),
            "intended_user_id".to_owned(),
            "created_by".to_owned(),
            "target_key".to_owned(),
            "person_id".to_owned(),
            "credential_user_id".to_owned(),
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
