use super::*;
use crate::models::telegram_bot_request::{
    PromptDelivery, PromptOutcome, PromptPhase, TelegramDiagnostic,
};

// Each dispatch phase has a renewable lease. Reserve the entire bounded API call
// after transaction work; a successor may replace a claim only after this horizon.
pub(crate) const PROMPT_HORIZON_SECS: i64 = 45;

pub(super) fn prompt_dispatch_has_budget(
    phase_started_at: chrono::DateTime<Utc>,
    now: chrono::DateTime<Utc>,
) -> bool {
    now + Duration::seconds(super::super::telegram_new_api::CALL_TIMEOUT_SECS as i64)
        <= phase_started_at + Duration::seconds(PROMPT_HORIZON_SECS)
}

impl TelegramNewService<'_> {
    /// Real writes serialize admission with disable/delete/demotion and manager rotation.
    pub(super) async fn fence_prompt_owner(
        &self,
        request: &TelegramBotRequest,
        session: &mut mongodb::ClientSession,
    ) -> AppResult<()> {
        super::super::org_membership_authority::fence_live_member(
            self.db,
            &request.actor_user_id,
            session,
        )
        .await?;
        if request.actor_user_id != request.owner_user_id {
            super::super::org_membership_authority::fence_org(
                self.db,
                &request.owner_user_id,
                session,
            )
            .await?;
            let owner = self
                .db
                .collection::<bson::Document>(USERS)
                .find_one(doc! {"_id": &request.owner_user_id, "is_active": true})
                .session(&mut *session)
                .await?;
            let member = self.db.collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME).update_one(
                doc! {"org_user_id": &request.owner_user_id, "member_user_id": &request.actor_user_id, "role": "admin", "revoked_at": null},
                doc! {"$inc": {"telegram_creation_fence": 1_i64}},
            ).session(&mut *session).await?;
            if owner.is_none() || member.matched_count != 1 {
                return Err(conflict("Destination access changed"));
            }
        }
        let mut filter =
            super::super::telegram_manager_registry::filter(&request.manager_bot_id.to_string());
        filter.insert("fields.observation_id", &request.observation_id);
        filter.insert("fields.webhook_ready", "true");
        let manager = self
            .db
            .collection::<bson::Document>(crate::models::platform_credential::COLLECTION_NAME)
            .update_one(filter, doc! {"$inc": {"telegram_creation_fence": 1_i64}})
            .session(&mut *session)
            .await?;
        if manager.matched_count != 1 {
            return Err(conflict("Manager configuration changed"));
        }
        Ok(())
    }

    pub(super) fn prompt_filter(request: &TelegramBotRequest) -> bson::Document {
        let mut filter = doc! {"_id": &request.id, "actor_user_id": &request.actor_user_id, "owner_user_id": &request.owner_user_id,
        "manager_bot_id": request.manager_bot_id, "observation_id": &request.observation_id,
        "revision": request.revision, "connection_generation": request.connection_generation,
        "telegram_user_id": request.telegram_user_id, "start_update_id": request.start_update_id,
        "status": "waiting_bot", "active": true, "telegram_bot_id": null,
        "expires_at": {"$gt": bson::DateTime::now()}};
        if request.connection_generation == 0 {
            filter.insert(
                "connection_generation",
                doc! {"$in": [0_i64, bson::Bson::Null]},
            );
        }
        filter
    }

    pub(super) async fn send_creation_prompt(
        &self,
        manager_id: i64,
        token: &str,
        user: i64,
        update_id: i64,
        captured: Option<&TelegramBotRequest>,
    ) -> AppResult<()> {
        let Some(captured) = captured else {
            return self
                .send_unbound_creation_prompt(manager_id, token, user, None)
                .await;
        };
        // Never rely on the pre-handoff snapshot for retry authority.
        let Some(request) = self
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .find_one(Self::prompt_filter(captured))
            .await?
        else {
            return Ok(());
        };
        if !self.owner_still_authorized(&request).await? {
            return Ok(());
        }
        let retry_rejected_instructions = request.prompt_delivery.as_ref().is_some_and(|attempt| {
            attempt.update_id == update_id
                && attempt.phase == PromptPhase::Instructions
                && attempt.outcome == PromptOutcome::Rejected
        });
        if request
            .prompt_update_boundary
            .is_some_and(|boundary| update_id <= boundary)
            && !retry_rejected_instructions
        {
            return Ok(());
        }
        if request.prompt_delivery.as_ref().is_some_and(|attempt| {
            attempt.outcome == PromptOutcome::Sending
                && attempt.attempted_at + Duration::seconds(PROMPT_HORIZON_SECS) > Utc::now()
        }) {
            return Ok(());
        }
        let attempt = PromptDelivery {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            update_id,
            phase: PromptPhase::Instructions,
            outcome: PromptOutcome::Sending,
            attempted_at: Utc::now(),
            completed_at: None,
            diagnostic: None,
        };
        if !self.claim_prompt(&request, &attempt).await? {
            return Ok(());
        }
        let mut text = creation_message(&request);
        if request.auto_connect
            && request.setup_authority.is_none()
            && request.creation_context.is_some()
        {
            match self.creation_link(&request.actor_user_id, &request.id).await {
                Ok((_, url)) => text.push_str(&format!("\n\nButton missing? <a href=\"{}\">Open the same creation form</a>. Use the same Telegram account and check the NyxID destination above.", html_escape(&url))),
                Err(_) => tracing::info!(request_id = %request.id, "Telegram instruction recovery link unavailable"),
            }
        }
        let phase_started_at = match self
            .prepare_prompt_dispatch(&request, &attempt, PromptPhase::Instructions)
            .await
        {
            Ok(Some(started)) => started,
            Ok(None)
            | Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_)) => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if !prompt_dispatch_has_budget(phase_started_at, Utc::now()) {
            return Ok(());
        }
        let instructions = self.api.send_prompt(token, json!({"chat_id": user, "text": format!("{text}\n\nIf no Create bot button appears, send /start to try again."), "parse_mode": "HTML", "link_preview_options": {"is_disabled": true}})).await;
        if let Err(diagnostic) = instructions {
            self.set_prompt_outcome(
                &request,
                &attempt,
                PromptPhase::Instructions,
                Some(&diagnostic),
            )
            .await;
            // A definite first-send rejection may be redelivered. An uncertain send
            // stays acknowledged, requiring an explicit new /start to try again.
            return if diagnostic.uncertain() {
                Ok(())
            } else {
                Err(diagnostic.app_error())
            };
        }
        // Mark keyboard dispatch durably before the effect. A metadata failure
        // after accepted instructions must be acknowledged, never blindly replayed.
        let phase_started_at = self
            .prepare_prompt_dispatch(&request, &attempt, PromptPhase::Keyboard)
            .await;
        let Ok(Some(phase_started_at)) = phase_started_at else {
            tracing::warn!(request_id = %request.id, "Telegram keyboard dispatch withheld after live fence check");
            return Ok(());
        };
        if !prompt_dispatch_has_budget(phase_started_at, Utc::now()) {
            return Ok(());
        }
        let creation = json!({"request_id": 1, "suggested_name": request.label.chars().take(64).collect::<String>(), "suggested_username": suggested_bot_username(&request.label)});
        let result = self.api.send_prompt(token, json!({"chat_id": user, "text": "Tap Create bot below to open Telegram's creation form.", "reply_markup": {"keyboard": [[{"text": "Create bot", "request_managed_bot": creation}]], "resize_keyboard": true, "one_time_keyboard": true}})).await;
        self.set_prompt_outcome(
            &request,
            &attempt,
            PromptPhase::Keyboard,
            result.as_ref().err(),
        )
        .await;
        Ok(())
    }

    /// Compare the full observed lease as well as its identity. A concurrent
    /// renewal may keep the same attempt ID and update boundary while making an
    /// apparently expired read live again before this transaction begins.
    async fn claim_prompt(
        &self,
        request: &TelegramBotRequest,
        attempt: &PromptDelivery,
    ) -> AppResult<bool> {
        let mut filter = Self::prompt_filter(request);
        filter.insert("prompt_update_boundary", request.prompt_update_boundary);
        filter.insert(
            "prompt_delivery.attempt_id",
            request
                .prompt_delivery
                .as_ref()
                .map(|p| p.attempt_id.as_str()),
        );
        if let Some(observed) = &request.prompt_delivery {
            filter.insert(
                "prompt_delivery.attempted_at",
                bson::DateTime::from_chrono(observed.attempted_at),
            );
            filter.insert(
                "prompt_delivery.phase",
                bson::to_bson(&observed.phase)
                    .map_err(|_| AppError::Internal("Invalid prompt phase".into()))?,
            );
            filter.insert(
                "prompt_delivery.outcome",
                bson::to_bson(&observed.outcome)
                    .map_err(|_| AppError::Internal("Invalid prompt outcome".into()))?,
            );
        }
        let mut session = self.db.client().start_session().await?;
        session.start_transaction().await?;
        let admission: AppResult<_> = async {
        self.fence_prompt_owner(request, &mut session).await?;
        if request.creation_context.is_some() && request.setup_authority.is_none() {
            self.fence_creation_context(request, &mut session).await?;
        }
        super::super::platform_provisioning_service::fence_request(
            self.db,
            request,
            false,
            &mut session,
        )
        .await?;
        let claimed = self.db.collection::<bson::Document>(REQUESTS).update_one(filter, doc! {"$set": {"prompt_delivery": bson::to_bson(attempt).map_err(|_| AppError::Internal("Invalid prompt metadata".into()))?, "prompt_update_boundary": attempt.update_id}}).session(&mut session).await?;
            Ok::<_, AppError>(claimed)
        }.await;
        let claimed = match admission {
            Ok(claimed) => claimed,
            Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_)) => {
                session.abort_transaction().await?;
                return Ok(false);
            }
            Err(error) => {
                session.abort_transaction().await?;
                return Err(error);
            }
        };
        session.commit_transaction().await?;
        Ok(claimed.modified_count == 1)
    }

    /// Renew the phase lease immediately before dispatch, without reviving an
    /// expired claim. The final caller-side check reserves the full API timeout
    /// even if the write or commit took most of the renewed lease.
    async fn prepare_prompt_dispatch(
        &self,
        request: &TelegramBotRequest,
        attempt: &PromptDelivery,
        phase: PromptPhase,
    ) -> AppResult<Option<chrono::DateTime<Utc>>> {
        let mut session = self.db.client().start_session().await?;
        session.start_transaction().await?;
        let admitted: AppResult<_> = async {
            self.fence_prompt_owner(request, &mut session).await?;
            if request.creation_context.is_some() && request.setup_authority.is_none() {
                self.fence_creation_context(request, &mut session).await?;
            }
            super::super::platform_provisioning_service::fence_request(
                self.db, request, false, &mut session,
            ).await?;
            let started_at = Utc::now();
            let mut filter = Self::prompt_filter(request);
            filter.insert("prompt_delivery.attempt_id", &attempt.attempt_id);
            filter.insert("prompt_delivery.phase", "instructions");
            filter.insert("prompt_delivery.outcome", "sending");
            filter.insert("prompt_delivery.attempted_at", doc! {
                "$gt": bson::DateTime::from_chrono(started_at - Duration::seconds(PROMPT_HORIZON_SECS))
            });
            let saved = self.db.collection::<bson::Document>(REQUESTS).update_one(filter, doc! {
                "$set": {
                    "prompt_delivery.phase": bson::to_bson(&phase).map_err(|_| AppError::Internal("Invalid prompt phase".into()))?,
                    "prompt_delivery.attempted_at": bson::DateTime::from_chrono(started_at)
                }
            }).session(&mut session).await?;
            Ok((saved.matched_count == 1).then_some(started_at))
        }.await;
        // Preserve the distinction between deliberate guard closures and DB or
        // commit errors. Callers must not ACK a mapped transient commit conflict.
        let prepared = match admitted {
            Ok(prepared) => prepared,
            Err(error) => {
                session.abort_transaction().await?;
                return Err(error);
            }
        };
        session.commit_transaction().await?;
        Ok(prepared)
    }

    async fn set_prompt_outcome(
        &self,
        request: &TelegramBotRequest,
        attempt: &PromptDelivery,
        phase: PromptPhase,
        diagnostic: Option<&TelegramDiagnostic>,
    ) {
        let outcome = match diagnostic {
            None => PromptOutcome::Accepted,
            Some(d) if d.uncertain() => PromptOutcome::Unknown,
            Some(_) => PromptOutcome::Rejected,
        };
        let mut filter = Self::prompt_filter(request);
        filter.insert("prompt_delivery.attempt_id", &attempt.attempt_id);
        filter.insert("prompt_delivery.outcome", "sending");
        let result: AppResult<_> = async {
            let mut session = self.db.client().start_session().await?;
            session.start_transaction().await?;
            self.fence_prompt_owner(request, &mut session).await?;
            super::super::platform_provisioning_service::fence_request(self.db, request, false, &mut session).await?;
            let saved = self.db.collection::<bson::Document>(REQUESTS).update_one(filter, doc! {"$set": {
                "prompt_delivery.phase": bson::to_bson(&phase).map_err(|_| AppError::Internal("Invalid prompt phase".into()))?,
                "prompt_delivery.outcome": bson::to_bson(&outcome).map_err(|_| AppError::Internal("Invalid prompt outcome".into()))?,
                "prompt_delivery.completed_at": bson::DateTime::now(),
                "prompt_delivery.diagnostic": bson::to_bson(&diagnostic).map_err(|_| AppError::Internal("Invalid prompt diagnostic".into()))?
            }}).session(&mut session).await?;
            session.commit_transaction().await?;
            Ok::<_, AppError>(saved)
        }.await;
        if result.is_err() {
            tracing::warn!(request_id = %request.id, "Telegram prompt outcome write failed; dispatch remains unconfirmed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn telegram_new_stale_expired_observation_cannot_replace_committed_phase_renewal() {
        use crate::services::telegram_new_tests::{fixture, headers, message, service};
        for phase in [PromptPhase::Instructions, PromptPhase::Keyboard] {
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
            state.db.collection::<bson::Document>(REQUESTS).update_one(doc! {"_id":&request.id}, doc! {
                "$set": {"prompt_delivery.phase":"instructions", "prompt_delivery.outcome":"sending",
                    "prompt_delivery.attempted_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(46))}
            }).await.unwrap();
            let observed = service.get(&actor, &request.id).await.unwrap();
            let mut renewed = state.db.client().start_session().await.unwrap();
            renewed.start_transaction().await.unwrap();
            service
                .fence_prompt_owner(&observed, &mut renewed)
                .await
                .unwrap();
            state
                .db
                .collection::<bson::Document>(REQUESTS)
                .update_one(
                    doc! {"_id":&request.id},
                    doc! {
                        "$set":{"prompt_delivery.phase":bson::to_bson(&phase).unwrap(),
                            "prompt_delivery.attempted_at":bson::DateTime::now()}
                    },
                )
                .session(&mut renewed)
                .await
                .unwrap();
            // Outside readers still see the expired lease until renewal commits.
            let stale = state
                .db
                .collection::<TelegramBotRequest>(REQUESTS)
                .find_one(doc! {"_id":&request.id})
                .await
                .unwrap()
                .unwrap();
            assert!(
                stale.prompt_delivery.as_ref().unwrap().attempted_at
                    + Duration::seconds(PROMPT_HORIZON_SECS)
                    < Utc::now()
            );
            renewed.commit_transaction().await.unwrap();
            let successor = PromptDelivery {
                attempt_id: uuid::Uuid::new_v4().to_string(),
                update_id: 11,
                phase: PromptPhase::Instructions,
                outcome: PromptOutcome::Sending,
                attempted_at: Utc::now(),
                completed_at: None,
                diagnostic: None,
            };
            assert!(!service.claim_prompt(&stale, &successor).await.unwrap());
            let saved = service.get(&actor, &request.id).await.unwrap();
            assert_eq!(
                saved.prompt_delivery.as_ref().unwrap().attempt_id,
                observed.prompt_delivery.as_ref().unwrap().attempt_id
            );
            assert_eq!(saved.prompt_delivery.as_ref().unwrap().phase, phase);
            assert_eq!(saved.prompt_update_boundary, Some(10));
            assert_eq!(saved.start_update_id, observed.start_update_id);
            assert_eq!(saved.revision, observed.revision);
            assert_eq!(server.received_requests().await.unwrap().len(), 2);
        }
    }

    #[test]
    fn telegram_new_dispatch_reserves_full_provider_budget_after_slow_commit() {
        let started = Utc::now();
        assert!(prompt_dispatch_has_budget(
            started,
            started + Duration::seconds(24)
        ));
        assert!(prompt_dispatch_has_budget(
            started,
            started + Duration::seconds(25)
        ));
        assert!(!prompt_dispatch_has_budget(
            started,
            started + Duration::milliseconds(25_001)
        ));
        assert!(!prompt_dispatch_has_budget(
            started,
            started + Duration::seconds(44)
        ));
        assert!(!prompt_dispatch_has_budget(
            started,
            started + Duration::seconds(46)
        ));
    }
}
