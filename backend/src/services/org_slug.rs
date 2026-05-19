use deunicode::deunicode;
use mongodb::Database;
use mongodb::bson::{Document, doc};

use crate::errors::{AppError, AppResult};
use crate::models::user::COLLECTION_NAME as USERS;

const MAX_SLUG_LEN: usize = 64;

/// Convert a display name into the canonical org slug base.
pub fn slugify(input: &str) -> String {
    let folded = deunicode(input);
    let mut slug = String::with_capacity(folded.len().min(MAX_SLUG_LEN));
    let mut previous_was_dash = false;

    for ch in folded.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            previous_was_dash = false;
        } else if !previous_was_dash {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    let mut slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        slug = "org".to_string();
    }
    if slug.len() > MAX_SLUG_LEN {
        slug.truncate(MAX_SLUG_LEN);
        slug = slug.trim_matches('-').to_string();
        if slug.is_empty() {
            slug = "org".to_string();
        }
    }
    if is_uuid_shaped(&slug) {
        slug.push_str("-org");
    }
    slug
}

/// Return true when the value has the exact lowercase UUID textual shape.
pub fn is_uuid_shaped(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }

    value.chars().enumerate().all(|(idx, ch)| match idx {
        8 | 13 | 18 | 23 => ch == '-',
        _ => ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase(),
    })
}

/// Reserve a unique org slug, trying `base`, `base-2`, `base-3`, and so on.
pub async fn reserve_slug(
    db: &Database,
    base: &str,
    exclude_user_id: Option<&str>,
) -> AppResult<String> {
    reserve_slug_mongo(db, base, exclude_user_id)
        .await
        .map_err(AppError::from)
}

pub(crate) async fn reserve_slug_mongo(
    db: &Database,
    base: &str,
    exclude_user_id: Option<&str>,
) -> Result<String, mongodb::error::Error> {
    let users = db.collection::<Document>(USERS);
    let normalized_base = if base.is_empty() { "org" } else { base };

    for attempt in 1u32.. {
        let candidate = candidate_for_attempt(normalized_base, attempt);
        let mut filter = doc! {
            "user_type": "org",
            "slug": &candidate,
        };
        if let Some(id) = exclude_user_id {
            filter.insert("_id", doc! { "$ne": id });
        }

        if users.find_one(filter).await?.is_none() {
            return Ok(candidate);
        }
    }

    unreachable!("unbounded slug suffix search should always return")
}

fn candidate_for_attempt(base: &str, attempt: u32) -> String {
    if attempt == 1 {
        return truncate_slug_base(base, MAX_SLUG_LEN);
    }

    let suffix = format!("-{attempt}");
    let max_base_len = MAX_SLUG_LEN.saturating_sub(suffix.len()).max(1);
    let mut candidate = truncate_slug_base(base, max_base_len);
    candidate.push_str(&suffix);
    candidate
}

fn truncate_slug_base(base: &str, max_len: usize) -> String {
    let mut value = base.chars().take(max_len).collect::<String>();
    value = value.trim_matches('-').to_string();
    if value.is_empty() {
        "org".to_string()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_folds_unicode_and_lowercases() {
        assert_eq!(slugify("Crème Brûlée Labs"), "creme-brulee-labs");
        assert_eq!(slugify("東京 AI"), "dong-jing-ai");
    }

    #[test]
    fn slugify_collapses_punctuation_runs() {
        assert_eq!(slugify("  Chrono---AI___Project!! "), "chrono-ai-project");
    }

    #[test]
    fn slugify_caps_length() {
        let input = "a".repeat(80);
        let slug = slugify(&input);
        assert_eq!(slug.len(), 64);
        assert!(slug.chars().all(|c| c == 'a'));
    }

    #[test]
    fn slugify_empty_input_falls_back_to_org() {
        assert_eq!(slugify("!!!"), "org");
        assert_eq!(slugify(""), "org");
    }

    #[test]
    fn slugify_uuid_shape_gets_org_suffix() {
        assert_eq!(
            slugify("550e8400-e29b-41d4-a716-446655440000"),
            "550e8400-e29b-41d4-a716-446655440000-org"
        );
    }

    #[test]
    fn uuid_shape_requires_lowercase_hex_and_hyphen_positions() {
        assert!(is_uuid_shaped("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_uuid_shaped("550E8400-e29b-41d4-a716-446655440000"));
        assert!(!is_uuid_shaped("550e8400_e29b_41d4_a716_446655440000"));
    }

    #[tokio::test]
    async fn reserve_slug_walks_collisions_and_excludes_self() {
        let Some(db) = crate::test_utils::connect_test_database("org_slug_reserve").await else {
            eprintln!("Skipping MongoDB-backed test; no test database available");
            return;
        };

        let users = db.collection::<Document>(USERS);
        users
            .insert_many([
                doc! { "_id": "org-1", "user_type": "org", "slug": "chrono-ai" },
                doc! { "_id": "org-2", "user_type": "org", "slug": "chrono-ai-2" },
            ])
            .await
            .expect("insert org slug fixtures");

        let next = reserve_slug(&db, "chrono-ai", None)
            .await
            .expect("reserve next slug");
        assert_eq!(next, "chrono-ai-3");

        let existing = reserve_slug(&db, "chrono-ai", Some("org-1"))
            .await
            .expect("reserve existing slug for self");
        assert_eq!(existing, "chrono-ai");
    }
}
