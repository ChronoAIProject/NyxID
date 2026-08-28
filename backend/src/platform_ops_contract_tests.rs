//! Cross-language contract fixture for the platform-operations responses.
//!
//! The admin page parses `GET /api/v1/admin/platform-ops` and `/keys` parses
//! `GET /api/v1/platform-ops` with strict Zod schemas. Nothing previously tied
//! those schemas to the Rust serializers, so the frontend fixtures drifted into
//! describing a response the backend had stopped emitting: the schemas still
//! required `metric: "requests"` after `speak` moved to `characters` and
//! `call_and_say` moved to `seconds`, which made the admin page fail to parse
//! its own default state.
//!
//! This module serializes the real projection functions and pins the result to
//! a JSON file that the frontend test suite parses with the real schemas. A
//! change on either side that the other has not adopted fails a test.
//!
//! Regenerate after an intentional contract change:
//!
//! ```sh
//! UPDATE_PLATFORM_OPS_CONTRACT=1 cargo test -p nyxid --bin nyxid-server \
//!     platform_ops_contract
//! ```

use crate::handlers::admin_platform_ops::platform_operation_response;
use crate::handlers::platform_ops::{
    OwnConnectionDiscoveryResponse, platform_operation_discovery_response,
};
use crate::models::platform_operation::{
    CallAndSayConfig, FlightSearchConfig, OperationBilling, PlatformOperation,
    PlatformOperationConfig, PlatformOperationName, SpeakConfig,
};
use crate::models::platform_service_preference::CredentialIntent;
use crate::models::service_billing::{BillingMetric, PricingSyncStatus};
use crate::services::platform_operation_service::{
    PlatformCredentialSource, PlatformFallbackReason,
};

/// Path of the fixture, relative to the backend crate root. It lives in the
/// frontend tree because vitest cannot import JSON from outside its own root.
const FIXTURE_RELATIVE_PATH: &str =
    "../frontend/src/schemas/__fixtures__/platform-ops-contract.json";

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_RELATIVE_PATH)
}

fn operation(
    op: PlatformOperationName,
    config: PlatformOperationConfig,
    billing: OperationBilling,
) -> PlatformOperation {
    PlatformOperation {
        id: format!("00000000-0000-4000-8000-{:012}", op as u8),
        catalog_service_id: "catalog-service-id".to_string(),
        op,
        enabled: true,
        vendor_service_slug:
            crate::services::platform_operation_service::default_vendor_service_slug(op).to_string(),
        config,
        billing,
        billing_cleanup_metric_code: None,
        updated_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp"),
        updated_by: "admin-user-id".to_string(),
    }
}

/// One operation per priced shape the UI has to render:
/// a variable-only price, a base fee combined with a variable rate, and an
/// unpriced operation.
fn contract_operations() -> Vec<PlatformOperation> {
    vec![
        operation(
            PlatformOperationName::Speak,
            PlatformOperationConfig::Speak(SpeakConfig {
                allowed_voice_ids: vec!["voice-a".to_string()],
                max_chars: 1_000,
                model_id: "eleven_multilingual_v2".to_string(),
                max_calls_per_user_per_day: 50,
            }),
            OperationBilling {
                metric: BillingMetric::Characters,
                price_per_unit: "0.002".to_string(),
                secondary: None,
                base_fee_per_call: None,
                lago_metric_code: "platform_op_api_elevenlabs_speak".to_string(),
                sync_status: PricingSyncStatus::Synced,
                sync_error: None,
            },
        ),
        operation(
            PlatformOperationName::CallAndSay,
            PlatformOperationConfig::CallAndSay(CallAndSayConfig {
                allowed_destination_prefixes: vec!["+1".to_string()],
                max_message_chars: 500,
                max_duration_seconds: 60,
                voice: "alice".to_string(),
                max_calls_per_user_per_day: 10,
                account_sid: "AC00000000000000000000000000000000".to_string(),
                call_from: "+15550000000".to_string(),
            }),
            OperationBilling {
                metric: BillingMetric::Seconds,
                price_per_unit: "0.05".to_string(),
                secondary: None,
                base_fee_per_call: Some("1.5".to_string()),
                lago_metric_code: "platform_op_api_twilio_call_and_say".to_string(),
                sync_status: PricingSyncStatus::Failed,
                sync_error: Some("lago rejected the charge payload".to_string()),
            },
        ),
        operation(
            PlatformOperationName::FlightSearch,
            PlatformOperationConfig::FlightSearch(FlightSearchConfig {
                max_offers_cap: 20,
                max_searches_per_user_per_day: 25,
            }),
            OperationBilling::free(BillingMetric::Requests),
        ),
    ]
}

fn contract_document() -> serde_json::Value {
    let operations = contract_operations();

    let admin = crate::handlers::admin_platform_ops::AdminPlatformOperationListResponse {
        operations: operations
            .iter()
            .map(|operation| {
                platform_operation_response(operation.op, Some(operation.clone()), None)
            })
            .collect(),
    };

    // Discovery rows: the first resolves to the platform credential, the second
    // to a usable own connection, the third to a connection the caller cannot
    // use. Rollout is on so prices render rather than collapsing to "Free".
    let own_connection = OwnConnectionDiscoveryResponse {
        user_service_id: "11111111-1111-4111-8111-111111111111".to_string(),
        slug: "api-twilio".to_string(),
        label: "My Twilio".to_string(),
        is_active: true,
        usable: true,
        reason: None,
    };
    let blocked_connection = OwnConnectionDiscoveryResponse {
        user_service_id: "22222222-2222-4222-8222-222222222222".to_string(),
        slug: "duffel".to_string(),
        label: "My Duffel".to_string(),
        is_active: true,
        usable: false,
        reason: Some("approval_required"),
    };
    let sources = [
        (
            PlatformCredentialSource::Platform,
            CredentialIntent::Auto,
            None,
            Some(PlatformFallbackReason::OwnCredentialAbsent),
            None,
        ),
        (
            PlatformCredentialSource::OwnConnection,
            CredentialIntent::Auto,
            None,
            None,
            Some(own_connection),
        ),
        (
            PlatformCredentialSource::Unavailable,
            CredentialIntent::OwnOnly,
            Some("own_connection_disabled"),
            None,
            Some(blocked_connection),
        ),
    ];

    let discovery = crate::handlers::platform_ops::PlatformOperationsResponse {
        operations: operations
            .iter()
            .zip(sources)
            .map(
                |(operation, (source, intent, availability, fallback, connection))| {
                    platform_operation_discovery_response(
                        operation,
                        source,
                        intent,
                        availability,
                        fallback,
                        connection,
                        true,
                    )
                },
            )
            .collect(),
    };

    // Kept on one line: rustfmt reindents string continuations, which would
    // silently change the generated fixture's contents.
    const BANNER: &str = "Generated by backend/src/platform_ops_contract_tests.rs. Do not edit by hand; see that file for the regeneration command.";

    serde_json::json!({
        "//": BANNER,
        "admin": admin,
        "discovery": discovery,
    })
}

#[test]
fn platform_ops_contract_fixture_matches_serialized_responses() {
    let document = contract_document();
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(&document).expect("serialize contract document")
    );
    let path = fixture_path();

    if std::env::var("UPDATE_PLATFORM_OPS_CONTRACT").is_ok() {
        std::fs::create_dir_all(path.parent().expect("fixture parent"))
            .expect("create fixture directory");
        std::fs::write(&path, &rendered).expect("write contract fixture");
        return;
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing platform-ops contract fixture at {}: {error}. \
Regenerate with UPDATE_PLATFORM_OPS_CONTRACT=1",
            path.display()
        )
    });

    assert_eq!(
        existing.trim(),
        rendered.trim(),
        "platform-ops contract fixture is stale. The frontend Zod schemas parse this file, \
so a backend response change must be adopted there too. \
Regenerate with UPDATE_PLATFORM_OPS_CONTRACT=1 and update frontend/src/schemas/platform-ops.ts."
    );
}

#[test]
fn price_display_covers_every_metric_shape() {
    use crate::models::platform_operation::OperationBillingComponent;
    use crate::services::billing::pricing::format_operation_price;

    let variable_only = OperationBilling {
        metric: BillingMetric::Characters,
        price_per_unit: "0.002".to_string(),
        secondary: None,
        base_fee_per_call: None,
        lago_metric_code: String::new(),
        sync_status: PricingSyncStatus::Synced,
        sync_error: None,
    };
    assert_eq!(
        format_operation_price(&variable_only, true),
        "0.002 credits per character"
    );

    let with_base_fee = OperationBilling {
        metric: BillingMetric::Seconds,
        price_per_unit: "0.05".to_string(),
        base_fee_per_call: Some("1.5".to_string()),
        ..variable_only.clone()
    };
    assert_eq!(
        format_operation_price(&with_base_fee, true),
        "1.5 credits per call + 0.05 credits per second"
    );

    let per_call = OperationBilling {
        metric: BillingMetric::Requests,
        price_per_unit: "0.5".to_string(),
        base_fee_per_call: None,
        ..variable_only.clone()
    };
    assert_eq!(
        format_operation_price(&per_call, true),
        "0.5 credits per call"
    );

    // An operation with no configured price must not render as if it were free:
    // "Free" is a billing decision, an unset price is a configuration gap.
    let unpriced = OperationBilling::free(BillingMetric::Requests);
    assert_eq!(format_operation_price(&unpriced, true), "Price not set");
    assert_eq!(format_operation_price(&per_call, false), "Free");

    let split_tokens = OperationBilling {
        metric: BillingMetric::InputTokens,
        price_per_unit: "0.01".to_string(),
        secondary: Some(OperationBillingComponent {
            metric: BillingMetric::OutputTokens,
            price_per_unit: "0.03".to_string(),
            lago_metric_code: "output".to_string(),
        }),
        base_fee_per_call: Some("0.5".to_string()),
        ..variable_only
    };
    assert_eq!(
        format_operation_price(&split_tokens, true),
        "0.5 credits per call + 0.01 credits per input token + 0.03 credits per output token"
    );
}
