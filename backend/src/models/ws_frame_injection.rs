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
