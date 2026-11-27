//! Agent avatar validation and rollout gate.
//!
//! The catalogue is the shared, canonical `shared/agent-avatars/catalog.json`
//! (also imported by the web and mobile clients). The backend only needs its
//! ids, so part geometry is skipped while parsing. Validation runs on writes
//! only; reads never evaluate the catalogue or the rollout flag.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};
use crate::models::assistant_avatar::Avatar;
use crate::services::feature_flag_service;

pub const CATALOGUE_JSON: &str = include_str!("../../../shared/agent-avatars/catalog.json");

#[derive(Deserialize)]
struct Catalogue {
    template: String,
    max_choices: usize,
    max_identifier: usize,
    compatibility: Compatibility,
    fields: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}
#[derive(Deserialize)]
struct Compatibility {
    stroke_treatment: String,
    stroke_surface: String,
}
static CATALOGUE: LazyLock<Catalogue> =
    LazyLock::new(|| serde_json::from_str(CATALOGUE_JSON).expect("valid face catalogue"));

/// Tolerant projection retains bounded future choices, including unknown templates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AvatarView {
    pub template: String,
    pub choices: BTreeMap<String, String>,
}
impl From<&Avatar> for AvatarView {
    fn from(avatar: &Avatar) -> Self {
        Self {
            template: avatar.template.clone(),
            choices: avatar.choices.clone(),
        }
    }
}
pub fn view(avatar: Option<&Avatar>) -> Option<AvatarView> {
    avatar.map(AvatarView::from)
}
pub fn serialize_view<S: serde::Serializer>(
    avatar: &Option<Avatar>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    view(avatar.as_ref()).serialize(serializer)
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= CATALOGUE.max_identifier
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
pub fn validate(avatar: &Avatar) -> AppResult<()> {
    let c = &*CATALOGUE;
    let valid = avatar.template == c.template
        && avatar.choices.len() <= c.max_choices
        && avatar.choices.iter().all(|(key, id)| {
            identifier(key)
                && identifier(id)
                && c.fields
                    .get(key)
                    .is_some_and(|field| field.contains_key(id))
        })
        && !(avatar
            .choices
            .get("treatment")
            .is_some_and(|v| v == &c.compatibility.stroke_treatment)
            && avatar
                .choices
                .get("surface")
                .is_some_and(|v| v != &c.compatibility.stroke_surface));
    if valid {
        Ok(())
    } else {
        Err(AppError::ValidationError(
            "Choose compatible avatar settings from the face catalogue".into(),
        ))
    }
}
pub fn validate_for_kind(avatar: &Avatar, nyxbot: bool) -> AppResult<()> {
    validate(avatar)?;
    let kind = if nyxbot { "nyxbot" } else { "specialist" };
    for (field, id) in &avatar.choices {
        let entry = &CATALOGUE.fields[field][id];
        if entry
            .get("kinds")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|kinds| !kinds.iter().any(|allowed| allowed.as_str() == Some(kind)))
        {
            return Err(AppError::ValidationError(
                "This avatar choice is not available for this agent kind".into(),
            ));
        }
    }
    Ok(())
}

/// Avatar writes follow the acting person's `assistant:avatars` rollout
/// (default, global, org scopes, then their own override). Refused like the
/// other flag-gated assistant writes; stored avatars are kept while off.
pub async fn require_enabled(db: &mongodb::Database, actor: &str) -> AppResult<()> {
    if feature_flag_service::personal_flag_enabled(
        db,
        actor,
        feature_flag_service::ASSISTANT_AVATARS_FLAG_KEY,
    )
    .await?
    {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Agent avatars are not enabled for this rollout".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> Avatar {
        Avatar {
            template: "face-v1".into(),
            choices: BTreeMap::from([
                ("eyes".into(), "dots".into()),
                ("palette".into(), "core-violet".into()),
            ]),
        }
    }
    #[test]
    fn matches_shared_validation_fixtures() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../shared/agent-avatars/validation-fixture.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            // Requests retain invalid identifiers until strict validation; stored reads sanitize them.
            let avatar = Avatar {
                template: row["avatar"]["template"].as_str().unwrap().into(),
                choices: serde_json::from_value(row["avatar"]["choices"].clone()).unwrap(),
            };
            assert_eq!(
                validate_for_kind(&avatar, row["kind"] == "nyxbot").is_ok(),
                row["valid"].as_bool().unwrap(),
                "{}",
                row["name"]
            );
        }
    }
    #[test]
    fn full_catalogue_and_kind_validation() {
        assert_eq!(CATALOGUE.fields["palette"].len(), 35);
        assert_eq!(CATALOGUE.fields["ripple_palette"].len(), 12);
        assert_eq!(CATALOGUE.fields["surface"].len(), 19);
        assert!(validate_for_kind(&valid(), false).is_ok());
        let mut a = valid();
        a.choices.insert("head".into(), "logo".into());
        assert!(validate_for_kind(&a, false).is_err());
        a.choices.insert("react_listening".into(), "rain".into());
        assert!(validate(&a).is_err());
        a = valid();
        a.choices.insert("$x".into(), "dots".into());
        assert!(validate(&a).is_err());
        a = valid();
        a.choices.insert("surface".into(), "ascii-shaded".into());
        a.choices.insert("treatment".into(), "neon-line".into());
        assert!(validate(&a).is_err());
    }
    #[test]
    fn future_choices_round_trip_losslessly_and_bad_storage_is_defensive() {
        let stored: Avatar = bson::from_document(bson::doc! {"template":"face-v99","choices":{"future_field":"future-value","eyes":"future-eyes"}}).unwrap();
        assert_eq!(view(Some(&stored)).unwrap().choices, stored.choices);
        assert_eq!(
            bson::from_bson::<Avatar>(bson::to_bson(&stored).unwrap()).unwrap(),
            stored
        );
        assert!(validate(&stored).is_err());
        let bad: Avatar=bson::from_document(bson::doc! {"template":"face-v1","choices":{"eyes":"dots","bad.key":"code","bad_value":true}}).unwrap();
        assert_eq!(bad.choices.len(), 1);
    }
}
