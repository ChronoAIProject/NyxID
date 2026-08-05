#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAuthFlowKind {
    ChatConnect,
    KeyConnect,
    ProviderConnect,
    ServiceAccountConnect,
    WizardConnect,
    SocialLogin,
}

impl OAuthFlowKind {
    pub const ALL_WIRE_TOKENS: [&'static str; 6] = ["cc", "kc", "pc", "sa", "wz", "sl"];

    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::ChatConnect => "cc",
            Self::KeyConnect => "kc",
            Self::ProviderConnect => "pc",
            Self::ServiceAccountConnect => "sa",
            Self::WizardConnect => "wz",
            Self::SocialLogin => "sl",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        if !Self::ALL_WIRE_TOKENS.contains(&token) {
            return None;
        }
        match token {
            "cc" => Some(Self::ChatConnect),
            "kc" => Some(Self::KeyConnect),
            "pc" => Some(Self::ProviderConnect),
            "sa" => Some(Self::ServiceAccountConnect),
            "wz" => Some(Self::WizardConnect),
            "sl" => Some(Self::SocialLogin),
            _ => unreachable!("all known OAuth flow tokens are matched"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OAuthFlowKind;

    #[test]
    fn wire_round_trip() {
        let variants = [
            OAuthFlowKind::ChatConnect,
            OAuthFlowKind::KeyConnect,
            OAuthFlowKind::ProviderConnect,
            OAuthFlowKind::ServiceAccountConnect,
            OAuthFlowKind::WizardConnect,
            OAuthFlowKind::SocialLogin,
        ];
        for variant in variants {
            assert_eq!(OAuthFlowKind::parse(variant.as_wire()), Some(variant));
        }
    }

    #[test]
    fn parse_rejects_unknown_tokens() {
        for token in ["", "CC", "chat", "xx"] {
            assert_eq!(OAuthFlowKind::parse(token), None);
        }
    }

    #[test]
    fn wire_tokens_are_pinned() {
        assert_eq!(
            OAuthFlowKind::ALL_WIRE_TOKENS,
            ["cc", "kc", "pc", "sa", "wz", "sl"]
        );
    }
}
