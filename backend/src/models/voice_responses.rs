//! Native task execution and token settlement, with the existing BSON wire
//! format retained for rolling reads and recovery.
use super::service_billing::PlatformUsage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
type Usage = BTreeMap<String, PlatformUsage>;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(try_from = "Stored", into = "Stored")]
pub enum ResponsesState {
    #[default]
    Disabled,
    Planned {
        model: String,
    },
    Active {
        model: String,
        usage: Usage,
    },
    Settled {
        model: String,
        usage: Usage,
    },
}

impl ResponsesState {
    pub fn model(&self) -> Option<&str> {
        match self {
            Self::Disabled => None,
            Self::Planned { model } | Self::Active { model, .. } | Self::Settled { model, .. } => {
                Some(model)
            }
        }
    }
    pub fn usage(&self) -> Option<&Usage> {
        match self {
            Self::Active { usage, .. } | Self::Settled { usage, .. } => Some(usage),
            _ => None,
        }
    }
    pub fn active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }
    pub fn reserved(&self) -> bool {
        matches!(self, Self::Active { .. } | Self::Settled { .. })
    }
    pub fn finalized(&self) -> bool {
        matches!(self, Self::Settled { .. })
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct Stored {
    #[serde(default)]
    responses_model: Option<String>,
    #[serde(default)]
    responses_reserved: bool,
    #[serde(default)]
    responses_finalized: bool,
    #[serde(default)]
    responses_usage: Usage,
}

impl TryFrom<Stored> for ResponsesState {
    type Error = &'static str;
    fn try_from(value: Stored) -> Result<Self, Self::Error> {
        let Stored {
            responses_model,
            responses_reserved,
            responses_finalized,
            responses_usage: usage,
        } = value;
        match (responses_model, responses_reserved, responses_finalized) {
            (None, false, false) if usage.is_empty() => Ok(Self::Disabled),
            (Some(model), false, false) if !model.trim().is_empty() && usage.is_empty() => {
                Ok(Self::Planned { model })
            }
            (Some(model), true, false) if !model.trim().is_empty() => {
                Ok(Self::Active { model, usage })
            }
            (Some(model), true, true) if !model.trim().is_empty() => {
                Ok(Self::Settled { model, usage })
            }
            _ => Err("Invalid native Voice execution/settlement state"),
        }
    }
}
impl From<ResponsesState> for Stored {
    fn from(state: ResponsesState) -> Self {
        Self {
            responses_model: state.model().map(str::to_owned),
            responses_reserved: state.reserved(),
            responses_finalized: state.finalized(),
            responses_usage: state.usage().cloned().unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_storage_accepts_valid_transitions_and_rejects_impossible_states() {
        for (reserved, finalized, valid) in [
            (false, false, true),
            (true, false, true),
            (true, true, true),
            (false, true, false),
        ] {
            let value = serde_json::json!({"responses_model":"task-model","responses_reserved":reserved,"responses_finalized":finalized});
            let parsed = serde_json::from_value::<ResponsesState>(value);
            assert_eq!(parsed.is_ok(), valid);
            if let Ok(state) = parsed {
                let bson = bson::to_document(&state).unwrap();
                let restored: ResponsesState = bson::from_document(bson).unwrap();
                assert_eq!(restored.model(), Some("task-model"));
                assert_eq!(restored.finalized(), finalized);
            }
        }
        for value in [
            serde_json::json!({"responses_reserved":true}),
            serde_json::json!({"responses_finalized":true}),
            serde_json::json!({"responses_model":" "}),
        ] {
            assert!(serde_json::from_value::<ResponsesState>(value).is_err());
        }
        assert!(matches!(
            serde_json::from_value::<ResponsesState>(serde_json::json!({})).unwrap(),
            ResponsesState::Disabled
        ));
    }
}
