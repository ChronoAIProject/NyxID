//! Report labels indexed by checkpoints written under the previous registry digest.
pub(super) const DIGEST: &str = "cac28fc0711185b79f9a95ee65da056c67f87fdd703e789e058fc3a7a30e5beb";

pub(super) const LABELS: &[super::FieldLabels] = &[
    (
        "user_provider_tokens",
        &[
            "access_token_encrypted",
            "refresh_token_encrypted",
            "api_key_encrypted",
        ],
    ),
    (
        "user_api_keys",
        &[
            "credential_encrypted",
            "access_token_encrypted",
            "refresh_token_encrypted",
            "user_oauth_client_id_encrypted",
            "user_oauth_client_secret_encrypted",
        ],
    ),
    (
        "user_provider_credentials",
        &["client_id_encrypted", "client_secret_encrypted"],
    ),
    (
        "provider_configs",
        &["client_id_encrypted", "client_secret_encrypted"],
    ),
    ("platform_credentials", &["secrets"]),
    ("nodes", &["signing_secret_encrypted"]),
    ("mfa_factors", &["secret_encrypted"]),
    (
        "saved_logins",
        &[
            "username_encrypted",
            "password_encrypted",
            "totp_secret_encrypted",
        ],
    ),
    (
        "oauth_states",
        &[
            "code_verifier",
            "device_code_encrypted",
            "user_code_encrypted",
        ],
    ),
    ("assistant_agent_credentials", &["key_ciphertext"]),
    (
        "assistant_attachments",
        &["text_encrypted", "data_encrypted"],
    ),
    ("assistant_tool_results", &["data_encrypted"]),
    (
        "platform_public_sessions",
        &["credential_ciphertext", "upstream_binding_ciphertext"],
    ),
    (
        "platform_public_turns",
        &["input_ciphertext", "reply_ciphertext"],
    ),
    ("downstream_services", &["default_request_headers"]),
    ("user_services", &["default_request_headers"]),
    ("personal_erasure_effects", &["capability_encrypted"]),
    (
        "agent_key_login_requests",
        &["delivery_credential_encrypted"],
    ),
    (
        "assistant_agent_learning_proposals",
        &[
            "scope_encrypted",
            "body_encrypted",
            "publication.interface_encrypted",
        ],
    ),
    ("assistant_async_operations", &["result_encrypted"]),
    (
        "auth_device_codes",
        &[
            "delivery_access_token_encrypted",
            "delivery_refresh_token_encrypted",
        ],
    ),
    ("auth_device_codes_v2", &["delivery_credential_encrypted"]),
    (
        "device_codes",
        &[
            "delivery_api_key_encrypted",
            "delivery_refresh_token_encrypted",
        ],
    ),
    (
        "channel_bots",
        &[
            "registration_pin_encrypted",
            "webhook_secret_encrypted",
            "bot_token_encrypted",
            "app_secret_encrypted",
            "lark_verification_token_encrypted",
            "lark_encrypt_key_encrypted",
        ],
    ),
    ("channel_connect_links", &["receiver.secret_encrypted"]),
    ("downstream_services", &["credential_encrypted"]),
    (
        "downstream_services",
        &["ssh_config.ca_private_key_encrypted"],
    ),
    ("nyxbot_channels", &["agent_key_ciphertext"]),
    ("nyxbot_threads", &["event_ref_ciphertext"]),
    (
        "nyxbot_events",
        &["event_ref_ciphertext", "event_context_ciphertext"],
    ),
    ("nyxbot_watches", &["target_ciphertext"]),
    ("oauth_broker_bindings", &["refresh_token_encrypted"]),
    (
        "oauth_clients",
        &[
            "revocation_webhook_secret_encrypted",
            "connection_webhook_secret_encrypted",
        ],
    ),
    ("oracle_login_profiles", &["encrypted_envelope"]),
    ("oracle_login_snapshots", &["encrypted_envelope"]),
    ("platform_entry_gateway_transports", &["request_ciphertext"]),
    ("platform_native_reply_handles", &["ciphertext"]),
    ("platform_setups", &["facts_ciphertext"]),
    (
        "triggers",
        &["verification_secret_encrypted", "delivery_secret_encrypted"],
    ),
    ("trigger_deliveries", &["envelope_encrypted"]),
    ("user_service_connections", &["credential_encrypted"]),
    ("async_service_operation_artifacts", &["data_encrypted"]),
];

// #2896 replaced three entries and appended seven. Keep this view independent
// of future live-registry edits so retained counters keep their original names.
pub(super) const RECONCILED_DIGEST: &str =
    "97d55ec8edff7d0599c59dc342e6a46fb5d580650d4be4b6e9cad693773d3b4c";
// #2897 (0fcb789bf) changed the OAuth webhook adapter's entry contract without
// changing any report labels. The Meta consent entry subsequently changed the
// live digest; retain the exact former digest against this frozen label view.
pub(super) const OAUTH_WEBHOOK_DIGEST: &str =
    "b87c6a8c242ec34ae73c327f2ff7ed8ab332f07985afb8ed50ce2693d6270892";
pub(super) fn reconciled() -> impl Iterator<Item = super::FieldLabels> {
    const ADDED: &[super::FieldLabels] = &[
        ("assistant_cma_sandboxes", &["request_ciphertext"]),
        ("assistant_cma_creations", &["request_ciphertext"]),
        (
            "nyxbot_event_bindings",
            &["provider_key_ciphertext", "producer_secret_ciphertext"],
        ),
        ("nyxbot_gateway_responses", &["payload_ciphertext"]),
        (
            "channel_whatsapp_native",
            &["native_verify_token_encrypted"],
        ),
        (
            "assistant_conversations",
            &["active_turn.restart_admission.window.input"],
        ),
        (
            "approval_requests",
            &[
                "execution_proposal.v1.payload",
                "execution_proposal.v1.result",
                "execution_proposal.v2.payload",
                "execution_proposal.v2.result",
                "execution_proposal.v3.payload",
                "execution_proposal.v3.result",
                "execution_proposal.v4.payload",
                "execution_proposal.v4.result",
            ],
        ),
    ];
    LABELS
        .iter()
        .copied()
        .enumerate()
        .map(|(index, entry)| -> super::FieldLabels {
            match index {
                21 => (
                    "auth_device_codes_v2",
                    &[
                        "agent_key_grant.delivery_credential_encrypted",
                        "delivery_access_token_encrypted",
                        "delivery_refresh_token_encrypted",
                    ],
                ),
                29 => (
                    "nyxbot_events",
                    &[
                        "delivery.target_ciphertext",
                        "event_context_ciphertext",
                        "guest_setup_payload",
                    ],
                ),
                30 => ("mobile_social_deliveries", &["ciphertext"]),
                _ => entry,
            }
        })
        .chain(ADDED.iter().copied())
}

pub(super) const CHANNEL_BOT_DIGEST: &str =
    "fdb9b9bb2bba7447d7cef07e588e6c1d1ce9461af13bd17be7664d0a47dc1f12";

pub(super) const WHATSAPP_DIGEST: &str =
    "6c328214696d6e4cdbb50c0b9aa5ef852c20e6e480fd2203ed6002b48477a50b";

pub(super) const CHANNEL_KEY_DIGEST: &str =
    "417aa700dd5f5ce2adc8bc54a7c146f2b9b57e655c90c0e9a7fe642e4a9931d9";

pub(super) const EVENT_BINDING_DIGEST: &str =
    "98ef7eabeac5b92e4bc6f7c24b087b3b68d54fad47ac7ddf64c6eecdbe665108";

// These Meta registries share the reconciled field order with consent assets appended.
pub(super) const META_DIGESTS: [&str; 9] = [
    "8d3f9c267aa1346a94a79c061d35cbf8c0e844fba6a1d9144869116dcab42369", // #2902 on dev
    "4c651a86c5f4450b911a8052544564ca6659040d1cc429064a5fea1d2f0cb016", // 73318f7de
    "9a784b1659734c401dbb632bfda793658c1bd612819bef626a622bc87fb92189", // e97018750
    "dadee8cea3c5465d228d975ba5133278775891c5fbe31a537c972b2f9e0b92de", // f344f1f8e
    "ff0de0b1e3eb99c878d1bb06cec2c12df0ff618999171e584e13517a0ca2e4ed",
    "318c040e60d81d2b65df44fd6f94ce59bdafffebbd5b34860e80219891dd5241", // e42c02795
    "efd95c721ac4c351ee513eeee27edc75bc7fbee3da56497080c7bfaa4e42cc96", // a40f49119
    "0395aa4f3f1f25d653011ab735909a1c551f37fcfc3f50934b974b1189241a4a", // 5014e6111
    "bf6341a74abf17a8a1fe0f2b0425c6275553ed7d47824d9b869ae7008c6d0abe", // 96386ba2b
];

pub(super) const GATEWAY_SETUP_DIGEST: &str =
    "1d88cdb63f4b00b32616b0cb070acad44328105bbc9a1b7fd7f60959ef3f2d2d";

pub(super) const CMA_REQUEST_DIGEST: &str =
    "2586a1add34bc8bf42b81e020c60c6d2d14d4b3567cb29c2c8bf976d5100a99e";

pub(super) const LEARNING_PROPOSAL_DIGEST: &str =
    "3ba8ae303803460a8c0deaf93a19c637feffce97fc25e4887e2a2f5b86d39f51";

pub(super) const APPROVAL_PROPOSAL_DIGEST: &str =
    "89dd069d90525725f8d136619f99428d2f040d5176f66c9ef009d640e858c67c";

pub(super) const REATTACH_WINDOW_DIGEST: &str =
    "68bf534769f813c488cb4a87da6cf374632bede78e8d57b924cef52aecb2c78f";

pub(super) const THREAD_EVENT_DIGEST: &str =
    "5dfa37490ffad3c3a6cc3eb3fbe19ca6adf2c2291d78cc5d1c998a815055fb42";

pub(super) const EVENT_REPLY_DRAFT_DIGEST: &str =
    "1c55ec996722c504e1e0fad8cc55a31a178a515945c322484f8e1115ad59c649";

pub(super) fn event_reply_draft() -> impl Iterator<Item = super::FieldLabels> {
    reconciled().map(|entry| -> super::FieldLabels {
        if entry.0 == "nyxbot_events" {
            (
                entry.0,
                &[
                    "delivery.target_ciphertext",
                    "event_context_ciphertext",
                    "guest_setup_payload",
                    "response_reply.ciphertext",
                ],
            )
        } else {
            entry
        }
    })
}

pub(super) const EVENT_REPLY_DIGEST: &str =
    "e14a5e14ec97244a2d0df86ec47fd2023ead5f11257b2887521c04095138e75c";
pub(super) const META_ASSETS: super::FieldLabels =
    ("channel_meta_oauth_sessions", &["assets_encrypted"]);

pub(super) const EVENT_WRITERS_DIGEST: &str =
    "b1632ff0b29289fc5b9e88653fbce4769d3f778fbcfa4ed8494960ccf38d3ada";

pub(super) const DEVICE_BASE_DIGEST: &str =
    "3dccc8a1829fcd079496272ca807f3549396e93e611bb013f0a2ba9d1efa8513";

pub(super) const NODE_DEVICE_BASE_DIGEST: &str =
    "90d8a86835621035aac28bbb0e064ef45c90b76e3572949f4c41917c5ed9323a";

pub(super) const NATIVE_REPLY_BASE_DIGEST: &str =
    "c9160176c797bce8a685fbe1525b33093a611b56b2d9f0568523c478b3f57e50";

pub(super) const MOBILE_SOCIAL_BASE_DIGEST: &str =
    "72058b48a651932bcd6db8e41c61501152380812f83bdcbdd31d1273a982126b";

pub(super) const META_CONSENT_BASE_DIGEST: &str =
    "72d9fc2dca536136c4a9a4ae34ee542299ec7c517cae2fa5f039912f2a95169d";

pub(super) const BEFORE_CHANNEL_GUEST_DIGEST: &str =
    "c526844b3d17aa7a1211f84b3e167ffa92c1e7fae23853506517fc966a0a8dae";
