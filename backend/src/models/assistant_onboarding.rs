use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_onboarding";

/// The current first-run intro. Bump to show a revised intro once to people
/// who finished an older one.
pub const INTRO_VERSION: i32 = 1;

/// What the owner said NyxBot should help with. A closed set: answers drive
/// curated recommendations and one NyxBot memory note, never free text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterestTopic {
    Inbox,
    Calendar,
    Research,
    Social,
    Coding,
    Writing,
    Files,
    TeamChat,
    Customers,
    Automation,
    Sales,
    Reports,
    Billing,
    /// A topic written by a newer replica. Readable, never shown or matched.
    #[serde(other)]
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStyle {
    Solo,
    Team,
    Business,
    #[serde(other)]
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingOutcome {
    Completed,
    Skipped,
    #[serde(other)]
    Unsupported,
}

/// One person's first-run onboarding answers. `_id` is the user ID; a missing
/// row means the intro was never finished or skipped.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantOnboarding {
    #[serde(rename = "_id")]
    pub user_id: String,
    pub intro_version: i32,
    pub outcome: OnboardingOutcome,
    #[serde(default)]
    pub topics: Vec<InterestTopic>,
    #[serde(default)]
    pub work_style: Option<WorkStyle>,
    /// When the owner hid the home's suggestions; cleared by retaking the intro.
    #[serde(default, with = "super::bson_datetime::optional")]
    pub suggestions_hidden_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
