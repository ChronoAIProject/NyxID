//! Cosmetic suggestions only: never a creation, sender or destination credential.
use super::*;

pub(super) const USERNAME_HELP: &str = "Usernames must be unique, use 5–32 letters, digits or underscores, start with a letter, and end in bot. Telegram checks availability when you create the bot; these suggestions have not been checked.";

fn candidate(label: &str, seed: &str, index: u32) -> String {
    let mut stem = label
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
        .to_ascii_lowercase();
    if stem.is_empty() {
        stem.push_str("nyx");
    }
    if !stem.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
        stem.insert_str(0, "nyx_");
    }
    stem.truncate(21);
    let digest = hash(&format!("telegram-username-v1:{seed}"));
    let suffix = (u32::from_str_radix(&digest[..6], 16).expect("SHA-256 hex") + index) & 0xff_ffff;
    format!("{}_{suffix:06x}_bot", stem.trim_end_matches('_'))
}

pub fn username_suggestions(request: &TelegramBotRequest) -> Vec<String> {
    let seed = format!(
        "{}:{}",
        request.id,
        request.username_seed.as_deref().unwrap_or("initial")
    );
    (0..3)
        .map(|index| candidate(&request.label, &seed, index))
        .collect()
}

pub(super) fn unbound_suggestions() -> Vec<String> {
    let seed = uuid::Uuid::new_v4().to_string();
    (0..3).map(|index| candidate("Nyx", &seed, index)).collect()
}

fn callback_data(request: &TelegramBotRequest) -> String {
    let tag = hash(request.username_seed.as_deref().unwrap_or("initial"));
    format!("tgname:{}:{}", request.id, &tag[..8])
}

pub fn creation_suggestions(
    request: &TelegramBotRequest,
    primary_url: &str,
) -> AppResult<Vec<(String, String)>> {
    let url = url::Url::parse(primary_url).map_err(|_| conflict("Invalid creation form"))?;
    let manager = url
        .path_segments()
        .and_then(|mut parts| parts.nth(1))
        .ok_or_else(|| conflict("Invalid creation form"))?;
    username_suggestions(request)
        .into_iter()
        .map(|username| {
            let url = recovery::native_creation_url(manager, &request.label, &username)?;
            Ok((username, url))
        })
        .collect()
}

pub(super) fn username_keyboard(
    request: &TelegramBotRequest,
    choices: &[(String, String)],
) -> Value {
    choices_keyboard(choices, &callback_data(request))
}

pub(super) fn choices_keyboard(choices: &[(String, String)], callback: &str) -> Value {
    let mut rows = Vec::new();
    for (index, (username, url)) in choices.iter().enumerate() {
        rows.push(json!([{"text": if index == 0 { "Open creation form".to_owned() } else { format!("Use @{username}") }, "url": url}]));
    }
    rows.push(json!([{"text": "Regenerate username", "callback_data": callback}]));
    json!({"inline_keyboard": rows})
}

pub(super) fn username_message(request: &TelegramBotRequest) -> String {
    let names = username_suggestions(request);
    suggestions_message(&names)
}

pub(super) fn suggestions_message(names: &[String]) -> String {
    format!(
        "\n\nSuggested username: <code>{}</code>\nAlternatives: <code>{}</code>, <code>{}</code>\n{USERNAME_HELP}",
        names[0], names[1], names[2]
    )
}

impl TelegramNewService<'_> {
    pub async fn regenerate_username(
        &self,
        actor: &str,
        id: &str,
    ) -> AppResult<TelegramBotRequest> {
        let request = self.get(actor, id).await?;
        if !request.active
            || request.status != Status::WaitingBot
            || request.telegram_user_id.is_none()
            || request.start_update_id.is_none()
            || request.telegram_bot_id.is_some()
            || request.expires_at <= Utc::now()
            || request.prompt_delivery.as_ref().is_some_and(|p| {
                p.outcome == crate::models::telegram_bot_request::PromptOutcome::Sending
                    && p.attempted_at + Duration::seconds(PROMPT_HORIZON_SECS) > Utc::now()
            })
        {
            return Err(conflict(
                "Wait for the current setup message or check setup status before changing suggestions",
            ));
        }
        let seed = uuid::Uuid::new_v4().to_string();
        super::super::transaction_service::run_db_only(
            self.db,
            &(self, request, seed),
            |session, (service, request, seed)| {
                Box::pin(async move {
                    service.fence_prompt_owner(request, session).await?;
                    if request.creation_context.is_some() && request.setup_authority.is_none() {
                        service.fence_creation_context(request, session).await?;
                    }
                    super::super::platform_provisioning_service::fence_request(
                        service.db, request, false, session,
                    )
                    .await?;
                    let mut filter = Self::prompt_filter(request);
                    filter.insert("username_seed", request.username_seed.as_deref());
                    service
                        .db
                        .collection::<TelegramBotRequest>(REQUESTS)
                        .find_one_and_update(filter, doc! {"$set": {"username_seed": seed}})
                        .return_document(ReturnDocument::After)
                        .session(session)
                        .await?
                        .ok_or_else(|| conflict("Setup changed; refresh before regenerating"))
                })
            },
        )
        .await
    }

    pub(super) async fn username_callback(
        &self,
        manager: i64,
        token: &str,
        update_id: i64,
        callback: &Value,
    ) -> AppResult<()> {
        let Some(user) = callback["from"]["id"].as_i64().filter(|id| *id > 0) else {
            return Ok(());
        };
        let Some(data) = callback["data"].as_str() else {
            return Ok(());
        };
        // Buttons must belong to this manager's original private message. Use
        // the shared forward/quote checks with the authenticated button sender.
        let mut original = callback["message"].clone();
        if original["from"]["id"].as_i64() != Some(manager)
            || original["from"]["is_bot"] != true
            || original["chat"]["type"] != "private"
            || original["chat"]["id"].as_i64() != Some(user)
        {
            return Ok(());
        }
        original["from"] = callback["from"].clone();
        if !super::super::telegram_input::original(&json!({"message": original})) {
            return Ok(());
        }
        let permitted = crate::mw::rate_limit::PerKeyRateLimiter::with_db(
            self.db.clone(),
            "telegram_new_usernames",
            5,
            60,
        )
        .check_shared(&format!("{manager}:{user}"))
        .await?;
        if data == "tgname:unbound" {
            let has_request = self
                .db
                .collection::<TelegramBotRequest>(REQUESTS)
                .find_one(doc! {"manager_bot_id": manager, "telegram_user_id": user})
                .await?
                .is_some();
            let platform = self
                .db
                .collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME)
                .find_one(doc! {"manager_bot_id": manager.to_string()})
                .await?
                .is_some();
            self.api.call(token, "answerCallbackQuery", json!({"callback_query_id": callback["id"],
                "text": if permitted && !has_request && !platform { "New username suggestions" } else { "Use /start to review your current setup." }})).await?;
            if permitted && !has_request && !platform {
                self.send_unbound_creation_prompt(manager, token, user, None)
                    .await?;
            }
            return Ok(());
        }
        let Some(id) = data
            .strip_prefix("tgname:")
            .and_then(|s| s.split_once(':'))
            .map(|(id, _)| id)
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        else {
            return Ok(());
        };
        let request = self
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .find_one(doc! {
                "_id": id, "manager_bot_id": manager, "telegram_user_id": user
            })
            .await?;
        let Some(request) = request else {
            return Ok(());
        };
        if !Self::sender_matches(&request, user) {
            return Ok(());
        }
        let saved = if !permitted || data != callback_data(&request) {
            None
        } else {
            match self
                .regenerate_username(&request.actor_user_id, &request.id)
                .await
            {
                Ok(saved) => Some(saved),
                Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_)) => None,
                Err(error) => return Err(error),
            }
        };
        self.api.call(token, "answerCallbackQuery", json!({"callback_query_id": callback["id"],
            "text": if saved.is_some() { "New username suggestions" } else { "Setup changed or suggestions were refreshed. Use the latest message or check status in NyxID." }})).await?;
        if let Some(saved) = saved {
            self.send_creation_prompt(manager, token, user, update_id, Some(&saved))
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_new_username_suggestions_follow_telegram_constraints() {
        for label in [
            "Support Team!",
            "CustomerBot",
            "123 Support",
            "x",
            "客服 🤖",
            "---",
            &"a".repeat(128),
            &"9".repeat(128),
        ] {
            let names: Vec<_> = (0..3)
                .map(|index| candidate(label, "request-one", index))
                .collect();
            for name in &names {
                assert!((5..=32).contains(&name.len()));
                assert!(name.starts_with(|ch: char| ch.is_ascii_alphabetic()));
                assert!(name.ends_with("bot"));
                assert!(
                    name.bytes()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
                );
            }
            assert_eq!(
                names.iter().collect::<std::collections::HashSet<_>>().len(),
                3
            );
        }
        assert!(candidate("Support Team!", "one", 0).starts_with("support_team_"));
        assert!(candidate("云间甜序蛋糕店·顾客订单助手", "one", 0).starts_with("nyx_"));
        assert_ne!(candidate("客服", "one", 0), candidate("客服", "two", 0));
        assert_eq!(
            candidate("Support", "one", 0),
            candidate("Support", "one", 0)
        );
    }
}
