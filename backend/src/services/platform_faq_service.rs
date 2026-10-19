//! Public help is served from approved content without an account or model call.
use crate::errors::{AppError, AppResult};
use serde::Deserialize;

#[derive(Deserialize)]
struct Article {
    id: String,
    title: String,
    body: String,
}

fn articles() -> AppResult<Vec<Article>> {
    serde_json::from_str(include_str!("../../content/platform-public-faq.json"))
        .map_err(|_| AppError::Internal("Public help is unavailable".into()))
}

pub(crate) fn select(text: &str) -> AppResult<Vec<String>> {
    let query = text.trim().to_lowercase();
    let query = query.strip_prefix("/faq").unwrap_or(&query).trim();
    let articles = articles()?;
    if query.is_empty() || matches!(query, "/start" | "/help" | "help" | "hello" | "hi") {
        return Ok(Vec::new());
    }
    if let Some(article) = articles.iter().find(|article| article.id == query) {
        return Ok(vec![article.id.clone()]);
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
            let score: usize = words
                .iter()
                .map(|word| {
                    usize::from(title.contains(word)) * 3 + usize::from(body.contains(word))
                })
                .sum();
            (score > 0).then_some((score, article.id))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    Ok(ranked.into_iter().take(2).map(|(_, id)| id).collect())
}

pub(crate) fn render(ids: &[String]) -> AppResult<String> {
    let articles = articles()?;
    if ids.len() > 2 {
        return Err(AppError::ValidationError("Too many FAQ articles".into()));
    }
    let mut text = if ids.is_empty() {
        format!(
            "Welcome to NyxID. Browse approved public help.\n\n{}\n\nSend a text question, or /faq followed by a topic above. Attachments and other commands are unsupported; open NyxID for files and actions.",
            articles
                .iter()
                .map(|a| format!("• {} — /faq {}", a.title, a.id))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        ids.iter()
            .map(|id| {
                let article = articles
                    .iter()
                    .find(|a| &a.id == id)
                    .ok_or_else(|| AppError::ValidationError("Unknown FAQ article".into()))?;
                Ok(format!("{}\n{}", article.title, article.body))
            })
            .collect::<AppResult<Vec<_>>>()?
            .join("\n\n")
    };
    text.push_str("\n\nSend /faq to browse help.");
    Ok(text)
}

pub(crate) fn render_mode(
    ids: &[String],
    linked: bool,
    linking_enabled: bool,
) -> AppResult<String> {
    let mut text = render(ids)?;
    if linked {
        text.push_str("\n\nPersonal mode is linked. Send a text message to use your NyxBot, or /unlink to disconnect this conversation. FAQ answers never use your personal data.");
    } else if linking_enabled {
        text.push_str("\n\nPublic FAQ answers never use personal data. If this conversation is already linked, send a text message to use your NyxBot. Otherwise, send /link here to sign in and connect it.");
    } else {
        text.push_str("\n\nPublic FAQ mode. Personal work and linking are paused by the bot operator, including for previously linked conversations. Open NyxID to use your NyxBot.");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_questions_only_return_approved_content_and_linking_instructions() {
        let ids = select("How do I connect services?").unwrap();
        assert_eq!(ids.first().map(String::as_str), Some("connect-services"));
        let response = render(&ids).unwrap();
        assert!(
            render_mode(&ids, false, true)
                .unwrap()
                .contains("send /link")
        );
        assert!(response.encode_utf16().count() <= 4096);
        assert!(render(&["private-account".into()]).is_err());
    }

    #[test]
    fn start_and_unknown_questions_offer_the_help_menu() {
        for input in ["/start", "/faq", "zzzzzz", ""] {
            assert!(select(input).unwrap().is_empty());
            assert!(render(&[]).unwrap().contains("/faq personal-nyxbot"));
        }
        assert_eq!(select("/faq unlink").unwrap(), vec!["unlink"]);
    }
}
