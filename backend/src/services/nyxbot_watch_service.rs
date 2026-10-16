//! Report provenance for NyxBot watches (`nyxbot_watches`): taken from the
//! exact chat that caused the watch, never looked up again, so a replacement
//! turn is never captured.
use crate::models::{
    assistant_conversation::{AgentEvent, REPORT_PROVENANCE_V1, carried_report_to},
    nyxbot_channel::NyxbotWatch,
};
use mongodb::bson::{self, doc};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportProvenance {
    /// The causing turn's `ActiveTurn::report_to`, meaningful only with
    /// `version == REPORT_PROVENANCE_V1`.
    pub report_to: Option<String>,
    /// 0 = legacy, nothing proven; 1 = validated report snapshot.
    pub version: u8,
}

impl ReportProvenance {
    pub const LEGACY: Self = Self {
        report_to: None,
        version: 0,
    };

    /// Never re-read from the conversation, so a replacement turn cannot be
    /// captured.
    pub fn from_chat(chat: &super::assistant_acknowledgement_service::ChatAuthority) -> Self {
        if chat.turn_live && !chat.turn_stopped {
            Self {
                report_to: chat.report_to.clone(),
                version: REPORT_PROVENANCE_V1,
            }
        } else {
            Self::LEGACY
        }
    }

    pub fn from_watch(watch: &NyxbotWatch) -> Self {
        if watch.report_provenance == REPORT_PROVENANCE_V1 {
            Self {
                report_to: watch.report_to.clone(),
                version: REPORT_PROVENANCE_V1,
            }
        } else {
            Self::LEGACY
        }
    }

    pub fn fields(&self) -> bson::Document {
        doc! {"report_to": self.report_to.clone(), "report_provenance": i32::from(self.version)}
    }

    pub fn carried(&self) -> Option<String> {
        carried_report_to(self.report_to.as_deref(), self.version)
    }
}

pub fn watch_event(watch: &NyxbotWatch, kind: &str, text: String) -> AgentEvent {
    let event = super::assistant_team_service::event(kind, text, None);
    let provenance = ReportProvenance::from_watch(watch);
    if provenance.version == REPORT_PROVENANCE_V1 {
        event.bound_to(provenance.carried().as_deref())
    } else {
        event
    }
}
