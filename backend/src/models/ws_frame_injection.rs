use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameTrigger {
    FirstFrameFromDownstream,
    JsonFieldEquals {
        path: String,
        value: serde_json::Value,
    },
    FrameIndexFromDownstream {
        index: usize,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameKind {
    Text,
    Binary,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameDirection {
    Downstream,
    Upstream,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WsFrameInjection {
    pub trigger: WsFrameTrigger,
    /// Literal frame template. The only supported interpolation is
    /// `${credential}`.
    pub template: String,
    #[serde(default = "default_frame_kind_text")]
    pub frame_kind: WsFrameKind,
    #[serde(default = "default_true")]
    pub consume_trigger: bool,
    #[serde(default = "default_direction_downstream")]
    pub direction: WsFrameDirection,
}

pub fn default_frame_kind_text() -> WsFrameKind {
    WsFrameKind::Text
}

pub fn default_direction_downstream() -> WsFrameDirection {
    WsFrameDirection::Downstream
}

pub fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_serde_roundtrip() {
        let triggers = vec![
            WsFrameTrigger::FirstFrameFromDownstream,
            WsFrameTrigger::JsonFieldEquals {
                path: "type".to_string(),
                value: serde_json::json!("auth"),
            },
            WsFrameTrigger::FrameIndexFromDownstream { index: 0 },
        ];
        for trigger in triggers {
            let json = serde_json::to_string(&trigger).unwrap();
            let back: WsFrameTrigger = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }

    #[test]
    fn frame_kind_serde() {
        assert_eq!(
            serde_json::to_string(&WsFrameKind::Text).unwrap(),
            "\"text\""
        );
        assert_eq!(
            serde_json::to_string(&WsFrameKind::Binary).unwrap(),
            "\"binary\""
        );
    }

    #[test]
    fn direction_serde() {
        assert_eq!(
            serde_json::to_string(&WsFrameDirection::Downstream).unwrap(),
            "\"downstream\""
        );
        assert_eq!(
            serde_json::to_string(&WsFrameDirection::Upstream).unwrap(),
            "\"upstream\""
        );
    }

    #[test]
    fn injection_serde_roundtrip() {
        let injection = WsFrameInjection {
            trigger: WsFrameTrigger::FirstFrameFromDownstream,
            template: r#"{"auth":"${credential}"}"#.to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction: WsFrameDirection::Downstream,
        };
        let json = serde_json::to_string(&injection).unwrap();
        let back: WsFrameInjection = serde_json::from_str(&json).unwrap();
        assert_eq!(back.template, injection.template);
        assert_eq!(back.frame_kind, WsFrameKind::Text);
        assert!(back.consume_trigger);
        assert_eq!(back.direction, WsFrameDirection::Downstream);
    }

    #[test]
    fn defaults() {
        assert_eq!(default_frame_kind_text(), WsFrameKind::Text);
        assert_eq!(default_direction_downstream(), WsFrameDirection::Downstream);
        assert!(default_true());
    }

    #[test]
    fn injection_defaults_applied_on_missing_fields() {
        let json = r#"{"trigger":"first_frame_from_downstream","template":"test"}"#;
        let injection: WsFrameInjection = serde_json::from_str(json).unwrap();
        assert_eq!(injection.frame_kind, WsFrameKind::Text);
        assert!(injection.consume_trigger);
        assert_eq!(injection.direction, WsFrameDirection::Downstream);
    }
}
