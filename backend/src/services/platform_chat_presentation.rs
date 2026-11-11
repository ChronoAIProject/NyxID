//! The authenticated entry selects one usable control surface. Initial replies,
//! browser consent and final-claim rendering share these exact instructions.
use crate::{
    errors::AppResult,
    models::{
        platform_entry_identity::EntryPlatform,
        platform_entry_projection::{StoredEntryIdentity, entry_matches},
    },
};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChatPresentation {
    Telegram,
    Native(EntryPlatform),
}

impl ChatPresentation {
    pub(crate) fn for_entry(entry: &StoredEntryIdentity) -> AppResult<Self> {
        let manager = match entry {
            StoredEntryIdentity::TelegramV1 { telegram_bot_id } => telegram_bot_id.to_string(),
            StoredEntryIdentity::NativeV2 { platform_entry } => {
                platform_entry.platform_bot_id.clone()
            }
        };
        if !entry_matches(entry, &manager) {
            return Err(super::platform_entry_identity::refused());
        }
        Ok(match entry {
            StoredEntryIdentity::TelegramV1 { .. } => Self::Telegram,
            StoredEntryIdentity::NativeV2 { platform_entry } => {
                Self::Native(platform_entry.platform)
            }
        })
    }

    pub(crate) fn platform(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Native(EntryPlatform::Slack) => "slack",
            Self::Native(EntryPlatform::Discord) => "discord",
            Self::Native(EntryPlatform::Whatsapp) => "whatsapp",
            Self::Native(EntryPlatform::Lark) => "lark",
            Self::Native(EntryPlatform::Feishu) => "feishu",
            Self::Native(EntryPlatform::X) => "x",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Telegram => "Telegram",
            Self::Native(EntryPlatform::Slack) => "Slack",
            Self::Native(EntryPlatform::Discord) => "Discord",
            Self::Native(EntryPlatform::Whatsapp) => "WhatsApp",
            Self::Native(EntryPlatform::Lark) => "Lark",
            Self::Native(EntryPlatform::Feishu) => "Feishu",
            Self::Native(EntryPlatform::X) => "X",
        }
    }

    pub(crate) fn login(self) -> &'static str {
        match self {
            Self::Telegram => "/login",
            Self::Native(EntryPlatform::Discord) => "/nyx login",
            Self::Native(_) => "login",
        }
    }

    pub(crate) fn logout(self) -> &'static str {
        match self {
            Self::Telegram => "/logout",
            Self::Native(EntryPlatform::Discord) => "/nyx logout",
            Self::Native(_) => "logout",
        }
    }

    pub(crate) fn status(self) -> &'static str {
        match self {
            Self::Telegram => "/status",
            Self::Native(EntryPlatform::Discord) => "/nyx status",
            Self::Native(_) => "status",
        }
    }

    pub(crate) fn help(self) -> &'static str {
        match self {
            Self::Telegram => "/help",
            Self::Native(EntryPlatform::Discord) => "/nyx help",
            Self::Native(_) => "help",
        }
    }

    pub(crate) fn confirmation(self, code: &str) -> AppResult<Zeroizing<String>> {
        let bytes = code.as_bytes();
        if bytes.len() != 11
            || !code.starts_with("P-")
            || bytes[6] != b'-'
            || !bytes[2..6]
                .iter()
                .chain(&bytes[7..])
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(super::platform_entry_identity::refused());
        }
        Ok(Zeroizing::new(match self {
            Self::Telegram => format!("/confirm {code}"),
            Self::Native(EntryPlatform::Discord) => format!("/nyx confirm code:{code}"),
            Self::Native(_) => format!("confirm {code}"),
        }))
    }

    pub(crate) fn question_instruction(self) -> &'static str {
        match self {
            Self::Native(EntryPlatform::Discord) => {
                "Use /nyx ask with the question option in this bot's direct message. For example: /nyx ask question:\"What is NyxID?\"."
            }
            Self::Native(EntryPlatform::Slack) => {
                "Send ordinary text in this bot's direct message. Send login, status, logout or help as plain messages, without a slash."
            }
            _ => "Send ordinary text in this bot's private chat.",
        }
    }

    pub(crate) fn transport_disclosure(self) -> &'static str {
        match self {
            Self::Telegram => {
                "This Telegram bot is operated by the platform. Telegram bot chats are not end-to-end encrypted."
            }
            _ => {
                "This bot is operated by the platform. The chat provider and platform process messages and action confirmations; use NyxID to review sensitive work."
            }
        }
    }

    pub(crate) fn action_instruction(self, code: &str) -> AppResult<String> {
        if code.is_empty()
            || code.len() > 32
            || !code.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(super::platform_entry_identity::refused());
        }
        Ok(match self {
            Self::Native(EntryPlatform::Discord) => format!(
                "Use /nyx ask question:\"yes {code}\" to allow or /nyx ask question:\"no {code}\" to deny."
            ),
            _ => format!("Reply yes {code} to allow or no {code} to deny."),
        })
    }

    pub(crate) fn text_limit(self) -> usize {
        match self {
            Self::Native(EntryPlatform::Discord) => 2000,
            _ => 4096,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_controls_are_explicit_and_telegram_bytes_stay_exact() {
        let telegram = ChatPresentation::Telegram;
        assert_eq!(
            telegram.confirmation("P-ABCD-EFGH").unwrap().as_str(),
            "/confirm P-ABCD-EFGH"
        );
        let slack = ChatPresentation::Native(EntryPlatform::Slack);
        assert_eq!(slack.login(), "login");
        assert_eq!(
            slack.confirmation("P-ABCD-EFGH").unwrap().as_str(),
            "confirm P-ABCD-EFGH"
        );
        assert!(slack.question_instruction().contains("without a slash"));
        let discord = ChatPresentation::Native(EntryPlatform::Discord);
        assert_eq!(
            discord.confirmation("P-ABCD-EFGH").unwrap().as_str(),
            "/nyx confirm code:P-ABCD-EFGH"
        );
        assert_eq!(
            discord.action_instruction("ABC123").unwrap(),
            "Use /nyx ask question:\"yes ABC123\" to allow or /nyx ask question:\"no ABC123\" to deny."
        );
        assert_eq!(discord.text_limit(), 2000);
        for invalid in [
            "",
            "P-ABCD-EFGHI",
            "P-abcD-EFGH",
            "P-ABCD EFGH",
            "P-ABCD-EFGH\n",
        ] {
            assert!(discord.confirmation(invalid).is_err());
        }
    }
}
