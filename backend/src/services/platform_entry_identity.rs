//! Typed shared-entry identities and content commitments. This module does not
//! grant authority: live profile, sender binding and ingress fences still apply.
use crate::{
    errors::{AppError, AppResult},
    models::platform_entry_identity::{
        EntryPlatform, NativeControlSurface, NativeEntryConfiguration, NativeEntryIdentity,
        NativeEntryReceipt,
    },
};
use zeroize::Zeroizing;

pub(crate) fn refused() -> AppError {
    AppError::Forbidden("Original private platform identity is unavailable".into())
}

pub(crate) fn opaque(value: &str) -> AppResult<&str> {
    if value.is_empty()
        || value.len() > 256
        || value.chars().any(char::is_control)
        || value.trim() != value
    {
        return Err(refused());
    }
    Ok(value)
}

/// CMA channel pointers have their own bounded sealed grammar; native sender
/// and platform IDs retain the separate 256-byte opaque-ID boundary.
pub(crate) fn gateway_channel_id(value: &str) -> AppResult<&str> {
    use base64::Engine;
    if value.len() > 512 {
        return Err(refused());
    }
    let mut parts = value.split('.');
    if parts.next() != Some("cmaeg1") || parts.next() != Some("ch") {
        return Err(refused());
    }
    let kid = parts.next().ok_or_else(refused)?;
    if !kid
        .parse::<u8>()
        .is_ok_and(|parsed| parsed.to_string() == kid)
    {
        return Err(refused());
    }
    let payload = parts.next().ok_or_else(refused)?;
    if parts.next().is_some()
        || !payload
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .map_or(true, |bytes| bytes.len() < 28)
    {
        return Err(refused());
    }
    Ok(value)
}

pub(crate) fn validate_entry(entry: &NativeEntryIdentity) -> AppResult<()> {
    if entry.version != 2
        || !uuid::Uuid::parse_str(&entry.channel_bot_id).is_ok_and(|id| id.get_version_num() == 4)
    {
        return Err(refused());
    }
    opaque(&entry.platform_bot_id)?;
    Ok(())
}

pub(crate) fn validate_configuration(configuration: &NativeEntryConfiguration) -> AppResult<()> {
    validate_entry(&configuration.identity)?;
    if configuration.transport_generation <= 0
        || configuration.credential_fingerprint.len() != 64
        || !configuration
            .credential_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(refused());
    }
    match (
        &configuration.identity.platform,
        &configuration.control_surface,
    ) {
        (
            EntryPlatform::Slack,
            NativeControlSurface::SlackDirectMessage {
                app_id,
                team_id,
                bot_user_id,
            },
        ) => {
            opaque(app_id)?;
            opaque(team_id)?;
            opaque(bot_user_id)?;
        }
        (
            EntryPlatform::Discord,
            NativeControlSurface::DiscordCommand {
                application_id,
                command_id,
                command_digest,
            },
        ) => {
            opaque(application_id)?;
            opaque(command_id)?;
            if command_digest != &super::platform_entry_discord_controls::schema_digest() {
                return Err(refused());
            }
        }
        (
            EntryPlatform::Whatsapp
            | EntryPlatform::Lark
            | EntryPlatform::Feishu
            | EntryPlatform::X,
            NativeControlSurface::PrivateMessage,
        ) => {}
        _ => return Err(refused()),
    }
    if let crate::models::platform_entry_identity::EntryTransport::NyxagentGateway {
        channel_id,
        gateway_channel_id,
        gateway_version,
    } = &configuration.transport
    {
        if uuid::Uuid::parse_str(channel_id).is_err() || *gateway_version <= 0 {
            return Err(refused());
        }
        self::gateway_channel_id(gateway_channel_id)?;
    }
    Ok(())
}

pub(crate) const MAX_NATIVE_TEXT_BYTES: usize = 4096;

pub(crate) fn canonical_text(text: &str) -> AppResult<&str> {
    if text.is_empty()
        || text.len() > MAX_NATIVE_TEXT_BYTES
        || text
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(refused());
    }
    // Adapter-owned control normalization happens once before commitment. Do not
    // trim, lowercase or otherwise reinterpret ordinary or already-normalized text.
    Ok(text)
}

pub(crate) fn content_commitment(
    key: &[u8],
    receipt: &NativeEntryReceipt,
    text: &str,
) -> AppResult<String> {
    validate_configuration(&receipt.configuration)?;
    opaque(&receipt.sender.sender_id)?;
    opaque(&receipt.sender.private_conversation_id)?;
    opaque(&receipt.event_id)?;
    opaque(&receipt.message_id)?;
    canonical_text(text)?;
    if receipt.manager_authority_epoch <= 0 || !receipt.original_private_human {
        return Err(refused());
    }
    let material = Zeroizing::new(
        serde_json::to_vec(&(
            "nyxid-platform-native-content:v2",
            &receipt.profile_id,
            receipt.manager_authority_epoch,
            &receipt.configuration,
            &receipt.sender,
            &receipt.event_id,
            &receipt.message_id,
            text,
        ))
        .map_err(|_| refused())?,
    );
    Ok(super::auth_device_service::hmac_hex(key, &material))
}

pub(crate) fn verify_content(
    key: &[u8],
    receipt: &NativeEntryReceipt,
    text: &str,
) -> AppResult<()> {
    let expected = content_commitment(key, receipt, text)?;
    if !crate::crypto::token::constant_time_eq(
        expected.as_bytes(),
        receipt.content_commitment.as_bytes(),
    ) {
        return Err(refused());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        platform_entry_identity::{EntryTransport, NativeSenderIdentity},
        platform_entry_projection::{
            StoredEntryIdentity as EntryIdentity, StoredSenderIdentity as SenderIdentity,
        },
    };
    use crate::services::platform_entry_scope::validate;

    fn receipt() -> NativeEntryReceipt {
        NativeEntryReceipt {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: uuid::Uuid::new_v4().to_string(),
            manager_authority_epoch: 1,
            configuration: NativeEntryConfiguration {
                identity: NativeEntryIdentity {
                    version: 2,
                    platform: EntryPlatform::Slack,
                    channel_bot_id: uuid::Uuid::new_v4().to_string(),
                    platform_bot_id: "B-manager".into(),
                },
                transport: EntryTransport::Direct,
                transport_generation: 1,
                credential_fingerprint: "11".repeat(32),
                control_surface: NativeControlSurface::SlackDirectMessage {
                    app_id: "A-app".into(),
                    team_id: "T-team".into(),
                    bot_user_id: "U-bot".into(),
                },
            },
            sender: NativeSenderIdentity {
                sender_id: "U-person".into(),
                private_conversation_id: "D-private".into(),
            },
            event_id: "Ev-event".into(),
            message_id: "1730000000.000001".into(),
            original_private_human: true,
            content_commitment: String::new(),
            received_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        }
    }

    #[test]
    fn opaque_subject_is_distinct_from_telegram_and_mixed_projections_refuse() {
        let native = receipt().configuration.identity;
        let entry: EntryIdentity =
            serde_json::from_value(serde_json::json!({"platform_entry":&native})).unwrap();
        validate(&entry, "B-manager", None).unwrap();
        assert!(
            serde_json::from_value::<EntryIdentity>(serde_json::json!({
                "telegram_bot_id":99,"platform_entry":&native
            }))
            .is_err()
        );
        let legacy_sender = SenderIdentity::TelegramV1 {
            telegram_sender_id: 4,
            telegram_chat_id: 4,
        };
        assert!(validate(&entry, "B-manager", Some(&legacy_sender)).is_err());
        let subject = NativeSenderIdentity {
            sender_id: "opaque:user".into(),
            private_conversation_id: "different/conversation".into(),
        };
        let sender: SenderIdentity =
            serde_json::from_value(serde_json::json!({"platform_sender":&subject})).unwrap();
        assert_eq!(
            sender,
            SenderIdentity::NativeV2 {
                platform_sender: subject
            }
        );
        validate(&entry, "B-manager", Some(&sender)).unwrap();
        assert!(
            validate(
                &EntryIdentity::TelegramV1 {
                    telegram_bot_id: 99
                },
                "99",
                Some(&SenderIdentity::TelegramV1 {
                    telegram_sender_id: 4,
                    telegram_chat_id: 5,
                }),
            )
            .is_err()
        );
    }

    #[test]
    fn gateway_content_commitment_fences_command_code_text_identity_and_transport() {
        for original in ["/help", "/confirm P-ABCD-EFGH", "What does NyxID do? 中文"] {
            let mut exact = receipt();
            exact.content_commitment = content_commitment(&[7; 32], &exact, original).unwrap();
            verify_content(&[7; 32], &exact, original).unwrap();
            for replacement in ["/logout", "/confirm P-ZZZZ-ZZZZ", "Read my personal keys"] {
                assert!(verify_content(&[7; 32], &exact, replacement).is_err());
            }
            let mut changed = exact.clone();
            changed.configuration.transport_generation += 1;
            assert!(verify_content(&[7; 32], &changed, original).is_err());
            let mut changed = exact.clone();
            changed.sender.private_conversation_id.push('x');
            assert!(verify_content(&[7; 32], &changed, original).is_err());
            let mut changed = exact.clone();
            changed.configuration.identity.platform_bot_id.push('x');
            assert!(verify_content(&[7; 32], &changed, original).is_err());
            assert!(verify_content(&[8; 32], &exact, original).is_err());
        }
    }
}
