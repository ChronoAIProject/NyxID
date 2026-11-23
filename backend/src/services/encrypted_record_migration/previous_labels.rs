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
