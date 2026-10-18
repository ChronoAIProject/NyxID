//! First-party administrative conversion owns native provider observations and
//! joins them with permanent routing separation in the conversion transaction.
use super::{
    platform_entry_credentials::EntryCredentialSnapshot,
    platform_entry_discord_installation::{
        self as discord, InstallationAuthority, InstalledControl,
    },
    platform_entry_identity::{opaque, refused, validate_configuration},
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_bot::{COLLECTION_NAME as BOTS, ChannelBot},
        platform_bot_config::PlatformBotConfig,
        platform_entry_identity::{
            EntryPlatform, EntryTransport, NativeControlSurface, NativeEntryConfiguration,
            NativeEntryIdentity,
        },
    },
};
use bson::{Document, doc};
use mongodb::{ClientSession, Database};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeTransportChoice {
    Direct,
    Gateway,
}

pub(crate) enum NativeConversionOutcome {
    Converted(Box<PlatformBotConfig>),
    DiscordRecovery(discord::RecoveryReview),
    GatewayRecovery(super::platform_entry_gateway_service::GatewayRecoveryReview),
}

/// Neither this authority nor provider observations can be deserialized from an
/// HTTP request. Pending setup remains a permanent ordinary-routing refusal.
pub(crate) struct NativeConversionAuthority {
    pub(crate) profile_id: String,
    actor: String,
    funding: String,
    bot: ChannelBot,
    bot_filter: Document,
    credentials: EntryCredentialSnapshot,
    configuration: NativeEntryConfiguration,
    discord: Option<(InstallationAuthority, InstalledControl)>,
    gateway: Option<super::platform_entry_gateway_service::PreparedGateway>,
    previous_profile: Option<PlatformBotConfig>,
}
impl NativeConversionAuthority {
    pub(crate) fn configuration(&self) -> &NativeEntryConfiguration {
        &self.configuration
    }
    pub(crate) fn bot(&self) -> &ChannelBot {
        &self.bot
    }
    pub(crate) fn actor(&self) -> &str {
        &self.actor
    }
    pub(crate) fn previous_profile(&self) -> Option<&PlatformBotConfig> {
        self.previous_profile.as_ref()
    }
    pub(crate) fn funding(&self) -> &str {
        &self.funding
    }
    pub(crate) async fn fence_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
    ) -> AppResult<()> {
        super::platform_bot_config_service::fence_admin(db, &self.actor, session).await?;
        super::platform_bot_config_service::fence_funding_admin(
            db,
            &self.actor,
            &self.funding,
            session,
        )
        .await?;

        if !super::platform_entry_migration::native_writes_ready(db).await?
            || !self.credentials.fence_in_session(db, session).await?
        {
            return Err(refused());
        }
        validate_configuration(&self.configuration)?;
        if let Some(profile) = &self.previous_profile
            && db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(
                doc! {"_id":&profile.id,"generation":profile.generation,"native_entry":bson::to_bson(&profile.native_entry).map_err(|_|refused())?,"enabled":false,"deleted_at":bson::Bson::Null},
                doc! {"$inc":{"native_admin_effect_revision":1_i64}}).session(&mut *session).await?.matched_count!=1
        {
            return Err(refused());
        }
        if db
            .collection::<Document>(BOTS)
            .update_one(
                self.bot_filter.clone(),
                doc! {"$inc":{"platform_entry_admin_revision":1_i64}},
            )
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(refused());
        }
        if let Some((authority, installed)) = &self.discord {
            authority.fence_in_session(db, session).await?;
            installed.fence_in_session(db, session).await?;
            let NativeControlSurface::DiscordCommand {
                application_id,
                command_id,
                command_digest,
            } = &self.configuration.control_surface
            else {
                return Err(refused());
            };
            if application_id != &installed.application_id
                || command_id != &installed.command_id
                || command_digest != &installed.command_digest
            {
                return Err(refused());
            }
        }
        if let Some(gateway) = &self.gateway {
            gateway
                .activate_in_session(db, session, &self.configuration)
                .await?;
        } else if self.configuration.transport != EntryTransport::Direct {
            return Err(refused());
        }
        Ok(())
    }
}

fn platform(value: &str) -> AppResult<EntryPlatform> {
    Ok(match value {
        "slack" => EntryPlatform::Slack,
        "discord" => EntryPlatform::Discord,
        "lark" => EntryPlatform::Lark,
        "feishu" => EntryPlatform::Feishu,
        "whatsapp" => EntryPlatform::Whatsapp,
        "x" => EntryPlatform::X,
        _ => {
            return Err(AppError::ValidationError(
                "Select a supported native chat platform".into(),
            ));
        }
    })
}

async fn pending(
    state: &AppState,
    actor: &str,
    bot: &str,
    funding: &str,
    transport: NativeTransportChoice,
) -> AppResult<String> {
    let profile_id = uuid::Uuid::new_v4().to_string();
    let transport = bson::to_bson(&transport).map_err(|_| refused())?;
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((state,actor,bot,funding,&profile_id,&transport), |session,(state,actor,bot,funding,profile_id,transport)|Box::pin(async move {
        super::api_key_mutation_service::transaction_result(async {
            super::platform_bot_config_service::fence_admin(&state.db, actor, session).await?;
            if !super::platform_entry_migration::native_writes_ready(&state.db).await? {return Err(refused());}
            super::platform_bot_config_service::fence_funding_admin(&state.db,actor,funding,session).await?;
            let row=state.db.collection::<Document>(BOTS).find_one(doc! {"_id":*bot,"is_active":true,"status":"active","$or":[{"platform_profile_id":{"$exists":false}},{"platform_entry_version":2,"platform_entry_pending":{"$exists":true}}]})
                .session(&mut *session).await?.ok_or_else(refused)?;
            let selected = platform(row.get_str("platform").map_err(|_|refused())?)?;
            if **transport == bson::to_bson(&NativeTransportChoice::Gateway).map_err(|_|refused())? {
                super::platform_entry_gateway_service::require_supported_platform(selected)?;
            }
            if let Some(old)=row.get("platform_entry_pending") {
                let old=old.as_document().ok_or_else(refused)?;
                if old.get_str("actor").ok()!=Some(*actor) || old.get_str("funding").ok()!=Some(*funding) || old.get("transport")!=Some(*transport) {
                    return Err(AppError::Conflict("A different native conversion is pending; review its retained attempt".into()));
                }
                return old.get_str("profile_id").map(str::to_owned).map_err(|_|refused());
            }
            // Refuse deterministic blockers while ordinary owner cancellation is
            // still available. Keep the same evaluator in final conversion.
            super::platform_bot_config_service::conversion_prerequisites_in_session(&state.db,session,bot).await?;
            // Serialize against all existing ordinary route/connection mutations.
            let marker=doc! {"profile_id":*profile_id,"actor":*actor,"funding":*funding,"transport":(*transport).clone()};
            let claimed=state.db.collection::<Document>(BOTS).update_one(doc! {"_id":*bot,"platform_profile_id":{"$exists":false},"platform_entry_pending":{"$exists":false},
                "route_revision":row.get("route_revision").cloned().unwrap_or_else(||bson::Bson::Document(doc! {"$exists":false}))},
                doc! {"$set":{"platform_entry_pending":marker},"$inc":{"route_revision":1_i64}}).session(&mut *session).await?;
            if claimed.matched_count!=1 {return Err(refused());}
            Ok((*profile_id).clone())
        }.await)
    })).await.map_err(super::api_key_mutation_service::map_transaction_error)
}

pub(super) async fn provider_json(request: reqwest::RequestBuilder) -> AppResult<Value> {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let response = request.send().await.map_err(|_| refused())?;
        if !response.status().is_success() {
            return Err(refused());
        }
        if response.content_length().is_some_and(|n| n > 64 * 1024) {
            return Err(refused());
        }
        let mut response = response;
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| refused())? {
            if bytes.len().saturating_add(chunk.len()) > 64 * 1024 {
                return Err(refused());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| refused())
    })
    .await
    .map_err(|_| refused())?
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static PROVIDER_BASE: String;
    static BEFORE_PROVIDER_VERIFY: std::sync::Arc<tokio::sync::Barrier>;
}
fn provider_base(default: &str) -> String {
    #[cfg(test)]
    if let Ok(base) = PROVIDER_BASE.try_with(Clone::clone) {
        return base;
    }
    default.into()
}

pub(crate) async fn convert(
    state: &AppState,
    actor: &str,
    bot_id: &str,
    funding: &str,
    transport: NativeTransportChoice,
    recovery: Option<&discord::RecoveryReview>,
    gateway_recovery: Option<&super::platform_entry_gateway_service::GatewayRecoveryReview>,
) -> AppResult<NativeConversionOutcome> {
    // Capability checks precede the sticky marker, refresh and paid verification.
    let candidate = state
        .db
        .collection::<Document>(BOTS)
        .find_one(doc! {"_id":bot_id})
        .await?
        .ok_or_else(refused)?;
    let selected = platform(candidate.get_str("platform").map_err(|_| refused())?)?;
    if transport == NativeTransportChoice::Gateway {
        super::platform_entry_gateway_service::preflight(state, selected).await?;
    }
    let profile_id = pending(state, actor, bot_id, funding, transport).await?;
    let previous_profile = state
        .db
        .collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
        .find_one(doc! {"_id":&profile_id,"channel_bot_id":bot_id,"deleted_at":bson::Bson::Null})
        .await?;
    let raw = state
        .db
        .collection::<Document>(BOTS)
        .find_one(doc! {"_id":bot_id})
        .await?
        .ok_or_else(refused)?;
    let marker = raw
        .get_document("platform_entry_pending")
        .map_err(|_| refused())?;
    if marker.get_str("profile_id").ok() != Some(profile_id.as_str())
        || marker.get_str("actor").ok() != Some(actor)
        || marker.get_str("funding").ok() != Some(funding)
        || marker.get("transport") != Some(&bson::to_bson(&transport).map_err(|_| refused())?)
    {
        return Err(refused());
    }
    if !uuid::Uuid::parse_str(&profile_id).is_ok_and(|id| id.get_version_num() == 4) {
        return Err(refused());
    }
    let bot: ChannelBot = bson::from_document(raw.clone()).map_err(|_| refused())?;
    let selected = platform(&bot.platform)?;
    let adapter = super::platform_native_reply_service::send_adapter(state, selected)?;
    if bot.credential_source == "connection" {
        // Actual refresh before capturing stable epoch/scope identity and material.
        super::channel_credentials::resolve_bot_token(
            &state.db,
            &state.encryption_keys,
            adapter.as_ref(),
            &bot,
        )
        .await?;
    }
    let credentials = EntryCredentialSnapshot::load(&state.db, adapter.as_ref(), &bot).await?;
    let fingerprint = credentials.fingerprint()?;
    let token = credentials.send_token(&state.encryption_keys).await?;
    let secrets = credentials
        .secrets(&state.encryption_keys, adapter.as_ref())
        .await?;
    // Paid verification starts only after live actor, target and credentials
    // have been joined in one admission transaction.
    #[cfg(test)]
    if let Ok(barrier) = BEFORE_PROVIDER_VERIFY.try_with(std::sync::Arc::clone) {
        for _ in 0..2 {
            tokio::time::timeout(std::time::Duration::from_secs(20), barrier.wait())
                .await
                .expect("native provider verification barrier");
        }
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((state,actor,funding,&raw,&credentials),|session,(state,actor,funding,raw,credentials)|Box::pin(async move {
        super::api_key_mutation_service::transaction_result(async {
            super::platform_bot_config_service::fence_admin(&state.db,actor,session).await?;
            super::platform_bot_config_service::fence_funding_admin(&state.db,actor,funding,session).await?;
            if !credentials.fence_in_session(&state.db,session).await? {return Err(refused());}
            let filter=doc! {"_id":raw.get_str("_id").map_err(|_|refused())?,"platform_entry_pending":raw.get("platform_entry_pending").cloned().ok_or_else(refused)?,
                "route_revision":raw.get("route_revision").cloned().ok_or_else(refused)?,"platform_profile_id":raw.get("platform_profile_id").cloned().unwrap_or_else(||bson::Bson::Document(doc! {"$exists":false})),"is_active":true,"status":"active"};
            if state.db.collection::<Document>(BOTS).update_one(filter,doc! {"$inc":{"native_verification_revision":1_i64}}).session(&mut *session).await?.matched_count!=1{return Err(refused());}
            Ok(())
        }.await)
    })).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
    let billing = super::channel_billing_service::ChannelBilling::for_bot(
        &state.db,
        &state.billing,
        &bot,
        None,
    );
    let mut installed = None;
    let surface = match selected {
        EntryPlatform::Discord => {
            let authority = InstallationAuthority::capture(&state.db, actor, bot_id).await?;
            let outcome = discord::install_captured_at(
                &state.db,
                &state.encryption_keys,
                &state.http_client,
                &authority,
                None,
                recovery,
                &provider_base(super::platform_entry_discord_controls::DISCORD_BASE),
            )
            .await?;
            let command = match outcome {
                discord::InstallationOutcome::Installed(command) => command,
                discord::InstallationOutcome::RecoveryRequired(review) => {
                    return Ok(NativeConversionOutcome::DiscordRecovery(review));
                }
            };
            let surface = NativeControlSurface::DiscordCommand {
                application_id: command.application_id.clone(),
                command_id: command.command_id.clone(),
                command_digest: command.command_digest.clone(),
            };
            installed = Some((authority, command));
            surface
        }
        EntryPlatform::Slack => {
            let base = provider_base("https://slack.com/api");
            let auth = provider_json(
                state
                    .http_client
                    .post(format!("{base}/auth.test"))
                    .bearer_auth(token.as_str()),
            )
            .await?;
            if auth["ok"] != true || auth["bot_id"].as_str() != Some(&bot.platform_bot_id) {
                return Err(refused());
            }
            let bot_user_id = opaque(auth["user_id"].as_str().ok_or_else(refused)?)?.to_owned();
            let team_id = opaque(auth["team_id"].as_str().ok_or_else(refused)?)?.to_owned();
            let info = provider_json(
                state
                    .http_client
                    .get(format!("{base}/bots.info"))
                    .bearer_auth(token.as_str())
                    .query(&[("bot", &bot.platform_bot_id)]),
            )
            .await?;
            if info["ok"] != true || info["bot"]["id"].as_str() != Some(&bot.platform_bot_id) {
                return Err(refused());
            }
            let app_id = opaque(info["bot"]["app_id"].as_str().ok_or_else(refused)?)?.to_owned();
            NativeControlSurface::SlackDirectMessage {
                app_id,
                team_id,
                bot_user_id,
            }
        }
        _ => {
            let observed = adapter
                .verify_bot_token(
                    &state.http_client,
                    &super::channel_platform::BotCredentials {
                        billing: billing.as_ref(),
                        token: &token,
                        platform_bot_id: Some(&bot.platform_bot_id),
                        platform_secrets: Some(&secrets),
                    },
                )
                .await?;
            if observed.platform_bot_id != bot.platform_bot_id {
                return Err(refused());
            }
            NativeControlSurface::PrivateMessage
        }
    };
    let mut filter = doc! {"_id":&bot.id,"user_id":&bot.user_id,"platform":&bot.platform,"platform_bot_id":&bot.platform_bot_id,"is_active":true,"status":"active",
    "platform_profile_id":raw.get("platform_profile_id").cloned().unwrap_or_else(||bson::Bson::Document(doc! {"$exists":false}))};
    for field in ["platform_entry_pending", "route_revision"] {
        filter.insert(field, raw.get(field).cloned().ok_or_else(refused)?);
    }
    let configuration = NativeEntryConfiguration {
        identity: NativeEntryIdentity {
            version: 2,
            platform: selected,
            channel_bot_id: bot.id.clone(),
            platform_bot_id: bot.platform_bot_id.clone(),
        },
        transport: EntryTransport::Direct,
        transport_generation: previous_profile
            .as_ref()
            .and_then(|profile| profile.native_entry.as_ref())
            .map_or(Ok(1), |configuration| {
                configuration
                    .transport_generation
                    .checked_add(1)
                    .ok_or_else(refused)
            })?,
        credential_fingerprint: fingerprint,
        control_surface: surface,
    };
    validate_configuration(&configuration)?;
    let mut authority = NativeConversionAuthority {
        profile_id,
        actor: actor.into(),
        funding: funding.into(),
        bot,
        bot_filter: filter,
        credentials,
        configuration,
        discord: installed,
        gateway: None,
        previous_profile,
    };
    if transport == NativeTransportChoice::Gateway {
        match super::platform_entry_gateway_service::prepare(state, &authority, gateway_recovery)
            .await?
        {
            super::platform_entry_gateway_service::GatewayPreparation::Ready(prepared) => {
                authority.configuration = prepared.configuration().clone();
                authority.gateway = Some(*prepared);
            }
            super::platform_entry_gateway_service::GatewayPreparation::Recovery(review) => {
                return Ok(NativeConversionOutcome::GatewayRecovery(review));
            }
        }
    }
    let profile = super::platform_bot_config_service::convert_native(state, &authority).await?;
    Ok(NativeConversionOutcome::Converted(Box::new(profile)))
}

/// Inspection issues no provider mutation, token refresh or paid verification.
/// It returns only an exact review of the retained setup under current authority.
pub(crate) async fn inspect_gateway(
    state: &AppState,
    actor: &str,
    bot_id: &str,
) -> AppResult<super::platform_entry_gateway_service::GatewayRecoveryReview> {
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    super::platform_bot_config_service::fence_admin(&state.db, actor, &mut session).await?;
    session.commit_transaction().await?;
    let raw=state.db.collection::<Document>(BOTS).find_one(doc! {"_id":bot_id,"platform_profile_id":{"$exists":false},"is_active":true,"status":"active"}).await?.ok_or_else(refused)?;
    let marker = raw
        .get_document("platform_entry_pending")
        .map_err(|_| refused())?;
    if marker.get_str("actor").ok() != Some(actor)
        || marker.get_str("transport").ok() != Some("gateway")
    {
        return Err(refused());
    }
    let profile_id = marker
        .get_str("profile_id")
        .map_err(|_| refused())?
        .to_owned();
    let funding = marker.get_str("funding").map_err(|_| refused())?.to_owned();
    let bot: ChannelBot = bson::from_document(raw.clone()).map_err(|_| refused())?;
    let selected = platform(&bot.platform)?;
    super::platform_entry_gateway_service::preflight(state, selected).await?;
    let retained = state
        .db
        .collection::<crate::models::platform_entry_gateway::PlatformEntryGateway>(
            crate::models::platform_entry_gateway::COLLECTION_NAME,
        )
        .find_one(doc! {"profile_id":&profile_id,"state":"prepared"})
        .await?
        .ok_or_else(refused)?;
    let adapter = super::platform_native_reply_service::send_adapter(state, selected)?;
    let credentials = EntryCredentialSnapshot::load(&state.db, adapter.as_ref(), &bot).await?;
    if credentials.fingerprint()? != retained.configuration.credential_fingerprint {
        return Err(refused());
    }
    let mut configuration = retained.configuration;
    configuration.transport = EntryTransport::Direct;
    let mut bot_filter = doc! {"_id":&bot.id,"user_id":&bot.user_id,"platform":&bot.platform,"platform_bot_id":&bot.platform_bot_id,
    "platform_entry_pending":raw.get("platform_entry_pending").cloned().ok_or_else(refused)?,
    "route_revision":raw.get("route_revision").cloned().ok_or_else(refused)?,"platform_profile_id":{"$exists":false},"is_active":true,"status":"active"};
    let authority = NativeConversionAuthority {
        profile_id,
        actor: actor.into(),
        funding,
        bot,
        bot_filter: std::mem::take(&mut bot_filter),
        credentials,
        configuration,
        discord: None,
        gateway: None,
        previous_profile: None,
    };
    super::platform_entry_gateway_service::inspect_pending(state, &authority).await
}

#[cfg(test)]
#[path = "platform_entry_admin_tests.rs"]
pub(crate) mod tests;

/// A review is a comparison with retained server state, never provider authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeLifecycleReview {
    pub channel_bot_id: String,
    pub profile_id: String,
    pub generation: Option<i64>,
    pub reviewed_by: String,
    pub snapshot_digest: String,
}

struct LifecycleSnapshot {
    bot: Document,
    profile: Option<PlatformBotConfig>,
    gateways: Vec<Document>,
    review: NativeLifecycleReview,
}

fn review_fields(row: &Document, fields: &[&str]) -> Document {
    fields
        .iter()
        .map(|name| {
            (
                (*name).to_owned(),
                row.get(*name).cloned().unwrap_or(bson::Bson::Null),
            )
        })
        .collect()
}

async fn lifecycle_snapshot(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    bot_id: &str,
) -> AppResult<LifecycleSnapshot> {
    use sha2::{Digest, Sha256};
    let bot = db
        .collection::<Document>(BOTS)
        .find_one(doc! {"_id":bot_id,"is_active":true})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    platform(bot.get_str("platform").map_err(|_| refused())?)?;
    let (profile_id, profile) = if let Ok(id) = bot.get_str("platform_profile_id") {
        let profile = db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
            .find_one(doc! {"_id":id,"channel_bot_id":bot_id,"identity_version":2,"deleted_at":bson::Bson::Null})
            .session(&mut *session).await?.ok_or_else(refused)?;
        if profile.native_entry.is_none() {
            return Err(refused());
        }
        (id.to_owned(), Some(profile))
    } else {
        (
            bot.get_document("platform_entry_pending")
                .map_err(|_| refused())?
                .get_str("profile_id")
                .map_err(|_| refused())?
                .to_owned(),
            None,
        )
    };
    let mut gateway_filter = doc! {"profile_id":&profile_id,"state":{"$in":["prepared","active"]}};
    if let Some(EntryTransport::NyxagentGateway { channel_id, .. }) = profile
        .as_ref()
        .and_then(|profile| profile.native_entry.as_ref())
        .map(|configuration| &configuration.transport)
    {
        // A retired selected transport still owns its dedicated credentials and
        // tombstone. Reviewed recovery must capture and revoke that exact row.
        gateway_filter.remove("state");
        gateway_filter.insert(
            "$or",
            vec![
                doc! {"state":{"$in":["prepared","active"]}},
                doc! {"_id":channel_id},
            ],
        );
    }
    let mut cursor = db
        .collection::<Document>(crate::models::platform_entry_gateway::COLLECTION_NAME)
        .find(gateway_filter)
        .sort(doc! {"_id":1})
        .session(&mut *session)
        .await?;
    let mut gateways = Vec::new();
    while let Some(row) = cursor.next(&mut *session).await.transpose()? {
        gateways.push(row);
    }
    let bot_projection = review_fields(
        &bot,
        &[
            "_id",
            "user_id",
            "platform",
            "platform_bot_id",
            "app_id",
            "credential_source",
            "connection_id",
            "bot_token_encrypted",
            "app_secret_encrypted",
            "webhook_secret_hash",
            "lark_verification_token_encrypted",
            "lark_encrypt_key_encrypted",
            "public_key",
            "is_active",
            "status",
            "platform_profile_id",
            "platform_entry_version",
            "platform_entry_pending",
            "route_revision",
        ],
    );
    let profile_projection = profile
        .as_ref()
        .map(|profile| {
            bson::to_document(profile).map(|raw| {
                review_fields(
                    &raw,
                    &[
                        "_id",
                        "channel_bot_id",
                        "manager_bot_id",
                        "identity_version",
                        "native_entry",
                        "observation_id",
                        "execution_profile",
                        "generation",
                        "enabled",
                        "funding_owner_user_id",
                        "configured_by",
                        "deleted_at",
                    ],
                )
            })
        })
        .transpose()
        .map_err(|_| refused())?;
    let mut transport_projection = Vec::new();
    for gateway in &gateways {
        let mut projected = review_fields(
            gateway,
            &[
                "_id",
                "profile_id",
                "configuration",
                "configured_by",
                "credential_owner_id",
                "route_api_key_id",
                "route_id",
                "provider_api_key_id",
                "callback_url",
                "binding_id",
                "gateway_base",
                "setup_state",
                "setup_attempt_id",
                "setup_dispatch_deadline",
                "request_hash",
                "creation_fingerprint",
                "state",
                "revision",
            ],
        );
        let mut keys = Vec::new();
        for field in ["provider_api_key_id", "route_api_key_id"] {
            let key = match gateway.get_str(field) {
                Ok(id) => {
                    db.collection::<Document>(crate::models::api_key::COLLECTION_NAME)
                        .find_one(doc! {"_id":id})
                        .session(&mut *session)
                        .await?
                }
                Err(_) => None,
            };
            keys.push(key.map(|key| {
                review_fields(
                    &key,
                    &[
                        "_id",
                        "user_id",
                        "key_hash",
                        "purpose",
                        "is_active",
                        "expires_at",
                        "scopes",
                        "assistant_scope",
                    ],
                )
            }));
        }
        projected.insert("keys", bson::to_bson(&keys).map_err(|_| refused())?);
        transport_projection.push(projected);
    }
    // OAuth access-token refresh and benign API-key touches are not credential
    // replacement. Review only the persistent account/provider/epoch authority.
    let connection = if let Ok(id) = bot.get_str("connection_id") {
        db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME)
            .find_one(doc! {"_id":id})
            .session(&mut *session)
            .await?
            .map(|row| {
                let mut projection = review_fields(
                    &row,
                    &[
                        "_id",
                        "user_id",
                        "credential_type",
                        "credential_source",
                        "credential_epoch",
                        "provider_config_id",
                        "status",
                        "token_scopes",
                        "user_oauth_client_id_encrypted",
                    ],
                );
                let scopes = row
                    .get_str("token_scopes")
                    .unwrap_or_default()
                    .split_whitespace()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                projection.insert("token_scopes", scopes);
                projection
            })
    } else {
        None
    };
    let bot_model: ChannelBot = bson::from_document(bot.clone()).map_err(|_| refused())?;
    let cache =
        std::sync::Arc::new(super::provider_token_exchange_service::TokenExchangeCache::new());
    let adapter = super::channel_adapters::resolve_adapter(&bot_model.platform, &cache)?;
    let mut signing = Vec::new();
    if let Some(descriptor) = adapter.platform_credentials() {
        signing.push(
            db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
                .find_one(doc! {"provider":descriptor.provider})
                .session(&mut *session)
                .await?
                .map(|row| review_fields(&row, &["_id", "provider", "secrets", "fields"])),
        );
    }
    if let super::channel_platform::CredentialResolution::OAuthConnection {
        provider_slug, ..
    } = adapter.credential_resolution()
    {
        signing.push(
            db.collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
                .find_one(doc! {"slug":provider_slug})
                .session(&mut *session)
                .await?
                .map(|row| {
                    review_fields(
                        &row,
                        &[
                            "_id",
                            "slug",
                            "is_active",
                            "provider_type",
                            "client_id_encrypted",
                            "client_secret_encrypted",
                        ],
                    )
                }),
        );
    }
    let projection = doc! {"bot":bot_projection,"profile":bson::to_bson(&profile_projection).map_err(|_|refused())?,"gateways":transport_projection,
    "connection":bson::to_bson(&connection).map_err(|_|refused())?,"signing":bson::to_bson(&signing).map_err(|_|refused())?};
    let digest = hex::encode(Sha256::digest(
        bson::to_vec(&projection).map_err(|_| refused())?,
    ));
    Ok(LifecycleSnapshot {
        review: NativeLifecycleReview {
            channel_bot_id: bot_id.into(),
            profile_id,
            generation: profile.as_ref().map(|p| p.generation),
            reviewed_by: actor.into(),
            snapshot_digest: digest,
        },
        bot,
        profile,
        gateways,
    })
}

/// A successor human administrator can inspect failed setup without decrypting
/// or refreshing the old credentials. Separation and old ownership remain live.
pub(crate) async fn review_lifecycle(
    state: &AppState,
    actor: &str,
    bot: &str,
) -> AppResult<NativeLifecycleReview> {
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run((state, actor, bot), |session, (state, actor, bot)| {
            Box::pin(async move {
                super::api_key_mutation_service::transaction_result(
                    async {
                        super::platform_bot_config_service::fence_admin(&state.db, actor, session)
                            .await?;
                        Ok(lifecycle_snapshot(&state.db, session, actor, bot)
                            .await?
                            .review)
                    }
                    .await,
                )
            })
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

/// Reserve a reviewed replacement before provider work. The bot's original
/// owner remains the transport payer and the only owner of a replacement OAuth
/// connection. An unsuccessful verification retains this reachable reservation.
pub(crate) async fn reserve_reviewed_update(
    state: &AppState,
    actor: &str,
    funding: &str,
    transport: NativeTransportChoice,
    review: &NativeLifecycleReview,
    connection_id: Option<&str>,
    public_key: Option<&str>,
) -> AppResult<()> {
    if review.reviewed_by != actor || review.snapshot_digest.len() != 64 {
        return Err(refused());
    }
    let raw = state
        .db
        .collection::<Document>(BOTS)
        .find_one(doc! {"_id":&review.channel_bot_id,"is_active":true})
        .await?
        .ok_or_else(refused)?;
    let mut bot: ChannelBot = bson::from_document(raw).map_err(|_| refused())?;
    let selected = platform(&bot.platform)?;
    if transport == NativeTransportChoice::Gateway {
        super::platform_entry_gateway_service::preflight(state, selected).await?;
    }
    if let Some(id) = connection_id {
        if bot.credential_source != "connection"
            || !uuid::Uuid::parse_str(id).is_ok_and(|id| id.get_version_num() == 4)
        {
            return Err(refused());
        }
        bot.connection_id = Some(id.into());
    }
    if let Some(key) = public_key {
        if selected != EntryPlatform::Discord || key.len() != 64 || hex::decode(key).is_err() {
            return Err(refused());
        }
        bot.public_key = Some(key.into());
    }
    bot.status = "active".into();
    let adapter = super::platform_native_reply_service::send_adapter(state, selected)?;
    // Do not require the old credential to work: validation and the transaction
    // fence apply to the explicitly selected successor under the existing owner.
    if bot.credential_source == "connection" {
        let before_refresh =
            EntryCredentialSnapshot::load(&state.db, adapter.as_ref(), &bot).await?;
        let mut session = state.db.client().start_session().await?;
        session.start_transaction().await?;
        super::platform_bot_config_service::fence_admin(&state.db, actor, &mut session).await?;
        super::platform_bot_config_service::fence_funding_admin(
            &state.db,
            actor,
            funding,
            &mut session,
        )
        .await?;
        let snapshot =
            lifecycle_snapshot(&state.db, &mut session, actor, &review.channel_bot_id).await?;
        if snapshot.review != *review
            || !before_refresh
                .fence_refresh_dependencies_in_session(&state.db, &mut session)
                .await?
        {
            return Err(refused());
        }
        if state
            .db
            .collection::<Document>(BOTS)
            .update_one(
                snapshot.bot,
                doc! {"$inc":{"native_refresh_admission_revision":1_i64}},
            )
            .session(&mut session)
            .await?
            .matched_count
            != 1
        {
            return Err(refused());
        }
        session.commit_transaction().await?;
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            super::channel_credentials::resolve_bot_token(
                &state.db,
                &state.encryption_keys,
                adapter.as_ref(),
                &bot,
            ),
        )
        .await
        .map_err(|_| refused())??;
    }
    let credentials = EntryCredentialSnapshot::load(&state.db, adapter.as_ref(), &bot).await?;
    let fingerprint = credentials.fingerprint()?;
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((state,actor,funding,transport,review,&bot,&credentials,&fingerprint),|session,(state,actor,funding,transport,review,bot,credentials,fingerprint)|Box::pin(async move {
        super::api_key_mutation_service::transaction_result(async {
            super::platform_bot_config_service::fence_admin(&state.db,actor,session).await?;
            super::platform_bot_config_service::fence_funding_admin(&state.db,actor,funding,session).await?;
            if !super::platform_entry_migration::native_writes_ready(&state.db).await? {return Err(refused());}
            let snapshot=lifecycle_snapshot(&state.db,session,actor,&review.channel_bot_id).await?;
            if snapshot.review!=**review {return Err(refused());}
            super::platform_bot_config_service::conversion_prerequisites_in_session(&state.db,session,&bot.id).await?;
            // No worker may be superseded during its bounded provider operation.
            for gateway in &snapshot.gateways {
                if gateway.get_str("setup_state")==Ok("dispatched") {
                    let deadline=gateway.get_datetime("setup_dispatch_deadline").map_err(|_|refused())?;
                    if deadline.to_chrono()+chrono::Duration::seconds(120)>chrono::Utc::now() {return Err(AppError::Conflict("Retained gateway setup may still be in flight; inspect again after its deadline".into()));}
                }
            }
            // CredentialSnapshot also binds the bot fields. Claim replacement
            // fields first, then fence its exact owner/provider/epoch snapshot.
            let mut values=doc! {"status":"active","connection_id":bot.connection_id.clone(),"public_key":bot.public_key.clone(),
                "platform_entry_pending":{"profile_id":&review.profile_id,"actor":*actor,"funding":*funding,
                    "transport":bson::to_bson(transport).map_err(|_|refused())?,"attempt_id":uuid::Uuid::new_v4().to_string(),"credential_fingerprint":*fingerprint}};
            if snapshot.profile.is_some() {values.insert("platform_entry_version",2);}
            if state.db.collection::<Document>(BOTS).update_one(snapshot.bot.clone(),doc! {"$set":values,"$inc":{"route_revision":1_i64}}).session(&mut *session).await?.matched_count!=1 {return Err(refused());}
            if !credentials.fence_in_session(&state.db,session).await? {return Err(refused());}
            if let Some(profile)=snapshot.profile {
                if state.db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(
                    doc! {"_id":&profile.id,"generation":profile.generation,"deleted_at":bson::Bson::Null},
                    doc! {"$set":{"enabled":false,"configured_by":*actor,"funding_owner_user_id":*funding,"updated_at":bson::DateTime::now()},"$inc":{"generation":1_i64}}).session(&mut *session).await?.matched_count!=1 {return Err(refused());}
                // Preserve the controls row and old origins for exact asynchronous
                // stop recovery; every old authority is invalid immediately.
                state.db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME).update_one(
                    doc! {"platform_config_id":&profile.id,"retired":false},doc! {"$set":{"personal_enabled":false,"public_enabled":false,"cleanup_pending":true,"updated_at":bson::DateTime::now()},
                    "$inc":{"manager_authority_epoch":1_i64,"personal_control_revision":1_i64,"public_policy_revision":1_i64}}).session(&mut *session).await?;
                for collection in [crate::models::platform_link_intent::COLLECTION_NAME,crate::models::platform_link_intent::CHALLENGES] {
                    state.db.collection::<Document>(collection).update_many(doc! {"platform_config_id":&profile.id,"state":{"$in":["pending","review","awaiting_chat_confirmation"]}},doc! {"$set":{"state":"cancelled"}}).session(&mut *session).await?;
                }
            }
            for gateway in snapshot.gateways {
                let key_ids=[gateway.get_str("provider_api_key_id").map_err(|_|refused())?,gateway.get_str("route_api_key_id").map_err(|_|refused())?];
                super::api_key_mutation_service::update_many(&state.db,doc! {"_id":{"$in":key_ids.to_vec()}},doc! {"$set":{"is_active":false}},Some(&mut *session)).await?;
                if state.db.collection::<Document>(crate::models::platform_entry_gateway::COLLECTION_NAME).update_one(gateway,doc! {"$set":{"state":"retired","updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64}}).session(&mut *session).await?.matched_count!=1 {return Err(refused());}
            }
            Ok(())
        }.await)
    })).await.map_err(super::api_key_mutation_service::map_transaction_error)
}
