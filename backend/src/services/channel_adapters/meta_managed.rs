//! Dedicated app configuration; never shares WhatsApp or general Facebook OAuth.
use super::meta::{GRAPH_VERSION, Inbox};
use crate::services::channel_managed::*;

pub fn descriptor(inbox: Inbox) -> PlatformCredentialDescriptor {
    PlatformCredentialDescriptor {
        provider: inbox.id(),
        label: match inbox {
            Inbox::Messenger => "Messenger",
            Inbox::Instagram => "Instagram",
        },
        backing: PlatformCredentialBacking::Stored,
        fields: &[
            PlatformCredentialField {
                name: "app_id",
                label: "OAuth application ID",
                secret: false,
                required: true,
                numeric: true,
                help: "Dedicated Messenger or Instagram Login application ID.",
            },
            PlatformCredentialField {
                name: "client_secret",
                label: "OAuth application secret",
                secret: true,
                required: true,
                numeric: false,
                help: "Secret for this channel's login application.",
            },
            PlatformCredentialField {
                name: "webhook_signing_secret",
                label: "Webhook signing secret",
                secret: true,
                required: true,
                numeric: false,
                help: "Secret whose raw-body signatures were verified on a test event. May differ from the Instagram Login secret.",
            },
            PlatformCredentialField {
                name: "verify_token",
                label: "Webhook Verify Token",
                secret: true,
                required: true,
                numeric: false,
                help: "Configure this channel's shared callback with this token in Meta.",
            },
        ],
        webhook_secret_field: None,
        setup_checklist: &[
            "Pin Graph v25.0 and use this channel's dedicated app; do not change the WhatsApp Embedded Signup app.",
            "Configure the shared callback and verify a real raw-body signature using an app-role test asset.",
            "Subscribe to messages and messaging handover/control fields. Verify exclusive routing before enabling an inbox.",
            "Messenger: configure this app as the Page default app and organic/social conversation routing app so thread_owner?recipient=PSID can prove ownership. Business Suite and other inbox apps may take control by replying or moving a conversation.",
            "Messenger needs pages_show_list, pages_manage_metadata and pages_messaging. Instagram Login needs instagram_business_basic and instagram_business_manage_messages.",
            "Advanced Access, business verification, privacy policy and deletion callbacks are required before public availability.",
        ],
    }
}

pub fn onboarding(inbox: Inbox) -> ManagedOnboardingDescriptor {
    ManagedOnboardingDescriptor {
        flow: "meta_oauth",
        provider: inbox.id(),
        bootstrap_fields: &[],
        completion_fields: &["asset_id", "exclusive_routing_verified"],
        graph_version: GRAPH_VERSION,
        signup_version: "",
        signup_extras: |_| serde_json::json!({}),
        feature_types: &[],
    }
}

pub async fn enrolled(
    db: &mongodb::Database,
    actor: &str,
    platform: &str,
) -> crate::errors::AppResult<bool> {
    if !super::meta::is_meta(platform) {
        return Ok(true);
    }
    crate::services::feature_flag_service::personal_flag_enabled(
        db,
        actor,
        &format!("channels:{platform}"),
    )
    .await
}

pub async fn require_enrollment(
    db: &mongodb::Database,
    actor: &str,
    platform: &str,
) -> crate::errors::AppResult<()> {
    if !enrolled(db, actor, platform).await? {
        return Err(crate::errors::AppError::Forbidden(
            "This channel is not enabled for your account".into(),
        ));
    }
    Ok(())
}
