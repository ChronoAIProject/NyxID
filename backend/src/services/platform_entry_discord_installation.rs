//! Persist command ownership before native activation. An ambiguous provider
//! create requires explicit review of the exact observed command; it is never
//! retried as an upsert or silently adopted on ordinary enablement retry.
use super::platform_entry_discord_controls as controls;
use crate::{
    errors::AppResult,
    models::platform_entry_discord_installation::{
        COLLECTION_NAME, DiscordControlInstallation as Installation, HISTORY,
        InstallationState as State,
    },
};
use bson::{Document, doc};
use chrono::Utc;
use mongodb::{Database, IndexModel, options::IndexOptions};
use serde_json::Value;

#[cfg(test)]
tokio::task_local! {
    static LOSE_PROVIDER_RESULT: bool;
    static DISPATCH_DURATION: chrono::Duration;
    static AFTER_DISPATCH: std::sync::Arc<tokio::sync::Barrier>;
    static BEFORE_DISPATCH: std::sync::Arc<tokio::sync::Barrier>;
}

struct InstallationRequest<'a> {
    pub(crate) channel_bot_id: &'a str,
    pub(crate) requested_by: &'a str,
    pub(crate) fingerprint: &'a str,
    pub(crate) bot_id: &'a str,
    pub(crate) public_key: &'a str,
    pub(crate) token: &'a str,
    pub(crate) previously_owned_command: Option<&'a str>,
}

/// Only this captured authority can enter the production installer. Provider
/// discovery may await network I/O; dispatch joins these live fences and its
/// attempt claim in one transaction after those awaits.
pub(crate) struct InstallationAuthority {
    actor: String,
    funding: String,
    bot: crate::models::channel_bot::ChannelBot,
    credentials: super::platform_entry_credentials::EntryCredentialSnapshot,
    bot_filter: Document,
    profile_filter: Option<Document>,
}

impl InstallationAuthority {
    pub(crate) async fn capture(db: &Database, actor: &str, bot_id: &str) -> AppResult<Self> {
        let mut session = db.client().start_session().await?;
        session.start_transaction().await?;
        super::platform_bot_config_service::fence_admin(db, actor, &mut session).await?;
        session.commit_transaction().await?;
        let raw = db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id":bot_id,"platform":"discord","is_active":true,"status":"active"})
            .await?
            .ok_or_else(controls::unavailable)?;
        let bot: crate::models::channel_bot::ChannelBot =
            bson::from_document(raw.clone()).map_err(|_| controls::unavailable())?;
        let adapter = super::channel_adapters::discord::DiscordAdapter::default();
        let credentials =
            super::platform_entry_credentials::EntryCredentialSnapshot::load(db, &adapter, &bot)
                .await?;
        let mut bot_filter = doc! {"_id":&bot.id,"platform":"discord","is_active":true,"status":"active","user_id":&bot.user_id};
        for name in [
            "platform_profile_id",
            "platform_entry_version",
            "platform_entry_pending",
            "route_revision",
        ] {
            bot_filter.insert(
                name,
                raw.get(name)
                    .cloned()
                    .unwrap_or_else(|| bson::Bson::Document(doc! {"$exists":false})),
            );
        }
        let profile_filter = match raw.get("platform_profile_id") {
            None => None,
            Some(bson::Bson::String(id)) => {
                let profile = db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
                    .find_one(doc! {"_id":id,"channel_bot_id":&bot.id,"identity_version":2,"deleted_at":bson::Bson::Null}).await?
                    .ok_or_else(controls::unavailable)?;
                let mut filter = doc! {"_id":id,"channel_bot_id":&bot.id,"identity_version":2,"deleted_at":bson::Bson::Null};
                for name in [
                    "generation",
                    "native_entry",
                    "enabled",
                    "configured_by",
                    "funding_owner_user_id",
                ] {
                    filter.insert(
                        name,
                        profile
                            .get(name)
                            .cloned()
                            .ok_or_else(controls::unavailable)?,
                    );
                }
                Some(filter)
            }
            _ => return Err(controls::unavailable()),
        };
        let funding = if let Some(profile) = &profile_filter {
            profile
                .get_str("funding_owner_user_id")
                .map_err(|_| controls::unavailable())?
                .to_owned()
        } else {
            let pending = raw
                .get_document("platform_entry_pending")
                .map_err(|_| controls::unavailable())?;
            if pending.get_str("actor").ok() != Some(actor) {
                return Err(controls::unavailable());
            }
            pending
                .get_str("funding")
                .map_err(|_| controls::unavailable())?
                .to_owned()
        };
        Ok(Self {
            actor: actor.into(),
            funding,
            bot,
            credentials,
            bot_filter,
            profile_filter,
        })
    }

    pub(super) async fn fence_in_session(
        &self,
        db: &Database,
        session: &mut mongodb::ClientSession,
    ) -> AppResult<()> {
        super::platform_bot_config_service::fence_admin(db, &self.actor, session).await?;
        super::platform_bot_config_service::fence_funding_admin(
            db,
            &self.actor,
            &self.funding,
            session,
        )
        .await?;
        if !self.credentials.fence_in_session(db, session).await? {
            return Err(controls::unavailable());
        }
        if db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                self.bot_filter.clone(),
                doc! {"$inc":{"platform_entry_admin_revision":1_i64}},
            )
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(controls::unavailable());
        }
        if let Some(filter) = &self.profile_filter
            && db
                .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
                .update_one(
                    filter.clone(),
                    doc! {"$inc":{"native_admin_effect_revision":1_i64}},
                )
                .session(&mut *session)
                .await?
                .matched_count
                != 1
        {
            return Err(controls::unavailable());
        }
        Ok(())
    }
}

enum DispatchFence<'a> {
    Captured(&'a InstallationAuthority),
    #[cfg(test)]
    OwnershipFixture,
}

impl DispatchFence<'_> {
    fn matches(&self, request: &InstallationRequest<'_>) -> AppResult<()> {
        match self {
            Self::Captured(authority) => {
                if authority.actor != request.requested_by
                    || authority.bot.id != request.channel_bot_id
                    || authority.bot.platform_bot_id != request.bot_id
                    || authority.bot.public_key.as_deref() != Some(request.public_key)
                    || authority.credentials.fingerprint()? != request.fingerprint
                {
                    return Err(controls::unavailable());
                }
            }
            #[cfg(test)]
            Self::OwnershipFixture => {}
        }
        Ok(())
    }

    async fn fence_in_session(
        &self,
        db: &Database,
        session: &mut mongodb::ClientSession,
    ) -> AppResult<()> {
        match self {
            Self::Captured(authority) => authority.fence_in_session(db, session).await,
            #[cfg(test)]
            Self::OwnershipFixture => Ok(()),
        }
    }
}

/// This metadata is shown only in the authenticated administrative recovery
/// review. Acceptance binds the complete snapshot and reobserves the provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryReview {
    pub(crate) attempt_id: String,
    pub(crate) application_id: String,
    pub(crate) command_id: Option<String>,
    pub(crate) observed_schema_digest: Option<String>,
    pub(crate) schema_digest: String,
    pub(crate) requested_by: String,
    pub(crate) channel_bot_id: String,
    pub(crate) credential_fingerprint: String,
}

/// Activation must join this exact observed attempt with its live admin,
/// target/configuration and credentials in the final transaction. A concurrent
/// recovery/reconfiguration cannot turn a superseded result into activation.
pub(crate) struct InstalledControl {
    command: controls::InstalledCommand,
    installation_filter: Document,
}
impl std::ops::Deref for InstalledControl {
    type Target = controls::InstalledCommand;
    fn deref(&self) -> &Self::Target {
        &self.command
    }
}
impl InstalledControl {
    pub(crate) async fn fence_in_session(
        &self,
        db: &Database,
        session: &mut mongodb::ClientSession,
    ) -> AppResult<()> {
        if db
            .collection::<Document>(COLLECTION_NAME)
            .update_one(
                self.installation_filter.clone(),
                doc! {"$inc":{"activation_revision":1_i64}},
            )
            .session(session)
            .await?
            .matched_count
            != 1
        {
            return Err(controls::unavailable());
        }
        Ok(())
    }
}
fn installed_control(
    row: &Installation,
    command: controls::InstalledCommand,
) -> AppResult<InstalledControl> {
    if row.state != State::Observed
        || row.observed_command_id.as_deref() != Some(&command.command_id)
        || row.application_id != command.application_id
        || row.schema_digest != command.command_digest
    {
        return Err(controls::unavailable());
    }
    Ok(InstalledControl {
        command,
        installation_filter: snapshot(row),
    })
}

pub(crate) enum InstallationOutcome {
    Installed(InstalledControl),
    RecoveryRequired(RecoveryReview),
}

async fn observe(
    http: &reqwest::Client,
    request: &InstallationRequest<'_>,
    base: &str,
) -> AppResult<(String, Vec<Value>)> {
    let application = controls::response(
        http.get(format!("{base}/oauth2/applications/@me"))
            .header("authorization", format!("Bot {}", request.token)),
    )
    .await?;
    let app = application["id"]
        .as_str()
        .ok_or_else(controls::unavailable)?;
    let public_key = application["verify_key"]
        .as_str()
        .ok_or_else(controls::unavailable)?;
    let provider_key = hex::decode(public_key).map_err(|_| controls::unavailable())?;
    let expected_key = hex::decode(request.public_key).map_err(|_| controls::unavailable())?;
    if app.is_empty()
        || !app.bytes().all(|byte| byte.is_ascii_digit())
        || application["bot"]["id"].as_str() != Some(request.bot_id)
        || provider_key.len() != 32
        || expected_key.len() != 32
        || !crate::crypto::token::constant_time_eq(&provider_key, &expected_key)
    {
        return Err(controls::unavailable());
    }
    let commands = controls::response(
        http.get(format!("{base}/applications/{app}/commands"))
            .header("authorization", format!("Bot {}", request.token)),
    )
    .await?;
    let candidates = commands
        .as_array()
        .ok_or_else(controls::unavailable)?
        .iter()
        .filter(|command| command["type"] == 1 && command["name"] == "nyx")
        .cloned()
        .collect::<Vec<_>>();
    if candidates.len() > 1 {
        return Err(controls::unavailable());
    }
    Ok((app.to_owned(), candidates))
}

fn snapshot(row: &Installation) -> Document {
    doc! {"_id":&row.id,"application_id":&row.application_id,"channel_bot_id":&row.channel_bot_id,"attempt_id":&row.attempt_id,"state":bson::to_bson(&row.state).expect("state"),
    "requested_by":&row.requested_by,"credential_fingerprint":&row.credential_fingerprint,
    "platform_bot_id":&row.platform_bot_id,
    "schema_digest":&row.schema_digest,"owned_command_id":row.owned_command_id.clone(),
    "observed_command_id":row.observed_command_id.clone(),"dispatch_deadline":row.dispatch_deadline.map(bson::DateTime::from_chrono)}
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Installation>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"application_id":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("platform_discord_nyx_application".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub(super) async fn install_captured_at(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    http: &reqwest::Client,
    authority: &InstallationAuthority,
    previously_owned_command: Option<&str>,
    recovery: Option<&RecoveryReview>,
    base: &str,
) -> AppResult<InstallationOutcome> {
    let token = authority.credentials.send_token(keys).await?;
    let fingerprint = authority.credentials.fingerprint()?;
    let request = InstallationRequest {
        channel_bot_id: &authority.bot.id,
        requested_by: &authority.actor,
        fingerprint: &fingerprint,
        bot_id: &authority.bot.platform_bot_id,
        public_key: authority
            .bot
            .public_key
            .as_deref()
            .ok_or_else(controls::unavailable)?,
        token: &token,
        previously_owned_command,
    };
    install_with_fence_at(
        db,
        http,
        request,
        recovery,
        base,
        DispatchFence::Captured(authority),
    )
    .await
}

// Isolated ownership/recovery fixtures exercise the state machine separately.
// Production has no caller-supplied token or authority-free mutation entrypoint.
#[cfg(test)]
async fn install_at(
    db: &Database,
    http: &reqwest::Client,
    request: InstallationRequest<'_>,
    recovery: Option<&RecoveryReview>,
    base: &str,
) -> AppResult<InstallationOutcome> {
    install_with_fence_at(
        db,
        http,
        request,
        recovery,
        base,
        DispatchFence::OwnershipFixture,
    )
    .await
}

async fn install_with_fence_at(
    db: &Database,
    http: &reqwest::Client,
    request: InstallationRequest<'_>,
    recovery: Option<&RecoveryReview>,
    base: &str,
    authority: DispatchFence<'_>,
) -> AppResult<InstallationOutcome> {
    authority.matches(&request)?;
    if uuid::Uuid::parse_str(request.channel_bot_id).is_err()
        || uuid::Uuid::parse_str(request.requested_by).is_err()
        || request.fingerprint.len() != 64
    {
        return Err(controls::unavailable());
    }
    ensure_indexes(db).await?;
    let (application_id, candidates) = observe(http, &request, base).await?;
    let rows = db.collection::<Installation>(COLLECTION_NAME);
    // The authenticated application, rather than a local channel, owns /nyx.
    // Retiring/replacing a local channel does not release an in-flight claim.
    let mut row = match rows
        .find_one(doc! {"application_id":&application_id})
        .await?
    {
        Some(row) => row,
        None => {
            match (candidates.first(), request.previously_owned_command) {
                (None, None) => {}
                (Some(command), Some(owned)) if command["id"].as_str() == Some(owned) => {}
                _ => return Err(controls::unavailable()),
            }
            if recovery.is_some() {
                return Err(controls::unavailable());
            }
            let now = Utc::now();
            let row = Installation {
                id: uuid::Uuid::new_v4().to_string(),
                channel_bot_id: request.channel_bot_id.into(),
                attempt_id: uuid::Uuid::new_v4().to_string(),
                requested_by: request.requested_by.into(),
                credential_fingerprint: request.fingerprint.into(),
                platform_bot_id: request.bot_id.into(),
                application_id: application_id.clone(),
                schema_digest: controls::schema_digest(),
                owned_command_id: request.previously_owned_command.map(str::to_owned),
                observed_command_id: None,
                state: State::Prepared,
                dispatch_deadline: None,
                created_at: now,
                updated_at: now,
            };
            rows.insert_one(&row).await?;
            row
        }
    };
    if row.application_id != application_id || row.platform_bot_id != request.bot_id {
        return Err(controls::unavailable());
    }
    let same_request = row.requested_by == request.requested_by
        && row.credential_fingerprint == request.fingerprint
        && row.channel_bot_id == request.channel_bot_id
        && row.schema_digest == controls::schema_digest();
    let known = row
        .observed_command_id
        .as_deref()
        .or(row.owned_command_id.as_deref());
    let observed = candidates
        .first()
        .map(|command| {
            let id = command["id"].as_str().ok_or_else(controls::unavailable)?;
            if id.is_empty()
                || !id.bytes().all(|byte| byte.is_ascii_digit())
                || command["application_id"].as_str() != Some(application_id.as_str())
                || known.is_some_and(|owned| owned != id)
            {
                return Err(controls::unavailable());
            }
            Ok((
                id.to_owned(),
                super::mcp_service::canonical_sha256(controls::projection(command)?),
            ))
        })
        .transpose()?;
    if request
        .previously_owned_command
        .is_some_and(|owned| Some(owned) != known)
    {
        return Err(controls::unavailable());
    }
    let exact = observed
        .as_ref()
        .is_some_and(|(_, digest)| digest == &controls::schema_digest());
    if same_request
        && row.state == State::Observed
        && exact
        && observed.as_ref().map(|(id, _)| id.as_str()) == row.observed_command_id.as_deref()
    {
        let mut session = db.client().start_session().await?;
        session.start_transaction().await?;
        authority.fence_in_session(db, &mut session).await?;
        session.commit_transaction().await?;
        return Ok(InstallationOutcome::Installed(installed_control(
            &row,
            controls::InstalledCommand {
                application_id,
                command_id: row
                    .observed_command_id
                    .clone()
                    .ok_or_else(controls::unavailable)?,
                command_digest: row.schema_digest.clone(),
            },
        )?));
    }
    let prepared = same_request
        && row.state == State::Prepared
        && request.previously_owned_command == row.owned_command_id.as_deref();
    if !prepared {
        let review = RecoveryReview {
            attempt_id: row.attempt_id.clone(),
            application_id: application_id.clone(),
            command_id: observed.as_ref().map(|(id, _)| id.clone()),
            observed_schema_digest: observed.as_ref().map(|(_, digest)| digest.clone()),
            schema_digest: controls::schema_digest(),
            requested_by: request.requested_by.into(),
            channel_bot_id: request.channel_bot_id.into(),
            credential_fingerprint: request.fingerprint.into(),
        };
        let Some(accepted) = recovery else {
            return Ok(InstallationOutcome::RecoveryRequired(review));
        };
        if accepted != &review {
            return Err(controls::unavailable());
        }
        // A visible command may still be the stale pre-PATCH version. Neither
        // observation nor an administrative click supersedes an authorized
        // dispatch. Recovery reobserves after its deadline and uncertainty delay.
        if row.state == State::Dispatched {
            let deadline = row.dispatch_deadline.ok_or_else(controls::unavailable)?;
            if deadline + chrono::Duration::seconds(120) > Utc::now() {
                return Err(controls::unavailable());
            }
        }
        let old = row.clone();
        let now = Utc::now();
        let replacement = Installation {
            id: row.id.clone(),
            channel_bot_id: request.channel_bot_id.into(),
            attempt_id: uuid::Uuid::new_v4().to_string(),
            requested_by: request.requested_by.into(),
            credential_fingerprint: request.fingerprint.into(),
            platform_bot_id: request.bot_id.into(),
            application_id: application_id.clone(),
            schema_digest: controls::schema_digest(),
            owned_command_id: observed.as_ref().map(|(id, _)| id.clone()),
            observed_command_id: if exact {
                observed.as_ref().map(|(id, _)| id.clone())
            } else {
                None
            },
            state: if exact {
                State::Observed
            } else {
                State::Prepared
            },
            dispatch_deadline: None,
            created_at: now,
            updated_at: now,
        };
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run(
                (db, old, replacement.clone(), &authority),
                |session, (db, old, replacement, authority)| {
                    Box::pin(async move {
                        super::api_key_mutation_service::transaction_result(
                            async {
                                authority.fence_in_session(db, session).await?;
                                // Transfer is explicit and only possible after the old local
                                // channel is retired. Its ownership history remains durable.
                                if old.channel_bot_id != replacement.channel_bot_id {
                                    let retired = db
                                        .collection::<Document>(
                                            crate::models::channel_bot::COLLECTION_NAME,
                                        )
                                        .update_one(
                                            doc! {"_id":&old.channel_bot_id,"is_active":false},
                                            doc! {"$inc":{"platform_entry_effect_revision":1_i64}},
                                        )
                                        .session(&mut *session)
                                        .await?;
                                    if retired.matched_count != 1 {
                                        return Err(controls::unavailable());
                                    }
                                }
                                let mut history =
                                    bson::to_document(old).map_err(|_| controls::unavailable())?;
                                history.insert("_id", &old.attempt_id);
                                history.insert("application_id", &old.application_id);
                                db.collection::<Document>(HISTORY)
                                    .insert_one(history)
                                    .session(&mut *session)
                                    .await?;
                                if db
                                    .collection::<Installation>(COLLECTION_NAME)
                                    .replace_one(snapshot(old), replacement)
                                    .session(&mut *session)
                                    .await?
                                    .matched_count
                                    != 1
                                {
                                    return Err(controls::unavailable());
                                }
                                Ok(())
                            }
                            .await,
                        )
                    })
                },
            )
            .await
            .map_err(super::api_key_mutation_service::map_transaction_error)?;
        row = replacement;
        if row.state == State::Observed {
            return Ok(InstallationOutcome::Installed(installed_control(
                &row,
                controls::InstalledCommand {
                    application_id,
                    command_id: row
                        .observed_command_id
                        .clone()
                        .ok_or_else(controls::unavailable)?,
                    command_digest: row.schema_digest.clone(),
                },
            )?));
        }
    } else if recovery.is_some() {
        return Err(controls::unavailable());
    }
    // A fresh preflight can only mutate the exact previously owned ID or the
    // still absent namespace. No network reads happen after dispatch ownership.
    match (candidates.first(), row.owned_command_id.as_deref()) {
        (None, None) => {}
        (Some(command), Some(owned)) if command["id"].as_str() == Some(owned) => {}
        _ => return Err(controls::unavailable()),
    }
    #[cfg(test)]
    if let Ok(barrier) = BEFORE_DISPATCH.try_with(std::sync::Arc::clone) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            barrier.wait().await;
            barrier.wait().await;
        })
        .await
        .map_err(|_| controls::unavailable())?;
    }
    let duration = chrono::Duration::seconds(20);
    #[cfg(test)]
    let duration = DISPATCH_DURATION
        .try_with(|duration| *duration)
        .unwrap_or(duration);
    let deadline = Utc::now() + duration;
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db, &authority, &row, deadline), |session, (db, authority, row, deadline)| {
        Box::pin(async move {
            super::api_key_mutation_service::transaction_result(async {
                authority.fence_in_session(db, session).await?;
                let dispatched = db.collection::<Installation>(COLLECTION_NAME).update_one(snapshot(row),
                    doc! {"$set":{"state":"dispatched","dispatch_deadline":bson::DateTime::from_chrono(*deadline),"updated_at":bson::DateTime::now()}})
                    .session(&mut *session).await?;
                if dispatched.matched_count != 1 { return Err(controls::unavailable()); }
                Ok(())
            }.await)
        })
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
    row.state = State::Dispatched;
    row.dispatch_deadline = Some(deadline);
    #[cfg(test)]
    if let Ok(barrier) = AFTER_DISPATCH.try_with(std::sync::Arc::clone) {
        barrier.wait().await;
        barrier.wait().await;
    }
    if Utc::now() >= deadline {
        return Err(controls::unavailable());
    }
    let mut exact = snapshot(&row);
    exact.insert(
        "dispatch_deadline",
        doc! {"$eq":bson::DateTime::from_chrono(deadline),"$gt":bson::DateTime::now()},
    );
    if rows
        .update_one(exact, doc! {"$inc":{"dispatch_revision":1_i64}})
        .await?
        .matched_count
        != 1
    {
        return Err(controls::unavailable());
    }
    let installed = controls::mutate_verified_at(
        http,
        request.token,
        &application_id,
        &candidates,
        row.owned_command_id.as_deref(),
        base,
        deadline,
    )
    .await?;
    #[cfg(test)]
    if LOSE_PROVIDER_RESULT.try_with(|lose| *lose).unwrap_or(false) {
        return Err(controls::unavailable());
    }
    let observed=rows.update_one(snapshot(&row),doc! {"$set":{"state":"observed","observed_command_id":&installed.command_id,"updated_at":bson::DateTime::now()}}).await?;
    if observed.matched_count != 1 {
        return Err(controls::unavailable());
    }
    row.state = State::Observed;
    row.observed_command_id = Some(installed.command_id.clone());
    Ok(InstallationOutcome::Installed(installed_control(
        &row, installed,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    struct Fixture {
        db: Database,
        server: MockServer,
        bot_id: String,
        admin: String,
        key: String,
        fingerprint: String,
        failed_creates: Arc<AtomicUsize>,
    }
    impl Fixture {
        async fn new() -> Self {
            let db =
                crate::test_utils::connect_transaction_test_database("native_discord_installation")
                    .await;
            let server = MockServer::start().await;
            let created = Arc::new(AtomicBool::new(false));
            let failed_creates = Arc::new(AtomicUsize::new(0));
            Mock::given(method("GET"))
                .and(path("/oauth2/applications/@me"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"id":"100","bot":{"id":"200"},"verify_key":"a1".repeat(32)}),
                ))
                .mount(&server)
                .await;
            let observed = created.clone();
            Mock::given(method("GET"))
                .and(path("/applications/100/commands"))
                .respond_with(move |_: &wiremock::Request| {
                    let mut commands =
                        vec![serde_json::json!({"id":"999","type":1,"name":"unrelated"})];
                    if observed.load(Ordering::SeqCst) {
                        commands.push(registered());
                    }
                    ResponseTemplate::new(200).set_body_json(commands)
                })
                .mount(&server)
                .await;
            let failures = failed_creates.clone();
            Mock::given(method("POST"))
                .and(path("/applications/100/commands"))
                .respond_with(move |request: &wiremock::Request| {
                    assert_eq!(
                        serde_json::from_slice::<Value>(&request.body).unwrap(),
                        controls::schema()
                    );
                    if failures
                        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                            remaining.checked_sub(1)
                        })
                        .is_ok()
                    {
                        return ResponseTemplate::new(503);
                    }
                    assert!(
                        !created.swap(true, Ordering::SeqCst),
                        "provider upsert must not be retried"
                    );
                    ResponseTemplate::new(201).set_body_json(registered())
                })
                .mount(&server)
                .await;
            Self {
                db,
                server,
                bot_id: uuid::Uuid::new_v4().to_string(),
                admin: uuid::Uuid::new_v4().to_string(),
                key: "a1".repeat(32),
                fingerprint: "b2".repeat(32),
                failed_creates,
            }
        }
        fn request(&self) -> InstallationRequest<'_> {
            InstallationRequest {
                channel_bot_id: &self.bot_id,
                requested_by: &self.admin,
                fingerprint: &self.fingerprint,
                bot_id: "200",
                public_key: &self.key,
                token: "local-fixture-only",
                previously_owned_command: None,
            }
        }
        async fn run(&self, review: Option<&RecoveryReview>) -> AppResult<InstallationOutcome> {
            install_at(
                &self.db,
                &reqwest::Client::new(),
                self.request(),
                review,
                &self.server.uri(),
            )
            .await
        }
        async fn mutation_count(&self) -> usize {
            self.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.method != "GET")
                .count()
        }
        async fn expire_dispatch(&self) {
            self.db.collection::<Installation>(COLLECTION_NAME).update_one(
                doc! {"application_id":"100","state":"dispatched"},
                doc! {"$set":{
                    "dispatch_deadline":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(121)),
                    "updated_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(141))
                }}
            ).await.unwrap();
        }
    }
    fn registered() -> Value {
        let mut command = controls::schema();
        command["id"] = serde_json::json!("300");
        command["application_id"] = serde_json::json!("100");
        command
    }

    impl Fixture {
        async fn captured_authority(&self) -> (crate::AppState, InstallationAuthority) {
            let state = crate::test_utils::test_app_state(self.db.clone());
            let user =
                crate::test_utils::test_user(&self.admin, crate::models::user::UserType::Person);
            self.db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .insert_one(user)
                .await
                .unwrap();
            super::super::role_service::seed_system_roles(&self.db)
                .await
                .unwrap();
            let admin_role = super::super::role_service::get_platform_role_ids(&self.db)
                .await
                .unwrap()
                .admin;
            self.db
                .collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&self.admin},
                    doc! {"$set":{"role_ids":[admin_role]}},
                )
                .await
                .unwrap();
            let encrypted = state
                .encryption_keys
                .encrypt(b"local-fixture-only")
                .await
                .unwrap();
            let profile_id = uuid::Uuid::new_v4().to_string();
            let mut bot: crate::models::channel_bot::ChannelBot = bson::from_document(doc! {
                "_id":&self.bot_id,"user_id":&self.admin,"platform":"discord","label":"native installer",
                "platform_bot_id":"200","platform_bot_username":"local","app_id":"100","public_key":&self.key,
                "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
                "webhook_secret_hash":"local","webhook_registered":true,"status":"active","is_active":true,
                "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
            }).unwrap();
            bot.credential_source = "user".into();
            let snapshot = super::super::platform_entry_credentials::EntryCredentialSnapshot::load(
                &self.db,
                &super::super::channel_adapters::discord::DiscordAdapter::default(),
                &bot,
            )
            .await
            .unwrap();
            let mut raw = bson::to_document(&bot).unwrap();
            raw.insert("platform_profile_id", &profile_id);
            raw.insert("platform_entry_version", 2);
            self.db
                .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .insert_one(raw)
                .await
                .unwrap();
            let configuration = crate::models::platform_entry_identity::NativeEntryConfiguration {
                identity: crate::models::platform_entry_identity::NativeEntryIdentity {
                    version: 2,
                    platform: crate::models::platform_entry_identity::EntryPlatform::Discord,
                    channel_bot_id: self.bot_id.clone(),
                    platform_bot_id: "200".into(),
                },
                transport: crate::models::platform_entry_identity::EntryTransport::Direct,
                transport_generation: 1,
                credential_fingerprint: snapshot.fingerprint().unwrap(),
                control_surface:
                    crate::models::platform_entry_identity::NativeControlSurface::DiscordCommand {
                        application_id: "100".into(),
                        command_id: "300".into(),
                        command_digest: controls::schema_digest(),
                    },
            };
            let funding = super::super::org_service::create_org_user(
                &self.db,
                "Native installation funding",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
            super::super::org_service::create_membership(
                &self.db,
                &funding,
                &self.admin,
                crate::models::org_membership::OrgRole::Admin,
                crate::models::org_membership::MemberScopeSource::Inherit,
                None,
            )
            .await
            .unwrap();
            self.db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).insert_one(doc! {
                "_id":&profile_id,"channel_bot_id":&self.bot_id,"identity_version":2,"manager_bot_id":"200",
                "generation":1_i64,"native_entry":bson::to_bson(&configuration).unwrap(),"enabled":false,
                "configured_by":&self.admin,"funding_owner_user_id":&funding,"deleted_at":bson::Bson::Null
            }).await.unwrap();
            let authority = InstallationAuthority::capture(&self.db, &self.admin, &self.bot_id)
                .await
                .unwrap();
            (state, authority)
        }
    }

    #[tokio::test]
    async fn actual_admin_target_and_stored_credentials_are_fenced_after_provider_preflight() {
        for change in [
            "admin",
            "credential",
            "profile",
            "target",
            "funding_admin",
            "funding_org",
        ] {
            let fixture = Fixture::new().await;
            let (state, authority) = fixture.captured_authority().await;
            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let http = reqwest::Client::new();
            let base = fixture.server.uri();
            let run = BEFORE_DISPATCH.scope(
                barrier.clone(),
                install_captured_at(
                    &fixture.db,
                    &state.encryption_keys,
                    &http,
                    &authority,
                    None,
                    None,
                    &base,
                ),
            );
            let mutate = async {
                tokio::time::timeout(std::time::Duration::from_secs(10), barrier.wait())
                    .await
                    .unwrap();
                // These mutations happen after the real authenticated application
                // and commands GETs and before the actual dispatch transaction.
                match change {
                    "admin" => {
                        fixture
                            .db
                            .collection::<Document>(crate::models::user::COLLECTION_NAME)
                            .update_one(
                                doc! {"_id":&fixture.admin},
                                doc! {"$set":{"role_ids":[],"is_admin":false}},
                            )
                            .await
                            .unwrap();
                    }
                    "credential" => {
                        let next = state
                            .encryption_keys
                            .encrypt(b"replacement-token")
                            .await
                            .unwrap();
                        fixture.db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id":&fixture.bot_id},doc! {"$set":{"bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:next}}}).await.unwrap();
                    }
                    "profile" => {
                        fixture.db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(doc! {"channel_bot_id":&fixture.bot_id},doc! {"$inc":{"generation":1_i64,"native_entry.transport_generation":1_i64}}).await.unwrap();
                    }
                    "target" => {
                        fixture
                            .db
                            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                            .update_one(
                                doc! {"_id":&fixture.bot_id},
                                doc! {"$inc":{"route_revision":1_i64}},
                            )
                            .await
                            .unwrap();
                    }
                    "funding_admin" => {
                        fixture.db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
                            .update_one(doc! {"org_user_id":&authority.funding,"member_user_id":&fixture.admin},
                                doc! {"$set":{"revoked_at":bson::DateTime::now()}}).await.unwrap();
                    }
                    "funding_org" => {
                        fixture
                            .db
                            .collection::<Document>(crate::models::user::COLLECTION_NAME)
                            .update_one(
                                doc! {"_id":&authority.funding},
                                doc! {"$set":{"is_active":false}},
                            )
                            .await
                            .unwrap();
                    }
                    _ => unreachable!(),
                }
                tokio::time::timeout(std::time::Duration::from_secs(10), barrier.wait())
                    .await
                    .unwrap();
            };
            let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(25), async {
                tokio::join!(run, mutate)
            })
            .await
            .unwrap();
            assert!(result.is_err(), "{change}");
            assert_eq!(fixture.mutation_count().await, 0, "{change}");
            let row = fixture
                .db
                .collection::<Installation>(COLLECTION_NAME)
                .find_one(doc! {"application_id":"100"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                row.state,
                State::Prepared,
                "claim must roll back with live authority: {change}"
            );
            assert!(row.dispatch_deadline.is_none());
            fixture.db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn superseded_observed_installation_cannot_activate_from_a_retained_result() {
        let fixture = Fixture::new().await;
        let InstallationOutcome::Installed(installed) = fixture.run(None).await.unwrap() else {
            panic!("installed")
        };
        let mut session = fixture.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        installed
            .fence_in_session(&fixture.db, &mut session)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        fixture.db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"application_id":"100"},doc! {"$set":{"attempt_id":uuid::Uuid::new_v4().to_string(),"credential_fingerprint":"d4".repeat(32)}}).await.unwrap();
        let mut session = fixture.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            installed
                .fence_in_session(&fixture.db, &mut session)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        assert_eq!(fixture.mutation_count().await, 1);
        fixture.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn exact_captured_admin_authority_installs_and_reuses_only_the_owned_command() {
        let fixture = Fixture::new().await;
        let (state, authority) = fixture.captured_authority().await;
        let http = reqwest::Client::new();
        let base = fixture.server.uri();
        let first = install_captured_at(
            &fixture.db,
            &state.encryption_keys,
            &http,
            &authority,
            None,
            None,
            &base,
        )
        .await
        .unwrap();
        let InstallationOutcome::Installed(first) = first else {
            panic!("installed")
        };
        assert_eq!(first.command_id, "300");
        let retry = install_captured_at(
            &fixture.db,
            &state.encryption_keys,
            &http,
            &authority,
            Some(&first.command_id),
            None,
            &base,
        )
        .await
        .unwrap();
        assert!(matches!(retry, InstallationOutcome::Installed(_)));
        assert_eq!(fixture.mutation_count().await, 1);
        fixture.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn command_ownership_survives_later_activation_failure_without_new_upsert() {
        let fixture = Fixture::new().await;
        let InstallationOutcome::Installed(first) = fixture.run(None).await.unwrap() else {
            panic!("installed")
        };
        // No activation is committed. Its retry reuses durable command ownership.
        let InstallationOutcome::Installed(retry) = fixture.run(None).await.unwrap() else {
            panic!("installed")
        };
        assert_eq!(first.command_id, retry.command_id);
        assert_eq!(fixture.mutation_count().await, 1);
        let row = fixture
            .db
            .collection::<Installation>(COLLECTION_NAME)
            .find_one(doc! {"application_id":"100"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, State::Observed);
        assert_eq!(row.observed_command_id.as_deref(), Some("300"));
        fixture.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn lost_create_result_requires_exact_explicit_recovery_and_never_blindly_retries() {
        let fixture = Fixture::new().await;
        assert!(
            LOSE_PROVIDER_RESULT
                .scope(true, fixture.run(None))
                .await
                .is_err()
        );
        assert_eq!(fixture.mutation_count().await, 1);
        let InstallationOutcome::RecoveryRequired(review) = fixture.run(None).await.unwrap() else {
            panic!("explicit recovery required")
        };
        let mut substituted = review.clone();
        substituted.command_id = Some("999".into());
        assert!(fixture.run(Some(&substituted)).await.is_err());
        let mut substituted = review.clone();
        substituted.attempt_id = uuid::Uuid::new_v4().to_string();
        assert!(fixture.run(Some(&substituted)).await.is_err());
        assert_eq!(fixture.mutation_count().await, 1);
        assert!(fixture.run(Some(&review)).await.is_err());
        fixture.expire_dispatch().await;
        let InstallationOutcome::Installed(recovered) = fixture.run(Some(&review)).await.unwrap()
        else {
            panic!("recovered")
        };
        assert_eq!(recovered.command_id, "300");
        assert!(matches!(
            fixture.run(None).await.unwrap(),
            InstallationOutcome::Installed(_)
        ));
        assert_eq!(fixture.mutation_count().await, 1);
        fixture.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn preexisting_unowned_command_cannot_become_a_recovery_attempt() {
        let fixture = Fixture::new().await;
        // First caller has a genuine attempt; a different channel with the same
        // provider application cannot adopt its command through initial setup.
        fixture.run(None).await.unwrap();
        let other = uuid::Uuid::new_v4().to_string();
        let mut request = fixture.request();
        request.channel_bot_id = &other;
        let InstallationOutcome::RecoveryRequired(review) = install_at(
            &fixture.db,
            &reqwest::Client::new(),
            request,
            None,
            &fixture.server.uri(),
        )
        .await
        .unwrap() else {
            panic!("explicit transfer review required")
        };
        let mut request = fixture.request();
        request.channel_bot_id = &other;
        assert!(
            install_at(
                &fixture.db,
                &reqwest::Client::new(),
                request,
                Some(&review),
                &fixture.server.uri()
            )
            .await
            .is_err()
        );
        assert_eq!(
            fixture
                .db
                .collection::<Installation>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(fixture.mutation_count().await, 1);
        fixture.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn activated_command_and_explicit_credential_reconfiguration_reuse_owned_identity() {
        let mut fixture = Fixture::new().await;
        fixture.run(None).await.unwrap();
        let mut request = fixture.request();
        request.previously_owned_command = Some("300");
        assert!(matches!(
            install_at(
                &fixture.db,
                &reqwest::Client::new(),
                request,
                None,
                &fixture.server.uri()
            )
            .await
            .unwrap(),
            InstallationOutcome::Installed(_)
        ));
        fixture.fingerprint = "c3".repeat(32);
        fixture.admin = uuid::Uuid::new_v4().to_string();
        let InstallationOutcome::RecoveryRequired(review) = fixture.run(None).await.unwrap() else {
            panic!("reconfiguration review")
        };
        assert_eq!(review.command_id.as_deref(), Some("300"));
        assert!(matches!(
            fixture.run(Some(&review)).await.unwrap(),
            InstallationOutcome::Installed(_)
        ));
        let row = fixture
            .db
            .collection::<Installation>(COLLECTION_NAME)
            .find_one(doc! {"application_id":"100"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.requested_by, fixture.admin);
        assert_eq!(row.credential_fingerprint, fixture.fingerprint);
        assert_eq!(row.owned_command_id.as_deref(), Some("300"));
        assert_eq!(
            fixture
                .db
                .collection::<Document>(HISTORY)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            fixture.mutation_count().await,
            1,
            "reconfiguration of exact command needs no provider mutation"
        );
        fixture.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn uncertain_absent_create_requires_stable_absence_and_an_explicit_new_attempt() {
        let fixture = Fixture::new().await;
        fixture.failed_creates.store(1, Ordering::SeqCst);
        assert!(fixture.run(None).await.is_err());
        let InstallationOutcome::RecoveryRequired(review) = fixture.run(None).await.unwrap() else {
            panic!("absence review")
        };
        assert!(review.command_id.is_none());
        assert!(
            fixture.run(Some(&review)).await.is_err(),
            "uncertain create cannot immediately retry"
        );
        assert_eq!(fixture.mutation_count().await, 1);
        fixture.expire_dispatch().await;
        let result = fixture.run(Some(&review)).await;
        if let Err(error) = &result {
            let row = fixture
                .db
                .collection::<Installation>(COLLECTION_NAME)
                .find_one(doc! {"application_id":"100"})
                .await
                .unwrap()
                .unwrap();
            let history = fixture
                .db
                .collection::<Document>(HISTORY)
                .count_documents(doc! {})
                .await
                .unwrap();
            let requests = fixture
                .server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .map(|request| (request.method.to_string(), request.url.path().to_owned()))
                .collect::<Vec<_>>();
            panic!(
                "reviewed retry failed: {error:?}; state={:?}; same_attempt={}; history={history}; requests={requests:?}",
                row.state,
                row.attempt_id == review.attempt_id
            );
        }
        assert!(matches!(result.unwrap(), InstallationOutcome::Installed(_)));
        let installed = fixture
            .db
            .collection::<Installation>(COLLECTION_NAME)
            .find_one(doc! {"application_id":"100"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(installed.state, State::Observed);
        assert_eq!(installed.observed_command_id.as_deref(), Some("300"));
        assert_ne!(installed.attempt_id, review.attempt_id);
        assert_eq!(
            fixture.mutation_count().await,
            2,
            "one explicitly reviewed new creation"
        );
        assert_eq!(
            fixture
                .db
                .collection::<Document>(HISTORY)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        fixture.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn retired_inflight_channel_keeps_application_claim_until_explicit_replacement_review() {
        let fixture = Fixture::new().await;
        assert!(
            LOSE_PROVIDER_RESULT
                .scope(true, fixture.run(None))
                .await
                .is_err()
        );
        let new_channel = uuid::Uuid::new_v4().to_string();
        let mut request = fixture.request();
        request.channel_bot_id = &new_channel;
        let InstallationOutcome::RecoveryRequired(review) = install_at(
            &fixture.db,
            &reqwest::Client::new(),
            request,
            None,
            &fixture.server.uri(),
        )
        .await
        .unwrap() else {
            panic!("replacement review")
        };
        assert_eq!(
            fixture
                .db
                .collection::<Installation>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(fixture.mutation_count().await, 1);
        fixture
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(doc! {"_id":&fixture.bot_id,"is_active":false})
            .await
            .unwrap();
        fixture.expire_dispatch().await;
        let run = || async {
            let mut request = fixture.request();
            request.channel_bot_id = &new_channel;
            install_at(
                &fixture.db,
                &reqwest::Client::new(),
                request,
                Some(&review),
                &fixture.server.uri(),
            )
            .await
        };
        let (first, second) = tokio::join!(run(), run());
        assert!(first.is_ok() || second.is_ok());
        assert_eq!(
            fixture.mutation_count().await,
            1,
            "replacement never upserts an in-flight command"
        );
        let row = fixture
            .db
            .collection::<Installation>(COLLECTION_NAME)
            .find_one(doc! {"application_id":"100"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.channel_bot_id, new_channel);
        assert_eq!(row.observed_command_id.as_deref(), Some("300"));
        assert_eq!(
            fixture
                .db
                .collection::<Document>(HISTORY)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        fixture.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn delayed_expired_dispatch_cannot_mutate_after_explicit_absence_replacement() {
        let fixture = Arc::new(Fixture::new().await);
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let old_fixture = fixture.clone();
        let old_barrier = barrier.clone();
        let delayed = tokio::spawn(async move {
            AFTER_DISPATCH
                .scope(
                    old_barrier,
                    DISPATCH_DURATION
                        .scope(chrono::Duration::milliseconds(1), old_fixture.run(None)),
                )
                .await
        });
        barrier.wait().await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        fixture.expire_dispatch().await;
        let InstallationOutcome::RecoveryRequired(review) = fixture.run(None).await.unwrap() else {
            panic!("explicit absence review")
        };
        assert!(review.command_id.is_none());
        assert!(matches!(
            fixture.run(Some(&review)).await.unwrap(),
            InstallationOutcome::Installed(_)
        ));
        barrier.wait().await;
        assert!(delayed.await.unwrap().is_err());
        assert_eq!(
            fixture.mutation_count().await,
            1,
            "expired old worker never reaches provider mutation"
        );
        let row = fixture
            .db
            .collection::<Installation>(COLLECTION_NAME)
            .find_one(doc! {"application_id":"100"})
            .await
            .unwrap()
            .unwrap();
        assert!(uuid::Uuid::parse_str(&row.id).is_ok_and(|id| id.get_version_num() == 4));
        assert_eq!(row.state, State::Observed);
        assert_eq!(row.observed_command_id.as_deref(), Some("300"));
        fixture.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn visible_owned_command_cannot_replace_an_authorized_paused_patch() {
        let fixture = Arc::new(Fixture::new().await);
        fixture.run(None).await.unwrap();
        let mut stale = registered();
        stale["description"] = serde_json::json!("Previous reviewed command");
        let _stale_commands = Mock::given(method("GET"))
            .and(path("/applications/100/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![stale]))
            .with_priority(1)
            .mount_as_scoped(&fixture.server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/applications/100/commands/300"))
            .respond_with(ResponseTemplate::new(200).set_body_json(registered()))
            .expect(1)
            .mount(&fixture.server)
            .await;
        let InstallationOutcome::RecoveryRequired(review) = fixture.run(None).await.unwrap() else {
            panic!("owned command reconfiguration review")
        };
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let old_fixture = fixture.clone();
        let old_barrier = barrier.clone();
        let delayed = tokio::spawn(async move {
            AFTER_DISPATCH
                .scope(old_barrier, old_fixture.run(Some(&review)))
                .await
        });
        barrier.wait().await;
        let row = fixture
            .db
            .collection::<Installation>(COLLECTION_NAME)
            .find_one(doc! {"application_id":"100"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, State::Dispatched);
        assert!(row.dispatch_deadline.unwrap() > Utc::now());
        let InstallationOutcome::RecoveryRequired(review) = fixture.run(None).await.unwrap() else {
            panic!("fresh stale-command observation")
        };
        assert_eq!(review.command_id.as_deref(), Some("300"));
        assert!(
            fixture.run(Some(&review)).await.is_err(),
            "visible command cannot supersede active PATCH"
        );
        assert_eq!(fixture.mutation_count().await, 1);
        fixture.db.collection::<Installation>(COLLECTION_NAME).update_one(
            doc! {"application_id":"100"},
            doc! {"$set":{"dispatch_deadline":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}
        ).await.unwrap();
        assert!(
            fixture.run(Some(&review)).await.is_err(),
            "uncertainty delay also covers PATCH"
        );
        fixture.expire_dispatch().await;
        let requests_before = fixture.server.received_requests().await.unwrap().len();
        assert!(matches!(
            fixture.run(Some(&review)).await.unwrap(),
            InstallationOutcome::Installed(_)
        ));
        let requests = fixture.server.received_requests().await.unwrap();
        let recovery_requests = &requests[requests_before..];
        assert_eq!(
            recovery_requests
                .iter()
                .map(|request| (request.method.as_str(), request.url.path()))
                .collect::<Vec<_>>(),
            [
                ("GET", "/oauth2/applications/@me"),
                ("GET", "/applications/100/commands"),
                ("PATCH", "/applications/100/commands/300")
            ]
        );
        barrier.wait().await;
        assert!(delayed.await.unwrap().is_err());
        assert_eq!(
            fixture.mutation_count().await,
            2,
            "only the newly reviewed PATCH reaches the provider"
        );
        fixture.server.verify().await;
        fixture.db.drop().await.unwrap();
    }
}
