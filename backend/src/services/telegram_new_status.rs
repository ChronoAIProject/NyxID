//! Read-only setup explanations shared by the browser and manager. These never
//! grant creation authority or change an existing request's destination.
use super::*;
use crate::models::telegram_bot_request::CreationContext;
use crate::services::assistant_links::AssistantPage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SetupReason {
    Expired,
    Completed,
    Cancelled,
    Superseded,
    Missing,
    AccountMismatch,
    ManagerChanged,
    AccessChanged,
    ContextChanged,
    InProgress,
    Suspended,
    Retry,
}

impl SetupReason {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
            Self::Missing => "missing",
            Self::AccountMismatch => "account_mismatch",
            Self::ManagerChanged => "manager_changed",
            Self::AccessChanged => "access_changed",
            Self::ContextChanged => "context_changed",
            Self::InProgress => "in_progress",
            Self::Suspended => "suspended",
            Self::Retry => "retry",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Expired => {
                "This bot setup has expired. Start a new setup in NyxID. Any bot you already created still exists in Telegram."
            }
            Self::Completed => {
                "This bot setup is complete. Open your agent or check the saved bot in NyxID."
            }
            Self::Cancelled => {
                "This bot setup was cancelled. Start a new setup in NyxID when you are ready."
            }
            Self::Superseded => {
                "This setup link was replaced or its destination changed. Check the current setup in NyxID and use its latest Telegram link."
            }
            Self::Missing => {
                "This setup link could not be found. It may have been removed. Start a new setup in NyxID."
            }
            Self::AccountMismatch => {
                "This setup was opened by a different Telegram account. Use the original account, or start your own setup in NyxID."
            }
            Self::ManagerChanged => {
                "The Telegram setup manager changed. Check status in NyxID and start a new setup with the current manager."
            }
            Self::AccessChanged => {
                "Access to this setup has changed. Sign in to NyxID to review your access or start a new setup."
            }
            Self::ContextChanged => {
                "The account or agent context for this setup is no longer available. Check status in NyxID before starting a new setup."
            }
            Self::InProgress => {
                "Your bot has been created and is awaiting connection. Check status in NyxID; you do not need to create another bot."
            }
            Self::Suspended => {
                "This bot connection is suspended after a management change. Check its status in NyxID to review recovery."
            }
            Self::Retry => {
                "This setup changed while processing Start. Check its current status in NyxID, then reopen Telegram."
            }
        }
    }
}

pub struct SetupNotice {
    pub reason: &'static str,
    pub message: &'static str,
    pub status_url: String,
    pub new_setup_url: String,
    pub agent_url: Option<String>,
}

impl TelegramNewService<'_> {
    pub(super) fn sender_matches(request: &TelegramBotRequest, sender: i64) -> bool {
        request.telegram_user_id.is_none_or(|id| id == sender)
            && request
                .setup_authority
                .as_ref()
                .is_none_or(|a| a.telegram_sender_id == sender)
    }

    pub(super) async fn setup_reason(
        &self,
        request: &TelegramBotRequest,
    ) -> AppResult<Option<SetupReason>> {
        use SetupReason::*;
        let reason = match request.status {
            Status::Connected => Some(Completed),
            Status::Cancelled => Some(Cancelled),
            Status::Expired => Some(Expired),
            Status::Suspended => Some(Suspended),
            Status::Provisioning => Some(InProgress),
            _ if request.expires_at <= Utc::now() => Some(Expired),
            Status::Ready => Some(InProgress),
            _ => None,
        };
        if reason.is_some() {
            return Ok(reason);
        }
        let manager = super::super::telegram_manager_registry::load(
            self.db,
            Some(&request.manager_bot_id.to_string()),
        )
        .await?;
        if manager.is_none_or(|row| {
            row.fields.get("observation_id") != Some(&request.observation_id)
                || row.fields.get("webhook_ready").map(String::as_str) != Some("true")
                || !row.secrets.contains_key(MANAGER_TOKEN)
        }) {
            return Ok(Some(ManagerChanged));
        }
        if request.setup_authority.is_some() {
            let mut session = self.db.client().start_session().await?;
            session.start_transaction().await?;
            let result = super::super::platform_provisioning_service::fence_request(
                self.db,
                request,
                false,
                &mut session,
            )
            .await;
            session.abort_transaction().await?;
            match result {
                Ok(_) => {}
                Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_)) => {
                    return Ok(Some(ContextChanged));
                }
                Err(error) => return Err(error),
            }
        } else if request.creation_context.is_some() {
            let mut session = self.db.client().start_session().await?;
            session
                .start_transaction()
                .read_concern(mongodb::options::ReadConcern::snapshot())
                .await?;
            let result = self
                .check_creation_context(request, &mut session, false)
                .await;
            session.abort_transaction().await?;
            match result {
                Ok(()) => {}
                Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_)) => {
                    return self.closed_context_reason(request).await.map(Some);
                }
                Err(error) => return Err(error),
            }
        }
        Ok((!request.active).then_some(ContextChanged))
    }

    async fn closed_context_reason(&self, request: &TelegramBotRequest) -> AppResult<SetupReason> {
        use SetupReason::*;
        if let Some(CreationContext::Hosted { link_id, watch, .. }) = &request.creation_context {
            let link = self
                .db
                .collection::<bson::Document>(crate::models::channel_connect_link::COLLECTION_NAME)
                .find_one(doc! {"_id": link_id})
                .await?;
            let Some(link) = link else {
                return Ok(Missing);
            };
            match link.get_str("status").ok() {
                Some("cancelled") => return Ok(Cancelled),
                Some("expired") => return Ok(Expired),
                // A completed link cannot authorize another creation, even if a
                // stale request still says waiting_bot.
                Some("completed") => return Ok(Completed),
                _ => {}
            }
            if link.get_datetime("expires_at").is_ok_and(|expires_at| {
                expires_at.to_chrono()
                    <= Utc::now()
                        - Duration::seconds(
                            super::super::channel_connect_link_service::GRACE_SECONDS,
                        )
            }) {
                return Ok(Expired);
            }
            if let Some(watch) = watch {
                if watch.expires_at <= Utc::now() {
                    return Ok(Expired);
                }
                let current = self
                    .db
                    .collection::<bson::Document>(
                        crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
                    )
                    .find_one(doc! {"_id": &watch.id})
                    .await?;
                let Some(current) = current else {
                    return Ok(Missing);
                };
                if current.get_str("connect_link_id").ok() != Some(link_id)
                    || current.get_str("agent_id").ok() != watch.agent_id.as_deref()
                    || current.get_str("conversation_id").ok() != Some(&watch.conversation_id)
                {
                    return Ok(Superseded);
                }
                return Ok(ContextChanged);
            }
            return Ok(Superseded);
        }
        Ok(ContextChanged)
    }

    pub async fn setup_notice(
        &self,
        request: &TelegramBotRequest,
    ) -> AppResult<Option<SetupNotice>> {
        let Some(reason) = self.setup_reason(request).await? else {
            return Ok(None);
        };
        self.notice_for(reason, Some(request)).await.map(Some)
    }

    async fn notice_for(
        &self,
        reason: SetupReason,
        request: Option<&TelegramBotRequest>,
    ) -> AppResult<SetupNotice> {
        let mut agent_url = None;
        if let Some(request) = request {
            // Use only a live owner-bound agent. Do not adopt a replacement watch
            // or expose a hidden group member conversation as a return URL.
            let agent = match &request.creation_context {
                Some(CreationContext::Hosted {
                    watch: Some(watch), ..
                }) => watch.agent_id.as_deref(),
                _ => None,
            };
            if let Some(agent) = agent
                && self.db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                    .find_one(doc! {"_id": agent, "user_id": &request.owner_user_id, "destroyed_at": null}).await?.is_some() {
                agent_url = Some(AssistantPage::Agent { agent: Some(agent) }.url(&self.config.frontend_url));
            } else if reason == SetupReason::Completed {
                agent_url = Some(AssistantPage::Agent { agent: None }.url(&self.config.frontend_url));
            }
        }
        Ok(SetupNotice {
            reason: reason.code(),
            message: reason.message(),
            status_url: AssistantPage::TelegramSetup {
                request: request.map(|r| r.id.as_str()),
            }
            .url(&self.config.frontend_url),
            new_setup_url: AssistantPage::TelegramSetup { request: None }
                .url(&self.config.frontend_url),
            agent_url,
        })
    }

    pub(super) async fn send_setup_notice(
        &self,
        token: &str,
        sender: i64,
        request: Option<&TelegramBotRequest>,
        fallback: SetupReason,
    ) -> AppResult<()> {
        let matches = request.is_none_or(|r| Self::sender_matches(r, sender));
        let mut visible = request.filter(|r| {
            matches && (fallback != SetupReason::Superseded || r.telegram_user_id == Some(sender))
        });
        let mut reason = if matches {
            fallback
        } else {
            SetupReason::AccountMismatch
        };
        if let Some(request) = visible {
            match self
                .check_owner(&request.actor_user_id, &request.owner_user_id)
                .await
            {
                Ok(()) => {
                    reason = self.setup_reason(request).await?.unwrap_or(fallback);
                }
                Err(AppError::Forbidden(_) | AppError::NotFound(_)) => {
                    visible = None;
                    reason = SetupReason::AccessChanged;
                }
                Err(error) => return Err(error),
            }
        }
        let notice = self.notice_for(reason, visible).await?;
        tracing::info!(reason = notice.reason, request_id = request.map(|r| r.id.as_str()),
            status = ?request.map(|r| r.status), sender_matches = matches,
            "Telegram setup recovery");
        // Plain links let us also remove a stale reply keyboard in this one send.
        let mut text = format!(
            "{}\n\nCheck status: {}\nStart a new setup: {}",
            notice.message, notice.status_url, notice.new_setup_url
        );
        if let Some(url) = notice.agent_url {
            text.push_str(&format!("\nOpen the agent: {url}"));
        }
        self.api.call(token, "sendMessage", json!({"chat_id": sender, "text": text,
            "link_preview_options": {"is_disabled": true}, "reply_markup": {"remove_keyboard": true}})).await?;
        Ok(())
    }
}
