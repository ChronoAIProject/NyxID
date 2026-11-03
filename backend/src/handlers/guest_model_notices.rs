//! Delivery is fixed text, independent of the model whose setup failed.
use super::*;
use crate::services::assistant_service_execution::{COLLECTION, readiness_notice};
pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    let rows: Vec<bson::Document> = state
        .db
        .collection::<bson::Document>(COLLECTION)
        .find(doc! {"kind":readiness_notice::KIND,"pending":true,"expires_at":{"$gt":bson::DateTime::now()}})
        .sort(doc! {"created_at":1})
        .limit(25)
        .await?
        .try_collect()
        .await?;
    for notice in rows {
        let owner = notice
            .get_str("user_id")
            .map_err(|_| AppError::Internal("Invalid setup notice".into()))?;
        let agent_id = notice
            .get_str("agent_id")
            .map_err(|_| AppError::Internal("Invalid setup notice".into()))?;
        let id = notice
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid setup notice".into()))?;
        let agent = match crate::services::assistant_team_service::agent(&state.db, owner, agent_id)
            .await
        {
            Ok(agent) if agent.destroyed_at.is_none() => agent,
            Ok(_) | Err(AppError::NotFound(_)) => {
                state
                    .db
                    .collection::<bson::Document>(COLLECTION)
                    .update_one(doc! {"_id":id}, doc! {"$set":{"pending":false}})
                    .await?;
                continue;
            }
            Err(error) => return Err(error),
        };
        let home = crate::services::assistant_team_service::home_thread_for(
            &state.db,
            &state.encryption_keys,
            owner,
            &agent,
        )
        .await?;
        let link = crate::services::assistant_links::AssistantPage::AgentDetails {
            agent: &agent.id,
            tab: crate::services::assistant_links::AGENT_DETAILS_GRANTS,
        }
        .url(&state.config.frontend_url);
        let text = format!(
            "{} could not finish model setup for visitors. {}\n\nOpen assistant settings: {link}",
            crate::services::assistant_nyxagent::identifier(&agent.name),
            notice.get_str("fix").unwrap_or(
                "Check the model connection and available credits. Visitor setup retries automatically."
            )
        );
        readiness_notice::in_app(&state.db, &notice, &home.id, &text).await?;
        // Each destination has an at-most-once send barrier. A process crash or
        // uncertain platform send never causes automatic duplicate messages.
        let chats: Vec<NyxbotThread> = state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find(doc! {"user_id":owner,"owner_chat":true,"kind":"private"})
            .limit(100)
            .await?
            .try_collect()
            .await?;
        let nyxbot = state
            .db
            .collection::<bson::Document>("assistant_agents")
            .find_one(doc! {"user_id":owner,"kind":"nyxbot"})
            .projection(doc! {"_id":1})
            .await?;
        let nyxbot_id = nyxbot.as_ref().and_then(|r| r.get_str("_id").ok());
        for chat in chats {
            let channel = match load_channel(state, owner, &chat.channel_id).await {
                Ok(channel) => channel,
                Err(AppError::NotFound(_)) => continue,
                Err(error) => return Err(error),
            };
            if channel.status != "active"
                || channel.owner_sender_ids.is_empty()
                || channel.agent_id.as_deref().or(nyxbot_id) != Some(agent_id)
            {
                continue;
            }
            let claimed = state
                .db
                .collection::<bson::Document>(COLLECTION)
                .update_one(
                    doc! {"_id":id,"pending":true,"sent_chats":{"$ne":&chat.id}},
                    doc! {"$addToSet":{"sent_chats":&chat.id}},
                )
                .await?;
            if claimed.modified_count == 1 {
                let _ = chats::setup_notice(state, owner, &chat.id, &text, id).await;
            }
        }
        state
            .db
            .collection::<bson::Document>(COLLECTION)
            .update_one(doc! {"_id":id}, doc! {"$set":{"pending":false}})
            .await?;
    }
    Ok(())
}
