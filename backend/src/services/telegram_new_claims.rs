use bson::doc;
use chrono::{DateTime, Duration, Utc};
use mongodb::{IndexModel, options::IndexOptions};
use serde_json::json;
use zeroize::Zeroizing;

use super::telegram_new_service::{
    CREATION_RECOVERY_MINUTES, TelegramNewService, hash, is_duplicate_key_error, setup_destination,
    with_operation,
};
use crate::errors::{AppError, AppResult};
use crate::models::telegram_bot_claim::{
    COLLECTION_NAME as CLAIMS, TelegramBotClaim, TelegramClaimStatus as ClaimStatus,
};
use crate::models::telegram_bot_request::{
    COLLECTION_NAME as REQUESTS, MANAGED_BOTS, ManagedBotEvents, TelegramBotRequest,
    TelegramRequestStatus as Status,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};

const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

fn unavailable() -> AppError {
    AppError::NotFound(
        "Telegram claim not found or expired. If you already connected this bot, check Channel Bots. Otherwise request a new code in the manager chat.".into(),
    )
}

fn existing_actor_request() -> AppError {
    AppError::Conflict("Finish or cancel your existing Telegram creation request first".into())
}

fn existing_creator_request() -> AppError {
    AppError::Conflict("The bot creator has another Telegram setup in progress. Finish or cancel it in the account where it was started.".into())
}

fn code_hash(code: &str) -> AppResult<String> {
    if code.len() > 64 {
        return Err(unavailable());
    }
    let normalized = Zeroizing::new(
        code.chars()
            .filter(|ch| *ch != '-' && !ch.is_ascii_whitespace())
            .map(|ch| ch.to_ascii_uppercase())
            .collect::<String>(),
    );
    if normalized.len() != 20 || !normalized.bytes().all(|ch| CODE_ALPHABET.contains(&ch)) {
        return Err(unavailable());
    }
    Ok(hash(&normalized))
}

pub(super) const CLAIM_CODE_REDACTED: &str = "[claim code removed]";

fn is_code_byte(byte: u8) -> bool {
    CODE_ALPHABET.contains(&byte.to_ascii_uppercase())
}

/// A single token the normalizer above would accept as a whole code.
fn token_is_code(token: &str) -> bool {
    code_hash(token).is_ok()
}

/// One uppercase group of a code typed with spaces.
fn token_is_uppercase_group(token: &str) -> bool {
    token.len() == 5
        && token
            .bytes()
            .all(|byte| !byte.is_ascii_lowercase() && is_code_byte(byte))
}

/// Replaces claim-shaped codes in relayed chat text with [`CLAIM_CODE_REDACTED`].
/// Matches only the exact shapes the normalizer accepts: twenty code characters
/// optionally hyphenated, or four uppercase five-character groups separated by
/// whitespace. Returns `None` when nothing was replaced.
pub(super) fn scrub_claim_codes(text: &str) -> Option<String> {
    // A link may percent-encode the code, so redact the entire bearer query
    // value before scanning visible code shapes.
    static CLAIM_QUERY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let query = CLAIM_QUERY.get_or_init(|| {
        regex::Regex::new(r#"([?&])([^=&#\s<>"']+)=([^&#\s<>"']+)"#)
            .expect("claim query expression is valid")
    });
    let query_redacted = query.replace_all(text, |capture: &regex::Captures<'_>| {
        if urlencoding::decode(&capture[2]).is_ok_and(|name| name.eq_ignore_ascii_case("claim")) {
            format!("{}{}={}", &capture[1], &capture[2], CLAIM_CODE_REDACTED)
        } else {
            capture[0].to_string()
        }
    });
    let query_changed = query_redacted != text;
    let text = query_redacted.as_ref();
    let mut cores = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        // URL query separators and Markdown punctuation delimit a pasted code
        // just like whitespace; underscores remain part of opaque identifiers.
        if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
            start.get_or_insert(index);
        } else if let Some(begin) = start.take() {
            cores.push((begin, index));
        }
    }
    if let Some(begin) = start {
        cores.push((begin, text.len()));
    }
    let core = |index: usize| &text[cores[index].0..cores[index].1];
    let mut spans = Vec::new();
    let mut index = 0;
    while index < cores.len() {
        if token_is_code(core(index)) {
            spans.push(cores[index]);
            index += 1;
        } else if index + 4 <= cores.len()
            && (index..index + 4).all(|group| token_is_uppercase_group(core(group)))
            && (index..index + 3).all(|group| {
                text[cores[group].1..cores[group + 1].0]
                    .chars()
                    .all(char::is_whitespace)
            })
        {
            spans.push((cores[index].0, cores[index + 3].1));
            index += 4;
        } else {
            index += 1;
        }
    }
    if spans.is_empty() {
        return query_changed.then(|| text.to_string());
    }
    let mut scrubbed = String::with_capacity(text.len());
    let mut cursor = 0;
    for (begin, end) in spans {
        scrubbed.push_str(&text[cursor..begin]);
        scrubbed.push_str(CLAIM_CODE_REDACTED);
        cursor = end;
    }
    scrubbed.push_str(&text[cursor..]);
    Some(scrubbed)
}

pub(super) async fn ensure_indexes(db: &mongodb::Database) -> Result<(), mongodb::error::Error> {
    for (keys, options) in [
        (
            doc! {"code_hash": 1},
            IndexOptions::builder().unique(true).build(),
        ),
        (
            doc! {"manager_bot_id": 1, "telegram_bot_id": 1},
            IndexOptions::builder().unique(true).build(),
        ),
        (
            doc! {"purge_after": 1},
            IndexOptions::builder()
                .expire_after(std::time::Duration::ZERO)
                .build(),
        ),
    ] {
        db.collection::<TelegramBotClaim>(CLAIMS)
            .create_index(IndexModel::builder().keys(keys).options(options).build())
            .await?;
    }
    Ok(())
}

impl TelegramNewService<'_> {
    pub(super) async fn send_claim(
        &self,
        manager: i64,
        bot: i64,
        creator: i64,
        token: &str,
        renew: bool,
    ) -> AppResult<()> {
        with_operation(
            self.db,
            &format!("telegram-child:{bot}"),
            self.send_claim_inner(manager, bot, creator, token, renew),
        )
        .await
    }

    async fn send_claim_inner(
        &self,
        manager: i64,
        bot: i64,
        creator: i64,
        token: &str,
        renew: bool,
    ) -> AppResult<()> {
        self.expire().await?;
        let (_, _, values) = self.manager().await?;
        let Some(observation) = values.get("observation_id") else {
            return Err(unavailable());
        };
        let event = self.db.collection::<ManagedBotEvents>(MANAGED_BOTS).find_one(doc! {
            "manager_bot_id": manager, "telegram_bot_id": bot, "created_by": creator,
            "telegram_user_id": creator, "observation_id": observation, "retired": {"$ne": true},
            "revision": {"$in": [0_i64, 1_i64]},
            "created_at": {"$gt": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(CREATION_RECOVERY_MINUTES))},
        }).await?;
        let Some(event) = event else {
            return Ok(());
        };
        // A pending website handoff keeps priority over a bearer claim.
        if self.db.collection::<TelegramBotRequest>(REQUESTS).find_one(doc! {
            "manager_bot_id": manager, "active": true,
            "$or": [
                {"telegram_user_id": creator, "status": "waiting_bot"},
                {"telegram_bot_id": bot, "status": {"$in": ["waiting_consent", "ready", "provisioning"]}},
            ],
        }).await?.is_some() { return Ok(()); }
        let claims = self.db.collection::<TelegramBotClaim>(CLAIMS);
        let existing = claims
            .find_one(doc! {"manager_bot_id": manager, "telegram_bot_id": bot})
            .await?;
        if let Some(existing) = &existing {
            if existing.status == ClaimStatus::Redeemed {
                let request = self
                    .db
                    .collection::<TelegramBotRequest>(REQUESTS)
                    .find_one(doc! {"_id": &existing.request_id})
                    .await?;
                if !renew
                    || request.as_ref().is_none_or(|request| {
                        !matches!(request.status, Status::Cancelled | Status::Expired)
                    })
                {
                    if renew {
                        self.send_connection_progress(token, creator).await?;
                    }
                    return Ok(());
                }
            } else if existing.delivered && !renew {
                return Ok(());
            }
        }
        let random = Zeroizing::new(rand::random::<[u8; 20]>());
        let code = Zeroizing::new(random.iter().enumerate().fold(
            String::new(),
            |mut code, (index, byte)| {
                if index > 0 && index % 5 == 0 {
                    code.push('-');
                }
                code.push(CODE_ALPHABET[usize::from(*byte & 31)] as char);
                code
            },
        ));
        let now = Utc::now();
        let created_at = event.created_at.ok_or_else(unavailable)?;
        let expires_at = (now + Duration::minutes(15))
            .min(created_at + Duration::minutes(CREATION_RECOVERY_MINUTES));
        let claim = TelegramBotClaim {
            id: existing
                .as_ref()
                .map(|claim| claim.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            code_hash: code_hash(&code)?,
            manager_bot_id: manager,
            telegram_bot_id: bot,
            telegram_user_id: creator,
            bot_username: event.bot_username.ok_or_else(unavailable)?,
            observation_id: observation.into(),
            manager_revision: 1,
            status: ClaimStatus::Pending,
            request_id: None,
            actor_user_id: None,
            owner_user_id: None,
            label: None,
            delivered: false,
            created_at: now,
            expires_at,
            purge_after: Some(expires_at + Duration::days(1)),
        };
        claims
            .replace_one(doc! {"_id": &claim.id}, &claim)
            .upsert(true)
            .await?;
        let bare_url = format!(
            "{}/channel-bots?connect=telegram-new&claim_entry=true",
            self.config.frontend_url.trim_end_matches('/')
        );
        let url = format!("{bare_url}&claim={}", code.as_str());
        self.api.call(token, "sendMessage", json!({
            "chat_id": creator,
            "text": format!("@{} is ready to connect. Open NyxID, choose an account, and tap Connect.\n\nClaim code: {}\nValid until {} UTC. Only enter or share this code with NyxID; anyone with it can claim this bot.\n\nIf the button opens a different browser, open {} in your signed-in browser and enter the code.\n\nTo replace an expired code, send /recover @{}.", claim.bot_username, code.as_str(), expires_at.format("%H:%M"), bare_url, claim.bot_username),
            "reply_markup": {"inline_keyboard": [[{"text": "Connect in NyxID", "url": url}]]},
            "link_preview_options": {"is_disabled": true},
            // The code is bearer proof; keep Telegram from forwarding or saving the message.
            "protect_content": true,
        })).await?;
        claims
            .update_one(
                doc! {"_id": &claim.id, "code_hash": &claim.code_hash},
                doc! {"$set": {"delivered": true}},
            )
            .await?;
        Ok(())
    }

    async fn load_claim(&self, code: &str) -> AppResult<TelegramBotClaim> {
        self.db
            .collection::<TelegramBotClaim>(CLAIMS)
            .find_one(
                doc! {"code_hash": code_hash(code)?, "expires_at": {"$gt": bson::DateTime::now()}},
            )
            .await?
            .ok_or_else(unavailable)
    }

    async fn verify_claim_provenance(&self, claim: &TelegramBotClaim) -> AppResult<DateTime<Utc>> {
        let (manager, _, values) = self.manager().await?;
        if manager != claim.manager_bot_id
            || values.get("observation_id") != Some(claim.observation_id.as_str())
        {
            return Err(unavailable());
        }
        let event = self.db.collection::<ManagedBotEvents>(MANAGED_BOTS).find_one(doc! {
            "manager_bot_id": claim.manager_bot_id, "telegram_bot_id": claim.telegram_bot_id,
            "created_by": claim.telegram_user_id, "telegram_user_id": claim.telegram_user_id,
            "observation_id": &claim.observation_id, "retired": {"$ne": true},
            "created_at": {"$gt": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(CREATION_RECOVERY_MINUTES))},
        }).await?.ok_or_else(unavailable)?;
        if event.revision == 0 {
            return Err(AppError::Conflict(
                "Telegram is still confirming creation. Try again in a moment.".into(),
            ));
        }
        if event.revision != claim.manager_revision {
            return Err(unavailable());
        }
        event.created_at.ok_or_else(unavailable)
    }

    pub async fn preview_claim(&self, actor: &str, code: &str) -> AppResult<TelegramBotClaim> {
        self.check_owner(actor, actor).await?;
        let claim = self.load_claim(code).await?;
        if claim.status != ClaimStatus::Pending {
            return Err(unavailable());
        }
        self.verify_claim_provenance(&claim).await?;
        Ok(claim)
    }

    pub async fn redeem_claim(
        &self,
        actor: &str,
        owner: &str,
        code: &str,
        label: &str,
    ) -> AppResult<TelegramBotRequest> {
        let claim = self.load_claim(code).await?;
        with_operation(
            self.db,
            &format!("telegram-child:{}", claim.telegram_bot_id),
            self.redeem_claim_inner(actor, owner, code, label),
        )
        .await
    }

    async fn redeem_claim_inner(
        &self,
        actor: &str,
        owner: &str,
        code: &str,
        label: &str,
    ) -> AppResult<TelegramBotRequest> {
        // Reload after acquiring the lease so a concurrent retry observes the saved pin.
        let claim = self.load_claim(code).await?;
        if claim.status == ClaimStatus::Redeemed && claim.actor_user_id.as_deref() != Some(actor) {
            return Err(unavailable());
        }
        self.check_owner(actor, owner).await?;
        let label = label.trim();
        if label.is_empty() || label.len() > 128 {
            return Err(AppError::ValidationError(
                "Label must be between 1 and 128 characters".into(),
            ));
        }
        if claim.status == ClaimStatus::Redeemed {
            if claim.actor_user_id.as_deref() != Some(actor) {
                return Err(unavailable());
            }
            if claim.owner_user_id.as_deref() != Some(owner)
                || claim.label.as_deref() != Some(label)
            {
                return Err(AppError::Conflict(
                    "The destination and label are already saved. Resume the existing setup."
                        .into(),
                ));
            }
            let request = self
                .get(actor, claim.request_id.as_deref().ok_or_else(unavailable)?)
                .await?;
            if !request.active {
                return Err(AppError::Conflict(
                    "This claim has already been used. Open the saved bot in Channel Bots.".into(),
                ));
            }
            return Ok(request);
        }
        let created_at = self.verify_claim_provenance(&claim).await?;
        self.expire().await?;
        let destination = self
            .db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": owner})
            .await?
            .ok_or_else(unavailable)?;
        let initiator = self
            .db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": actor})
            .await?
            .ok_or_else(unavailable)?;
        let request = TelegramBotRequest {
            id: uuid::Uuid::new_v4().to_string(),
            actor_user_id: actor.into(),
            owner_user_id: owner.into(),
            manager_bot_id: claim.manager_bot_id,
            observation_id: claim.observation_id.clone(),
            label: label.into(),
            destination: setup_destination(&destination, &initiator, &self.config.frontend_url),
            status: Status::Ready,
            active: true,
            revision: 0,
            challenge_hash: hash(&hex::encode(rand::random::<[u8; 24]>())),
            telegram_user_id: Some(claim.telegram_user_id),
            telegram_bot_id: Some(claim.telegram_bot_id),
            bot_username: Some(claim.bot_username.clone()),
            consent_hash: None,
            manager_revision: Some(1),
            auto_connect: true,
            start_update_id: None,
            connection_attempts: 0,
            next_connection_attempt_at: None,
            connection_error: None,
            created_at: Utc::now(),
            expires_at: (Utc::now() + Duration::minutes(15))
                .min(created_at + Duration::minutes(CREATION_RECOVERY_MINUTES)),
            purge_after: None,
        };
        use super::api_key_mutation_service::{map_transaction_error, transaction_result};
        let mut session = self.db.client().start_session().await?;
        let db = self.db.clone();
        let saved = request.clone();
        let result = session.start_transaction().and_run2(async move |session| {
            let operation: AppResult<()> = async {
                let requests = db.collection::<TelegramBotRequest>(REQUESTS);
                if requests.find_one(doc! {"active": true, "actor_user_id": &saved.actor_user_id}).session(&mut *session).await?.is_some() {
                    return Err(existing_actor_request());
                }
                if requests.find_one(doc! {"active": true, "manager_bot_id": saved.manager_bot_id, "telegram_user_id": saved.telegram_user_id}).session(&mut *session).await?.is_some() {
                    return Err(existing_creator_request());
                }
                let result = db.collection::<TelegramBotClaim>(CLAIMS).update_one(
                    doc! {"_id": &claim.id, "code_hash": &claim.code_hash, "status": "pending", "expires_at": {"$gt": bson::DateTime::now()}},
                    doc! {"$set": {"status": "redeemed", "actor_user_id": &saved.actor_user_id, "owner_user_id": &saved.owner_user_id, "label": &saved.label, "request_id": &saved.id}, "$unset": {"purge_after": ""}},
                ).session(&mut *session).await?;
                if result.modified_count != 1 { return Err(unavailable()); }
                let event = db.collection::<ManagedBotEvents>(MANAGED_BOTS).update_one(
                    doc! {"manager_bot_id": saved.manager_bot_id, "telegram_bot_id": saved.telegram_bot_id, "created_by": saved.telegram_user_id, "telegram_user_id": saved.telegram_user_id, "observation_id": &saved.observation_id, "revision": 1_i64, "retired": {"$ne": true}},
                    doc! {"$inc": {"claim_reservation_revision": 1_i64}},
                ).session(&mut *session).await?;
                if event.matched_count != 1 { return Err(unavailable()); }
                requests.insert_one(&saved).session(&mut *session).await?;
                Ok(())
            }.await;
            transaction_result(operation)
        }).await;
        if let Err(error) = result {
            if is_duplicate_key_error(&error) {
                if self
                    .db
                    .collection::<TelegramBotRequest>(REQUESTS)
                    .find_one(doc! {"active": true, "actor_user_id": actor})
                    .await?
                    .is_some()
                {
                    return Err(existing_actor_request());
                }
                return Err(existing_creator_request());
            }
            return Err(map_transaction_error(error));
        }
        super::audit_service::log_async(
            self.db.clone(),
            Some(actor.into()),
            "telegram_bot_claim_redeemed".into(),
            Some(
                json!({"request_id": request.id, "owner_user_id": owner, "platform": "telegram-new"}),
            ),
            None,
            None,
            None,
            None,
        );
        Ok(request)
    }
}
