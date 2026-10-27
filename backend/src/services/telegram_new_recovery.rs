use super::*;
use crate::models::telegram_bot_request::CreationContext;
use mongodb::ClientSession;

fn unavailable() -> AppError {
    conflict("This setup cannot open the creation form; refresh its progress or reopen Telegram")
}

pub(super) fn native_creation_url(manager: &str, label: &str) -> AppResult<String> {
    if !(5..=32).contains(&manager.len())
        || !manager
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(unavailable());
    }
    let mut url = url::Url::parse("https://t.me").map_err(|_| unavailable())?;
    url.path_segments_mut().map_err(|_| unavailable())?.extend([
        "newbot",
        manager,
        &suggested_bot_username(label),
    ]);
    url.query_pairs_mut()
        .append_pair("name", &label.chars().take(64).collect::<String>());
    Ok(url.into())
}

impl TelegramNewService<'_> {
    /// Fresh private creation proof is checked by the webhook before this call.
    /// This is the one modern-request consumption point, not a new claim path.
    pub(super) async fn consume_created_bot(
        &self,
        request: &TelegramBotRequest,
        bot_id: i64,
        username: &str,
    ) -> AppResult<()> {
        let mut session = self.db.client().start_session().await?;
        session.start_transaction().await?;
        let result: AppResult<_> = async {
            self.fence_prompt_owner(request, &mut session).await?;
            self.fence_creation_context(request, &mut session).await?;
            let mut filter = Self::prompt_filter(request);
            filter.insert(
                "creation_context",
                bson::to_bson(&request.creation_context).map_err(|_| unavailable())?,
            );
            filter.insert("auto_connect", true);
            filter.insert("setup_authority", bson::Bson::Null);
            filter.insert("bot_username", bson::Bson::Null);
            self.db
                .collection::<TelegramBotRequest>(REQUESTS)
                .update_one(
                    filter,
                    doc! {"$set": {"telegram_bot_id": bot_id, "bot_username": username,
                    "status": "ready", "manager_revision": 1_i64}, "$inc": {"revision": 1_i64}},
                )
                .session(&mut session)
                .await
                .map_err(AppError::from)
        }
        .await;
        match result {
            Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_)) => {
                session.abort_transaction().await?;
                Ok(())
            }
            result => {
                // A lost CAS has no effect and is acknowledged. Database and
                // transaction failures remain retryable; never turn them into ACKs.
                super::super::org_membership_authority::finish(session, result).await?;
                Ok(())
            }
        }
    }

    pub(super) async fn fence_creation_context(
        &self,
        request: &TelegramBotRequest,
        session: &mut ClientSession,
    ) -> AppResult<()> {
        use crate::models::{channel_connect_link as links, nyxbot_channel as watches};
        match request.creation_context.as_ref().ok_or_else(unavailable)? {
            CreationContext::Standalone => Ok(()),
            CreationContext::Hosted {
                link_id,
                link_created_by_user_id,
                link_created_at,
                watch,
            } => {
                let saved = self.db.collection::<bson::Document>(links::COLLECTION_NAME).update_one(
                    doc! {"_id": link_id, "user_id": &request.owner_user_id, "created_by_user_id": link_created_by_user_id,
                        "platform": "telegram-new", "telegram_request_id": &request.id, "label": &request.label,
                        "created_at": bson::DateTime::from_chrono(*link_created_at), "status": "pending", "bot_id": null, "connection_id": null,
                        "expires_at": {"$gt": bson::DateTime::now()}},
                    doc! {"$inc": {"manager_delivery_fence": 1_i64}},
                ).session(&mut *session).await?;
                if saved.matched_count != 1 {
                    return Err(unavailable());
                }
                let Some(watch) = watch else {
                    if self
                        .db
                        .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
                        .find_one(doc! {"connect_link_id": link_id, "kind": "channel_bot"})
                        .session(&mut *session)
                        .await?
                        .is_some()
                    {
                        return Err(unavailable());
                    }
                    return Ok(());
                };
                if watch.user_id != request.owner_user_id
                    || watch.kind != "channel_bot"
                    || watch.platform.as_deref() != Some("telegram")
                {
                    return Err(unavailable());
                }
                let mut filter = doc! {"_id": &watch.id, "user_id": &watch.user_id, "kind": &watch.kind, "platform": &watch.platform,
                "agent_id": &watch.agent_id, "conversation_id": &watch.conversation_id, "connect_link_id": link_id,
                "invocation_id": &watch.invocation_id, "report_to": &watch.report_to,
                "report_provenance": watch.report_provenance as i32,
                "created_at": bson::DateTime::from_chrono(watch.created_at), "expires_at": bson::DateTime::from_chrono(watch.expires_at), "status": "pending"};
                if watch.report_provenance == 0 {
                    filter.insert("report_provenance", doc! {"$in": [0, bson::Bson::Null]});
                }
                if watch.expires_at <= Utc::now() {
                    return Err(unavailable());
                }
                let saved = self
                    .db
                    .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
                    .update_one(filter, doc! {"$inc": {"telegram_creation_fence": 1_i64}})
                    .session(&mut *session)
                    .await?;
                if saved.matched_count != 1 {
                    return Err(unavailable());
                }
                let agent = watch.agent_id.as_deref().ok_or_else(unavailable)?;
                let saved = self.db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME).update_one(
                    doc! {"_id": agent, "user_id": &request.owner_user_id, "destroyed_at": null}, doc! {"$inc": {"telegram_creation_fence": 1_i64}},
                ).session(&mut *session).await?;
                if saved.matched_count != 1 {
                    return Err(unavailable());
                }
                // Hidden member threads are valid sources. Validate their existing
                // group and membership without replacing the saved source identity.
                let source = self
                    .db
                    .collection::<bson::Document>(
                        crate::models::assistant_conversation::COLLECTION_NAME,
                    )
                    .find_one(
                        doc! {"_id": &watch.conversation_id, "user_id": &request.owner_user_id},
                    )
                    .session(&mut *session)
                    .await?
                    .ok_or_else(unavailable)?;
                if let Ok(group) = source.get_str("group_id") {
                    let source_agent = source.get_str("agent_id").map_err(|_| unavailable())?;
                    let saved = self.db.collection::<bson::Document>(crate::models::assistant_group::COLLECTION_NAME).update_one(
                        doc! {"_id": group, "user_id": &request.owner_user_id, "member_agent_ids": source_agent}, doc! {"$inc": {"telegram_creation_fence": 1_i64}},
                    ).session(&mut *session).await?;
                    if saved.matched_count != 1 {
                        return Err(unavailable());
                    }
                }
                let saved = self.db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME).update_one(
                    doc! {"_id": &watch.conversation_id, "user_id": &request.owner_user_id, "group_id": source.get("group_id").cloned().unwrap_or(bson::Bson::Null), "agent_id": source.get("agent_id").cloned().unwrap_or(bson::Bson::Null)},
                    doc! {"$inc": {"telegram_creation_fence": 1_i64}},
                ).session(&mut *session).await?;
                if saved.matched_count != 1 {
                    return Err(unavailable());
                }
                Ok(())
            }
        }
    }

    pub async fn creation_link(
        &self,
        actor: &str,
        id: &str,
    ) -> AppResult<(TelegramBotRequest, String)> {
        let captured = self.get(actor, id).await?;
        if !captured.auto_connect
            || captured.setup_authority.is_some()
            || captured.creation_context.is_none()
            || captured.telegram_user_id.is_none()
            || captured.start_update_id.is_none()
            || captured.telegram_bot_id.is_some()
            || captured.bot_username.is_some()
            || !captured.active
            || captured.status != Status::WaitingBot
            || captured.expires_at <= Utc::now()
        {
            return Err(unavailable());
        }
        let db = self.db.clone();
        let mut session = db.client().start_session().await?;
        let request = captured.clone();
        session.start_transaction().await?;
        let result: AppResult<String> = async {
            self.fence_prompt_owner(&request, &mut session).await?;
            self.fence_creation_context(&request, &mut session).await?;
            let mut filter = Self::prompt_filter(&request);
            filter.insert(
                "creation_context",
                bson::to_bson(&request.creation_context).map_err(|_| unavailable())?,
            );
            filter.insert("setup_authority", bson::Bson::Null);
            filter.insert("auto_connect", true);
            filter.insert("bot_username", bson::Bson::Null);
            let saved = db
                .collection::<bson::Document>(REQUESTS)
                .update_one(filter, doc! {"$inc": {"creation_link_generation": 1_i64}})
                .session(&mut session)
                .await?;
            if saved.modified_count != 1 {
                return Err(unavailable());
            }
            let manager = db
                .collection::<crate::models::platform_credential::PlatformCredential>(
                    crate::models::platform_credential::COLLECTION_NAME,
                )
                .find_one(super::super::telegram_manager_registry::filter(
                    &request.manager_bot_id.to_string(),
                ))
                .session(&mut session)
                .await?
                .ok_or_else(unavailable)?;
            if !manager.secrets.contains_key("manager_bot_token") {
                return Err(unavailable());
            }
            let username = manager
                .fields
                .get("manager_username")
                .ok_or_else(unavailable)?;
            // Validate the URL before committing the issuance fence.
            native_creation_url(username, &request.label)?;
            Ok(username.clone())
        }
        .await;
        let manager_name = super::super::org_membership_authority::finish(session, result).await?;
        let url = native_creation_url(&manager_name, &captured.label)?;
        Ok((captured, url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::platform_credential::COLLECTION_NAME as CREDENTIALS;
    use crate::services::telegram_new_tests::{fixture, headers, message, service};
    #[tokio::test]
    async fn telegram_new_consumption_rechecks_expiry_and_exact_request_snapshot() {
        for change in ["expiry", "generation", "context", "owner", "manager"] {
            let (state, actor, server) = fixture().await;
            let base = server.uri();
            let service = service(&state, &base);
            let (request, launch) = service
                .begin(&actor, &actor, "Support", true)
                .await
                .unwrap();
            let challenge = url::Url::parse(&launch)
                .unwrap()
                .query_pairs()
                .find(|(key, _)| key == "start")
                .unwrap()
                .1
                .into_owned();
            let mut command = message(json!({"text":format!("/start {challenge}")}));
            command["update_id"] = json!(10);
            service
                .webhook(
                    &headers(&state).await,
                    &serde_json::to_vec(&command).unwrap(),
                )
                .await
                .unwrap();
            let captured = service.get(&actor, &request.id).await.unwrap();
            match change {
                "expiry" => {
                    state.db.collection::<bson::Document>(REQUESTS).update_one(doc! {"_id":&request.id}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
                }
                "generation" => {
                    state
                        .db
                        .collection::<bson::Document>(REQUESTS)
                        .update_one(
                            doc! {"_id":&request.id},
                            doc! {"$inc":{"connection_generation":1_i64}},
                        )
                        .await
                        .unwrap();
                }
                "context" => {
                    state
                        .db
                        .collection::<bson::Document>(REQUESTS)
                        .update_one(
                            doc! {"_id":&request.id},
                            doc! {"$unset":{"creation_context":""}},
                        )
                        .await
                        .unwrap();
                }
                "owner" => {
                    state
                        .db
                        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
                        .update_one(doc! {"_id":&actor}, doc! {"$set":{"is_active":false}})
                        .await
                        .unwrap();
                }
                "manager" => {
                    state
                        .db
                        .collection::<bson::Document>(CREDENTIALS)
                        .update_one(
                            doc! {"provider":"telegram-new"},
                            doc! {"$set":{"fields.observation_id":"rotated"}},
                        )
                        .await
                        .unwrap();
                }
                _ => unreachable!(),
            }
            service
                .consume_created_bot(&captured, 900, "CustomerBot")
                .await
                .unwrap();
            let saved = state
                .db
                .collection::<TelegramBotRequest>(REQUESTS)
                .find_one(doc! {"_id": &request.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved.status, Status::WaitingBot, "{change}");
            assert!(saved.telegram_bot_id.is_none(), "{change}");
        }
    }
}
