//! Public help is served from approved content without an account or model call.
use crate::errors::{AppError, AppResult};

use crate::models::platform_help::{HelpArticle as Article, HelpContent};
#[cfg(test)]
fn articles() -> AppResult<Vec<Article>> {
    Ok(super::platform_help_service::seed_content()?.articles)
}

pub(crate) fn all_articles(content: &HelpContent) -> Vec<Article> {
    let mut articles = content.articles.clone();
    articles.extend(content.use_cases.iter().map(|c| Article {
        id: c.id.clone(),
        title: c.title.clone(),
        body: format!(
            "{}\n\nExample prompts:\n{}",
            c.description,
            c.example_prompts.join("\n")
        ),
        tags: vec!["use case".into()],
    }));
    articles
}
#[cfg(test)]
pub(crate) fn select(text: &str) -> AppResult<Vec<String>> {
    Ok(select_from(text, articles()?))
}
pub(crate) fn select_from(text: &str, articles: Vec<Article>) -> Vec<String> {
    let query = text.trim().to_lowercase();
    let query = query.strip_prefix("/faq").unwrap_or(&query).trim();
    if query.is_empty() || matches!(query, "/start" | "/help" | "help" | "hello" | "hi") {
        return Vec::new();
    }
    if let Some(article) = articles.iter().find(|article| article.id == query) {
        return vec![article.id.clone()];
    }
    let words: std::collections::BTreeSet<_> = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.len() > 2)
        .filter(|word| {
            !matches!(
                *word,
                "the"
                    | "can"
                    | "how"
                    | "what"
                    | "does"
                    | "are"
                    | "you"
                    | "your"
                    | "with"
                    | "for"
                    | "and"
                    | "this"
                    | "that"
                    | "about"
            )
        })
        .collect();
    let mut ranked: Vec<_> = articles
        .into_iter()
        .filter_map(|article| {
            let title = article.title.to_lowercase();
            let body = article.body.to_lowercase();
            let tags = article.tags.join(" ").to_lowercase();
            let score: usize = words
                .iter()
                .map(|word| {
                    usize::from(title.contains(word)) * 3
                        + usize::from(tags.contains(word)) * 2
                        + usize::from(body.contains(word))
                })
                .sum();
            (score > 0).then_some((score, article.id))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().take(2).map(|(_, id)| id).collect()
}

/// Only menu and mode instructions are composed per provider. Approved article
/// bodies retain their original bytes, including examples for other platforms.
pub(crate) fn render_content(
    articles: Vec<Article>,
    reference: &crate::models::platform_manager_delivery::DeliveryReference,
    entry: &super::platform_manager_contract::EntryIdentity,
) -> AppResult<String> {
    use super::platform_chat_presentation::ChatPresentation;
    let crate::models::platform_manager_delivery::DeliveryReference::Faq {
        article_ids,
        linking_enabled,
        ..
    } = reference
    else {
        return Err(AppError::ValidationError("FAQ reference required".into()));
    };
    if article_ids.len() > 2 || (article_ids.len() == 2 && article_ids[0] == article_ids[1]) {
        return Err(AppError::ValidationError(
            "Invalid FAQ article selection".into(),
        ));
    }
    let chat = ChatPresentation::for_entry(entry)?;
    let navigation = match chat {
        ChatPresentation::Telegram => "Send /faq to browse help, or /faq followed by a topic.",
        ChatPresentation::Native(
            crate::models::platform_entry_identity::EntryPlatform::Discord,
        ) => {
            "Choose /nyx ask and enter faq followed by a topic in its question option. Choose /nyx help to browse help."
        }
        _ => "Send faq to browse help, or faq followed by a topic as a plain message.",
    };
    let guidance = if *linking_enabled
        && matches!(
            chat,
            ChatPresentation::Native(
                crate::models::platform_entry_identity::EntryPlatform::Discord
            )
        ) {
        "FAQ never uses personal data. Already linked? Use /nyx ask for personal questions. Otherwise choose /nyx login here to sign in and link this conversation.".into()
    } else if *linking_enabled {
        format!(
            "FAQ never uses personal data. Already linked? {} Otherwise send {} here to sign in and connect this conversation.",
            chat.question_instruction(),
            chat.login()
        )
    } else {
        "Personal work and linking are paused, including for previously linked conversations. Open NyxID to use your NyxBot.".into()
    };
    let suffix = format!("\n\n{navigation}\n\n{guidance}");
    let mut body = String::new();
    if article_ids.is_empty() {
        body = format!(
            "Welcome to NyxID. Browse approved public help.\n\n{}",
            articles
                .iter()
                .map(|a| format!("• {} — {}", a.title, a.id))
                .collect::<Vec<_>>()
                .join("\n")
        );
    } else {
        for id in article_ids {
            let article = articles
                .iter()
                .find(|a| &a.id == id)
                .ok_or_else(|| AppError::ValidationError("Unknown FAQ article".into()))?;
            let candidate = format!(
                "{}{}{}\n{}",
                body,
                if body.is_empty() { "" } else { "\n\n" },
                article.title,
                article.body
            );
            body = candidate;
        }
    }
    body.push_str(&suffix);
    Ok(body)
}

pub(crate) async fn render_saved(
    db: &mongodb::Database,
    reference: &crate::models::platform_manager_delivery::DeliveryReference,
    entry: &super::platform_manager_contract::EntryIdentity,
) -> AppResult<String> {
    let content = match reference {
        crate::models::platform_manager_delivery::DeliveryReference::Faq {
            help: Some(reference),
            ..
        } => {
            super::platform_help_service::revision(db, reference)
                .await?
                .content
        }
        _ => super::platform_help_service::seed_content()?,
    };
    render_content(all_articles(&content), reference, entry)
}
#[cfg(test)]
fn render_for(
    reference: &crate::models::platform_manager_delivery::DeliveryReference,
    entry: &super::platform_manager_contract::EntryIdentity,
) -> AppResult<String> {
    render_content(articles()?, reference, entry)
}

#[cfg(test)]
mod tests {
    use super::super::platform_manager_contract::EntryIdentity;
    use super::*;
    use crate::models::{
        platform_entry_identity::{EntryPlatform, NativeEntryIdentity},
        platform_manager_delivery::DeliveryReference,
    };
    #[test]
    fn corpus_menu_and_all_pairs_fit_real_provider_limits_without_rewriting_articles() {
        let articles = articles().unwrap();
        let mut selections = vec![vec![]];
        for a in &articles {
            selections.push(vec![a.id.clone()]);
            for b in &articles {
                if a.id != b.id {
                    selections.push(vec![a.id.clone(), b.id.clone()]);
                }
            }
        }
        for entry in [
            EntryIdentity::TelegramV1 {
                telegram_bot_id: 100,
            },
            EntryIdentity::NativeV2 {
                platform_entry: NativeEntryIdentity {
                    version: 2,
                    channel_bot_id: uuid::Uuid::new_v4().to_string(),
                    platform: EntryPlatform::Slack,
                    platform_bot_id: "B123".into(),
                },
            },
            EntryIdentity::NativeV2 {
                platform_entry: NativeEntryIdentity {
                    version: 2,
                    channel_bot_id: uuid::Uuid::new_v4().to_string(),
                    platform: EntryPlatform::Discord,
                    platform_bot_id: "100".into(),
                },
            },
        ] {
            for ids in &selections {
                for linking in [false, true] {
                    let reference = DeliveryReference::Faq {
                        help: None,
                        article_ids: ids.clone(),
                        linking_enabled: linking,
                        public_policy_revision: 1,
                    };
                    let rendered = render_for(&reference, &entry).unwrap();
                    assert!(rendered.encode_utf16().count()<=super::super::platform_chat_presentation::ChatPresentation::for_entry(&entry).unwrap().text_limit());
                    if linking {
                        assert!(rendered.contains("Already linked?"));
                    } else {
                        assert!(rendered.contains("Personal work and linking are paused"));
                    }
                    for id in ids {
                        assert!(
                            rendered.contains(&articles.iter().find(|a| &a.id == id).unwrap().body)
                        );
                    }
                    assert!(!rendered.contains("/nyx help unlink"));
                    if matches!(entry, EntryIdentity::NativeV2 { ref platform_entry } if platform_entry.platform == EntryPlatform::Discord)
                    {
                        assert!(
                            rendered.contains("/nyx ask")
                                && rendered
                                    .contains("faq followed by a topic in its question option")
                        );
                        assert!(rendered.contains("/nyx help"));
                    }
                }
            }
            assert!(
                render_for(
                    &DeliveryReference::Faq {
                        help: None,
                        article_ids: vec![articles[0].id.clone(); 2],
                        linking_enabled: true,
                        public_policy_revision: 1,
                    },
                    &entry
                )
                .is_err()
            );
        }
        assert_eq!(
            select("How do I connect services?").unwrap()[0],
            "connect-services"
        );
    }
}
