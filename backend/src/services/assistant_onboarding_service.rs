//! First-run NyxBot onboarding: the owner's interest answers, the one NyxBot
//! memory note they produce, and curated next-step recommendations.
//!
//! Recommendations come from a fixed catalog ranked by the answers and
//! filtered by what the owner already set up. Clicking one sends its prompt
//! to NyxBot, which does the work with its normal tools and confirmations.
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Document, doc},
};

use crate::{
    errors::{AppError, AppResult},
    models::assistant_onboarding::{
        AssistantOnboarding, COLLECTION_NAME, INTRO_VERSION, InterestTopic, OnboardingOutcome,
        WorkStyle,
    },
};

/// The NyxBot memory note that carries the answers. Fixed, so a later answer
/// replaces it instead of adding another.
pub const MEMORY_NOTE_ID: &str = "nyxid-onboarding-interests";
pub const DEFAULT_RECOMMENDATIONS: usize = 3;
pub const MAX_RECOMMENDATIONS: usize = 6;

/// Topics in display order.
pub const TOPICS: &[InterestTopic] = &[
    InterestTopic::Customers,
    InterestTopic::Sales,
    InterestTopic::Inbox,
    InterestTopic::Calendar,
    InterestTopic::TeamChat,
    InterestTopic::Research,
    InterestTopic::Social,
    InterestTopic::Reports,
    InterestTopic::Billing,
    InterestTopic::Coding,
    InterestTopic::Writing,
    InterestTopic::Files,
    InterestTopic::Automation,
];

/// Who a topic is offered to. Personal and team picks are about getting work
/// done; business picks look after customers, revenue and operations.
pub fn topic_styles(topic: InterestTopic) -> &'static [WorkStyle] {
    use WorkStyle::{Business, Solo, Team};
    match topic {
        InterestTopic::Inbox | InterestTopic::Automation => &[Solo, Team, Business],
        InterestTopic::Calendar
        | InterestTopic::Research
        | InterestTopic::Coding
        | InterestTopic::Files => &[Solo, Team],
        InterestTopic::Writing => &[Solo],
        InterestTopic::Social => &[Solo, Business],
        InterestTopic::TeamChat => &[Team, Business],
        InterestTopic::Reports => &[Team, Business],
        InterestTopic::Customers | InterestTopic::Sales | InterestTopic::Billing => &[Business],
        InterestTopic::Unsupported => &[],
    }
}

pub const WORK_STYLES: &[WorkStyle] = &[WorkStyle::Solo, WorkStyle::Team, WorkStyle::Business];

/// Label and lucide icon name.
pub fn topic_label(topic: InterestTopic) -> (&'static str, &'static str) {
    match topic {
        InterestTopic::Inbox => ("Email & inbox", "mail"),
        InterestTopic::Calendar => ("Calendar & meetings", "calendar"),
        InterestTopic::Research => ("Research & news", "search"),
        InterestTopic::Social => ("Social & marketing", "megaphone"),
        InterestTopic::Coding => ("Code & GitHub", "code"),
        InterestTopic::Writing => ("Writing", "pen-line"),
        InterestTopic::Files => ("Docs & files", "folder"),
        InterestTopic::TeamChat => ("Team chat", "messages-square"),
        InterestTopic::Customers => ("Customer support", "headset"),
        InterestTopic::Automation => ("Routines & automation", "zap"),
        InterestTopic::Sales => ("Sales & leads", "target"),
        InterestTopic::Reports => ("Reports & metrics", "chart-no-axes-combined"),
        InterestTopic::Billing => ("Payments & billing", "credit-card"),
        InterestTopic::Unsupported => ("Other", "sparkles"),
    }
}

pub fn work_style_label(style: WorkStyle) -> (&'static str, &'static str) {
    match style {
        WorkStyle::Solo => ("Just me", "user"),
        WorkStyle::Team => ("My team", "users"),
        WorkStyle::Business => ("My business", "store"),
        WorkStyle::Unsupported => ("Other", "sparkles"),
    }
}

/// How NyxID decides a recommendation is already done.
#[derive(Clone, Copy, Debug)]
enum Done {
    /// An active service whose slug contains one of these tokens.
    Service(&'static [&'static str]),
    /// A live specialist whose name contains one of these tokens.
    Specialist(&'static [&'static str]),
    Schedule,
    Channel,
    Never,
}

#[derive(Debug)]
pub struct Recommendation {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Sent to NyxBot as the owner's message when the card is clicked.
    pub prompt: &'static str,
    /// A lucide icon name; clients fall back to a sparkle.
    pub icon: &'static str,
    topics: &'static [InterestTopic],
    styles: &'static [WorkStyle],
    base: i32,
    done: Done,
}

const EMAIL: &[&str] = &[
    "gmail",
    "zoho-mail",
    "aurinko",
    "outlook",
    "google-workspace",
];
const CALENDAR: &[&str] = &["calendar", "outlook", "google-workspace"];

pub(super) static CATALOG: &[Recommendation] = &[
    Recommendation {
        id: "connect_inbox",
        title: "Connect your inbox",
        description: "NyxBot triages mail and drafts replies for you to approve.",
        prompt: "Help me connect my email so you can triage my inbox and draft replies.",
        icon: "mail",
        topics: &[InterestTopic::Inbox],
        styles: &[],
        base: 30,
        done: Done::Service(EMAIL),
    },
    Recommendation {
        id: "connect_calendar",
        title: "Connect your calendar",
        description: "Get a plan for your day and help scheduling meetings.",
        prompt: "Help me connect my calendar so you can plan my day and schedule meetings.",
        icon: "calendar",
        topics: &[InterestTopic::Calendar],
        styles: &[],
        base: 25,
        done: Done::Service(CALENDAR),
    },
    Recommendation {
        id: "morning_brief",
        title: "Get a daily brief",
        description: "A short weekday summary of what needs you, every morning.",
        prompt: "Set up a weekday morning brief at 8am covering my calendar, important email and anything I should know.",
        icon: "sunrise",
        topics: &[
            InterestTopic::Inbox,
            InterestTopic::Calendar,
            InterestTopic::Research,
            InterestTopic::Automation,
        ],
        styles: &[],
        base: 20,
        done: Done::Schedule,
    },
    Recommendation {
        id: "chat_anywhere",
        title: "Message NyxBot from your phone",
        description: "Link Telegram or another chat app and reach NyxBot anywhere.",
        prompt: "Set up a chat app bot so I can message you from my phone.",
        icon: "smartphone",
        topics: &[InterestTopic::TeamChat, InterestTopic::Customers],
        styles: &[],
        base: 18,
        done: Done::Channel,
    },
    Recommendation {
        id: "research_specialist",
        title: "Create a Research specialist",
        description: "A specialist that searches the web and summarises sources.",
        prompt: "Create a Research specialist that can search the web and summarise sources for me, then show me how to use it.",
        icon: "search",
        topics: &[InterestTopic::Research, InterestTopic::Writing],
        styles: &[],
        base: 15,
        done: Done::Specialist(&["research"]),
    },
    Recommendation {
        id: "connect_github",
        title: "Connect GitHub",
        description: "Track pull requests and issues with a Code specialist.",
        prompt: "Help me connect GitHub, then create a Code specialist that can track my pull requests and issues.",
        icon: "code",
        topics: &[InterestTopic::Coding],
        styles: &[],
        base: 15,
        done: Done::Service(&["github"]),
    },
    Recommendation {
        id: "connect_social",
        title: "Connect X",
        description: "Draft posts, track mentions and keep up with your feed.",
        prompt: "Help me connect my X account so you can draft posts and track mentions.",
        icon: "at-sign",
        topics: &[InterestTopic::Social],
        styles: &[],
        base: 15,
        done: Done::Service(&["twitter"]),
    },
    Recommendation {
        id: "connect_docs",
        title: "Connect your docs",
        description: "Let NyxBot find, read and write in Google Drive or Notion.",
        prompt: "Help me connect Google Drive or Notion so you can find and work with my documents.",
        icon: "file-text",
        topics: &[InterestTopic::Files, InterestTopic::Writing],
        styles: &[],
        base: 15,
        done: Done::Service(&["drive", "notion", "google-workspace"]),
    },
    Recommendation {
        id: "connect_team_chat",
        title: "Connect your team chat",
        description: "Search conversations and post updates in Slack, Lark or Discord.",
        prompt: "Help me connect my team chat (Slack, Lark or Discord) so you can search conversations and post updates.",
        icon: "messages-square",
        topics: &[InterestTopic::TeamChat],
        styles: &[WorkStyle::Team],
        base: 15,
        done: Done::Service(&["slack", "lark", "feishu", "discord"]),
    },
    Recommendation {
        id: "front_desk",
        title: "Set up a customer front desk",
        description: "A specialist that answers customers in a chat app, with you in control.",
        prompt: "Help me set up a front-desk specialist that answers customer questions in a chat app, and explain what it can and can't do.",
        icon: "store",
        topics: &[InterestTopic::Customers],
        styles: &[WorkStyle::Business],
        base: 15,
        done: Done::Specialist(&["desk", "support"]),
    },
    Recommendation {
        id: "track_leads",
        title: "Track new leads",
        description: "A specialist that collects leads from email and chats and sums them up daily.",
        prompt: "Help me set up a Sales specialist that collects new leads from my email and chat apps and summarises them every day.",
        icon: "target",
        topics: &[InterestTopic::Sales],
        styles: &[WorkStyle::Business],
        base: 15,
        done: Done::Specialist(&["lead", "sales"]),
    },
    Recommendation {
        id: "watch_payments",
        title: "Watch your payments",
        description: "Connect Stripe to get failed payments flagged and a weekly revenue summary.",
        prompt: "Help me connect Stripe so you can flag failed payments and summarise revenue each week.",
        icon: "credit-card",
        topics: &[InterestTopic::Billing],
        styles: &[WorkStyle::Business],
        base: 15,
        done: Done::Service(&["stripe"]),
    },
    Recommendation {
        id: "weekly_report",
        title: "Get a weekly report",
        description: "Every Monday, a summary of activity across your connected tools.",
        prompt: "Set up a weekly report every Monday at 9am that summarises activity across my connected services.",
        icon: "chart-no-axes-combined",
        topics: &[InterestTopic::Reports],
        styles: &[WorkStyle::Team, WorkStyle::Business],
        base: 12,
        done: Done::Schedule,
    },
    Recommendation {
        id: "first_routine",
        title: "Automate a routine",
        description: "Describe a repetitive task and NyxBot puts it on a schedule.",
        prompt: "Suggest a routine you could run on a schedule for me, then set it up once I agree.",
        icon: "repeat",
        topics: &[InterestTopic::Automation],
        styles: &[],
        base: 10,
        done: Done::Schedule,
    },
    Recommendation {
        id: "what_can_you_do",
        title: "See what NyxBot can do",
        description: "Three ideas tailored to what you told us.",
        prompt: "Based on what you know about me, suggest three things you could do for me this week.",
        icon: "sparkles",
        topics: &[],
        styles: &[],
        base: 5,
        done: Done::Never,
    },
];

/// What the owner already has, read only when recommendations are requested.
#[derive(Clone, Debug, Default)]
pub struct Signals {
    pub service_slugs: Vec<String>,
    pub specialist_names: Vec<String>,
    pub has_schedule: bool,
    pub has_channel: bool,
}

fn contains_any(values: &[String], tokens: &[&str]) -> bool {
    values.iter().any(|value| {
        let value = value.to_ascii_lowercase();
        tokens.iter().any(|token| value.contains(token))
    })
}

/// `api-google` (optionally `-N`) is the combined Gmail + Calendar + Drive service.
fn has_combined_google(slugs: &[String]) -> bool {
    slugs.iter().any(|slug| {
        slug.strip_prefix("api-google").is_some_and(|rest| {
            rest.is_empty()
                || rest
                    .strip_prefix('-')
                    .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
        })
    })
}

fn is_done(rec: &Recommendation, signals: &Signals) -> bool {
    match rec.done {
        Done::Service(tokens) => {
            contains_any(&signals.service_slugs, tokens)
                || (matches!(
                    rec.id,
                    "connect_inbox" | "connect_calendar" | "connect_docs"
                ) && has_combined_google(&signals.service_slugs))
        }
        Done::Specialist(tokens) => contains_any(&signals.specialist_names, tokens),
        Done::Schedule => signals.has_schedule,
        Done::Channel => signals.has_channel,
        Done::Never => false,
    }
}

/// The best `limit` recommendations: matching topics and work style first,
/// then the catalog's own priority. Done ones are left out.
pub fn recommend(
    profile: Option<&AssistantOnboarding>,
    signals: &Signals,
    limit: usize,
) -> Vec<&'static Recommendation> {
    let topics = profile.map(|row| row.topics.as_slice()).unwrap_or_default();
    let style = profile.and_then(|row| row.work_style);
    let mut ranked: Vec<(i32, usize, &'static Recommendation)> = CATALOG
        .iter()
        .enumerate()
        .filter(|(_, rec)| !is_done(rec, signals))
        .map(|(order, rec)| {
            let topic_hits = rec.topics.iter().filter(|t| topics.contains(t)).count() as i32;
            let style_hit = style.is_some_and(|s| rec.styles.contains(&s));
            (
                rec.base + 40 * topic_hits + if style_hit { 20 } else { 0 },
                order,
                rec,
            )
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked
        .into_iter()
        .take(limit.clamp(1, MAX_RECOMMENDATIONS))
        .map(|(_, _, rec)| rec)
        .collect()
}

pub async fn load_signals(db: &Database, owner: &str) -> AppResult<Signals> {
    let slugs = async {
        let values = db
            .collection::<Document>(crate::models::user_service::COLLECTION_NAME)
            .distinct("slug", doc! {"user_id": owner, "is_active": true})
            .await?;
        Ok::<_, AppError>(
            values
                .into_iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect::<Vec<_>>(),
        )
    };
    let specialists = async {
        let rows: Vec<Document> = db
            .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .find(doc! {"user_id": owner, "kind": "specialist", "destroyed_at": bson::Bson::Null})
            .projection(doc! {"name": 1, "display_name": 1})
            .limit(100)
            .await?
            .try_collect()
            .await?;
        Ok::<_, AppError>(
            rows.iter()
                .flat_map(|row| {
                    ["name", "display_name"]
                        .into_iter()
                        .filter_map(|key| row.get_str(key).ok().map(str::to_owned))
                })
                .collect::<Vec<_>>(),
        )
    };
    let schedule = async {
        Ok::<_, AppError>(
            db.collection::<Document>(crate::models::trigger::COLLECTION_NAME)
                .find_one(doc! {"user_id": owner, "source": "schedule", "status": "active"})
                .projection(doc! {"_id": 1})
                .await?
                .is_some(),
        )
    };
    let channel = async {
        Ok::<_, AppError>(
            db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
                .find_one(doc! {"user_id": owner, "status": "active"})
                .projection(doc! {"_id": 1})
                .await?
                .is_some(),
        )
    };
    let (service_slugs, specialist_names, has_schedule, has_channel) =
        tokio::try_join!(slugs, specialists, schedule, channel)?;
    Ok(Signals {
        service_slugs,
        specialist_names,
        has_schedule,
        has_channel,
    })
}

/// Upsert the owner's row behind the personal-erasure fence, so a request in
/// flight cannot recreate it after erasure.
async fn fenced_upsert(db: &Database, owner: &str, update: Document) -> AppResult<Document> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<Document> = async {
        super::erasure_admission::fence_in_session(db, owner, &mut session).await?;
        db.collection::<Document>(COLLECTION_NAME)
            .find_one_and_update(doc! {"_id": owner}, update)
            .upsert(true)
            .return_document(mongodb::options::ReturnDocument::After)
            .session(&mut session)
            .await?
            .ok_or_else(|| AppError::Internal("Onboarding state unavailable".into()))
    }
    .await;
    super::org_membership_authority::finish(session, result).await
}

fn encode<T: serde::Serialize>(value: &T) -> AppResult<bson::Bson> {
    bson::to_bson(value).map_err(|_| AppError::Internal("Onboarding encoding failed".into()))
}

pub async fn get(db: &Database, owner: &str) -> AppResult<Option<AssistantOnboarding>> {
    let Some(raw) = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id": owner})
        .await?
    else {
        return Ok(None);
    };
    bson::from_document(raw)
        .map(Some)
        .map_err(|_| AppError::Internal("Onboarding state unavailable".into()))
}

/// Whether the first-run intro should open: a revised intro the owner has not
/// seen, or never finished or skipped while the owner has no NyxBot thread yet (people who already use NyxBot are not
/// interrupted; they can replay the intro from the account menu).
pub async fn should_show_intro(
    db: &Database,
    owner: &str,
    profile: Option<&AssistantOnboarding>,
) -> AppResult<bool> {
    if let Some(profile) = profile {
        // A revised intro (a bumped INTRO_VERSION) shows once more.
        return Ok(profile.intro_version < INTRO_VERSION);
    }
    Ok(db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"user_id": owner})
        .projection(doc! {"_id": 1})
        .await?
        .is_none())
}

pub struct Answers {
    pub outcome: OnboardingOutcome,
    pub topics: Vec<InterestTopic>,
    pub work_style: Option<WorkStyle>,
}

/// Save the answers (replacing earlier ones) and keep NyxBot's note in step.
pub async fn save(db: &Database, owner: &str, answers: Answers) -> AppResult<AssistantOnboarding> {
    if answers.outcome == OnboardingOutcome::Unsupported {
        return Err(AppError::ValidationError(
            "Unknown onboarding outcome".into(),
        ));
    }
    let topics: Vec<InterestTopic> = TOPICS
        .iter()
        .copied()
        .filter(|topic| answers.topics.contains(topic))
        .collect();
    let work_style = answers
        .work_style
        .filter(|style| *style != WorkStyle::Unsupported);
    let now = bson::DateTime::from_chrono(Utc::now());
    let row = fenced_upsert(
        db,
        owner,
        doc! {
            "$set": {
                "intro_version": INTRO_VERSION,
                "outcome": encode(&answers.outcome)?,
                "topics": encode(&topics)?,
                "work_style": encode(&work_style)?,
                // Retaking the intro brings the suggestions back.
                "suggestions_hidden_at": bson::Bson::Null,
                "updated_at": now,
            },
            "$setOnInsert": {"created_at": now},
        },
    )
    .await?;
    let row: AssistantOnboarding = bson::from_document(row)
        .map_err(|_| AppError::Internal("Onboarding state unavailable".into()))?;
    let note = memory_note(&row.topics, row.work_style);
    if let Err(error) = super::assistant_team_service::set_nyxbot_fixed_note(
        db,
        owner,
        MEMORY_NOTE_ID,
        note.as_deref(),
    )
    .await
    {
        // The answers still drive recommendations; NyxBot learns them later
        // when the owner retakes the intro.
        tracing::warn!(error = %error, "Onboarding memory note not updated");
    }
    Ok(row)
}

/// Hide the home's suggestions until the owner restores them or retakes the intro.
pub async fn hide_suggestions(db: &Database, owner: &str) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    // Hiding before the intro was ever answered records a skipped intro.
    fenced_upsert(
        db,
        owner,
        doc! {
            "$set": {"suggestions_hidden_at": now, "updated_at": now},
            "$setOnInsert": {
                "intro_version": INTRO_VERSION,
                "outcome": "skipped",
                "topics": [],
                "work_style": bson::Bson::Null,
                "created_at": now,
            },
        },
    )
    .await?;
    Ok(())
}

/// Restore suggestions on an existing row only. This plain update cannot
/// resurrect an erased row, so it needs no erasure fence.
pub async fn show_suggestions(db: &Database, owner: &str) -> AppResult<()> {
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": owner},
            doc! {"$set": {
                "suggestions_hidden_at": bson::Bson::Null,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            }},
        )
        .await?;
    Ok(())
}

/// Plain facts from the closed answer set; `None` removes the note.
pub fn memory_note(topics: &[InterestTopic], work_style: Option<WorkStyle>) -> Option<String> {
    let labels: Vec<String> = topics
        .iter()
        .filter(|topic| **topic != InterestTopic::Unsupported)
        .map(|topic| topic_label(*topic).0.to_lowercase())
        .collect();
    let style = match work_style {
        Some(WorkStyle::Solo) => Some("works solo"),
        Some(WorkStyle::Team) => Some("works with a team"),
        Some(WorkStyle::Business) => Some("runs a business"),
        _ => None,
    };
    if labels.is_empty() && style.is_none() {
        return None;
    }
    let mut parts = Vec::new();
    if !labels.is_empty() {
        parts.push(format!("wants help with {}", labels.join(", ")));
    }
    if let Some(style) = style {
        parts.push(style.to_owned());
    }
    Some(format!(
        "From the owner's onboarding answers: {}. Use this to suggest relevant next steps; \
        the owner can change these answers at any time.",
        parts.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(topics: &[InterestTopic], style: Option<WorkStyle>) -> AssistantOnboarding {
        AssistantOnboarding {
            user_id: "u".into(),
            intro_version: INTRO_VERSION,
            outcome: OnboardingOutcome::Completed,
            topics: topics.to_vec(),
            work_style: style,
            suggestions_hidden_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn ids(recs: &[&Recommendation]) -> Vec<&'static str> {
        recs.iter().map(|rec| rec.id).collect()
    }

    #[test]
    fn catalog_ids_are_unique() {
        let mut ids: Vec<_> = CATALOG.iter().map(|rec| rec.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
    }

    #[test]
    fn every_audience_gets_a_full_topic_list() {
        for style in WORK_STYLES {
            let offered = TOPICS
                .iter()
                .filter(|topic| topic_styles(**topic).contains(style))
                .count();
            assert_eq!(offered, 8, "{style:?}");
        }
        assert!(TOPICS.iter().all(|topic| !topic_styles(*topic).is_empty()));
    }

    #[test]
    fn business_answers_lead_with_business_steps() {
        let row = profile(
            &[InterestTopic::Sales, InterestTopic::Billing],
            Some(WorkStyle::Business),
        );
        let recs = recommend(Some(&row), &Signals::default(), 3);
        assert_eq!(ids(&recs), ["track_leads", "watch_payments", "front_desk"]);
    }

    #[test]
    fn no_answers_get_general_defaults() {
        let recs = recommend(None, &Signals::default(), 3);
        assert_eq!(
            ids(&recs),
            ["connect_inbox", "connect_calendar", "morning_brief"]
        );
    }

    #[test]
    fn topics_and_work_style_lead() {
        let row = profile(
            &[InterestTopic::Coding, InterestTopic::Customers],
            Some(WorkStyle::Business),
        );
        let recs = recommend(Some(&row), &Signals::default(), 3);
        assert_eq!(
            ids(&recs),
            ["front_desk", "chat_anywhere", "connect_github"]
        );
    }

    #[test]
    fn done_steps_are_left_out() {
        let row = profile(&[InterestTopic::Inbox, InterestTopic::Calendar], None);
        let signals = Signals {
            service_slugs: vec!["api-google-2".into()],
            has_schedule: true,
            ..Signals::default()
        };
        let recs = recommend(Some(&row), &signals, 3);
        let ids = ids(&recs);
        assert!(!ids.contains(&"connect_inbox"));
        assert!(!ids.contains(&"connect_calendar"));
        assert!(!ids.contains(&"morning_brief"));
    }

    #[test]
    fn combined_google_slug_is_exact() {
        assert!(has_combined_google(&["api-google".into()]));
        assert!(has_combined_google(&["api-google-12".into()]));
        assert!(!has_combined_google(&["api-google-gmail".into()]));
    }

    #[test]
    fn specialist_and_channel_signals_mark_done() {
        let row = profile(&[InterestTopic::Research, InterestTopic::TeamChat], None);
        let signals = Signals {
            specialist_names: vec!["Research-Desk".into()],
            has_channel: true,
            ..Signals::default()
        };
        let ids = ids(&recommend(Some(&row), &signals, 6));
        assert!(!ids.contains(&"research_specialist"));
        assert!(!ids.contains(&"chat_anywhere"));
        assert!(!ids.contains(&"front_desk"));
    }

    #[test]
    fn memory_note_uses_only_closed_labels() {
        assert_eq!(memory_note(&[], None), None);
        let note = memory_note(
            &[InterestTopic::Inbox, InterestTopic::Research],
            Some(WorkStyle::Solo),
        )
        .expect("note");
        assert!(note.contains("wants help with email & inbox, research & news; works solo"));
        assert!(note.chars().count() <= crate::models::assistant_agent::MAX_MEMORY_NOTE_CHARS);
    }

    #[test]
    fn unknown_topic_from_a_newer_replica_is_readable() {
        let topic: InterestTopic = serde_json::from_value(serde_json::json!("gardening")).unwrap();
        assert_eq!(topic, InterestTopic::Unsupported);
    }

    #[tokio::test]
    async fn answers_drive_intro_gate_memory_note_and_hidden_suggestions() {
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};
        let db = crate::test_utils::connect_transaction_test_database("assistant_onboarding").await;
        let owner = "12345678-1234-4123-8123-123456789abc";
        db.collection(USERS)
            .insert_one(crate::test_utils::test_user(owner, UserType::Person))
            .await
            .unwrap();
        assert!(should_show_intro(&db, owner, None).await.unwrap());

        let row = save(
            &db,
            owner,
            Answers {
                outcome: OnboardingOutcome::Completed,
                topics: vec![
                    InterestTopic::Research,
                    InterestTopic::Inbox,
                    InterestTopic::Unsupported,
                ],
                work_style: Some(WorkStyle::Solo),
            },
        )
        .await
        .unwrap();
        // Display order, unknown values dropped.
        assert_eq!(row.topics, [InterestTopic::Inbox, InterestTopic::Research]);
        assert!(!should_show_intro(&db, owner, Some(&row)).await.unwrap());
        let nyxbot = super::super::assistant_team_service::ensure_nyxbot(&db, owner)
            .await
            .unwrap();
        let notes: Vec<_> = nyxbot
            .memory
            .iter()
            .filter(|note| note.id == MEMORY_NOTE_ID)
            .collect();
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0]
                .text
                .contains("email & inbox, research & news; works solo")
        );

        // A revised intro shows once more.
        let older = AssistantOnboarding {
            intro_version: INTRO_VERSION - 1,
            ..row.clone()
        };
        assert!(should_show_intro(&db, owner, Some(&older)).await.unwrap());

        // A retake moves the note after newer notes, so it stays in the budget.
        super::super::assistant_team_service::remember(
            &db,
            owner,
            &nyxbot.id,
            "Prefers short replies",
            None,
        )
        .await
        .unwrap();
        save(
            &db,
            owner,
            Answers {
                outcome: OnboardingOutcome::Completed,
                topics: vec![InterestTopic::Inbox],
                work_style: None,
            },
        )
        .await
        .unwrap();
        let nyxbot = super::super::assistant_team_service::ensure_nyxbot(&db, owner)
            .await
            .unwrap();
        assert_eq!(
            nyxbot.memory.last().map(|note| note.id.as_str()),
            Some(MEMORY_NOTE_ID)
        );
        assert_eq!(
            nyxbot
                .memory
                .iter()
                .filter(|note| note.id == MEMORY_NOTE_ID)
                .count(),
            1
        );

        // Retaking replaces the note; no answers remove it.
        save(
            &db,
            owner,
            Answers {
                outcome: OnboardingOutcome::Skipped,
                topics: vec![],
                work_style: None,
            },
        )
        .await
        .unwrap();
        let nyxbot = super::super::assistant_team_service::ensure_nyxbot(&db, owner)
            .await
            .unwrap();
        assert!(nyxbot.memory.iter().all(|note| note.id != MEMORY_NOTE_ID));

        hide_suggestions(&db, owner).await.unwrap();
        let row = get(&db, owner).await.unwrap().unwrap();
        assert!(row.suggestions_hidden_at.is_some());
        assert_eq!(row.outcome, OnboardingOutcome::Skipped);
        // Retaking the intro brings them back.
        let row = save(
            &db,
            owner,
            Answers {
                outcome: OnboardingOutcome::Completed,
                topics: vec![],
                work_style: None,
            },
        )
        .await
        .unwrap();
        assert!(row.suggestions_hidden_at.is_none());
    }

    #[tokio::test]
    async fn showing_suggestions_preserves_answers_and_intro_version() {
        let db = crate::test_utils::connect_transaction_test_database("onboarding_show").await;
        let owner = "32345678-1234-4123-8123-123456789abc";
        let old = bson::DateTime::from_millis(1);
        let original = doc! {
            "_id": owner,
            "intro_version": INTRO_VERSION - 1,
            "outcome": "future_outcome",
            "topics": ["inbox", "future_topic"],
            "work_style": "team",
            "suggestions_hidden_at": bson::Bson::Null,
            "created_at": old,
            "updated_at": old,
        };
        let rows = db.collection::<Document>(COLLECTION_NAME);
        rows.insert_one(original.clone()).await.unwrap();
        hide_suggestions(&db, owner).await.unwrap();
        assert!(
            get(&db, owner)
                .await
                .unwrap()
                .unwrap()
                .suggestions_hidden_at
                .is_some()
        );
        show_suggestions(&db, owner).await.unwrap();
        let restored = rows.find_one(doc! {"_id": owner}).await.unwrap().unwrap();
        assert_eq!(
            restored.get("suggestions_hidden_at"),
            Some(&bson::Bson::Null)
        );
        assert_ne!(restored.get("updated_at"), original.get("updated_at"));
        for field in [
            "outcome",
            "topics",
            "work_style",
            "intro_version",
            "created_at",
        ] {
            assert_eq!(restored.get(field), original.get(field), "{field}");
        }
    }

    #[tokio::test]
    async fn showing_suggestions_without_a_row_creates_nothing() {
        let db =
            crate::test_utils::connect_transaction_test_database("onboarding_show_absent").await;
        show_suggestions(&db, "42345678-1234-4123-8123-123456789abc")
            .await
            .unwrap();
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn showing_suggestions_only_changes_the_callers_row() {
        let db =
            crate::test_utils::connect_transaction_test_database("onboarding_show_owner").await;
        let owner = "52345678-1234-4123-8123-123456789abc";
        let other = "62345678-1234-4123-8123-123456789abc";
        hide_suggestions(&db, owner).await.unwrap();
        hide_suggestions(&db, other).await.unwrap();
        let rows = db.collection::<Document>(COLLECTION_NAME);
        let before = rows.find_one(doc! {"_id": other}).await.unwrap().unwrap();
        show_suggestions(&db, owner).await.unwrap();
        assert!(
            get(&db, owner)
                .await
                .unwrap()
                .unwrap()
                .suggestions_hidden_at
                .is_none()
        );
        assert_eq!(
            rows.find_one(doc! {"_id": other}).await.unwrap().unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn existing_nyxbot_users_skip_the_intro_and_hiding_first_records_a_skip() {
        let db =
            crate::test_utils::connect_transaction_test_database("assistant_onboarding_existing")
                .await;
        let owner = "22345678-1234-4123-8123-123456789abc";
        db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .insert_one(doc! {"_id": "nyxa-existing", "user_id": owner})
            .await
            .unwrap();
        assert!(!should_show_intro(&db, owner, None).await.unwrap());
        hide_suggestions(&db, owner).await.unwrap();
        let row = get(&db, owner).await.unwrap().unwrap();
        assert_eq!(row.outcome, OnboardingOutcome::Skipped);
        assert_eq!(row.intro_version, INTRO_VERSION);
    }
}
