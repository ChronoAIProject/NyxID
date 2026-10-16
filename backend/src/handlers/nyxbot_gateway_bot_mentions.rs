//! Management-time, reader-before-writer negotiation for exact native bot input.
//! A failed/old discovery keeps existing human-compatible connections working.
use super::*;

pub(super) fn desired(row: &NyxbotChannel, bot: &ChannelBot) -> bool {
    if !row.gateway_bot_mentions.supported {
        return false;
    }
    match canonical_platform(&row.platform) {
        "telegram" => {
            bot.platform_bot_id
                .parse::<i64>()
                .is_ok_and(|id| id > 0 && id.to_string() == bot.platform_bot_id)
                && (5..=32).contains(&bot.platform_bot_username.trim_start_matches('@').len())
                && bot
                    .platform_bot_username
                    .trim_start_matches('@')
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }
        "lark" | "feishu" => row
            .gateway_bot_id
            .as_deref()
            .is_some_and(|id| id.starts_with("ou_") && id.len() > 3),
        _ => false,
    }
}

pub(super) async fn discover(state: &AppState, row: &mut NyxbotChannel) -> AppResult<()> {
    if !matches!(
        canonical_platform(&row.platform),
        "telegram" | "lark" | "feishu"
    ) || row
        .gateway_bot_mentions
        .checked_at
        .is_some_and(|at| at + ChronoDuration::minutes(10) > Utc::now())
    {
        return Ok(());
    }
    let creator = creator_bearer(state, &row.user_id)?;
    let response = gateway_call(
        state,
        reqwest::Method::GET,
        "/channel-capabilities",
        &creator,
        None,
        None,
    )
    .await;
    row.gateway_bot_mentions.checked_at = Some(Utc::now());
    if let Ok(response) = response {
        if response.status == 200 {
            row.gateway_bot_mentions.supported = response.body["source_features"]
                .as_array()
                .is_some_and(|v| v.iter().any(|v| v == "native_bot_mentions_v1"));
        } else if response.status == 404 {
            row.gateway_bot_mentions.supported = false;
        }
    }
    if matches!(row.platform.as_str(), "lark" | "feishu")
        && row
            .gateway_bot_id
            .as_deref()
            .is_none_or(|id| !id.starts_with("ou_"))
    {
        let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
        row.gateway_bot_id = bot_user_id(state, &bot)
            .await
            .filter(|id| id.starts_with("ou_"));
    }
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id,"transport":"gateway"},
        doc! {"$set":{"gateway_bot_mentions.supported":row.gateway_bot_mentions.supported,
            "gateway_bot_mentions.checked_at":bson::DateTime::from_chrono(row.gateway_bot_mentions.checked_at.unwrap())}}).await?;
    Ok(())
}
