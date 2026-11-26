use super::*;
use crate::handlers::{admin_sa_providers, auth_agent_key, devices, user_tokens};
use crate::services::{auth_agent_key_login_service, device_code_service};
use serde_json::json;

const DEVICE: &str = "fixture-private-device";
const USER: &str = "fixture-private-user";
const COMPLETE: &str = "https://example.test/login?code=fixture-private-web";
const APP: &str = "https://app.example.test/login?user_code=fixture-private-app";

fn assert_redacted(value: &impl std::fmt::Debug) {
    for output in [format!("{value:?}"), format!("{value:#?}")] {
        assert!(
            !output.contains("fixture-private"),
            "{}",
            std::any::type_name_of_val(value)
        );
    }
}

#[test]
fn account_login_request_and_delivery_codes() {
    let response = AuthDeviceRequestResponse {
        device_code: DEVICE.into(),
        user_code: USER.into(),
        verification_uri: "https://example.test/login".into(),
        verification_uri_complete: COMPLETE.into(),
        app_verification_uri: Some(APP.into()),
        expires_in: 600,
        interval: 5,
    };
    assert_redacted(&response);
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "device_code": DEVICE, "user_code": USER, "verification_uri": "https://example.test/login",
            "verification_uri_complete": COMPLETE, "app_verification_uri": APP, "expires_in": 600, "interval": 5
        })
    );
    assert_redacted(&AuthDevicePollBody {
        device_code: DEVICE.into(),
    });
    assert_redacted(&AuthDeviceApproveBody {
        user_code: USER.into(),
        selection: None,
        credential_expires_at: None,
    });
    assert_redacted(&AuthDevicePreviewBody {
        user_code: USER.into(),
    });
    assert_redacted(&AuthDeviceDenyBody {
        user_code: USER.into(),
    });
    assert_redacted(&auth_device_service::InitiateOutput {
        device_code: DEVICE.into(),
        user_code: USER.into(),
        expires_in: 600,
        interval: 5,
    });
    assert_redacted(&auth_device_service::ApproveInput {
        user_code: USER.into(),
        user_id: "owner".into(),
        approver_ip: None,
        approver_user_agent: None,
    });
    assert_redacted(&auth_device_service::DenyInput {
        user_code: USER.into(),
        user_id: "owner".into(),
        denier_ip: None,
        denier_user_agent: None,
    });
}

#[test]
fn agent_key_and_device_binding_codes() {
    let output = auth_agent_key_login_service::RequestOutput {
        device_code: DEVICE.into(),
        user_code: USER.into(),
        expires_in: 60,
        interval: 5,
    };
    assert_redacted(&output);
    let response = auth_agent_key::RequestResponse {
        request: output,
        verification_uri: "https://example.test/login".into(),
    };
    assert_redacted(&response);
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "device_code": DEVICE, "user_code": USER, "expires_in": 60, "interval": 5, "verification_uri":"https://example.test/login"
        })
    );
    assert_redacted(&auth_agent_key::CodeBody {
        user_code: USER.into(),
    });
    assert_redacted(&auth_agent_key::PollBody {
        device_code: DEVICE.into(),
    });
    assert_redacted(&auth_agent_key::ApproveBody {
        user_code: USER.into(),
        selection: auth_agent_key_login_service::Selection::Existing {
            api_key_id: "key".into(),
            permission_snapshot: None,
        },
        credential_expires_at: None,
    });
    assert_redacted(&devices::PollDeviceCodeRequest {
        device_code: DEVICE.into(),
        timestamp: 123,
        signature: "fixture-private-signature".into(),
    });
    assert_redacted(
        &serde_json::from_value::<devices::ApproveDeviceCodeRequest>(json!({"user_code": USER}))
            .unwrap(),
    );
    let response = device_code_service::DeviceCodeInitiate {
        device_code: DEVICE.into(),
        user_code: USER.into(),
        verification_uri: "https://example.test/login".into(),
        verification_uri_complete: COMPLETE.into(),
        expires_in: 60,
        poll_interval: 5,
    };
    assert_redacted(&response);
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({"device_code":DEVICE,"user_code":USER,"verification_uri":"https://example.test/login","verification_uri_complete":COMPLETE,"expires_in":60,"poll_interval":5})
    );
    assert_redacted(&device_code_service::DeviceCodePollInput {
        device_code: DEVICE.into(),
        timestamp: 123,
        signature: [42; 64],
    });
    assert_redacted(&device_code_service::DeviceCodeApproveInput {
        user_code: USER.into(),
        org_id: None,
        label: None,
        default_services: None,
        allow_auto_connected_services: false,
    });
    assert_redacted(&device_code_service::DeviceCodePoll::Pending {
        current_user_code: USER.into(),
        interval: 5,
    });
}

#[test]
fn provider_device_codes_and_poll_state() {
    let response = user_tokens::DeviceCodeInitiateResponse {
        user_code: USER.into(),
        verification_uri: COMPLETE.into(),
        state: DEVICE.into(),
        expires_in: 60,
        interval: 5,
    };
    let admin = admin_sa_providers::AdminSaDeviceCodeInitiateResponse {
        user_code: USER.into(),
        verification_uri: COMPLETE.into(),
        state: DEVICE.into(),
        expires_in: 60,
        interval: 5,
    };
    assert_redacted(&response);
    assert_redacted(&admin);
    assert_redacted(&user_tokens::DeviceCodePollRequest {
        state: DEVICE.into(),
    });
    assert_redacted(&admin_sa_providers::AdminSaDeviceCodePollRequest {
        state: DEVICE.into(),
    });
    let expected = json!({"user_code":USER,"verification_uri":COMPLETE,"state":DEVICE,"expires_in":60,"interval":5});
    assert_eq!(serde_json::to_value(response).unwrap(), expected);
    assert_eq!(serde_json::to_value(admin).unwrap(), expected);
}
