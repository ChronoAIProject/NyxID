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
pub enum ErasureDomain {
    Learning,
    LocalEvidence,
    Voice,
    Channel,
    NodeDrain,
    Nodes,
    ChannelBots,
    ServiceAccounts,
    Oracle,
    OrgChannel,
    Platform,
    Source,
}

impl ErasureDomain {
    pub const fn execution(self) -> (&'static str, &'static str) {
        match self {
            Self::LocalEvidence => ("local_inventory", "local::retain_settled_evidence"),
            Self::Learning => ("local_inventory", "learning::clear"),
            Self::Voice => ("local_inventory", "local::pass"),
            Self::Channel => ("local_inventory", "local::joined_parents"),
            Self::NodeDrain => ("source", "local::source"),
            Self::Nodes => ("local_inventory", "local::pass"),
            Self::ChannelBots => ("channel_bots", "channel::pass"),
            Self::ServiceAccounts => ("local_inventory", "local::pass"),
            Self::Oracle => ("oracle", "oracle_pool_service::personal_erasure_pass"),
            Self::OrgChannel => (
                "local_inventory",
                "org_channel_service::lifecycle::erase_person",
            ),
            Self::Platform => ("local_inventory", "local::joined_parents"),
            Self::Source => ("source", "local::source"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Erasure {
    LocalInventory,
    JoinedChild {
        parent: &'static str,
    },
    Domain(ErasureDomain),
    /// The owning feature expires this personal row after its documented TTL.
    FeatureRetention(&'static str),
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

/// Account-export access is separate from destructive ownership and attribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportSelector {
    None,
    Person(&'static str),
    AnyPerson(&'static [&'static str]),
    LegacyServiceAccount,
    AuthoredOwnedGroup,
    JoinedPerson {
        parent: &'static str,
        field: &'static str,
        owner_field: &'static str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExportViewSelector {
    AuthoredGroupMessagesOutsideOwnedGroups,
    AuditMetadata,
}

/// An explicitly scoped second export view of an already registered source.
/// Each (collection, selector) maps to one distinct section/subsection pair.
pub struct AdditionalExportView {
    pub collection: &'static str,
    pub section: &'static str,
    pub subsection: &'static str,
    pub selector: ExportViewSelector,
}

pub static ADDITIONAL_EXPORT_VIEWS: &[AdditionalExportView] = &[
    AdditionalExportView {
        collection: "assistant_group_messages",
        section: "assistant",
        subsection: "authored_group_messages",
        selector: ExportViewSelector::AuthoredGroupMessagesOutsideOwnedGroups,
    },
    AdditionalExportView {
        collection: "audit_log",
        section: "account_activity",
        subsection: "audit_log",
        selector: ExportViewSelector::AuditMetadata,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub collection: &'static str,
    pub owner: OwnerSelector,
    pub erasure: Erasure,
    pub export: Export,
    pub export_selector: ExportSelector,
    pub why: &'static str,
}

/// Persisted human-readable attribution, independent of resource ownership.
/// The opaque person key is retained; these paths do not confer ownership of
/// the containing shared resource. Authored-content exports use explicit views.
pub struct Attribution {
    pub collection: &'static str,
    pub person_field: &'static str,
    pub name_fields: &'static [&'static str],
    pub email_fields: &'static [&'static str],
}

pub static ATTRIBUTIONS: &[Attribution] = &[
    Attribution {
        collection: "service_change_events",
        person_field: "actor.person_id",
        name_fields: &["actor.name", "actor.display_name"],
        email_fields: &["actor.email", "actor.email_address"],
    },
    Attribution {
        collection: "user_services",
        person_field: "created_by.actor.person_id",
        name_fields: &["created_by.actor.name", "created_by.actor.display_name"],
        email_fields: &["created_by.actor.email", "created_by.actor.email_address"],
    },
    Attribution {
        collection: "user_services",
        person_field: "last_change.actor.person_id",
        name_fields: &["last_change.actor.name", "last_change.actor.display_name"],
        email_fields: &["last_change.actor.email", "last_change.actor.email_address"],
    },
    Attribution {
        collection: "oauth_states",
        person_field: "history_context.actor.person_id",
        name_fields: &[
            "history_context.actor.name",
            "history_context.actor.display_name",
        ],
        email_fields: &[
            "history_context.actor.email",
            "history_context.actor.email_address",
        ],
    },
    Attribution {
        collection: "telegram_bot_requests",
        person_field: "actor_user_id",
        name_fields: &["destination"],
        email_fields: &[],
    },
    Attribution {
        collection: "assistant_group_messages",
        person_field: "author_user_id",
        name_fields: &["author_display_name"],
        email_fields: &[],
    },
];

pub fn attribution(collection: &str, field: &str) -> Option<&'static Attribution> {
    ATTRIBUTIONS
        .iter()
        .find(|spec| spec.collection == collection && spec.person_field == field)
}

/// The authoritative parent/child relationships used by the local erasure
/// pass and by its behavioral fixtures. Keeping the child key here means a
/// newly added joined collection cannot silently bypass the worker test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinedChildSpec {
    pub parent: &'static str,
    pub child: &'static str,
    pub child_parent_field: &'static str,
}

pub static JOINED_CHILD_SPECS: &[JoinedChildSpec] = &[
    JoinedChildSpec {
        parent: "mcp_sessions",
        child: "mcp_session_notifications",
        child_parent_field: "session_id",
    },
    JoinedChildSpec {
        parent: "assistant_agents",
        child: "assistant_agent_learning_rejections",
        child_parent_field: "agent_id",
    },
    JoinedChildSpec {
        parent: "channel_conversations",
        child: "channel_event_logs",
        child_parent_field: "conversation_id",
    },
    JoinedChildSpec {
        parent: "channel_conversations",
        child: "channel_send_claims",
        child_parent_field: "conversation_id",
    },
    JoinedChildSpec {
        parent: "platform_bot_installations",
        child: "platform_delivery_tests",
        child_parent_field: "installation_id",
    },
    JoinedChildSpec {
        parent: "platform_setups",
        child: "platform_setup_notices",
        child_parent_field: "setup_id",
    },
    JoinedChildSpec {
        parent: "platform_setups",
        child: "platform_setup_steps",
        child_parent_field: "setup_id",
    },
    JoinedChildSpec {
        parent: "user_services",
        child: "service_history_heads",
        child_parent_field: "_id",
    },
    JoinedChildSpec {
        parent: "service_pools",
        child: "service_pool_health_observations",
        child_parent_field: "pool_id",
    },
];

pub static ENTRIES: &[Entry] = &[
    Entry {
        collection: "client_error_reports",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("user_id"),
        why: "client_error_reports stores consented crash/error diagnostics linked to the reporting account by user_id (sanitized categories, versions and code locations; no message content); the local inventory erases them with the account, and crash_ttl otherwise expires them within 30 days (CRASH_REPORTS_RETENTION_DAYS, at most 30). Exports include diagnostic metadata without stack frames or native images.",
    },
    Entry {
        collection: "client_error_report_counters",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("hourly anonymous crash-ingestion counters"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "client_error_report_counters stores hourly aggregate ingestion counts keyed only by time bucket, platform and outcome (no account, device or network identifiers); the counters expire after 30 days.",
    },
    Entry {
        collection: "client_release_registry",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "client_release_registry stores shared app-release identities used to interpret crash reports; registered_by keeps only the opaque id of the administrator who registered a release (also in the audit log), retained as shared attribution without names or emails.",
    },
    Entry {
        collection: "agent_key_login_requests",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("approved_user_id"),
        why: "agent_key_login_requests stores the person-owned agent key login requests record keyed by approved_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "agent_service_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "agent_service_bindings stores the person-owned agent service bindings record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "anonymous_endpoint_usage",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "anonymous_endpoint_usage is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "api_key_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "api_key_credentials stores the person-owned api key credentials record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "api_keys stores the person-owned api keys record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "approval_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "approval_grants stores the person-owned approval grants record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "approval_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "approval_requests stores the person-owned approval requests record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_acknowledgements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_acknowledgements stores the person-owned assistant acknowledgements record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_action_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_action_receipts stores the person-owned assistant action receipts record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_consents",
        owner: OwnerSelector::Fields(&["actor_user_id", "owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("actor_user_id"),
        why: "assistant_agent_consents stores the person-owned assistant agent consents record keyed by actor_user_id, owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        export_selector: ExportSelector::None,
        why: "assistant_agent_credentials stores the person-owned assistant agent credentials record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "assistant_agent_learning stores the person-owned assistant agent learning record keyed by owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning_members",
        owner: OwnerSelector::Fields(&["member_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("member_user_id"),
        why: "assistant_agent_learning_members stores the person-owned assistant agent learning members record keyed by member_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning_proposals",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "assistant_agent_learning_proposals stores the person-owned assistant agent learning proposals record keyed by owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning_rejections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "assistant_agents",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "assistant_agent_learning_rejections is lifecycle data owned through assistant_agents; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "assistant_agent_learning_runs",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "assistant_agent_learning_runs stores the person-owned assistant agent learning runs record keyed by owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_learning_skill_roots",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "assistant_agent_learning_skill_roots stores the person-owned assistant agent learning skill roots record keyed by owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_private_notes",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("actor_user_id"),
        why: "assistant_agent_private_notes stores the person-owned assistant agent private notes record keyed by actor_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_shares",
        owner: OwnerSelector::Fields(&["owner_id", "subject_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::AnyPerson(&["owner_id", "subject_id"]),
        why: "assistant_agent_shares stores the person-owned assistant agent shares record keyed by owner_id, subject_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agent_sharing",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "assistant_agent_sharing stores the person-owned assistant agent sharing record keyed by owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_agents stores the person-owned assistant agents record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_analytics_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "assistant_analytics_settings is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_artifact_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_artifact_deliveries stores the person-owned assistant artifact deliveries record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_async_operations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_async_operations stores the person-owned assistant async operations record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_async_quotas",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_async_quotas stores the person-owned assistant async quotas record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_attachment_expiry",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_attachment_expiry stores the person-owned assistant attachment expiry record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_attachments",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_attachments stores the person-owned assistant attachments record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_authored_draft_claims",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "assistant_authored_draft_claims stores the person-owned assistant authored draft claims record keyed by owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_checkpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_checkpoints stores the person-owned assistant checkpoints record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_cma_agents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_cma_agents stores the person-owned assistant cma agents record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_cma_creations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_cma_creations stores the person-owned assistant cma creations record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_cma_slot_audit_claims",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::FeatureRetention("assistant_cma_slot_audit one-hour expires_at TTL"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "assistant_cma_slot_audit_claims stores only the owner/reservation hourly diagnostic claim; the one-hour expires_at TTL removes it independently of account lifecycle and no request or credential is retained.",
    },
    Entry {
        collection: "assistant_cma_sandboxes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_cma_sandboxes stores the person-owned assistant cma sandboxes record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_cma_session_origins",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_cma_session_origins stores the person-owned assistant cma session origins record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_conversations stores the person-owned assistant conversations record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_direct_handoffs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_direct_handoffs stores the person-owned assistant direct handoffs record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_agent_access",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "assistant_group_agent_access stores the person-owned assistant group agent access record keyed by owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_agent_proposals",
        owner: OwnerSelector::Fields(&["proposed_by"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("proposed_by"),
        why: "assistant_group_agent_proposals stores the person-owned assistant group agent proposals record keyed by proposed_by; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_execution_slots",
        owner: OwnerSelector::Fields(&["original_requester_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("original_requester_id"),
        why: "assistant_group_execution_slots stores the person-owned assistant group execution slots record keyed by original_requester_id; bounded local cleanup removes the row after the erasure admission fence.",
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
        export_selector: ExportSelector::AnyPerson(&[
            "issuer_user_id",
            "accepted_by_user_id",
            "intended_user_id",
        ]),
        why: "assistant_group_invites stores the person-owned assistant group invites record keyed by issuer_user_id, accepted_by_user_id, intended_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_mention_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_group_mention_notifications stores the person-owned assistant group mention notifications record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_messages",
        owner: OwnerSelector::Fields(&["user_id", "author_user_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::AuthoredOwnedGroup,
        why: "assistant_group_messages removes person-owned transcripts through the local inventory and redacts human author labels on surviving shared transcripts while keeping opaque author ids.",
    },
    Entry {
        collection: "assistant_group_read_states",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_group_read_states stores the person-owned assistant group read states record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("actor_user_id"),
        why: "assistant_group_requests stores the person-owned assistant group requests record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_group_transfers",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "assistant_group_transfers is retained as SharedAttribution evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "assistant_groups",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_groups stores the person-owned assistant groups record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_invocation_turns",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_invocation_turns stores the person-owned assistant invocation turns record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_invocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_invocations stores the person-owned assistant invocations record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_learning_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("global package-format migration marker"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "assistant_learning_migrations: backfill_package_format writes a global publication-package-format marker and completed_at; no owner_id or personal content is stored.",
    },
    Entry {
        collection: "assistant_learning_publication_targets",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain(ErasureDomain::Learning),
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "assistant_learning_publication_targets is cleaned by the assistant_learning_reconciliation lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "assistant_message_feedback",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_message_feedback stores the person-owned assistant message feedback record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_messages stores the person-owned assistant messages record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "assistant_migrations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_notification_push_budgets",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("_id"),
        why: "assistant_notification_push_budgets stores the person-owned assistant notification push budgets record keyed by _id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_notification_push_sources",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_notification_push_sources stores the person-owned assistant notification push sources record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_notification_worker",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "assistant_notification_worker is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_notifications stores the person-owned assistant notifications record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_onboarding",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("_id"),
        why: "assistant_onboarding stores the person-owned assistant onboarding record keyed by _id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_profile_routes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "assistant_profile_routes is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_resources",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_resources stores the person-owned assistant resources record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_service_execution",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_service_execution stores the person-owned assistant service execution record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_settings",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("_id"),
        why: "assistant_settings stores the person-owned assistant settings record keyed by _id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_skill_draft_attempts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_skill_draft_attempts stores the person-owned assistant skill draft attempts record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_tool_results",
        owner: OwnerSelector::Fields(&["user_id", "actor_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("actor_id"),
        why: "assistant_tool_results stores the person-owned assistant tool results record keyed by user_id, actor_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_upload_limits",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "assistant_upload_limits is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "assistant_voice_announcements",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_voice_announcements stores the person-owned assistant voice announcements record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_voice_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_voice_requests stores the person-owned assistant voice requests record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_voice_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Voice),
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_voice_sessions stores personal voice session metadata; voice purge settles and removes it.",
    },
    Entry {
        collection: "assistant_voice_windows",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_voice_windows stores the person-owned assistant voice windows record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "assistant_wire_logs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("user_id"),
        why: "assistant_wire_logs stores the person-owned assistant wire logs record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "async_service_operation_artifacts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "async_service_operation_artifacts stores the person-owned async service operation artifacts record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "audit_log",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::Audit),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "audit_log is retained as Audit evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "auth_device_code_reservations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("transient device-code coordination with TTL"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "auth_device_code_reservations stores only a one-way code uniqueness HMAC for transient coordination; its pending and terminal rows expire under the device-code TTL and contain no person owner field.",
    },
    Entry {
        collection: "auth_device_codes",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("approved_user_id"),
        why: "auth_device_codes stores the person-owned auth device codes record keyed by approved_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "auth_device_codes_v2",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("approved_user_id"),
        why: "auth_device_codes_v2 stores the person-owned auth device codes v2 record keyed by approved_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "authorization_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "authorization_codes stores the person-owned authorization codes record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "billing_account_balances",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "billing_account_balances is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_account_verify_lease",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_account_verify_lease is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_account_verify_state",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_account_verify_state is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_cutover_operations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_cutover_operations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_hold_observations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "billing_hold_observations links wallet, grant, and allowance-period ids to retained financial and billing records; reconciliation removes resolved observations, and personal erasure must preserve outstanding hold-verification evidence.",
    },
    Entry {
        collection: "billing_lago_carry",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "billing_lago_carry is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_ledger",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "billing_ledger is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_migration_errors",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_migration_errors is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_migrations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_plan_charges",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal(
            "platform plan and metric pricing intents without person identifiers",
        ),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_plan_charges stores one desired or removed platform price per plan/metric, optional catalog service id, and repair backoff; it has no person, wallet, grant, period, or actor id.",
    },
    Entry {
        collection: "billing_plan_sync",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal(
            "platform plan synchronization generations without person identifiers",
        ),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_plan_sync is keyed by platform plan code and records synchronization generations, dirty/uncertain flags, and an opaque provider-attempt UUID, with no person or financial-owner id.",
    },
    Entry {
        collection: "billing_pool_recovery",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "billing_pool_recovery is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_rate_cache",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "billing_rate_cache is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_rate_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_rate_diagnostics is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "billing_rate_epoch",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("singleton platform rate-cache ordering generation"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_rate_epoch stores only the rate-cache singleton id and its generation counter to order platform price publication; neither field identifies a person or financial owner.",
    },
    Entry {
        collection: "billing_stuck_request_recovery",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("singleton billing recovery coordination"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "billing_stuck_request_recovery stores the fixed ordinary singleton id, cursor_0/cursor_1/cursor_2 as opaque usage-meter row ids, next_source as a rotating source index, aggregate ended_requests/ended_rows counters, and last_ended_at; it has no person, owner, wallet, or request identifiers and no financial amounts.",
    },
    Entry {
        collection: "billing_tax_profiles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "billing_tax_profiles is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_tax_plan_epoch",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("singleton platform tax-plan ordering generation"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_tax_plan_epoch stores the all-plans singleton id, generation, and random publication guard token; it contains no tax profile, person, financial-owner, or actor id.",
    },
    Entry {
        collection: "billing_topup_sessions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "billing_topup_sessions is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_usage_comparison_control",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("singleton usage-comparison coverage and schedule state"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: concat!(
            "billing_usage_comparison_control stores two fixed-id singletons for provider usage ",
            "comparison: coverage readiness (generation, ready flag and timestamps) and the ",
            "schedule's read budget and opaque billing-wallet scan cursor; it identifies no person.",
        ),
    },
    Entry {
        collection: "billing_usage_comparisons",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: concat!(
            "billing_usage_comparisons stores per-owner billing-provider usage-capture coverage ",
            "metadata (wallet, customer and subscription identifiers, acknowledgement sequence ",
            "and coverage flags) and per-metric usage projections and comparisons with the ",
            "provider's recorded usage (local and provider totals and their differences); ",
            "retained as FinancialBilling reconciliation evidence.",
        ),
    },
    Entry {
        collection: "billing_usage_deliveries",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: concat!(
            "billing_usage_deliveries stores per-transaction usage delivery receipts sent to the ",
            "billing provider (owner, wallet, customer and subscription identifiers, metric code, ",
            "quantity and acknowledgement state); retained as FinancialBilling settlement ",
            "evidence until its expires_at TTL; the same usage is exported through usage_meter.",
        ),
    },
    Entry {
        collection: "billing_wallet",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "billing_wallet is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "billing_wallet_refresh_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "billing_wallet_refresh_requests is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "catalog_delegation_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "catalog_delegation_grants stores the person-owned catalog delegation grants record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "catalog_icons",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "catalog_icons is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "catalog_skill_operations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "catalog_skill_operations stores shared skill publication operations; retention keeps opaque attribution while redacting human names; deletion retains this record for its documented lifecycle.",
    },
    Entry {
        collection: "catalog_skill_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "catalog_skill_revisions stores shared published skill revision history; retention keeps opaque attribution while redacting human names; deletion retains this record for its documented lifecycle.",
    },
    Entry {
        collection: "chain_verify_status",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "chain_verify_status is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "channel_activity_notifications",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_activity_notifications stores the person-owned channel activity notifications record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_accounts",
        owner: OwnerSelector::Fields(&["configured_by"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "channel_adapter_accounts: call_channels::management::configure_account writes shared platform provider trust configuration keyed by provider/account_id; configured_by is opaque administrator attribution, not personal resource ownership. Preserve the trusted account used by other bindings.",
    },
    Entry {
        collection: "channel_adapter_allocations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_adapter_allocations stores the person-owned channel adapter allocations record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_adapter_bindings stores the person-owned channel adapter bindings record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_adapter_events stores the person-owned channel adapter events record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_resource_claims",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_adapter_resource_claims stores the person-owned channel adapter resource claims record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_adapter_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_adapter_sessions stores the person-owned channel adapter sessions record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_bots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_bots stores the person-owned channel bots record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_connect_links stores the person-owned channel connect links record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_conversations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Channel),
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_conversations is cleaned by the channel lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "channel_delivery_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_delivery_receipts stores the person-owned channel delivery receipts record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_email_batches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_email_batches stores the person-owned channel email batches record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_email_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_email_receipts stores the person-owned channel email receipts record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_email_sends",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_email_sends stores the person-owned channel email sends record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_email_subscriptions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_email_subscriptions stores the person-owned channel email subscriptions record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_event_logs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "channel_conversations",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "channel_event_logs is lifecycle data owned through channel_conversations; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "channel_messages",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_messages stores the person-owned channel messages record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_reply_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_reply_deliveries stores the person-owned channel reply deliveries record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_send_claims",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "channel_conversations",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "channel_send_claims is lifecycle data owned through channel_conversations; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "channel_text_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "channel_text_deliveries stores the person-owned channel text deliveries record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "channel_webhook_rejections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "channel_webhook_rejections is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "channel_whatsapp_native",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "channel_whatsapp_native stores the person-owned channel whatsapp native record keyed by owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "cli_pairings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "cli_pairings stores the person-owned cli pairings record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "connect_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "connect_links stores the person-owned connect links record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "consent_revocation_fences",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "consent_revocation_fences: consent_service writes per-person revocation fences; local::inventory deletes them after credential revocation has completed.",
    },
    Entry {
        collection: "consents",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "consents stores the person-owned consents record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "coordination_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "coordination_leases is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "coordination_rate_windows",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "coordination_rate_windows is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "coordination_replay_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "coordination_replay_records is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "coordination_slots",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "coordination_slots is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "coordination_token_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "coordination_token_buckets is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "credit_allocation_requests",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "credit_allocation_requests is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "credit_grants",
        owner: OwnerSelector::Fields(&["recipient_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        export_selector: ExportSelector::Person("recipient_user_id"),
        why: "credit_grants is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "credit_schedule_periods",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "credit_schedule_periods stores frozen scheduled-grant policy and recipient snapshots; financial retention preserves other recipients; deletion retains this record for its documented lifecycle.",
    },
    Entry {
        collection: "credit_schedule_recipient_scans",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared schedule discovery progress"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "credit_schedule_recipient_scans holds one disposable cursor per shared schedule, not a person's resource. cursor_user_id is only an opaque ordering boundary, is replaced as discovery advances and resets at the end of each pass; keeping it after account erasure neither restores the account nor grants credits. It contains no personal content or financial entitlement and is excluded from account export.",
    },
    Entry {
        collection: "credit_schedule_recipients",
        owner: OwnerSelector::Fields(&["recipient_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        export_selector: ExportSelector::Person("recipient_user_id"),
        why: "credit_schedule_recipients retains the opaque recipient identity and immutable renewal anchor alongside grants and their journal history. Deleting an anchor could change financial identity and permit duplicate issuance; account erasure preserves this financial evidence, while live eligibility prevents issuance to deleted people. Export exposes only the requesting recipient's own cycle metadata.",
    },
    Entry {
        collection: "credit_schedules",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "credit_schedules is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "device_codes",
        owner: OwnerSelector::Fields(&["approved_by_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        export_selector: ExportSelector::Person("approved_by_user_id"),
        why: "device_codes stores the person-owned device codes record keyed by approved_by_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "device_onboard_credentials",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("owner_user_id"),
        why: "device_onboard_credentials stores the person-owned device onboard credentials record keyed by owner_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "device_pubkey_lockouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "device_pubkey_lockouts is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "downstream_services",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "downstream_services is retained as SharedAttribution evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "durable_operation_executions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::LocalEvidence),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "durable_operation_executions is compacted by local::retain_settled_evidence after terminal execution; the original row is deleted and only opaque user_id, row identity and settlement reason remain in personal_erasure_local_receipts.",
    },
    Entry {
        collection: "durable_operation_grants",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::LocalEvidence),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "durable_operation_grants is compacted by local::retain_settled_evidence after terminal execution; the original row is deleted and only opaque user_id, row identity and settlement reason remain in personal_erasure_local_receipts.",
    },
    Entry {
        collection: "encrypted_record_migration_findings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "encrypted_record_migration_findings: Encryption migration findings retain keyed diagnostic identifiers and verification metadata for unresolved records. lookup_finding returns a 15-minute encrypted lookup cursor to the operator, never persisting it; owner_uuid is computed only during lookup, never persisted. Repair evidence is restricted bookkeeping.",
    },
    Entry {
        collection: "encrypted_record_migration_runs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("global encryption migration progress"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "encrypted_record_migration_runs: Encryption migration runs store global generation, counters and lease progress. owner_uuid is not a persisted run field.",
    },
    Entry {
        collection: "event_dedup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "event_dedup_records is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "execution_enrollment_jobs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "execution_enrollment_jobs is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "execution_enrollment_repairs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "execution_enrollment_repairs is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "execution_permission_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "execution_permission_deliveries stores the person-owned execution permission deliveries record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "feature_flag_definitions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "feature_flag_definitions is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "feature_flag_metadata",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "feature_flag_metadata is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "feature_flag_overrides",
        owner: OwnerSelector::Fields(&["target_key"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("target_key"),
        why: "feature_flag_overrides stores the person-owned feature flag overrides record keyed by target_key; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "generated_artifact_content_series",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "generated_artifact_content_series stores the person-owned generated artifact content series record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "groups",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "groups is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "invite_codes",
        owner: OwnerSelector::Fields(&["usages.user_id", "created_by"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("usages.user_id"),
        why: "invite_codes: Shared invitation codes preserve opaque creator and usage IDs and capacity counters. Erasure strips the deleted creator's free-text note and their usage email; export contains only the caller's own usage, never creator notes or other usages.",
    },
    Entry {
        collection: "invocation_diagnostics",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("actor_user_id"),
        why: "invocation_diagnostics stores the person-owned invocation diagnostics record keyed by actor_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "lark_base_write_photos",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "lark_base_write_photos stores the person-owned lark base write photos record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "lark_base_writes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "lark_base_writes stores the person-owned lark base writes record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "login_approvals",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        export_selector: ExportSelector::Person("user_id"),
        why: "login_approvals stores the person-owned login approvals record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "login_codes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        export_selector: ExportSelector::Person("user_id"),
        why: "login_codes stores the person-owned login codes record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "machine_authority_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "machine_authority_leases retains signed Machine authority, execution, cleanup or investigation evidence under the existing Machine lifecycle; personal erasure does not assert its destruction.",
    },
    Entry {
        collection: "machine_confirmations",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Section("machines"),
        export_selector: ExportSelector::Person("user_id"),
        why: "machine_confirmations retains signed Machine authority, execution, cleanup or investigation evidence under the existing Machine lifecycle; personal erasure does not assert its destruction.",
    },
    Entry {
        collection: "machine_contexts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "machine_contexts retains signed Machine authority, execution, cleanup or investigation evidence under the existing Machine lifecycle; personal erasure does not assert its destruction.",
    },
    Entry {
        collection: "machine_desktops",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "machine_desktops: Person-owned Machine session, setup, update or consent metadata is keyed by user_id and removed by the explicit local inventory after admission closes.",
    },
    Entry {
        collection: "machine_effect_outbox",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "machine_effect_outbox is retained as MachineLifecycle evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_external_grants",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "machine_external_grants retains signed Machine authority, execution, cleanup or investigation evidence under the existing Machine lifecycle; personal erasure does not assert its destruction.",
    },
    Entry {
        collection: "machine_free_slots",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Section("machines"),
        export_selector: ExportSelector::Person("_id"),
        why: "machine_free_slots retains signed Machine authority, execution, cleanup or investigation evidence under the existing Machine lifecycle; personal erasure does not assert its destruction.",
    },
    Entry {
        collection: "machine_jobs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Section("machines"),
        export_selector: ExportSelector::Person("user_id"),
        why: "machine_jobs retains signed Machine authority, execution, cleanup or investigation evidence under the existing Machine lifecycle; personal erasure does not assert its destruction.",
    },
    Entry {
        collection: "machine_lifecycle_events",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "machine_lifecycle_events is retained as MachineLifecycle evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_oauth_consent_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "machine_oauth_consent_requests: Person-owned Machine session, setup, update or consent metadata is keyed by user_id and removed by the explicit local inventory after admission closes.",
    },
    Entry {
        collection: "machine_operations",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "machine_operations is retained as MachineLifecycle evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_orphan_inventory",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Section("machines"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "machine_orphan_inventory retains signed Machine authority, execution, cleanup or investigation evidence under the existing Machine lifecycle; personal erasure does not assert its destruction.",
    },
    Entry {
        collection: "machine_revocations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "machine_revocations retains signed Machine authority, execution, cleanup or investigation evidence under the existing Machine lifecycle; personal erasure does not assert its destruction.",
    },
    Entry {
        collection: "machine_runtime_heads",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "machine_runtime_heads is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_runtime_intervals",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "machine_runtime_intervals is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_runtime_quotes",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "machine_runtime_quotes is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_runtime_rates",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "machine_runtime_rates is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_runtime_runs",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "machine_runtime_runs is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "machine_separation_compatibility",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "machine_separation_compatibility retains signed Machine authority, execution, cleanup or investigation evidence under the existing Machine lifecycle; personal erasure does not assert its destruction.",
    },
    Entry {
        collection: "machine_setups",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "machine_setups: Person-owned Machine session, setup, update or consent metadata is keyed by user_id and removed by the explicit local inventory after admission closes.",
    },
    Entry {
        collection: "machine_update_attempts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "machine_update_attempts: Person-owned Machine session, setup, update or consent metadata is keyed by user_id and removed by the explicit local inventory after admission closes.",
    },
    Entry {
        collection: "machine_updates",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "machine_updates: Person-owned Machine session, setup, update or consent metadata is keyed by user_id and removed by the explicit local inventory after admission closes.",
    },
    Entry {
        collection: "managed_executions",
        owner: OwnerSelector::Fields(&["billing_owner_id", "actor_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "managed_executions is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "managed_machine_bootstrap_deliveries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "managed_machine_bootstrap_deliveries is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "managed_machines",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::MachineLifecycle),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "managed_machines is retained as MachineLifecycle evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "mcp_session_notifications",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "mcp_sessions",
        },
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::JoinedPerson {
            parent: "mcp_sessions",
            field: "session_id",
            owner_field: "user_id",
        },
        why: "mcp_session_notifications: McpSessionNotification is keyed by session_id, not a person ID; notifications are removed before their person-owned session. The session bearer is never exported.",
    },
    Entry {
        collection: "mcp_session_slots",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "mcp_session_slots stores the person-owned mcp session slots record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "mcp_sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        export_selector: ExportSelector::None,
        why: "mcp_sessions stores the person-owned mcp sessions record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "mobile_social_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        export_selector: ExportSelector::None,
        why: "mobile_social_deliveries holds a short-lived encrypted sign-in response. Account erasure deletes it by user_id.",
    },
    Entry {
        collection: "mobile_social_flows",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("pre-authentication flow with TTL"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "mobile_social_flows binds an anonymous, expiring social login attempt before the provider identifies a person.",
    },
    Entry {
        collection: "mfa_challenges",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "mfa_challenges stores the person-owned mfa challenges record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "mfa_factors",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        export_selector: ExportSelector::Person("user_id"),
        why: "mfa_factors stores the person-owned mfa factors record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "node_credential_desired",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Domain(ErasureDomain::NodeDrain),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "node_credential_desired is metadata-only delivery intent: main's local::pass drains it by node_id and final_metadata deletes owner_id; org-owned rows retain only opaque actor_user_id, and the transient queue is excluded from export.",
    },
    Entry {
        collection: "node_pending_credentials",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Nodes),
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        export_selector: ExportSelector::None,
        why: "node_pending_credentials stores pending credential injection metadata and encrypted payloads; the entire transient row is excluded because its crypto bundle is secret material.",
    },
    Entry {
        collection: "node_registration_tokens",
        owner: OwnerSelector::Fields(&["user_id", "issuer_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "node_registration_tokens stores the person-owned node registration tokens record keyed by user_id, issuer_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "node_service_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Nodes),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "node_service_bindings is cleaned by the nodes lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "nodes",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Nodes),
        export: Export::Section("machines"),
        export_selector: ExportSelector::Person("user_id"),
        why: "nodes is cleaned by the nodes lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "notification_channels",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        export_selector: ExportSelector::Person("user_id"),
        why: "notification_channels stores the person-owned notification channels record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_channels",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::ChannelBots),
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "nyxbot_channels is cleaned by the channel_bots lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "nyxbot_event_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "nyxbot_event_bindings stores the person-owned nyxbot event bindings record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_events",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "nyxbot_events stores the person-owned nyxbot events record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_gateway_releases",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "nyxbot_gateway_releases stores Discord gateway drain guards required to block unsafe reconnects; deletion retains this record for its documented lifecycle.",
    },
    Entry {
        collection: "nyxbot_gateway_responses",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "nyxbot_gateway_responses stores the person-owned nyxbot gateway responses record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_image_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "nyxbot_image_deliveries stores the person-owned nyxbot image deliveries record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_image_deliveries_v2",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "nyxbot_image_deliveries_v2 stores the person-owned nyxbot image deliveries v2 record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_threads",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "nyxbot_threads stores the person-owned nyxbot threads record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "nyxbot_watches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("user_id"),
        why: "nyxbot_watches stores the person-owned nyxbot watches record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "oauth_broker_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "oauth_broker_bindings stores the person-owned oauth broker bindings record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "oauth_clients",
        owner: OwnerSelector::Fields(&["created_by"]),
        erasure: Erasure::Domain(ErasureDomain::ServiceAccounts),
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("created_by"),
        why: "oauth_clients stores person-created application metadata; redirect URIs and scopes are exported without secrets.",
    },
    Entry {
        collection: "oauth_consent_requests",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "oauth_consent_requests stores the person-owned oauth consent requests record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "oauth_delegation_authorities",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "oauth_delegation_authorities stores the person-owned oauth delegation authorities record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "oauth_states",
        owner: OwnerSelector::Fields(&["user_id", "history_context.actor.person_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "oauth_states removes person-owned protocol rows through the local inventory and redacts saved history actor labels on surviving flows owned by another account.",
    },
    Entry {
        collection: "openclaw_channel_mappings",
        owner: OwnerSelector::Fields(&["nyxid_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("nyxid_user_id"),
        why: "openclaw_channel_mappings: OpenClawChannelMapping stores the NyxID owner in nyxid_user_id; local erasure uses that exact field.",
    },
    Entry {
        collection: "oracle_login_profiles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain(ErasureDomain::Oracle),
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        export_selector: ExportSelector::None,
        why: "oracle_login_profiles stores an enrolled browser login profile, encrypted session material, and pool bindings; Oracle pool cleanup removes one profile per bounded cleanup chunk after the owner's pool is retired.",
    },
    Entry {
        collection: "oracle_login_snapshots",
        owner: OwnerSelector::Fields(&["created_by_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "oracle_login_snapshots: This is a shared Oracle pool resource whose created_by_user_id is opaque administrator provenance, not resource ownership. The pool's existing lifecycle and expiry govern removal; personal erasure preserves other owners' shared pool data.",
    },
    Entry {
        collection: "oracle_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Oracle),
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "oracle_pools is cleaned by the oracle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "oracle_sessions",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("owner_user_id"),
        why: "oracle_sessions stores a person's own Oracle conversation routing and lifecycle metadata keyed by owner_user_id; personal erasure removes only that person's rows while Oracle pool cleanup retains its existing pool semantics.",
    },
    Entry {
        collection: "oracle_tasks",
        owner: OwnerSelector::Fields(&["submitter_user_id"]),
        erasure: Erasure::FeatureRetention("ORACLE_TASK_RETENTION_DAYS / expires_at"),
        export: Export::Section("assistant"),
        export_selector: ExportSelector::Person("submitter_user_id"),
        why: "oracle_tasks stores submitted prompts and worker responses; terminal rows expire at the configured ORACLE_TASK_RETENTION_DAYS TTL.",
    },
    Entry {
        collection: "oracle_worker_commands",
        owner: OwnerSelector::Fields(&["created_by_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "oracle_worker_commands: This is a shared Oracle pool resource whose created_by_user_id is opaque administrator provenance, not resource ownership. The pool's existing lifecycle and expiry govern removal; personal erasure preserves other owners' shared pool data.",
    },
    Entry {
        collection: "oracle_workers",
        owner: OwnerSelector::Fields(&["enrollment.owner_user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Oracle),
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("enrollment.owner_user_id"),
        why: "oracle_workers stores enrolled browser-installation status and diagnostics; the Oracle lifecycle retires and removes the owner enrollment.",
    },
    Entry {
        collection: "org_channel_guest_budgets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "org_channel_guest_budgets stores organization-channel financial budget accounting; deletion retains this record for its documented lifecycle.",
    },
    Entry {
        collection: "org_channel_guest_holds",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "org_channel_guest_holds stores organization-channel financial reservation holds; deletion retains this record for its documented lifecycle.",
    },
    Entry {
        collection: "org_channel_lanes",
        owner: OwnerSelector::Fields(&["actor_id"]),
        erasure: Erasure::Domain(ErasureDomain::OrgChannel),
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("actor_id"),
        why: "org_channel_lanes stores a person’s organization-channel transcript lane; lifecycle erasure deletes the lane.",
    },
    Entry {
        collection: "org_channel_link_challenges",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::OrgChannel),
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("user_id"),
        why: "org_channel_link_challenges stores per-person organization-channel setup requests and short-lived codes.",
    },
    Entry {
        collection: "org_channel_messages",
        owner: OwnerSelector::Fields(&["author_user_id", "actor_id"]),
        erasure: Erasure::Domain(ErasureDomain::OrgChannel),
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("author_user_id"),
        why: "org_channel_messages stores organization-channel transcript text and actor attribution; lifecycle erasure deletes rows authored or initiated by the departing person.",
    },
    Entry {
        collection: "org_channel_sender_links",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::OrgChannel),
        export: Export::Section("channels"),
        export_selector: ExportSelector::Person("user_id"),
        why: "org_channel_sender_links stores a person’s organization-channel sender linkage; lifecycle erasure revokes the link and keeps only its tombstone.",
    },
    Entry {
        collection: "org_invites",
        owner: OwnerSelector::Fields(&["created_by"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("redeemed_by"),
        why: "org_invites stores the person-owned org invites record keyed by created_by; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "org_memberships",
        owner: OwnerSelector::Fields(&["member_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("member_user_id"),
        why: "org_memberships stores the person-owned org memberships record keyed by member_user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "org_role_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "org_role_scopes is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "ownership_transfers",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "ownership_transfers stores shared ownership-transfer history; retention keeps opaque account ids while redacting human names; deletion retains this record for its documented lifecycle.",
    },
    Entry {
        collection: "permission_key_policies",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "permission_key_policies stores the person-owned permission key policies record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "personal_erasure_effects",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "personal_erasure_effects: Provider revocation effects retain opaque completion evidence after capability removal. local::inventory compacts only completed oauth_issuance receipts with credential_installed/protocol_no_issuance and no capability or completion_basis; all durable effect evidence remains.",
    },
    Entry {
        collection: "personal_erasure_jobs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "personal_erasure_jobs is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "personal_erasure_orphan_sweep_fences",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("ephemeral exclusive apply holder keyed by reviewed digest"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "personal_erasure_orphan_sweep_fences stores only a reviewed digest and apply job id; admission inserts the fence and terminal completion or failure removes it transactionally, without person identifiers or document content.",
    },
    Entry {
        collection: "personal_erasure_orphan_sweep_jobs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "personal_erasure_orphan_sweep_jobs: Restartable orphan-sweep checkpoints, reviewed candidate identifiers and reports are retained as erasure accountability evidence; no document content is copied.",
    },
    Entry {
        collection: "personal_erasure_orphan_sweep_pages",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "personal_erasure_orphan_sweep_pages: Restartable orphan-sweep checkpoints, reviewed candidate identifiers and reports are retained as erasure accountability evidence; no document content is copied.",
    },
    Entry {
        collection: "personal_erasure_local_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "personal_erasure_local_receipts is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "personal_erasure_preparations",
        owner: OwnerSelector::Fields(&["_id", "user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "personal_erasure_preparations stores the person-owned personal erasure preparations record keyed by _id, user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "personal_erasure_revocation_receipts",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Source),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "personal_erasure_revocation_receipts: Provider revocation receipts survive until source cleanup; personal_erasure_registry::final_metadata removes user_id rows transactionally before account completion.",
    },
    Entry {
        collection: "personal_owner_write_fences",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "personal_owner_write_fences is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "platform_bot_cleanup",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "platform_bot_cleanup: PlatformBotCleanup retains only exact remote-cleanup identifiers and completion evidence. It is restricted erasure bookkeeping, not a deleted person-owned payload; pending effects must complete before local parents are removed.",
    },
    Entry {
        collection: "platform_bot_configs",
        owner: OwnerSelector::Fields(&["configured_by", "funding_owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "platform_bot_configs: PlatformBotConfig is shared administrative configuration. configured_by is an opaque actor ID and funding_owner_user_id is the sponsoring organization; neither makes the config a personal resource.",
    },
    Entry {
        collection: "platform_bot_installations",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Platform),
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("owner_user_id"),
        why: "platform_bot_installations stores a person-owned platform installation and its approved child state.",
    },
    Entry {
        collection: "platform_bot_runtime_controls",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_bot_runtime_controls: Shared runtime revisions, retirement and emergency-stop fences survive as access-restriction bookkeeping; no private person-authored content is stored.",
    },
    Entry {
        collection: "platform_credentials",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_credentials is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_delivery_tests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "platform_bot_installations",
        },
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::JoinedPerson {
            parent: "platform_bot_installations",
            field: "installation_id",
            owner_field: "owner_user_id",
        },
        why: "platform_delivery_tests stores short-lived delivery-test state joined to a person-owned platform installation.",
    },
    Entry {
        collection: "platform_discord_control_installation_history",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "platform_discord_control_installation_history: Discord command installation history retains opaque requested_by provenance for shared administrative attempts, not personal command content.",
    },
    Entry {
        collection: "platform_discord_control_installations",
        owner: OwnerSelector::Fields(&["requested_by"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "platform_discord_control_installations: DiscordControlInstallation retains requested_by as an opaque administrator ID on a shared command-installation attempt; no person name, email or message content is stored.",
    },
    Entry {
        collection: "platform_entry_gateway_partitions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_entry_gateway_partitions is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_entry_gateway_transports",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_entry_gateway_transports is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_entry_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_entry_migrations is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_guide_selections",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::FeatureRetention("platform_binding_service purge_after TTL"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "platform_guide_selections: guide_select_current stores binding_id, generation, setup_id and purge_after. It has no NyxID user_id; the selection is joined to a personal onboarding binding and its existing purge_after TTL.",
    },
    Entry {
        collection: "platform_help_assignments",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_help_assignments is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_help_libraries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_help_libraries is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_help_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_help_revisions is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_manager_deliveries",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "platform_manager_deliveries: PlatformManagerDelivery stores external opaque sender/message IDs, delivery state and references, never reply bodies. Non-TTL delivery recovery and no-replay evidence outlives account authority; external sender IDs are personal metadata, not NyxID owner IDs.",
    },
    Entry {
        collection: "platform_manager_ingress_receipts",
        owner: OwnerSelector::Fields(&["audience.user_id", "execution.payer_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "platform_manager_ingress_receipts: PlatformManagerIngress contains opaque sender, execution payer and recovery identities. Non-TTL admission receipts fence replay and pending delivery; they contain no message body and survive as restricted access/recovery bookkeeping.",
    },
    Entry {
        collection: "platform_model_attempts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_model_attempts is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_model_routing_revisions",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_model_routing_revisions is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_model_tariffs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_model_tariffs is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_models",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_models is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_native_entry_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::FeatureRetention("platform_native_receipt_service purge_after TTL"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "platform_native_entry_receipts: External platform sender/event metadata is personal even without a NyxID person ID. platform_native_receipt_service::ensure_indexes applies the existing purge_after TTL to these keyed admission receipts.",
    },
    Entry {
        collection: "platform_native_receipts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_native_receipts is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_native_replies",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::FeatureRetention("platform_binding_service purge_after TTL"),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "platform_native_replies: admit_reply stores an external platform sender and source_id, with a one-day purge_after TTL. No NyxID user_id is written; no message body is retained.",
    },
    Entry {
        collection: "platform_native_reply_handles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_native_reply_handles is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_onboarding_binding_requests",
        owner: OwnerSelector::Fields(&["approved_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("approved_user_id"),
        why: "platform_onboarding_binding_requests: Platform binding requests record approved_user_id; owner retirement cancels pending approval before bounded local removal.",
    },
    Entry {
        collection: "platform_onboarding_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("user_id"),
        why: "platform_onboarding_bindings stores the person-owned platform onboarding bindings record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "platform_onboarding_decision_spacing",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_onboarding_decision_spacing is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_personal_bindings",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "platform_personal_bindings: The platform domain first revokes active consent and captures required stop effects; local inventory removes terminal binding/link metadata once platform cleanup and active local-node dependencies are clear; retained stop captures do not need these rows.",
    },
    Entry {
        collection: "platform_personal_decision_spacing",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("_id"),
        why: "platform_personal_decision_spacing: The personal decision-spacing writer uses the actor account ID as _id, with last_decision_at; it has no user_id field.",
    },
    Entry {
        collection: "platform_personal_link_challenges",
        owner: OwnerSelector::Fields(&["reviewed_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("reviewed_user_id"),
        why: "platform_personal_link_challenges: The platform domain first revokes active consent and captures required stop effects; local inventory removes terminal binding/link metadata once platform cleanup and active local-node dependencies are clear; retained stop captures do not need these rows.",
    },
    Entry {
        collection: "platform_personal_link_intents",
        owner: OwnerSelector::Fields(&["claimant_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("claimant_user_id"),
        why: "platform_personal_link_intents: The platform domain first revokes active consent and captures required stop effects; local inventory removes terminal binding/link metadata once platform cleanup and active local-node dependencies are clear; retained stop captures do not need these rows.",
    },
    Entry {
        collection: "platform_personal_setup_selections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("user_id"),
        why: "platform_personal_setup_selections stores the person-owned platform personal setup selections record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "platform_personal_stop_effects",
        owner: OwnerSelector::Fields(&["active_turn.origin.user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "platform_personal_stop_effects: Exact stop captures and completion progress survive as restricted erasure bookkeeping; machine and delivery workers need these identifiers to settle captured effects.",
    },
    Entry {
        collection: "platform_public_budget_buckets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_public_budget_buckets is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_budget_loops",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_public_budget_loops is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_call_reservations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_public_call_reservations is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_cleanup_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_public_cleanup_records is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_deployment_activations",
        owner: OwnerSelector::Fields(&["activated_by"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "platform_public_deployment_activations: PublicDeploymentActivation retains the opaque activated_by administrator ID as provenance of a shared organization deployment, without names or emails.",
    },
    Entry {
        collection: "platform_public_deployment_proofs",
        owner: OwnerSelector::Fields(&["evidence.binding.sponsor_owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::SharedAttribution),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "platform_public_deployment_proofs: PublicDeploymentRecord has no actor_user_id; its proof binds an organization sponsor and contains deployment evidence, not a person-owned resource.",
    },
    Entry {
        collection: "platform_public_probe_consents",
        owner: OwnerSelector::Fields(&["actor_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("actor_user_id"),
        why: "platform_public_probe_consents: ProbeConsent is the human actor's short-lived readiness consent; local erasure removes that actor's consent without deleting organization deployment proofs.",
    },
    Entry {
        collection: "platform_public_quotas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_public_quotas is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_readiness_probes",
        owner: OwnerSelector::Fields(&["binding.sponsor_owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "platform_public_readiness_probes: PublicProbe binds metered readiness work, maximum credits and meter_row_ids to the sponsoring organization; preserve financial evidence and opaque IDs, not a fictitious actor_user_id selector.",
    },
    Entry {
        collection: "platform_public_readiness_records",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_public_readiness_records is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_revocation_scopes",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_public_revocation_scopes is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_public_sessions",
        owner: OwnerSelector::Fields(&["sponsor_owner_user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "platform_public_sessions: Public sessions bind external sender IDs to an organization sponsor. platform_public_agent_service revokes at the fixed 24-hour lifetime and clears encrypted bindings only after confirmed remote cleanup; the opaque revocation/recovery tombstone survives. They are not a person's private assistant history.",
    },
    Entry {
        collection: "platform_public_turns",
        owner: OwnerSelector::Fields(&["sponsor_owner_user_id"]),
        erasure: Erasure::FeatureRetention("platform_public_credential_service expires_at TTL"),
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        export_selector: ExportSelector::None,
        why: "platform_public_turns: Encrypted public input/reply transcripts belong to a public session and organization sponsor, not a NyxID person. platform_public_credential_service::ensure_indexes expires them at expires_at; session authority and cleanup proof remain outside the TTL collection.",
    },
    Entry {
        collection: "platform_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "platform_settings is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "platform_setup_notices",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "platform_setups",
        },
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::JoinedPerson {
            parent: "platform_setups",
            field: "setup_id",
            owner_field: "owner_user_id",
        },
        why: "platform_setup_notices stores short-lived setup notices joined to a person-owned setup.",
    },
    Entry {
        collection: "platform_setup_steps",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "platform_setups",
        },
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::JoinedPerson {
            parent: "platform_setups",
            field: "setup_id",
            owner_field: "owner_user_id",
        },
        why: "platform_setup_steps stores short-lived setup work joined to a person-owned setup.",
    },
    Entry {
        collection: "platform_setups",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::Domain(ErasureDomain::Platform),
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("owner_user_id"),
        why: "platform_setups is cleaned by the platform_or_channel_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "pool_recovery_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "pool_recovery_diagnostics is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "provider_configs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("platform-wide configuration or coordination"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "provider_configs is platform-wide configuration or coordination and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "pushed_authorization_requests",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "pushed_authorization_requests is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "refresh_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        export_selector: ExportSelector::Person("user_id"),
        why: "refresh_tokens stores the person-owned refresh tokens record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "reply_token_uses",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "reply_token_uses is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "roles",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "roles is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "sandbox_billing_cursors",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_billing_cursors is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "sandbox_billing_evidence",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_billing_evidence is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "sandbox_billing_quotes",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_billing_quotes is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "sandbox_billing_settings",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "sandbox_billing_settings is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "sandbox_billing_windows",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_billing_windows is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "sandbox_rate_cards",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "sandbox_rate_cards is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "sandbox_usage_producers_v1",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared producer authorization"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_producers_v1 — Sandbox usage producer registrations govern machine custody for the namespace, independently of the owners whose evidence it carries.",
    },
    Entry {
        collection: "sandbox_usage_events_v1",
        owner: OwnerSelector::Fields(&["owner_subject"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_events_v1 — Retain the exact accepted usage evidence and receipt with opaque owner attribution so replay and corrections survive account deletion.",
    },
    Entry {
        collection: "sandbox_usage_streams_v1",
        owner: OwnerSelector::Fields(&["owner_subject"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_streams_v1 — Retain stream ordering and the accepted evidence head for recovery after account deletion.",
    },
    Entry {
        collection: "sandbox_usage_effective_facts_v1",
        owner: OwnerSelector::Fields(&["owner_subject"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_effective_facts_v1 — Retain the effective evidence revision per fact to prevent a stale correction from replacing it after account deletion.",
    },
    Entry {
        collection: "sandbox_usage_owner_pins_v1",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_owner_pins_v1 — Keep the historical owner binding for a Sandbox source epoch so erasure and replay cannot reassign corrections.",
    },
    Entry {
        collection: "sandbox_usage_shadow_bucket_revisions_v1",
        owner: OwnerSelector::Fields(&["owner_subject"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_shadow_bucket_revisions_v1 retains immutable provisional totals, evidence references and signed correction differences with opaque owner attribution for historical reconciliation.",
    },
    Entry {
        collection: "sandbox_usage_rates_v1",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared usage pricing configuration"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_rates_v1 is shared tariff configuration retained to explain historical prices; account deletion cannot reprice usage.",
    },
    Entry {
        collection: "sandbox_usage_policies_v1",
        owner: OwnerSelector::Fields(&["scope.owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_policies_v1 retains immutable policy revisions and opaque billing-owner attribution; erasure cannot change historical classification.",
    },
    Entry {
        collection: "sandbox_usage_price_pins_v1",
        owner: OwnerSelector::Fields(&["scope.owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_price_pins_v1 preserves each minute's tariff and basis decision with opaque billing-owner attribution for replay and corrections.",
    },
    Entry {
        collection: "sandbox_usage_pricing_schema_v1",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared usage pricing configuration"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "sandbox_usage_pricing_schema_v1 coordinates explicit schema preparation and publication transactions without person-owned data.",
    },
    Entry {
        collection: "saved_logins",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "saved_logins stores the person-owned saved logins record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "schema_migrations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "schema_migrations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "security_hmac_keysets",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "security_hmac_keysets is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "security_remediation_progress",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "security_remediation_progress is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "security_replicas",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "security_replicas is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "security_effect_intents",
        owner: OwnerSelector::Fields(&["event.user_id"]),
        erasure: Erasure::Retained(RetainedCategory::Audit),
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "security_effect_intents preserves pre-chain revocation audit events (event.user_id is the acting person, with bounded IP, user-agent and event metadata) and access-restriction notices (binding_id/client_id indirectly identify the subject through OAuth bindings/clients). Erasure must not discard unchained audit evidence or pending delivery bookkeeping. Audit append failures retry without a lifetime cap; pending/claimed work has no TTL. Delivered CAE notices and failed/uncertain outcomes expire 30 days after completed_at; successfully chained audit intents are deleted. The internal outbox is excluded from export; delivered audit event metadata is available through audit_log.",
    },
    Entry {
        collection: "security_rollouts",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "security_rollouts is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "service_account_key_read_grants",
        owner: OwnerSelector::Fields(&["_id", "owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "service_account_key_read_grants stores the person-owned service account key read grants record keyed by _id, owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "service_account_tokens",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Domain(ErasureDomain::ServiceAccounts),
        export: Export::Excluded(ExclusionReason::SecretMaterial),
        export_selector: ExportSelector::None,
        why: "service_account_tokens is cleaned by the local_service_account_lifecycle lifecycle, which applies the domain-specific external-effect and ownership fences.",
    },
    Entry {
        collection: "service_accounts",
        owner: OwnerSelector::Fields(&["owner_user_id", "created_by"]),
        erasure: Erasure::Domain(ErasureDomain::ServiceAccounts),
        export: Export::Section("services"),
        export_selector: ExportSelector::LegacyServiceAccount,
        why: "service_accounts stores person-owned service-account metadata; credentials and tokens are excluded.",
    },
    Entry {
        collection: "service_approval_configs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "service_approval_configs stores the person-owned service approval configs record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "service_change_events",
        owner: OwnerSelector::Fields(&["owner_id", "actor.person_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::AnyPerson(&["owner_id", "actor.person_id"]),
        why: "service_change_events contains shared history; cleanup removes personal ownership or human-readable attribution while preserving shared counters and opaque identifiers.",
    },
    Entry {
        collection: "service_concurrency_leases",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "service_concurrency_leases is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "service_endpoints",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "service_endpoints is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "service_history_heads",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "user_services",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "service_history_heads is lifecycle data owned through user_services; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "service_pool_health_observations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::JoinedChild {
            parent: "service_pools",
        },
        export: Export::Excluded(ExclusionReason::TransientCoordination),
        export_selector: ExportSelector::None,
        why: "service_pool_health_observations is lifecycle data owned through service_pools; cleanup removes the child only after that parent is removed.",
    },
    Entry {
        collection: "service_pool_member_health",
        owner: OwnerSelector::Fields(&["owner_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("owner_id"),
        why: "service_pool_member_health stores the person-owned service pool member health record keyed by owner_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "service_pools",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "service_pools stores the person-owned service pools record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "service_preferences",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("_id"),
        why: "service_preferences stores the person-owned service preferences record keyed by _id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "service_provider_requirements",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "service_provider_requirements is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "sessions",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("security"),
        export_selector: ExportSelector::Person("user_id"),
        why: "sessions stores the person-owned sessions record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "startup_diagnostics",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "startup_diagnostics is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "telegram_bot_claims",
        owner: OwnerSelector::Fields(&["owner_user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("owner_user_id"),
        why: "telegram_bot_claims stores one-time Telegram setup claims and their owner linkage; every redeemed claim is erased with the owner, while pending or undelivered claims expire through the purge_after TTL.",
    },
    Entry {
        collection: "telegram_bot_requests",
        owner: OwnerSelector::Fields(&["owner_user_id", "actor_user_id"]),
        erasure: Erasure::Redacted,
        export: Export::Section("account_activity"),
        export_selector: ExportSelector::Person("owner_user_id"),
        why: "telegram_bot_requests belongs to owner_user_id; actor_user_id is creator attribution and does not transfer an organization request. Active provisioning is retained until its existing lifecycle settles.",
    },
    Entry {
        collection: "telegram_managed_bot_events",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "telegram_managed_bot_events: ManagedBotEvents stores Telegram bot/user identifiers and monotonic update/revision observations used to reject stale creation events. There is no NyxID user_id; the restricted creation/revocation receipt survives as replay evidence, not a personal request.",
    },
    Entry {
        collection: "telemetry_capture_accounts",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "telemetry_capture_accounts retains restricted capture-generation and dispatch outcome evidence used by vendor erasure to prevent replay and attest processing drain; the telemetry lifecycle does not hard-delete these receipts.",
    },
    Entry {
        collection: "telemetry_dispatches",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::ErasureBookkeeping),
        export_selector: ExportSelector::None,
        why: "telemetry_dispatches retains restricted capture-generation and dispatch outcome evidence used by vendor erasure to prevent replay and attest processing drain; the telemetry lifecycle does not hard-delete these receipts.",
    },
    Entry {
        collection: "telemetry_erasure_destinations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::NotPersonal("shared or coordination record"),
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "telemetry_erasure_destinations is shared or coordination record and has no person-owned erasure path, so account deletion leaves it unchanged.",
    },
    Entry {
        collection: "telemetry_erasure_jobs",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "telemetry_erasure_jobs is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "telemetry_erasure_reconciliations",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::ErasureBookkeeping),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "telemetry_erasure_reconciliations is retained as ErasureBookkeeping evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "trigger_deliveries",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "trigger_deliveries stores the person-owned trigger deliveries record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "trigger_owner_limits",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("_id"),
        why: "trigger_owner_limits stores the person-owned trigger owner limits record keyed by _id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "trigger_run_budgets",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Excluded(ExclusionReason::NotPersonal),
        export_selector: ExportSelector::None,
        why: "trigger_run_budgets: trigger_schedule::admit_budget writes a person-keyed expiring admission budget; local::inventory removes it on account erasure.",
    },
    Entry {
        collection: "trigger_runs",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "trigger_runs stores the person-owned trigger runs record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "trigger_work",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "trigger_work stores the person-owned trigger work record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "triggers",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("user_id"),
        why: "triggers stores the person-owned triggers record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "usage_allowance_periods",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "usage_allowance_periods stores frozen allowance windows and settlement state shared by recipients; financial retention preserves accounting; deletion retains this record for its documented lifecycle.",
    },
    Entry {
        collection: "usage_allowances",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "usage_allowances is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_meter",
        owner: OwnerSelector::Fields(&["billing_owner_id"]),
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Section("billing"),
        export_selector: ExportSelector::Person("billing_owner_id"),
        why: "usage_meter links billable usage and numeric stream-recovery checkpoints to billing_owner_id; financial and billing retention preserves settlement evidence, while export exposes only its explicit billing metadata allowlist; individual usage rows expire 30 days after acknowledgement or counting through the usage_meter TTL.",
    },
    Entry {
        collection: "usage_rollup_daily",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "usage_rollup_daily is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_rollup_hourly",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "usage_rollup_hourly is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_rollup_state",
        owner: OwnerSelector::NotPersonal,
        erasure: Erasure::Retained(RetainedCategory::FinancialBilling),
        export: Export::Excluded(ExclusionReason::RetainedRecord),
        export_selector: ExportSelector::None,
        why: "usage_rollup_state is retained as FinancialBilling evidence because deleting it would change shared, financial, lifecycle, or erasure accountability; only permitted opaque attribution remains.",
    },
    Entry {
        collection: "usage_workspaces",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("billing"),
        export_selector: ExportSelector::Person("_id"),
        why: "usage_workspaces stores the person-owned usage workspaces record keyed by _id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "user_api_keys",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "user_api_keys stores the person-owned user api keys record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "user_endpoints",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "user_endpoints stores the person-owned user endpoints record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "user_provider_credentials",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "user_provider_credentials stores the person-owned user provider credentials record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "user_provider_tokens",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "user_provider_tokens stores the person-owned user provider tokens record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "user_service_connections",
        owner: OwnerSelector::Fields(&["user_id"]),
        erasure: Erasure::LocalInventory,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "user_service_connections stores the person-owned user service connections record keyed by user_id; bounded local cleanup removes the row after the erasure admission fence.",
    },
    Entry {
        collection: "user_services",
        owner: OwnerSelector::Fields(&[
            "user_id",
            "created_by.actor.person_id",
            "last_change.actor.person_id",
        ]),
        erasure: Erasure::Redacted,
        export: Export::Section("services"),
        export_selector: ExportSelector::Person("user_id"),
        why: "user_services removes person-owned rows through the local inventory and redacts creator/last-change labels on surviving shared rows; actor ids remain opaque attribution.",
    },
    Entry {
        collection: "users",
        owner: OwnerSelector::Fields(&["_id"]),
        erasure: Erasure::Domain(ErasureDomain::Source),
        export: Export::Section("account"),
        export_selector: ExportSelector::Person("_id"),
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

/// The joined export access path needs these formerly unindexed child keys.
/// Owning index initialization lives in platform_provisioning_service.
pub(crate) const EXPORT_JOIN_INDEX_GAPS: &[(&str, &str)] = &[
    ("platform_delivery_tests", "installation_id"),
    ("platform_setup_steps", "setup_id"),
];

/// Unbounded event-history selectors whose existing indexes do not serve the
/// complete account export. Each addition is owned by its writer module.
#[cfg(test)]
pub(crate) const EXPORT_SELECTOR_INDEX_GAPS: &[(&str, &str)] = &[
    ("assistant_attachments", "user_id"),
    ("assistant_checkpoints", "user_id"),
    ("assistant_group_requests", "actor_user_id"),
    ("org_channel_messages", "author_user_id"),
];

/// Owner predicates that are not covered by an index in the origin/main
/// startup definitions and need a new index because the collection is a large,
/// per-event stream. This is an explicit rollout manifest rather than a
/// blanket "one index per field" loop: existing indexes are reused whenever
/// their first key covers the predicate.
pub(crate) const OWNER_INDEX_GAPS: &[(&str, &str)] = &[
    ("assistant_group_mention_notifications", "user_id"),
    ("assistant_group_messages", "author_user_id"),
    ("assistant_group_messages", "user_id"),
    ("assistant_group_requests", "user_id"),
    ("channel_adapter_events", "user_id"),
    ("channel_delivery_receipts", "user_id"),
    ("channel_email_batches", "user_id"),
    ("channel_email_receipts", "user_id"),
    ("channel_email_sends", "user_id"),
    ("channel_reply_deliveries", "user_id"),
    ("nyxbot_events", "user_id"),
    ("refresh_tokens", "user_id"),
    ("service_change_events", "actor.person_id"),
    ("service_pool_health_observations", "pool_id"),
    ("trigger_deliveries", "user_id"),
    ("trigger_work", "user_id"),
];

/// Predicates deliberately left without a new index. Each one is either a
/// short-lived row, a small/resource-bounded lifecycle collection, a joined
/// child bounded by its parent, or (for channel claims) already served by the
/// composite `_id` prefix used by erasure and the orphan sweep.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnerIndexBoundedReason {
    ShortTtl,
    SmallOrResourceBounded,
    ExistingIdPrefix,
}

#[cfg(test)]
const OWNER_INDEX_BOUNDED: &[(&str, &str, OwnerIndexBoundedReason)] = &[
    (
        "user_services",
        "created_by.actor.person_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "user_services",
        "last_change.actor.person_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "oauth_states",
        "history_context.actor.person_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "assistant_wire_logs",
        "user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "invocation_diagnostics",
        "actor_user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "agent_key_login_requests",
        "approved_user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "assistant_agent_consents",
        "owner_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_agent_learning_members",
        "member_user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_agent_learning_runs",
        "owner_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_agent_learning_skill_roots",
        "owner_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_agent_private_notes",
        "actor_user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_agent_shares",
        "owner_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_agent_sharing",
        "owner_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_async_quotas",
        "user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_group_agent_access",
        "owner_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_group_agent_proposals",
        "proposed_by",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_group_invites",
        "accepted_by_user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_group_invites",
        "intended_user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_group_invites",
        "issuer_user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_group_read_states",
        "user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_skill_draft_attempts",
        "user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "assistant_voice_announcements",
        "user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "assistant_voice_windows",
        "user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "auth_device_codes",
        "approved_user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "auth_device_codes_v2",
        "approved_user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "authorization_codes",
        "user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "channel_adapter_allocations",
        "user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "channel_email_subscriptions",
        "user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "device_codes",
        "approved_by_user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "feature_flag_overrides",
        "target_key",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "invite_codes",
        "created_by",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "invite_codes",
        "usages.user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "login_approvals",
        "user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "mfa_challenges",
        "user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "machine_updates",
        "user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "machine_oauth_consent_requests",
        "user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "oauth_consent_requests",
        "user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "openclaw_channel_mappings",
        "nyxid_user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "org_invites",
        "created_by",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        // Human readiness consent expires after five minutes and is purged by
        // platform_public_contract_service::sweep_probes.
        "platform_public_probe_consents",
        "actor_user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "platform_personal_setup_selections",
        "user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "service_account_key_read_grants",
        "owner_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "service_pool_member_health",
        "owner_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "telegram_bot_claims",
        "owner_user_id",
        OwnerIndexBoundedReason::SmallOrResourceBounded,
    ),
    (
        "telegram_bot_requests",
        "owner_user_id",
        OwnerIndexBoundedReason::ShortTtl,
    ),
    (
        "channel_send_claims",
        "conversation_id",
        OwnerIndexBoundedReason::ExistingIdPrefix,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::Path,
    };

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
                    // Test-only directories (named `tests`, `*_tests` or `test_*`) are
                    // compiled only through #[cfg(test)] modules or test-only files, so
                    // they hold no production collection calls. Match these exact shapes
                    // rather than any name containing "test", which would also hide
                    // production names such as "attestation".
                    let test_only_directory = p
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name == "tests" || name.ends_with("_tests") || name.starts_with("test_")
                        });
                    if test_only_directory {
                        continue;
                    }
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
    fn domain_variants_are_bound_to_durable_dispatch_and_real_entry_points() {
        let durable = include_str!("personal_erasure_service.rs");
        let worker = include_str!("personal_erasure_service/worker.rs");
        let local = include_str!("personal_erasure_service/local.rs");
        for entry in entries() {
            if let Erasure::Domain(domain) = entry.erasure {
                let (checkpoint, handler) = domain.execution();
                assert!(
                    durable.contains(&format!("\"{checkpoint}\"")),
                    "{}: unknown checkpoint",
                    entry.collection
                );
                let normalized = format!("{worker}{local}")
                    .split_whitespace()
                    .collect::<String>();
                assert!(
                    normalized.contains(handler)
                        || normalized
                            .contains(&format!("{}(", handler.rsplit("::").next().unwrap())),
                    "{}: handler {handler} is not invoked",
                    entry.collection
                );
            }
        }
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
    fn security_effect_outbox_retains_pre_chain_audit_and_excludes_internal_delivery_state() {
        use crate::models::security_effect_intent::{COLLECTION_NAME, SecurityEffect};
        use crate::services::{audit_service, security_effect_service};

        let entry = find(COLLECTION_NAME).unwrap();
        assert_eq!(entry.owner, OwnerSelector::Fields(&["event.user_id"]));
        assert_eq!(entry.erasure, Erasure::Retained(RetainedCategory::Audit));
        assert_eq!(
            entry.export,
            Export::Excluded(ExclusionReason::TransientCoordination)
        );
        assert_eq!(entry.export_selector, ExportSelector::None);
        let actor = audit_service::AuditActor {
            user_id: "departing-person".into(),
            ip_address: Some("192.0.2.1".into()),
            user_agent: Some("fixture-agent".into()),
            api_key_id: None,
            api_key_name: None,
        };
        let audit =
            audit_service::required::intent(Some(&actor), "oauth_consent_revoked", None).unwrap();
        let audit = bson::to_document(&audit).unwrap();
        assert_eq!(
            audit.get_document("event").unwrap().get_str("user_id"),
            Ok("departing-person")
        );
        let cae = security_effect_service::new_intent(
            "notice".into(),
            SecurityEffect::Cae {
                binding_id: "opaque-binding-hash".into(),
                client_id: "application-id".into(),
                revoke_source: "user".into(),
            },
        );
        let cae = bson::to_document(&cae).unwrap();
        assert_eq!(cae.get_str("binding_id"), Ok("opaque-binding-hash"));
        assert_eq!(cae.get_str("client_id"), Ok("application-id"));
        assert!(!cae.contains_key("user_id") && !cae.contains_key("event"));
        assert!(
            !crate::services::personal_erasure_service::local::inventory(&actor.user_id)
                .iter()
                .any(|(name, _)| *name == COLLECTION_NAME)
        );
        assert!(
            !ADDITIONAL_EXPORT_VIEWS
                .iter()
                .any(|view| view.collection == COLLECTION_NAME)
        );
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
    fn node_delivery_intent_pins_existing_node_drain_and_final_metadata_cleanup() {
        let collection = crate::models::node_credential_desired::COLLECTION_NAME;
        let entry = find(collection).expect("registered delivery intent");
        assert_eq!(entry.owner, OwnerSelector::Fields(&["owner_id"]));
        assert_eq!(entry.erasure, Erasure::Domain(ErasureDomain::NodeDrain));
        assert_eq!(
            entry.export,
            Export::Excluded(ExclusionReason::TransientCoordination)
        );
        assert!(
            crate::services::personal_erasure_service::local::NODE_DRAIN_COLLECTIONS
                .contains(&collection)
        );
        let final_metadata = crate::services::personal_erasure_registry::final_metadata("person");
        assert_eq!(
            final_metadata
                .iter()
                .find(|(name, _)| *name == collection)
                .map(|(_, filter)| filter),
            Some(&bson::doc! {"owner_id": "person"})
        );
        assert!(
            !crate::services::personal_erasure_service::local::inventory("person")
                .iter()
                .any(|(name, _)| *name == collection),
            "no competing local-inventory deletion"
        );
    }

    #[test]
    fn local_inventory_owner_selectors_match_registry_predicates_and_indexes() {
        let owner = "00000000-0000-0000-0000-000000000001";
        let filters =
            crate::services::personal_erasure_service::local::inventory_owner_fields(owner);
        for entry in ENTRIES {
            if !matches!(entry.erasure, Erasure::LocalInventory | Erasure::Redacted) {
                continue;
            }
            if matches!(entry.erasure, Erasure::Redacted) && !filters.contains_key(entry.collection)
            {
                continue;
            }
            let OwnerSelector::Fields(fields) = entry.owner else {
                panic!(
                    "local inventory entry {} has no owner selector",
                    entry.collection
                );
            };
            let actual = filters
                .get(entry.collection)
                .unwrap_or_else(|| panic!("missing inventory filter for {}", entry.collection));
            let expected = fields
                .iter()
                .filter(|field| attribution(entry.collection, field).is_none())
                .map(|field| (*field).to_owned())
                .collect();
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
    fn claim_and_reservation_inventory_only_erases_settled_owner_rows() {
        let owner = "00000000-0000-0000-0000-000000000001";
        let inventory = crate::services::personal_erasure_service::local::inventory(owner);
        let claims = inventory
            .iter()
            .find(|(collection, _)| *collection == "telegram_bot_claims")
            .map(|(_, filter)| filter)
            .expect("telegram claims inventory");
        assert_eq!(claims.get_str("owner_user_id"), Ok(owner));
        assert_eq!(claims.get_str("status"), Ok("redeemed"));
        assert_eq!(claims.get_bool("delivered"), Ok(true));
    }

    async fn index_models(db: &mongodb::Database, collection: &str) -> Vec<mongodb::IndexModel> {
        let Ok(cursor) = db
            .collection::<mongodb::bson::Document>(collection)
            .list_indexes()
            .await
        else {
            return Vec::new();
        };
        cursor.try_collect().await.unwrap()
    }

    fn affected_collections() -> BTreeSet<&'static str> {
        let mut collections = registry_owner_predicates()
            .into_iter()
            .map(|(collection, _)| collection)
            .collect::<BTreeSet<_>>();
        collections.extend(JOINED_CHILD_SPECS.iter().map(|spec| spec.child));
        collections.extend(export_index_coverage::collections());
        // Main-owned domain queues and retained meters fall outside the local
        // inventory. Their indexes must still survive branch and mixed-fleet startup.
        collections.insert(crate::models::node_credential_desired::COLLECTION_NAME);
        collections.insert(crate::models::usage_meter::COLLECTION_NAME);
        collections
    }

    async fn index_snapshot(
        db: &mongodb::Database,
    ) -> BTreeMap<String, Vec<mongodb::bson::Document>> {
        let mut snapshot = BTreeMap::new();
        for collection in affected_collections() {
            let mut definitions = index_models(db, collection)
                .await
                .into_iter()
                .map(|index| bson::to_document(&index).expect("serialize index fixture"))
                .collect::<Vec<_>>();
            definitions.sort_by_key(|definition| bson::to_vec(definition).unwrap());
            snapshot.insert(collection.to_owned(), definitions);
        }
        snapshot
    }

    fn index_identity(definition: &mongodb::bson::Document) -> Vec<u8> {
        // MongoDB 8.0 permits same-key indexes distinguished by collation,
        // partialFilterExpression, unique or sparse; name, hidden and TTL do
        // not distinguish them. The manual documents these options in:
        // https://www.mongodb.com/docs/v8.0/reference/command/createIndexes/#index-options
        // https://www.mongodb.com/docs/v8.0/core/index-partial/#restrictions
        // https://www.mongodb.com/docs/v8.0/core/index-unique/#basic-and-unique-indexes-with-duplicate-key-patterns
        // The unique-index page also covers differing sparse options. The live
        // coexistence test below pins the option list against the server.
        // IndexModel.options is serde(flatten): its partialFilterExpression,
        // collation, unique and sparse fields serialize beside key, not inside
        // an options document. listIndexes normalizes the stored definitions.
        let mut identity = bson::doc! {
            "key": definition.get_document("key").expect("index fixture key").clone(),
            "unique": definition.get_bool("unique").unwrap_or(false),
            "sparse": definition.get_bool("sparse").unwrap_or(false),
        };
        for option in ["partialFilterExpression", "collation"] {
            if let Some(value) = definition.get(option) {
                identity.insert(option, value.clone());
            }
        }
        bson::to_vec(&identity).expect("serialize index identity")
    }

    fn index_identity_probe_cases() -> [(&'static str, bson::Document, bool); 7] {
        [
            ("name", bson::doc! {}, false),
            (
                "partialFilterExpression",
                bson::doc! {"partialFilterExpression":{"value":{"$type":"date"}}},
                true,
            ),
            ("collation", bson::doc! {"collation":{"locale":"en"}}, true),
            ("unique", bson::doc! {"unique":true}, true),
            ("sparse", bson::doc! {"sparse":true}, true),
            ("hidden", bson::doc! {"hidden":true}, false),
            (
                "expireAfterSeconds",
                bson::doc! {"expireAfterSeconds":60_i32},
                false,
            ),
        ]
    }

    fn serialized_index(name: &str, options: bson::Document) -> bson::Document {
        let mut definition = bson::doc! {"key":{"value":1},"name":name};
        definition.extend(options);
        let model: mongodb::IndexModel = bson::from_document(definition).unwrap();
        bson::to_document(&model).unwrap()
    }

    #[test]
    fn index_identity_distinguishes_only_coexisting_options() {
        let base = serialized_index("base", bson::doc! {});
        for (option, options, distinct) in index_identity_probe_cases() {
            let candidate = serialized_index("candidate", options.clone());
            for (field, value) in options {
                assert_eq!(candidate.get(&field), Some(&value), "flattened {option}");
            }
            assert_eq!(
                index_identity(&base) != index_identity(&candidate),
                distinct,
                "{option}"
            );
        }
        assert_eq!(
            index_identity(&base),
            index_identity(&serialized_index(
                "explicit-defaults",
                bson::doc! {"unique":false,"sparse":false}
            )),
        );
        // Differing partial filters can coexist, but cannot hide a changed or
        // missing origin index; preservation still compares full definitions.
        let first = serialized_index(
            "first",
            bson::doc! {"partialFilterExpression":{"value":{"$gt":0}}},
        );
        let second = serialized_index(
            "second",
            bson::doc! {"partialFilterExpression":{"value":{"$gt":1}}},
        );
        let before = BTreeMap::from([("usage_meter".into(), vec![first.clone(), second.clone()])]);
        assert_no_duplicate_keys(&before);
        assert_origin_indexes_untouched(&before, &before);
        let missing = BTreeMap::from([("usage_meter".into(), vec![second])]);
        assert!(
            std::panic::catch_unwind(|| assert_origin_indexes_untouched(&before, &missing))
                .is_err()
        );
    }

    #[test]
    #[should_panic(expected = "duplicate index identity in usage_meter")]
    fn duplicate_index_identity_rejects_same_partial_filter_under_another_name() {
        let options = bson::doc! {"partialFilterExpression":{"value":{"$gt":0}}};
        let snapshot = BTreeMap::from([(
            "usage_meter".into(),
            vec![
                serialized_index("first", options.clone()),
                serialized_index("second", options),
            ],
        )]);
        assert_no_duplicate_keys(&snapshot);
    }

    #[tokio::test]
    async fn index_identity_matches_mongodb_same_key_coexistence() {
        let db = crate::test_utils::connect_transaction_test_database("index_identity_coexistence")
            .await;
        for (option, options, accepted) in index_identity_probe_cases() {
            let collection = db.collection::<bson::Document>(option);
            let base = serialized_index("base", bson::doc! {});
            collection
                .create_index(bson::from_document(base).unwrap())
                .await
                .unwrap();
            let candidate = serialized_index("candidate", options);
            let result = collection
                .create_index(bson::from_document(candidate).unwrap())
                .await;
            if accepted {
                result.unwrap_or_else(|error| panic!("{option} must permit coexistence: {error}"));
            } else {
                let error = result.expect_err("equivalent index must be refused");
                assert!(
                    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 85),
                    "{option}: {error}"
                );
            }
            let definitions = index_models(&db, option)
                .await
                .into_iter()
                .map(|index| bson::to_document(&index).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(definitions.len(), if accepted { 3 } else { 2 }, "{option}");
            assert_no_duplicate_keys(&BTreeMap::from([(option.to_owned(), definitions)]));
        }
        db.drop().await.unwrap();
    }

    fn assert_origin_indexes_untouched(
        before: &BTreeMap<String, Vec<mongodb::bson::Document>>,
        after: &BTreeMap<String, Vec<mongodb::bson::Document>>,
    ) {
        for (collection, definitions) in before {
            let after_definitions = after
                .get(collection)
                .unwrap_or_else(|| panic!("missing collection {collection} after startup"));
            for definition in definitions {
                assert!(
                    after_definitions.contains(definition),
                    "origin/main index changed or disappeared in {collection}: {definition:?}"
                );
            }
        }
    }

    fn snapshot_has_prefix(definitions: &[mongodb::bson::Document], field: &str) -> bool {
        definitions.iter().any(|definition| {
            definition
                .get_document("key")
                .ok()
                .and_then(|key| key.keys().next())
                .is_some_and(|first| first == field)
        })
    }

    fn assert_no_duplicate_keys(snapshot: &BTreeMap<String, Vec<mongodb::bson::Document>>) {
        for (collection, definitions) in snapshot {
            let mut keys = BTreeSet::new();
            for definition in definitions {
                assert!(
                    keys.insert(index_identity(definition)),
                    "duplicate index identity in {collection}: {definition:?}"
                );
            }
        }
    }

    fn assert_only_manifest_indexes_added(
        before: &BTreeMap<String, Vec<mongodb::bson::Document>>,
        after: &BTreeMap<String, Vec<mongodb::bson::Document>>,
    ) {
        let mut added = BTreeSet::new();
        let mut export_joins = BTreeSet::new();
        for (collection, definitions) in after {
            for definition in definitions {
                if before[collection].contains(definition) {
                    continue;
                }
                let keys = definition.get_document("key").expect("index keys");
                // Creating the first index on a formerly absent collection
                // also creates MongoDB's mandatory identity index.
                if before[collection].is_empty() && keys == &bson::doc! {"_id": 1} {
                    continue;
                }
                if collection == "assistant_messages"
                    && keys == &bson::doc! {"user_id":1,"created_at":-1,"_id":-1}
                {
                    assert_eq!(
                        definition.get_str("name"),
                        Ok("assistant_messages_account_export")
                    );
                    continue;
                }
                if keys.len() == 1
                    && EXPORT_JOIN_INDEX_GAPS
                        .iter()
                        .chain(EXPORT_SELECTOR_INDEX_GAPS)
                        .any(|predicate| {
                            predicate
                                == &(collection.as_str(), keys.keys().next().unwrap().as_str())
                        })
                {
                    let field = keys.keys().next().unwrap();
                    assert_eq!(
                        definition.get_str("name"),
                        Ok(format!("account_export_{collection}_{field}").as_str())
                    );
                    assert_eq!(keys.get_i32(field), Ok(1));
                    export_joins.insert((collection.as_str(), field.as_str()));
                    continue;
                }
                assert_eq!(keys.len(), 1, "unexpected added index: {definition:?}");
                let field = keys.keys().next().unwrap();
                assert!(
                    OWNER_INDEX_GAPS.contains(&(collection.as_str(), field.as_str())),
                    "index added outside the minimal manifest: {collection}.{field}"
                );
                assert_eq!(keys.get_i32(field), Ok(1));
                let expected_name = format!(
                    "personal_erasure_owner_{}_{}",
                    collection.replace('.', "_"),
                    field.replace('.', "_")
                );
                assert_eq!(definition.get_str("name"), Ok(expected_name.as_str()));
                assert!(added.insert((collection.as_str(), field.as_str())));
            }
        }
        assert_eq!(added, OWNER_INDEX_GAPS.iter().copied().collect());
        assert_eq!(
            export_joins,
            EXPORT_JOIN_INDEX_GAPS
                .iter()
                .chain(EXPORT_SELECTOR_INDEX_GAPS)
                .copied()
                .collect()
        );
        assert!(
            after["assistant_messages"]
                .iter()
                .any(|d| d.get_str("name") == Ok("assistant_messages_account_export"))
        );
    }

    include!("account_export_index_tests.rs");

    fn registry_owner_predicates() -> BTreeSet<(&'static str, &'static str)> {
        ENTRIES
            .iter()
            .filter_map(|entry| match entry.erasure {
                Erasure::LocalInventory | Erasure::Redacted => match entry.owner {
                    OwnerSelector::Fields(fields) => {
                        Some(fields.iter().map(move |field| (entry.collection, *field)))
                    }
                    OwnerSelector::NotPersonal => None,
                },
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn owner_index_bounded_predicates() -> BTreeSet<(&'static str, &'static str)> {
        OWNER_INDEX_BOUNDED
            .iter()
            .map(|(collection, field, _)| (*collection, *field))
            .collect()
    }

    fn export_owner_index_predicates() -> impl Iterator<Item = (&'static str, &'static str)> {
        let owners = registry_owner_predicates();
        EXPORT_SELECTOR_INDEX_GAPS
            .iter()
            .copied()
            .filter(move |predicate| owners.contains(predicate))
    }

    fn owner_index_manifest_predicates() -> BTreeSet<(&'static str, &'static str)> {
        OWNER_INDEX_GAPS
            .iter()
            .chain(EXPORT_JOIN_INDEX_GAPS)
            .copied()
            .chain(export_owner_index_predicates())
            .chain(
                OWNER_INDEX_BOUNDED
                    .iter()
                    .map(|(collection, field, _)| (*collection, *field)),
            )
            .collect()
    }

    #[test]
    fn persisted_history_actor_shapes_and_redaction_registry_are_complete() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/models");
        let structures = regex::Regex::new(r"(?s)pub struct (\w+)\s*\{(.*?)\n\}").unwrap();
        let fields = regex::Regex::new(
            r"(?m)^\s*pub\s+(\w+):[^\n]*\b(HistoryActor|HistoryContext|ServiceChangeSummary)\b",
        )
        .unwrap();
        let mut persisted = BTreeSet::new();
        for path in fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
        {
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let source = strip_cfg_test_modules(&fs::read_to_string(path).unwrap());
            for record in structures.captures_iter(&source) {
                if matches!(&record[1], "HistoryContext" | "ServiceChangeSummary") {
                    continue;
                }
                for field in fields.captures_iter(&record[2]) {
                    persisted.insert((
                        record[1].to_owned(),
                        field[1].to_owned(),
                        field[2].to_owned(),
                    ));
                }
            }
        }
        let expected: BTreeSet<_> = [
            ("ServiceChangeEvent", "actor", "HistoryActor"),
            ("UserService", "created_by", "ServiceChangeSummary"),
            ("UserService", "last_change", "ServiceChangeSummary"),
            ("OAuthState", "history_context", "HistoryContext"),
        ]
        .into_iter()
        .map(|(record, field, kind)| (record.to_owned(), field.to_owned(), kind.to_owned()))
        .collect();
        assert_eq!(
            persisted, expected,
            "persisted actor schema changed: update attribution redaction and its global-scan fixtures"
        );
        let paths: BTreeSet<_> = ATTRIBUTIONS
            .iter()
            .map(|spec| (spec.collection, spec.person_field))
            .collect();
        assert_eq!(
            paths,
            BTreeSet::from([
                ("service_change_events", "actor.person_id"),
                ("user_services", "created_by.actor.person_id"),
                ("user_services", "last_change.actor.person_id"),
                ("oauth_states", "history_context.actor.person_id"),
                ("assistant_group_messages", "author_user_id"),
                ("telegram_bot_requests", "actor_user_id"),
            ])
        );
        assert_eq!(paths.len(), ATTRIBUTIONS.len());
        for spec in ATTRIBUTIONS {
            let entry = find(spec.collection).unwrap();
            assert_eq!(entry.erasure, Erasure::Redacted);
            assert!(
                matches!(entry.owner, OwnerSelector::Fields(fields) if fields.contains(&spec.person_field))
            );
            assert!(!spec.name_fields.is_empty() || !spec.email_fields.is_empty());
            assert!(!matches!(spec.collection, "audit_log" | "billing_ledger"));
        }
    }

    #[test]
    fn rollout_prebuild_commands_match_the_index_manifest() {
        let docs = include_str!("../../../docs/security/PERSONAL_ERASURE_INVENTORY.md");
        let commands: Vec<_> = docs
            .lines()
            .filter(|line| line.starts_with("db.") && line.contains("personal_erasure_owner_"))
            .collect();
        assert_eq!(commands.len(), OWNER_INDEX_GAPS.len());
        for (collection, field) in EXPORT_JOIN_INDEX_GAPS
            .iter()
            .chain(EXPORT_SELECTOR_INDEX_GAPS)
        {
            let command = format!(
                "db.{collection}.createIndex({{{field}:1}}, {{name:\"account_export_{collection}_{field}\"}});"
            );
            assert!(
                docs.lines().any(|line| line == command),
                "missing joined export prebuild: {command}"
            );
        }
        assert!(docs.contains("db.assistant_messages.createIndex({user_id:1, created_at:-1, _id:-1}, {name:\"assistant_messages_account_export\"});"));
        for (collection, field) in OWNER_INDEX_GAPS {
            let command = format!(
                "db.{collection}.createIndex({{\"{field}\": 1}}, {{name: \"personal_erasure_owner_{collection}_{}\"}});",
                field.replace('.', "_")
            );
            assert!(
                commands.contains(&command.as_str()),
                "missing prebuild command for {collection}.{field}"
            );
        }
    }

    #[test]
    fn owner_index_manifest_classifies_every_origin_gap() {
        let kept: BTreeSet<_> = OWNER_INDEX_GAPS
            .iter()
            .chain(EXPORT_JOIN_INDEX_GAPS)
            .copied()
            .chain(export_owner_index_predicates())
            .collect();
        let bounded = owner_index_bounded_predicates();
        assert_eq!(
            kept.len(),
            OWNER_INDEX_GAPS.len()
                + EXPORT_JOIN_INDEX_GAPS.len()
                + export_owner_index_predicates().count(),
            "duplicate kept index"
        );
        assert_eq!(
            bounded.len(),
            OWNER_INDEX_BOUNDED.len(),
            "duplicate bounded owner predicate"
        );
        assert!(
            kept.is_disjoint(&bounded),
            "a predicate cannot be both kept and bounded"
        );

        let mut known = registry_owner_predicates();
        known.extend(
            JOINED_CHILD_SPECS
                .iter()
                .filter(|spec| spec.child_parent_field != "_id")
                .map(|spec| (spec.child, spec.child_parent_field)),
        );
        for predicate in owner_index_manifest_predicates() {
            assert!(
                known.contains(&predicate),
                "owner-index manifest predicate is not in the registry or joined-child specs: {predicate:?}"
            );
        }
    }

    async fn assert_owner_prefix_coverage(db: &mongodb::Database) {
        let mut missing: Vec<(&str, &str)> = Vec::new();
        for (collection, field) in OWNER_INDEX_GAPS
            .iter()
            .chain(EXPORT_JOIN_INDEX_GAPS)
            .copied()
            .chain(export_owner_index_predicates())
        {
            let covered = index_models(db, collection)
                .await
                .iter()
                .any(|index| index.keys.keys().next().is_some_and(|first| first == field));
            if !covered {
                missing.push((collection, field));
            }
        }
        for &(collection, field, _) in OWNER_INDEX_BOUNDED {
            let covered = index_models(db, collection)
                .await
                .iter()
                .any(|index| index.keys.keys().next().is_some_and(|first| first == field));
            assert!(
                !covered,
                "bounded owner predicate unexpectedly received an index: {collection}.{field}"
            );
        }
        assert!(
            missing.is_empty(),
            "owner predicates lack prefix indexes: {missing:?}"
        );
    }

    #[tokio::test]
    async fn startup_indexes_are_idempotent_and_cover_registry_predicates() {
        let db =
            crate::test_utils::connect_transaction_test_database("privacy_startup_indexes").await;
        crate::db::ensure_indexes(&db).await.unwrap();
        crate::services::personal_erasure_service::ensure_startup_indexes(&db)
            .await
            .unwrap_or_else(|error| panic!("startup indexes: {error}"));
        crate::db::ensure_indexes(&db).await.unwrap();
        crate::services::personal_erasure_service::ensure_startup_indexes(&db)
            .await
            .unwrap_or_else(|error| panic!("startup indexes second run: {error}"));
        assert_owner_prefix_coverage(&db).await;
        export_index_coverage::assert_export_prefix_coverage(&db).await;
    }

    #[tokio::test]
    async fn origin_main_indexes_survive_branch_startup_and_old_fleet_restart() {
        let db =
            crate::test_utils::connect_transaction_test_database("privacy_startup_upgrade_indexes")
                .await;

        // Checked-in origin/main fixture: db::ensure_indexes plus the exact
        // security and personal-erasure calls that origin/main runs at startup.
        crate::db::ensure_indexes(&db).await.unwrap();
        crate::services::personal_erasure_service::ensure_origin_main_indexes_for_test(&db)
            .await
            .unwrap();
        let origin = index_snapshot(&db).await;
        assert!(snapshot_has_prefix(
            &origin["node_credential_desired"],
            "node_id"
        ));
        assert!(snapshot_has_prefix(
            &origin["node_credential_desired"],
            "pending"
        ));
        assert!(snapshot_has_prefix(
            &origin["usage_meter"],
            "usage_checkpoint.recover_after"
        ));

        // The checked-in classification is complete relative to the
        // origin/main fixture: every uncovered registry/join predicate is
        // either a kept large-stream index or an explicitly bounded drop.
        export_index_coverage::assert_export_origin_classification(&origin);
        let mut classification_errors = Vec::new();
        for (collection, field) in registry_owner_predicates() {
            let covered = field == "_id" || snapshot_has_prefix(&origin[collection], field);
            let classified = OWNER_INDEX_GAPS.contains(&(collection, field))
                || EXPORT_JOIN_INDEX_GAPS.contains(&(collection, field))
                || owner_index_bounded_predicates().contains(&(collection, field));
            // A broad export can require a full prefix where a partial owner
            // prefix exists already. Export coverage checks that separately.
            let export_index =
                export_owner_index_predicates().any(|predicate| predicate == (collection, field));
            if !covered && !classified && !export_index {
                classification_errors.push(format!(
                    "unclassified origin/main owner gap for {collection}.{field}"
                ));
            }
            if covered && classified {
                classification_errors.push(format!("owner-index classification redundantly covers existing index for {collection}.{field}"));
            }
        }
        for spec in JOINED_CHILD_SPECS {
            if spec.child_parent_field == "_id" {
                continue;
            }
            let covered = snapshot_has_prefix(&origin[spec.child], spec.child_parent_field);
            let classified = OWNER_INDEX_GAPS.contains(&(spec.child, spec.child_parent_field))
                || EXPORT_JOIN_INDEX_GAPS.contains(&(spec.child, spec.child_parent_field))
                || owner_index_bounded_predicates()
                    .contains(&(spec.child, spec.child_parent_field));
            assert!(
                covered || classified,
                "unclassified origin/main joined-child gap for {}.{}",
                spec.child,
                spec.child_parent_field
            );
            assert!(
                !covered || !classified,
                "joined-child classification redundantly covers existing index for {}.{}",
                spec.child,
                spec.child_parent_field
            );
        }

        assert!(
            classification_errors.is_empty(),
            "{}",
            classification_errors.join("\n")
        );

        // The branch must be safe for an existing deployment and idempotent
        // when both a new and an old replica restart during the rollout.
        crate::services::personal_erasure_service::ensure_startup_indexes(&db)
            .await
            .unwrap();
        let first_branch = index_snapshot(&db).await;
        crate::services::personal_erasure_service::ensure_startup_indexes(&db)
            .await
            .unwrap();
        let branch = index_snapshot(&db).await;
        assert_eq!(
            first_branch, branch,
            "second branch startup changed indexes"
        );
        assert_origin_indexes_untouched(&origin, &branch);
        assert_only_manifest_indexes_added(&origin, &branch);
        assert_no_duplicate_keys(&branch);
        assert_owner_prefix_coverage(&db).await;
        export_index_coverage::assert_export_prefix_coverage(&db).await;

        // A mixed fleet still invokes origin/main's sequence after the branch
        // has added its indexes; no old index may conflict with the additions.
        crate::db::ensure_indexes(&db).await.unwrap();
        crate::services::personal_erasure_service::ensure_origin_main_indexes_for_test(&db)
            .await
            .unwrap();
        let mixed = index_snapshot(&db).await;
        assert_origin_indexes_untouched(&origin, &mixed);
        assert_no_duplicate_keys(&mixed);
        assert_eq!(
            branch, mixed,
            "old-fleet restart changed the branch index set"
        );
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
                Erasure::Retained(_)
                | Erasure::Domain(_)
                | Erasure::FeatureRetention(_)
                | Erasure::NotPersonal(_) => {}
            }
        }
        assert!(cases > 0);
    }

    #[test]
    fn joined_child_specs_are_the_registry_joined_child_set() {
        let expected: BTreeSet<_> = JOINED_CHILD_SPECS
            .iter()
            .map(|spec| (spec.child, spec.parent))
            .collect();
        let actual: BTreeSet<_> = ENTRIES
            .iter()
            .filter_map(|entry| match entry.erasure {
                Erasure::JoinedChild { parent } => Some((entry.collection, parent)),
                _ => None,
            })
            .collect();
        assert_eq!(actual, expected);
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
            ("org_channel_messages", &["author_user_id", "actor_id"][..]),
            ("billing_wallet", &["owner_id"][..]),
            ("credit_grants", &["recipient_user_id"][..]),
            ("credit_schedule_recipients", &["recipient_user_id"][..]),
            ("usage_meter", &["billing_owner_id"][..]),
            ("platform_setups", &["owner_user_id"][..]),
            ("platform_bot_installations", &["owner_user_id"][..]),
            (
                "platform_personal_stop_effects",
                &["active_turn.origin.user_id"][..],
            ),
            ("platform_personal_bindings", &["user_id"][..]),
            (
                "platform_personal_link_challenges",
                &["reviewed_user_id"][..],
            ),
            ("platform_personal_link_intents", &["claimant_user_id"][..]),
            ("oracle_sessions", &["owner_user_id"][..]),
            ("oracle_tasks", &["submitter_user_id"][..]),
            ("oracle_worker_commands", &["created_by_user_id"][..]),
            ("oracle_workers", &["enrollment.owner_user_id"][..]),
        ] {
            let entry = find(collection).unwrap_or_else(|| panic!("missing {collection}"));
            assert_eq!(entry.owner, OwnerSelector::Fields(expected));
        }
    }

    #[test]
    fn verified_person_owner_selectors_reject_non_identity_fields() {
        let person_id_fields = BTreeSet::from([
            "active_turn.origin.user_id",
            "claimant_user_id",
            "created_by_user_id",
            "owner_user_id",
            "public_policy.sponsor_owner_user_id",
            "reviewed_user_id",
            "submitter_user_id",
            "user_id",
            "enrollment.owner_user_id",
        ]);
        for entry in ENTRIES {
            let OwnerSelector::Fields(fields) = entry.owner else {
                continue;
            };
            let checked = matches!(
                entry.collection,
                "platform_setups"
                    | "platform_bot_installations"
                    | "platform_personal_stop_effects"
                    | "platform_personal_bindings"
                    | "platform_personal_link_challenges"
                    | "platform_personal_link_intents"
                    | "oracle_sessions"
                    | "oracle_tasks"
                    | "oracle_worker_commands"
                    | "oracle_workers"
            );
            if checked {
                for field in fields {
                    assert!(
                        person_id_fields.contains(field),
                        "{} uses non-person owner selector {}",
                        entry.collection,
                        field
                    );
                }
            }
        }
    }

    #[test]
    fn registry_has_no_empty_owner_selector_fields() {
        for entry in ENTRIES {
            if let OwnerSelector::Fields(fields) = entry.owner {
                assert!(
                    !fields.is_empty(),
                    "{} must use NotPersonal or a concrete owner field",
                    entry.collection
                );
            }
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
            // The storage model is in services::openclaw_channel_service.
            "nyxid_user_id".to_owned(),
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
    fn additional_export_views_have_registered_sources_and_unique_selectors() {
        let mut selectors = BTreeSet::new();
        let mut subsections = BTreeSet::new();
        for view in ADDITIONAL_EXPORT_VIEWS {
            let source =
                find(view.collection).expect("additional export source must be registered");
            if view.selector == ExportViewSelector::AuditMetadata {
                assert_eq!(source.erasure, Erasure::Retained(RetainedCategory::Audit));
            } else {
                assert_eq!(source.export, Export::Section(view.section));
            }
            assert!(
                selectors.insert((view.collection, view.selector)),
                "duplicate export view selector"
            );
            assert!(
                subsections.insert((view.section, view.subsection)),
                "duplicate export view subsection"
            );
            match view.selector {
                ExportViewSelector::AuditMetadata => {
                    assert_eq!(view.collection, "audit_log");
                }
                ExportViewSelector::AuthoredGroupMessagesOutsideOwnedGroups => {
                    assert_eq!(
                        view.collection,
                        crate::models::assistant_group::MESSAGES_COLLECTION_NAME
                    );
                    assert_eq!(
                        (view.section, view.subsection),
                        ("assistant", "authored_group_messages")
                    );
                }
            }
        }
        assert_eq!(
            selectors.len(),
            2,
            "authored group-message export view is missing"
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
