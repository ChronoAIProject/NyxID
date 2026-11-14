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
    #[allow(dead_code)]
    MachineLifecycle,
    ErasureBookkeeping,
    SharedAttribution,
}

impl RetainedCategory {
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    JoinedChild {
        parent: &'static str,
    },
    Domain(&'static str),
    Redacted,
    Retained(RetainedCategory),
    NotPersonal(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclusionReason {
    #[allow(dead_code)]
    SecretMaterial,
    #[allow(dead_code)]
    TransientCoordination,
    #[allow(dead_code)]
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
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "agent_service_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "anonymous_endpoint_usage",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "api_key_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "approval_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "approval_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_acknowledgements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_action_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agent_consents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agent_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agent_learning",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agent_learning_members",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agent_learning_proposals",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
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
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agent_learning_skill_roots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agent_private_notes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agent_shares",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agent_sharing",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_analytics_settings",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_artifact_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_async_operations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_async_quotas",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_attachment_expiry",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_attachments",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_authored_draft_claims",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_checkpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_cma_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_cma_creations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_cma_sandboxes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_cma_session_origins",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_direct_handoffs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_group_agent_access",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_group_agent_proposals",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_group_execution_slots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_group_invites",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_group_mention_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_group_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_group_read_states",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_group_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_group_transfers",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_groups",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_invocation_turns",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_invocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_learning_migrations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
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
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_notification_push_budgets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_notification_push_sources",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_notification_worker",
        owner: OwnerSelector::Fields(&["actor_type"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_onboarding",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_profile_routes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_resources",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_service_execution",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_settings",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_skill_draft_attempts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_tool_results",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_upload_limits",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_voice_announcements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_voice_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_voice_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_voice_windows",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "assistant_wire_logs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "async_service_operation_artifacts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "audit_log",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::Audit),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "auth_device_code_reservations",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "auth_device_codes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "auth_device_codes_v2",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "authorization_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_account_balances",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_account_verify_lease",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_account_verify_state",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_cutover_operations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_lago_carry",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_ledger",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_migration_errors",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_pool_recovery",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_rate_cache",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_rate_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_tax_profiles",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_topup_sessions",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_wallet",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "billing_wallet_refresh_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "catalog_delegation_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "catalog_icons",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "catalog_skill_operations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "catalog_skill_revisions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "chain_verify_status",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_activity_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_adapter_accounts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_adapter_allocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_adapter_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_adapter_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_adapter_resource_claims",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_adapter_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_bots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("channel lifecycle"),
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_delivery_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_email_batches",
        owner: OwnerSelector::Fields(&["account_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_email_receipts",
        owner: OwnerSelector::Fields(&["account_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_email_sends",
        owner: OwnerSelector::Fields(&["account_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_email_subscriptions",
        owner: OwnerSelector::Fields(&["account_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
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
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_reply_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
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
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_webhook_rejections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "channel_whatsapp_native",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "cli_pairings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "consent_revocation_fences",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "consents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "coordination_event_dedup",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "coordination_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "coordination_rate_windows",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "coordination_replay_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "coordination_slots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "coordination_token_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "credit_allocation_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "credit_grants",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "credit_schedule_periods",
        owner: OwnerSelector::Fields(&["target_user_ids"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("billing"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "credit_schedules",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "device_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "device_onboard_credentials",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "device_pubkey_lockouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "downstream_services",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "durable_operation_executions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "durable_operation_grants",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "encrypted_record_migration_findings",
        owner: OwnerSelector::Fields(&["owner_uuid"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "encrypted_record_migration_runs",
        owner: OwnerSelector::Fields(&["owner_uuid"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "event_dedup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "execution_enrollment_jobs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "execution_enrollment_repairs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "execution_permission_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "feature_flag_definitions",
        owner: OwnerSelector::Fields(&["created_by"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "feature_flag_metadata",
        owner: OwnerSelector::Fields(&["owner"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "feature_flag_overrides",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "generated_artifact_content_series",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "groups",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "invite_codes",
        owner: OwnerSelector::Fields(&["usages.user_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "invocation_diagnostics",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "lark_base_write_photos",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "lark_base_writes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "login_approvals",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "login_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_authority_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_confirmations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_contexts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_desktops",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_effect_outbox",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_external_grants",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_free_slots",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_jobs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_lifecycle_events",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_oauth_consent_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_operations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_orphan_inventory",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_revocations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_runtime_heads",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_runtime_intervals",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_runtime_quotes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_runtime_rates",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_runtime_runs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_separation_compatibility",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_setups",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_update_attempts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "machine_updates",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "managed_executions",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "managed_machine_bootstrap_deliveries",
        owner: OwnerSelector::Fields(&["recipient_key_digest"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "managed_machines",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("machine"),
        export: Export::Section("machines"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "mcp_session_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "mcp_session_slots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "mcp_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "mfa_challenges",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "mfa_factors",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "node_pending_credentials",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "node_registration_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "node_service_bindings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nodes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "notification_channels",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nyxbot_channels",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nyxbot_event_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nyxbot_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nyxbot_gateway_releases",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nyxbot_gateway_responses",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nyxbot_image_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nyxbot_image_deliveries_v2",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nyxbot_threads",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "nyxbot_watches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oauth_broker_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oauth_clients",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oauth_consent_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oauth_delegation_authorities",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oauth_states",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "openclaw_channel_mappings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oracle_login_profiles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oracle_login_snapshots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oracle_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oracle_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oracle_tasks",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oracle_worker_commands",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "oracle_workers",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "org_channel_guest_budgets",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "org_channel_guest_holds",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "org_channel_lanes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "org_channel_link_challenges",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "org_channel_messages",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "org_channel_sender_links",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "org_invites",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "org_memberships",
        owner: OwnerSelector::Fields(&["member_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "org_role_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "ownership_transfers",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "permission_key_policies",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "personal_erasure_effects",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "personal_erasure_jobs",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "personal_erasure_local_receipts",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "personal_erasure_preparations",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "personal_erasure_revocation_receipts",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "personal_owner_write_fences",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_bot_cleanup",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_bot_configs",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_bot_installations",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_bot_runtime_controls",
        owner: OwnerSelector::Fields(&["personal_control_revision"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_credentials",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_delivery_tests",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_discord_control_installation_history",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_discord_control_installations",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_entry_gateway_partitions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_entry_gateway_transports",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_entry_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_guide_selections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_help_assignments",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_help_libraries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_help_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_manager_deliveries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_manager_ingress_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_model_attempts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_model_routing_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_model_tariffs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_models",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_native_entry_receipts",
        owner: OwnerSelector::Fields(&["sender_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_native_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_native_replies",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_native_reply_handles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_onboarding_binding_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_onboarding_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_onboarding_decision_spacing",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_personal_bindings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_personal_decision_spacing",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_personal_link_challenges",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_personal_link_intents",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_personal_setup_selections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_personal_stop_effects",
        owner: OwnerSelector::Fields(&["personal_control_revision"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_budget_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_budget_loops",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_call_reservations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_cleanup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_deployment_activations",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_deployment_proofs",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_probe_consents",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_quotas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_readiness_probes",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_readiness_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_revocation_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_public_turns",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_setup_notices",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_setup_steps",
        owner: OwnerSelector::Fields(&["approved_child_bot_id"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "platform_setups",
        owner: OwnerSelector::Fields(&["approved_at"]),
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "pool_recovery_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "provider_configs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("platform_or_channel_lifecycle"),
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "pushed_authorization_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "refresh_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "reply_token_uses",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "roles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "sandbox_billing_cursors",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "sandbox_billing_evidence",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "sandbox_billing_quotes",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "sandbox_billing_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "sandbox_billing_windows",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "sandbox_rate_cards",
        owner: OwnerSelector::Fields(&["created_by"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "saved_logins",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "schema_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "security_hmac_keysets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "security_remediation_progress",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "security_replicas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "security_rollouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_account_key_read_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_account_tokens",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_accounts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain("local_service_account_lifecycle"),
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_approval_configs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_change_events",
        owner: OwnerSelector::Fields(&["owner_id", "actor.person_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_concurrency_leases",
        owner: OwnerSelector::Fields(&["users"]),
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_endpoints",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
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
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_preferences",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "service_provider_requirements",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "startup_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "telegram_bot_claims",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "telegram_bot_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "telegram_managed_bot_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "telemetry_capture_accounts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "telemetry_dispatches",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "telemetry_erasure_destinations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "telemetry_erasure_jobs",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "telemetry_erasure_reconciliations",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "trigger_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "trigger_owner_limits",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "trigger_run_budgets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "trigger_runs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "trigger_work",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "triggers",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "usage_allowance_periods",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("billing"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "usage_allowances",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "usage_meter",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "usage_rollup_daily",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "usage_rollup_hourly",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "usage_rollup_state",
        owner: OwnerSelector::Fields(&[]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "usage_workspaces",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("billing"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "user_api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "user_endpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "user_provider_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "user_provider_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "user_service_connections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "user_services",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        why: "Reviewed against the production collection call sites.",
    },
    Entry {
        collection: "users",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        why: "Reviewed against the production collection call sites.",
    },
];

pub fn entries() -> &'static [Entry] {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, fs, path::Path};

    #[test]
    fn registry_has_unique_collection_names() {
        let names: BTreeSet<_> = ENTRIES.iter().map(|e| e.collection).collect();
        assert_eq!(names.len(), ENTRIES.len());
    }

    #[test]
    fn every_production_collection_literal_is_registered() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        fn strip_test_code(source: &str) -> String {
            let mut output = String::with_capacity(source.len());
            let mut cursor = 0;
            while let Some(relative) = source[cursor..].find("#[cfg(test)]") {
                let start = cursor + relative;
                output.push_str(&source[cursor..start]);
                let mut item = start + "#[cfg(test)]".len();
                while let Some(rest) = source.get(item..) {
                    let trimmed = rest.trim_start();
                    item += rest.len() - trimmed.len();
                    if trimmed.starts_with("#[")
                        && let Some(end) = trimmed.find(']')
                    {
                        item += end + 1;
                        continue;
                    }
                    break;
                }
                let rest = &source[item..];
                let Some(open) = rest.find(['{', ';']) else {
                    break;
                };
                if rest.as_bytes()[open] == b';' {
                    cursor = item + open + 1;
                    continue;
                }
                let bytes = rest.as_bytes();
                let mut depth = 0usize;
                let mut end = item + open;
                let mut state = 0u8; // normal, string, char, line comment, block comment
                let mut escaped = false;
                let mut index = open;
                while index < bytes.len() {
                    let byte = bytes[index];
                    match state {
                        0 => match byte {
                            b'"' => state = 1,
                            b'\'' => state = 2,
                            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                                state = 3;
                                index += 1;
                            }
                            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                                state = 4;
                                index += 1;
                            }
                            b'{' => depth += 1,
                            b'}' => {
                                depth = depth.saturating_sub(1);
                                if depth == 0 {
                                    end = item + index + 1;
                                    break;
                                }
                            }
                            _ => {}
                        },
                        1 => {
                            if escaped {
                                escaped = false;
                            } else if byte == b'\\' {
                                escaped = true;
                            } else if byte == b'"' {
                                state = 0;
                            }
                        }
                        2 => {
                            if escaped {
                                escaped = false;
                            } else if byte == b'\\' {
                                escaped = true;
                            } else if byte == b'\'' {
                                state = 0;
                            }
                        }
                        3 if byte == b'\n' => state = 0,
                        4 if byte == b'*' && bytes.get(index + 1) == Some(&b'/') => {
                            state = 0;
                            index += 1;
                        }
                        _ => {}
                    }
                    index += 1;
                }
                cursor = end;
            }
            output.push_str(&source[cursor..]);
            output
        }
        fn gather(path: &Path, registry: &Path, files: &mut Vec<(std::path::PathBuf, String)>) {
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
                files.push((p, strip_test_code(&source)));
            }
        }
        let registry = root.join("services/personal_data_registry.rs");
        let mut files = Vec::new();
        gather(&root, &registry, &mut files);

        // Resolve collection constants through their call sites. Keeping the
        // constant name in the map also handles module-qualified and
        // associated constants; all collection constants resolve to a unique
        // MongoDB collection value.
        let constant_pattern = regex::Regex::new(
            r#"\b(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*\"([^\"]+)\""#,
        )
        .unwrap();
        let call_pattern =
            regex::Regex::new(r#"\.collection(?:::[^\(]+)?\(\s*([^\s,\)]+)"#).unwrap();
        let collection_region_pattern =
            regex::Regex::new(r#"(?s)\.collection(?:::[^\(]+)?\((.*?)\)"#).unwrap();
        let mut constants = std::collections::HashMap::<String, BTreeSet<String>>::new();
        for (_, source) in &files {
            for caps in constant_pattern.captures_iter(source) {
                constants
                    .entry(caps[1].to_owned())
                    .or_default()
                    .insert(caps[2].to_owned());
            }
        }
        let mut seen = BTreeSet::new();
        for (path, source) in files {
            for region in collection_region_pattern.captures_iter(&source) {
                let expression = &region[1];
                for (name, values) in &constants {
                    if expression.contains(name) {
                        seen.extend(values.iter().cloned());
                    }
                }
            }
            for caps in call_pattern.captures_iter(&source) {
                let argument = &caps[1];
                if argument.starts_with('"') {
                    let name = argument.trim_matches('"');
                    seen.insert(name.to_owned());
                    if find(name).is_none() {
                        missing.push(format!(
                            "{}: {}",
                            path.strip_prefix(&root).unwrap().display(),
                            name
                        ));
                    }
                    continue;
                }
                let name = argument.rsplit("::").next().unwrap_or(argument);
                if let Some(values) = constants.get(name) {
                    seen.extend(values.iter().cloned());
                }
            }
        }
        // The local erasure inventory is itself a production collection use;
        // some entries are represented only by a model constant in that
        // table and never appear in a direct `.collection(...)` call here.
        seen.extend(
            crate::services::personal_erasure_service::local::inventory("source-scan-owner")
                .into_iter()
                .map(|(collection, _)| collection.to_owned()),
        );
        assert!(
            missing.is_empty(),
            "unregistered collection literals: {missing:?}"
        );
        let stale: Vec<_> = ENTRIES
            .iter()
            .filter(|entry| !seen.contains(entry.collection))
            .map(|entry| entry.collection)
            .collect();
        assert!(stale.is_empty(), "stale registry entries: {stale:?}");
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
